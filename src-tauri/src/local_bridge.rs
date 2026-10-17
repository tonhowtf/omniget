use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, AtomicU16, Ordering};
use std::sync::Arc;

use axum::{
    extract::DefaultBodyLimit,
    extract::State,
    http::{HeaderMap, Method, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use tauri::{AppHandle, Emitter};
use tokio::net::TcpListener;
use tower_http::cors::{Any, CorsLayer};

use crate::extension_storage::{
    cookie_limit, write_extension_metadata, ExtensionCookie, ExtensionPayload,
};
use crate::storage::config;
use tauri::Manager;

const PORT_RANGE: std::ops::Range<u16> = 47720..47730;

const DEV_PORT_RANGE: std::ops::Range<u16> = 47760..47770;

fn port_range() -> std::ops::Range<u16> {
    if cfg!(debug_assertions) && std::env::var("OMNIGET_DEV_PROFILE").ok().as_deref() == Some("1") {
        DEV_PORT_RANGE
    } else {
        PORT_RANGE
    }
}

#[derive(Clone)]
pub struct BridgeState {
    pub app: AppHandle,
    pub token: Arc<String>,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    ok: bool,
    version: &'static str,
}

#[derive(Debug, Serialize)]
struct EnqueueResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnqueueRequest {
    #[serde(flatten)]
    payload: ExtensionPayload,
    #[serde(default)]
    protocol_version: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CookiesRequest {
    #[serde(default)]
    cookies: Vec<ExtensionCookie>,
    #[serde(default)]
    protocol_version: Option<u32>,
    #[serde(default)]
    source_url: Option<String>,
    #[serde(default)]
    page_title: Option<String>,
    #[serde(default)]
    alias: Option<String>,
}

const HOST_MAX_PROTOCOL_VERSION: u32 = 1;

const MCP_BODY_LIMIT: usize = 36 * 1024 * 1024;
const ENQUEUE_BODY_LIMIT: usize = 8 * 1024 * 1024;

pub fn generate_token() -> String {
    let mut buf = [0u8; 32];
    rand::Rng::fill_bytes(&mut rand::rng(), &mut buf);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

fn bind_with_reuse(port: u16) -> std::io::Result<TcpListener> {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let socket = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(128)?;
    let std_listener: std::net::TcpListener = socket.into();
    TcpListener::from_std(std_listener)
}

async fn bind_listener(preferred: u16) -> std::io::Result<(u16, TcpListener)> {
    let range = port_range();
    let keep_preferred = range != DEV_PORT_RANGE || range.contains(&preferred);
    if preferred != 0 && keep_preferred {
        if let Ok(listener) = bind_with_reuse(preferred) {
            return Ok((preferred, listener));
        }
    }

    for port in range {
        if let Ok(listener) = bind_with_reuse(port) {
            return Ok((port, listener));
        }
    }

    let listener = bind_with_reuse(0)?;
    let port = listener.local_addr()?.port();
    Ok((port, listener))
}

fn ensure_bridge_settings(app: &AppHandle, bound_port: u16) -> anyhow::Result<(String, u16)> {
    let settings = config::update_settings(app, |settings| {
        if settings.bridge.token.trim().is_empty() {
            settings.bridge.token = generate_token();
        }
        settings.bridge.port = bound_port;
    })?;

    Ok((settings.bridge.token, settings.bridge.port))
}

static MAIN_PORT: AtomicU16 = AtomicU16::new(0);
static CANVAS_PORT: AtomicU16 = AtomicU16::new(0);
static CANVAS_START: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn ensure_running_for_canvas(app: &AppHandle) -> Result<u16, String> {
    let main = MAIN_PORT.load(Ordering::Relaxed);
    if main != 0 {
        return Ok(main);
    }
    let settings = config::load_settings(app);
    if settings.bridge.enabled && settings.bridge.port != 0 {
        return Ok(settings.bridge.port);
    }
    let _g = CANVAS_START.lock().unwrap_or_else(|e| e.into_inner());
    let port = CANVAS_PORT.load(Ordering::Relaxed);
    if port != 0 {
        return Ok(port);
    }
    let bind = |p: u16| std::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], p)));
    let std_listener = match settings.bridge.canvas_port {
        0 => bind(0),
        saved => bind(saved).or_else(|_| bind(0)),
    }
    .map_err(|e| format!("CANVAS_IO: {e}"))?;
    std_listener
        .set_nonblocking(true)
        .map_err(|e| format!("CANVAS_IO: {e}"))?;
    let port = std_listener
        .local_addr()
        .map_err(|e| format!("CANVAS_IO: {e}"))?
        .port();
    let token = if settings.bridge.token.trim().is_empty() {
        generate_token()
    } else {
        settings.bridge.token.clone()
    };
    let state = BridgeState {
        app: app.clone(),
        token: Arc::new(token),
    };
    let router = Router::new()
        .route("/v1/health", get(health))
        .merge(crate::local_bridge_canvas::router(
            state.app.clone(),
            crate::local_bridge_jobs::JobsBridgeState::from_bridge(&state).guard(),
        ))
        .with_state(state);
    tauri::async_runtime::spawn(async move {
        let listener = match TcpListener::from_std(std_listener) {
            Ok(l) => l,
            Err(error) => {
                tracing::warn!("canvas bridge failed to start: {error}");
                CANVAS_PORT.store(0, Ordering::Relaxed);
                return;
            }
        };
        if let Err(error) = axum::serve(listener, router).await {
            tracing::warn!("canvas bridge stopped: {error}");
        }
        CANVAS_PORT.store(0, Ordering::Relaxed);
    });
    CANVAS_PORT.store(port, Ordering::Relaxed);
    if settings.bridge.canvas_port != port {
        if let Err(e) = config::update_settings(app, |s| s.bridge.canvas_port = port) {
            tracing::warn!("canvas bridge port not saved: {e}");
        }
    }
    tracing::info!("canvas-only bridge listening on http://127.0.0.1:{port} (full bridge is off)");
    Ok(port)
}

pub fn canvas_port() -> Option<u16> {
    Some(CANVAS_PORT.load(Ordering::Relaxed)).filter(|p| *p != 0)
}

pub async fn spawn(app: AppHandle) {
    let settings = config::load_settings(&app);

    if !settings.bridge.enabled {
        tracing::info!("local bridge disabled by settings; not starting HTTP server");
        return;
    }

    let preferred_port = settings.bridge.port;
    let (port, listener) = match bind_listener(preferred_port).await {
        Ok(pair) => pair,
        Err(error) => {
            tracing::warn!(
                "failed to bind local bridge on 127.0.0.1: {} — extension cookie path disabled",
                error
            );
            return;
        }
    };

    let (token, _port) = match ensure_bridge_settings(&app, port) {
        Ok(pair) => pair,
        Err(error) => {
            tracing::warn!("failed to persist bridge settings: {}", error);
            return;
        }
    };

    crate::bridge_tokens::legacy_flag().store(
        settings.bridge.legacy_agent_routes,
        std::sync::atomic::Ordering::Relaxed,
    );
    omniget_core::core::llm::cli_runtime::session::register_secret_value(&token);
    let state = BridgeState {
        app: app.clone(),
        token: Arc::new(token),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
        .allow_headers(Any);

    let router = Router::new()
        .route("/v1/health", get(health))
        .route("/v1/pair", get(pair))
        .route(
            "/v1/enqueue",
            post(enqueue).layer(DefaultBodyLimit::max(ENQUEUE_BODY_LIMIT)),
        )
        .route("/v1/queue", get(queue_state))
        .route("/v1/log/{id}", get(download_log))
        .route("/v1/cookies", post(cookies_export))
        .route(
            "/mcp",
            post(mcp_post)
                .get(mcp_get)
                .delete(mcp_get)
                .layer(DefaultBodyLimit::max(MCP_BODY_LIMIT)),
        )
        .route("/mcp/artifacts/{id}", get(mcp_artifact))
        .route(
            "/mcp/assist",
            post(assist_mcp_post).get(mcp_get).delete(mcp_get),
        )
        .merge(crate::local_bridge_llm::router(
            crate::local_bridge_llm::LlmBridgeState::from_bridge(&state),
        ))
        .merge(crate::local_bridge_company::router(
            crate::local_bridge_jobs::JobsBridgeState::from_bridge(&state).guard(),
        ))
        .merge(crate::local_bridge_jobs::router(
            crate::local_bridge_jobs::JobsBridgeState::from_bridge(&state),
        ))
        .merge(crate::local_bridge_observe::router(state.app.clone()))
        .merge(crate::connect::router(state.app.clone()))
        .merge(crate::local_bridge_agentkit::router(
            crate::local_bridge_jobs::JobsBridgeState::from_bridge(&state).guard(),
        ))
        .merge(crate::local_bridge_telegram::router(
            state.token.clone(),
            crate::local_bridge_jobs::JobsBridgeState::from_bridge(&state).guard(),
        ))
        .merge(crate::local_bridge_canvas::router(
            state.app.clone(),
            crate::local_bridge_jobs::JobsBridgeState::from_bridge(&state).guard(),
        ))
        .merge(crate::local_bridge_wmb::router(
            crate::local_bridge_wmb::WmbBridgeState::from_bridge(&state),
        ))
        .merge(crate::local_bridge_debug::router(
            state.app.clone(),
            state.token.clone(),
        ))
        .with_state(state)
        .layer(cors);

    tracing::info!("local bridge listening on http://127.0.0.1:{port}");
    MAIN_PORT.store(port, Ordering::Relaxed);
    omniget_core::core::assist::projection::set_endpoint(Some(format!(
        "http://127.0.0.1:{port}/mcp/assist"
    )));

    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            tracing::warn!("local bridge stopped: {error}");
        }
    });
}

async fn queue_state(State(state): State<BridgeState>, headers: HeaderMap) -> Response {
    if !check_bearer(&headers, &state.token) {
        return unauthorized();
    }
    let app_state = state.app.state::<crate::AppState>();
    let items = {
        let q = app_state.download_queue.lock().await;
        q.get_state()
    };
    let items = crate::core::queue::redacted_for_display(items);
    (StatusCode::OK, Json(items)).into_response()
}

async fn download_log(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<u64>,
) -> Response {
    if !check_bearer(&headers, &state.token) {
        return unauthorized();
    }
    let lines = crate::core::download_log::get(id);
    (StatusCode::OK, Json(lines)).into_response()
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        ok: true,
        version: env!("CARGO_PKG_VERSION"),
    })
}

pub const PAIR_WINDOW_SECS: i64 = 120;

static PAIR_OPEN_UNTIL_MS: AtomicI64 = AtomicI64::new(0);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn open_pairing_window() -> i64 {
    PAIR_OPEN_UNTIL_MS.store(now_ms() + PAIR_WINDOW_SECS * 1000, Ordering::SeqCst);
    PAIR_WINDOW_SECS
}

fn pairing_open() -> bool {
    PAIR_OPEN_UNTIL_MS.load(Ordering::SeqCst) > now_ms()
}

#[derive(Debug, Serialize)]
struct PairResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
}

async fn pair(State(state): State<BridgeState>, headers: HeaderMap) -> Response {
    if !pairing_open() {
        return (
            StatusCode::FORBIDDEN,
            Json(PairResponse {
                ok: false,
                token: None,
            }),
        )
            .into_response();
    }
    PAIR_OPEN_UNTIL_MS.store(0, Ordering::SeqCst);
    tracing::info!("[bridge] pairing token handed out (window closed, single-use)");

    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    if let Err(error) = state.app.emit(
        "bridge-paired",
        serde_json::json!({ "userAgent": user_agent }),
    ) {
        tracing::warn!("[bridge] failed to emit bridge-paired event: {error}");
    }

    (
        StatusCode::OK,
        Json(PairResponse {
            ok: true,
            token: Some((*state.token).clone()),
        }),
    )
        .into_response()
}

fn unauthorized() -> Response {
    let body = Json(EnqueueResponse {
        ok: false,
        code: Some("UNAUTHORIZED"),
        message: Some("Invalid or missing bearer token".to_string()),
    });
    (StatusCode::UNAUTHORIZED, body).into_response()
}

fn check_bearer(headers: &HeaderMap, expected: &str) -> bool {
    let header = match headers.get("authorization") {
        Some(v) => v,
        None => return false,
    };
    let raw = match header.to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };
    let provided = match raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
    {
        Some(s) => s.trim(),
        None => return false,
    };
    constant_time_eq::constant_time_eq(provided.as_bytes(), expected.as_bytes())
}

async fn enqueue(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Json(request): Json<EnqueueRequest>,
) -> Response {
    if !check_bearer(&headers, &state.token) {
        return unauthorized();
    }

    if let Some(client_version) = request.protocol_version {
        if client_version > HOST_MAX_PROTOCOL_VERSION {
            return error_response(
                StatusCode::BAD_REQUEST,
                "UNSUPPORTED_PROTOCOL",
                format!(
                    "Extension protocol v{client_version} is newer than host v{HOST_MAX_PROTOCOL_VERSION}. Please update OmniGet."
                ),
            );
        }
    }

    let payload = request.payload;

    if !crate::external_url::is_external_url(&payload.url) {
        return error_response(
            StatusCode::BAD_REQUEST,
            "INVALID_URL",
            "The requested URL is invalid".to_string(),
        );
    }

    if omniget_core::core::platform_optout::is_opted_out(&payload.url) {
        return error_response(
            StatusCode::FORBIDDEN,
            "PLATFORM_OPTED_OUT",
            omniget_core::core::platform_optout::OPTED_OUT_ERROR.to_string(),
        );
    }

    if let Some(ref cookies) = payload.cookies {
        if cookies.len() > cookie_limit() {
            return error_response(
                StatusCode::BAD_REQUEST,
                "TOO_MANY_COOKIES",
                format!(
                    "Request contains {} cookies; max allowed is {}",
                    cookies.len(),
                    cookie_limit()
                ),
            );
        }
        if !cookies.is_empty() {
            if let Err(error) = crate::cookies::ingest_batch(
                cookies,
                crate::cookies::IngestSource {
                    source_url: payload
                        .page_url
                        .clone()
                        .or_else(|| Some(payload.url.clone())),
                    source_label: "Browser extension".to_string(),
                    alias_hint: payload.title.clone(),
                },
            ) {
                tracing::warn!("failed to ingest cookies into manager: {error}");
            }
        }
    }

    if payload.referer.is_some()
        || payload.headers.is_some()
        || payload.media_type.is_some()
        || payload.title.is_some()
        || payload.thumbnail.is_some()
        || payload.open_app.is_some()
        || payload.page_url.is_some()
        || payload.user_agent.is_some()
    {
        if let Err(error) = write_extension_metadata(&payload) {
            tracing::warn!("failed to write extension metadata: {error}");
        }
    }

    if let Some(text) = payload.manifest_text.as_deref() {
        if !text.trim().is_empty() {
            match omniget_core::core::extension_manifest::store_manifest(&payload.url, text) {
                Ok(()) => tracing::info!(
                    "stored extension-captured playlist ({} bytes) for {}",
                    text.len(),
                    payload.url
                ),
                Err(error) => {
                    tracing::warn!("failed to store extension playlist text: {error}")
                }
            }
        }
    }

    let app = state.app.clone();
    let url = payload.url.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = crate::external_url::handle_external_url(&app, url, "bridge").await {
            tracing::warn!("failed to handle bridge URL: {error}");
        }
    });

    let body = Json(EnqueueResponse {
        ok: true,
        code: None,
        message: None,
    });
    (StatusCode::OK, body).into_response()
}

async fn cookies_export(
    State(state): State<BridgeState>,
    headers: HeaderMap,
    Json(request): Json<CookiesRequest>,
) -> Response {
    if !check_bearer(&headers, &state.token) {
        return unauthorized();
    }

    if let Some(client_version) = request.protocol_version {
        if client_version > HOST_MAX_PROTOCOL_VERSION {
            return error_response(
                StatusCode::BAD_REQUEST,
                "UNSUPPORTED_PROTOCOL",
                format!(
                    "Extension protocol v{client_version} is newer than host v{HOST_MAX_PROTOCOL_VERSION}. Please update OmniGet."
                ),
            );
        }
    }

    if request.cookies.len() > cookie_limit() {
        return error_response(
            StatusCode::BAD_REQUEST,
            "TOO_MANY_COOKIES",
            format!(
                "cookies:export contains {} cookies; max is {}",
                request.cookies.len(),
                cookie_limit()
            ),
        );
    }

    if !request.cookies.is_empty() {
        let label = if request.source_url.is_some() || request.alias.is_some() {
            "Browser extension (manual capture)".to_string()
        } else {
            "Browser extension (auto-capture)".to_string()
        };
        let alias_hint = request.alias.clone().or_else(|| request.page_title.clone());
        if let Err(error) = crate::cookies::ingest_batch(
            &request.cookies,
            crate::cookies::IngestSource {
                source_url: request.source_url.clone(),
                source_label: label,
                alias_hint,
            },
        ) {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "WRITE_FAILED",
                format!("ingest cookies: {error}"),
            );
        }
    }

    let body = Json(EnqueueResponse {
        ok: true,
        code: None,
        message: None,
    });
    (StatusCode::OK, body).into_response()
}

fn error_response(status: StatusCode, code: &'static str, message: String) -> Response {
    let body = Json(EnqueueResponse {
        ok: false,
        code: Some(code),
        message: Some(message),
    });
    (status, body).into_response()
}

pub fn build_pairing_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

async fn mcp_post(
    State(state): State<BridgeState>,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let principal = match bearer_of(&headers).and_then(|t| {
        crate::mcp::policy::authenticate(&t)
            .ok()
            .or_else(|| crate::mcp::session_principal(&t))
    }) {
        Some(p) => p,
        None => return unauthorized(),
    };
    let host = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let valid_host = url::Url::parse(&format!("http://{host}"))
        .ok()
        .is_some_and(|u| {
            matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
                && u.username().is_empty()
                && u.path() == "/"
        });
    if !valid_host || headers.contains_key("origin") {
        return (
            StatusCode::FORBIDDEN,
            "MCP accepts authenticated native clients only",
        )
            .into_response();
    }
    if headers
        .get("mcp-protocol-version")
        .is_some_and(|v| !matches!(v.to_str(), Ok("2025-03-26" | "2025-06-18")))
    {
        return (StatusCode::BAD_REQUEST, "Unsupported MCP protocol version").into_response();
    }
    if !headers
        .get("content-type")
        .and_then(|h| h.to_str().ok())
        .is_some_and(|s| s.split(';').next() == Some("application/json"))
    {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    if body.len() > MCP_BODY_LIMIT {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            "PAYLOAD_TOO_LARGE: the request is over 36 MB; send files by path or url instead of base64",
        )
            .into_response();
    }
    let accept = headers
        .get("accept")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if !accept.contains("application/json") || !accept.contains("text/event-stream") {
        return StatusCode::NOT_ACCEPTABLE.into_response();
    }

    if !principal.id.starts_with(crate::mcp::SESSION_PREFIX) && !crate::mcp::enabled() {
        return (StatusCode::FORBIDDEN, Json(serde_json::json!({ "error": "MCP server is disabled in OmniGet → LLM → MCP → Your endpoint" }))).into_response();
    }
    let msg: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {}", e) } })),
            )
                .into_response()
        }
    };
    let protocol = headers
        .get("mcp-protocol-version")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let lean = uri
        .query()
        .is_some_and(|q| q.split('&').any(|p| p == "lean=1" || p == "lean=true"))
        || headers
            .get("x-omniget-lean")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    match crate::mcp::REQUEST_PROTOCOL
        .scope(
            protocol,
            crate::mcp::LEAN.scope(lean, crate::mcp::handle_body(&state.app, &principal, &msg)),
        )
        .await
    {
        Some(resp) => (StatusCode::OK, Json(resp)).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

fn bearer_of(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get("authorization")?.to_str().ok()?;
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .map(|s| s.trim().to_string())
}

async fn mcp_artifact(
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
) -> Response {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    if !crate::mcp::enabled() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let principal =
        match bearer_of(&headers).and_then(|t| crate::mcp::policy::authenticate(&t).ok()) {
            Some(p) => p,
            None => return unauthorized(),
        };
    let host = headers
        .get("host")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    if headers.contains_key("origin")
        || !url::Url::parse(&format!("http://{host}"))
            .ok()
            .is_some_and(|u| {
                matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
                    && u.username().is_empty()
                    && u.path() == "/"
            })
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(digest) = headers
        .get("if-match")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim_matches('"').to_owned())
    else {
        return StatusCode::PRECONDITION_REQUIRED.into_response();
    };
    let (_, last) = match crate::mcp::artifacts::preflight(&principal, &id, &digest, None) {
        Ok(size) => size,
        Err(e) if e == "ARTIFACT_REVISION_MISMATCH" => {
            return StatusCode::PRECONDITION_FAILED.into_response()
        }
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    if crate::mcp::artifacts::range(headers.get("range").and_then(|v| v.to_str().ok()), last + 1)
        .is_err()
    {
        return (
            StatusCode::RANGE_NOT_SATISFIABLE,
            [("content-range", format!("bytes */{}", last + 1))],
        )
            .into_response();
    }
    let permit = match crate::mcp::artifacts::admit() {
        Ok(p) => p,
        Err(_) => return StatusCode::TOO_MANY_REQUESTS.into_response(),
    };
    let p = principal.clone();
    let aid = id.clone();
    let (snapshot, permit) = match tokio::task::spawn_blocking(move || {
        crate::mcp::artifacts::open(&p, &aid, &digest).map(|s| (s, permit))
    })
    .await
    {
        Ok(Ok(s)) => s,
        Ok(Err(e)) if e == "ARTIFACT_REVISION_MISMATCH" => {
            return StatusCode::PRECONDITION_FAILED.into_response()
        }
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    let bytes = snapshot.bytes;
    let (start, end) = match crate::mcp::artifacts::range(
        headers.get("range").and_then(|v| v.to_str().ok()),
        bytes,
    ) {
        Ok(r) => r,
        Err(_) => {
            return (
                StatusCode::RANGE_NOT_SATISFIABLE,
                [("content-range", format!("bytes */{bytes}"))],
            )
                .into_response()
        }
    };
    let digest = snapshot.digest;
    let mut file = tokio::fs::File::from_std(snapshot.file);
    if file.seek(std::io::SeekFrom::Start(start)).await.is_err() {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }
    let length = end - start + 1;
    let stream = futures::stream::try_unfold(
        (file, length, principal, id, permit),
        |(mut file, left, p, id, permit)| async move {
            if left == 0 {
                return Ok::<_, std::io::Error>(None);
            }
            let cp = p.clone();
            let cid = id.clone();
            let permitted =
                tokio::task::spawn_blocking(move || crate::mcp::artifacts::authorized(&cp, &cid))
                    .await;
            if !matches!(permitted, Ok(Ok(()))) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "ARTIFACT_ACCESS_REVOKED",
                ));
            }
            let mut buf = vec![0u8; left.min(65536) as usize];
            let n = file.read(&mut buf).await?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "ARTIFACT_INCOMPLETE",
                ));
            }
            buf.truncate(n);
            Ok(Some((buf, (file, left - n as u64, p, id, permit))))
        },
    );
    let partial = headers.contains_key("range");
    let mut response = Response::builder()
        .status(if partial {
            StatusCode::PARTIAL_CONTENT
        } else {
            StatusCode::OK
        })
        .header("content-type", "application/octet-stream")
        .header("content-length", length)
        .header("etag", format!("\"{digest}\""))
        .header("accept-ranges", "bytes")
        .header("cache-control", "private, no-store")
        .header("x-content-type-options", "nosniff")
        .header("content-disposition", "attachment");
    if partial {
        response = response.header("content-range", format!("bytes {start}-{end}/{bytes}"));
    }
    response
        .body(axum::body::Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

pub async fn assist_mcp_response(
    db: &omniget_core::core::assist::db::AssistDb,
    headers: &HeaderMap,
    body: &[u8],
) -> (StatusCode, Option<serde_json::Value>) {
    let msg: serde_json::Value = match serde_json::from_slice(body) {
        Ok(v) => v,
        Err(e) => {
            if bearer_of(headers)
                .and_then(|t| omniget_core::core::assist::projection::validate(db, &t, now_ms()))
                .is_none()
            {
                return (StatusCode::UNAUTHORIZED, None);
            }
            return (
                StatusCode::BAD_REQUEST,
                Some(
                    serde_json::json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {e}") } }),
                ),
            );
        }
    };
    match omniget_core::core::assist::projection::handle(db, bearer_of(headers).as_deref(), &msg)
        .await
    {
        Err(_) => (StatusCode::UNAUTHORIZED, None),
        Ok(Some(reply)) => (StatusCode::OK, Some(reply)),
        Ok(None) => (StatusCode::ACCEPTED, None),
    }
}

async fn assist_mcp_post(headers: HeaderMap, body: axum::body::Bytes) -> Response {
    let db = match omniget_core::core::assist::db::global() {
        Ok(db) => db,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    match assist_mcp_response(&db, &headers, &body).await {
        (status, Some(v)) => (status, Json(v)).into_response(),
        (StatusCode::UNAUTHORIZED, None) => unauthorized(),
        (status, None) => status.into_response(),
    }
}

async fn mcp_get() -> Response {
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}
