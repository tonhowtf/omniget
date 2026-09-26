//! Optional OAuth resource gateway. Only /mcp and resource metadata are routed.
//! An external OAuth provider owns authorization, PKCE, registration and login.
//! Start explicitly with a private config file; never auto-publish a tunnel.
use anyhow::{bail, Context, Result};
use axum::{
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use oauth2::{
    basic::{BasicClient, BasicTokenIntrospectionResponse},
    AccessToken, ClientId, ClientSecret, IntrospectionUrl, TokenIntrospectionResponse,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    enable_remote_write: bool,
    #[serde(default)]
    enable_artifact_transfer: bool,
    listen: SocketAddr,
    resource: String,
    issuer: String,
    introspection_url: String,
    introspection_client_id: String,
    introspection_client_secret: String,
    desktop_url: String,
    bindings: Vec<Binding>,
    #[serde(default)]
    cloudflare: Option<Cloudflare>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cloudflare {
    tunnel_id: String,
    credentials_file: std::path::PathBuf,
}
fn tunnel_config(c: &Config, tunnel: &Cloudflare) -> Result<String> {
    if tunnel.tunnel_id.is_empty()
        || !tunnel
            .tunnel_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        || !tunnel.credentials_file.is_absolute()
    {
        bail!("INVALID_TUNNEL_CONFIG");
    }
    let host = reqwest::Url::parse(&c.resource)?
        .host_str()
        .context("INVALID_HOST")?
        .to_string();
    Ok(format!("tunnel: {}\ncredentials-file: {}\ningress:\n  - hostname: {}\n    service: http://{}\n  - service: http_status:404\n",serde_json::to_string(&tunnel.tunnel_id)?,serde_json::to_string(&tunnel.credentials_file)?,serde_json::to_string(&host)?,c.listen))
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    client_id: String,
    subject: String,
    desktop_token: String,
    tools: Vec<String>,
}
#[derive(Clone)]
struct App {
    config: Arc<Config>,
    http: reqwest::Client,
    limits: Arc<Limits>,
}
fn https(value: &str) -> bool {
    reqwest::Url::parse(value).ok().is_some_and(|u| {
        u.scheme() == "https"
            && !u.host_str().unwrap_or("").is_empty()
            && u.username().is_empty()
            && u.password().is_none()
            && u.query().is_none()
            && u.fragment().is_none()
    })
}
fn validate(c: &Config) -> Result<()> {
    if c.listen.port() == 0
        || !c.listen.ip().is_loopback()
        || !https(&c.resource)
        || !https(&c.issuer)
        || !https(&c.introspection_url)
    {
        bail!("INVALID_GATEWAY_CONFIG");
    }
    let local = reqwest::Url::parse(&c.desktop_url)?;
    if local.scheme() != "http"
        || !matches!(local.host_str(), Some("127.0.0.1" | "[::1]"))
        || local.path() != "/mcp"
        || !local.username().is_empty()
        || local.password().is_some()
        || local.query().is_some()
    {
        bail!("INVALID_DESKTOP_ENDPOINT");
    }
    if local.fragment().is_some()
        || reqwest::Url::parse(&c.resource)?.path() != "/mcp"
        || c.bindings.is_empty()
        || c.bindings.len() > 64
        || c.introspection_client_id.is_empty()
        || c.introspection_client_id.len() > 256
        || c.introspection_client_secret.is_empty()
        || c.introspection_client_secret.len() > 8192
    {
        bail!("INVALID_GATEWAY_CONFIG");
    }
    let mut identities = BTreeSet::new();
    let mut desktop_tokens = BTreeSet::new();
    for b in &c.bindings {
        if b.subject.is_empty()
            || b.client_id.is_empty()
            || b.desktop_token.len() != 64
            || !b.desktop_token.bytes().all(|v| v.is_ascii_hexdigit())
            || b.tools.len() > 32
            || !identities.insert((&b.client_id, &b.subject))
            || !desktop_tokens.insert(&b.desktop_token)
            || b.tools.iter().any(|t| match tool_scope(t) {
                Some(WRITE) => !c.enable_remote_write,
                Some(_) => false,
                None => true,
            })
        {
            bail!("INVALID_GATEWAY_BINDING_OR_WRITE_OPT_IN_REQUIRED");
        }
    }
    Ok(())
}
#[tokio::main]
async fn main() -> Result<()> {
    let file = std::env::args_os()
        .nth(1)
        .context("usage: omniget-gateway PRIVATE_CONFIG.json")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(&file)?.permissions().mode() & 0o077 != 0 {
            bail!("GATEWAY_CONFIG_MUST_BE_PRIVATE_0600");
        }
    }
    let c: Config =
        serde_json::from_slice(&std::fs::read(&file)?).context("INVALID_GATEWAY_CONFIG")?;
    validate(&c)?;
    let addr = c.listen;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let mut tunnel = if let Some(t) = &c.cloudflare {
        let config = tunnel_config(&c, t)?;
        let path = std::path::PathBuf::from(&file).with_extension(format!(
            "cloudflared-{}.yml",
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
        ));
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options.open(&path)?.write_all(config.as_bytes())?;
        Some(
            tokio::process::Command::new("cloudflared")
                .arg("tunnel")
                .arg("--config")
                .arg(path)
                .arg("run")
                .arg(&t.tunnel_id)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .context("CLOUDFLARED_UNAVAILABLE")?,
        )
    } else {
        None
    };
    let limits = Arc::new(Limits::new(c.bindings.len()));
    let app = App {
        limits,
        config: Arc::new(c),
        http: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .no_proxy()
            .timeout(Duration::from_secs(30))
            .build()?,
    };
    let router = Router::new()
        .route(
            "/mcp",
            post(mcp).get(|| async { StatusCode::METHOD_NOT_ALLOWED }),
        )
        .route("/mcp/artifacts/{id}", get(artifact))
        .route("/.well-known/oauth-protected-resource", get(metadata))
        .route("/.well-known/oauth-protected-resource/mcp", get(metadata))
        .layer(DefaultBodyLimit::max(65536))
        .with_state(app);
    eprintln!("OmniGet OAuth gateway listening on loopback (local opt-ins apply)");
    let server = axum::serve(listener, router).with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    });
    if let Some(child) = tunnel.as_mut() {
        tokio::select! {
            result=server=>result?,
            _=child.wait()=>bail!("TUNNEL_EXITED: remote gateway stopped; local desktop continues"),
        }
    } else {
        server.await?;
    }
    Ok(())
}
const READ: &str = "omniget:read";
const WRITE: &str = "omniget:write";
const TRANSFER: &str = "omniget:transfer";
const PROTOCOL: &str = "2025-06-18";
const MAX_TRANSFER: u64 = 256 * 1024 * 1024;
fn tool_scope(name: &str) -> Option<&'static str> {
    match name {
        "omniget_capabilities"
        | "omniget_health"
        | "downloads_queue"
        | "download_status"
        | "download_wait"
        | "download_logs"
        | "download_diagnose"
        | "download_recovery_options"
        | "download_artifacts"
        | "download_history"
        | "destinations_list"
        | "downloads_preflight"
        | "agents_list"
        | "workspaces_list"
        | "missions_list"
        | "missions_get"
        | "missions_events" => Some(READ),
        "download_enqueue"
        | "downloads_batch_enqueue"
        | "download_cancel"
        | "download_pause"
        | "download_resume"
        | "download_retry"
        | "media_inspect"
        | "media_collection_list"
        | "missions_create"
        | "missions_cancel"
        | "missions_pause"
        | "missions_resume" => Some(WRITE),
        _ => None,
    }
}
struct Limits {
    requests: Arc<tokio::sync::Semaphore>,
    transfers: Arc<tokio::sync::Semaphore>,
    rate: Mutex<(std::time::Instant, u32, Vec<u32>)>,
}
impl Limits {
    fn new(bindings: usize) -> Self {
        Self {
            requests: Arc::new(tokio::sync::Semaphore::new(8)),
            transfers: Arc::new(tokio::sync::Semaphore::new(2)),
            rate: Mutex::new((std::time::Instant::now(), 0, vec![0; bindings])),
        }
    }
    fn admit(&self, index: Option<usize>) -> bool {
        let Ok(mut r) = self.rate.lock() else {
            return false;
        };
        if r.0.elapsed() >= Duration::from_secs(60) {
            r.0 = std::time::Instant::now();
            r.1 = 0;
            r.2.fill(0);
        }
        match index {
            None => {
                if r.1 >= 120 {
                    return false;
                }
                r.1 += 1;
            }
            Some(i) => {
                let Some(n) = r.2.get_mut(i) else {
                    return false;
                };
                if *n >= 60 {
                    return false;
                }
                *n += 1;
            }
        }
        true
    }
}
#[derive(Clone)]
struct Authorization {
    index: usize,
    scopes: BTreeSet<String>,
    bearer: String,
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn enabled_scopes(c: &Config) -> Vec<&'static str> {
    let mut s = vec![READ];
    if c.enable_remote_write {
        s.push(WRITE)
    }
    if c.enable_artifact_transfer {
        s.push(TRANSFER)
    }
    s
}
async fn metadata(State(app): State<App>) -> Json<Value> {
    Json(
        json!({"resource":app.config.resource,"authorization_servers":[app.config.issuer],"scopes_supported":enabled_scopes(&app.config),"bearer_methods_supported":["header"],"resource_name":"OmniGet on your computer"}),
    )
}
fn unauthorized(c: &Config) -> Response {
    let base = reqwest::Url::parse(&c.resource)
        .unwrap()
        .origin()
        .ascii_serialization();
    (StatusCode::UNAUTHORIZED,[("WWW-Authenticate",format!("Bearer resource_metadata=\"{base}/.well-known/oauth-protected-resource/mcp\", scope=\"{}\"",enabled_scopes(c).join(" ")))]).into_response()
}
fn binding<'a>(
    c: &'a Config,
    t: &BasicTokenIntrospectionResponse,
    now: i64,
) -> Option<&'a Binding> {
    if !t.active()
        || t.exp().is_none_or(|e| e.timestamp() <= now)
        || t.nbf().is_some_and(|n| n.timestamp() > now)
        || !t.aud().is_some_and(|a| a.contains(&c.resource))
        || t.iss() != Some(c.issuer.as_str())
        || !t.scopes().is_some_and(|s| {
            s.iter()
                .any(|s| [READ, WRITE, TRANSFER].contains(&s.as_str()))
        })
    {
        return None;
    }
    c.bindings.iter().find(|b| {
        Some(b.client_id.as_str()) == t.client_id().map(|v| v.as_str())
            && Some(b.subject.as_str()) == t.sub()
    })
}
fn valid_headers(c: &Config, h: &HeaderMap) -> bool {
    let expected = reqwest::Url::parse(&c.resource).unwrap();
    let host_valid = h
        .get("host")
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| reqwest::Url::parse(&format!("https://{raw}")).ok())
        .is_some_and(|actual| {
            actual.host_str() == expected.host_str()
                && actual.port_or_known_default() == expected.port_or_known_default()
                && actual.path() == "/"
                && actual.username().is_empty()
                && actual.password().is_none()
                && actual.query().is_none()
                && actual.fragment().is_none()
        });
    !h.contains_key("origin") && host_valid
}

async fn introspect(app: &App, bearer: &str) -> Option<Authorization> {
    let oauth = BasicClient::new(ClientId::new(app.config.introspection_client_id.clone()))
        .set_client_secret(ClientSecret::new(
            app.config.introspection_client_secret.clone(),
        ))
        .set_introspection_url(IntrospectionUrl::new(app.config.introspection_url.clone()).ok()?);
    let token = AccessToken::new(bearer.to_owned());
    let status = tokio::time::timeout(
        Duration::from_secs(5),
        oauth.introspect(&token).request_async(&app.http),
    )
    .await
    .ok()?
    .ok()?;
    let selected = binding(&app.config, &status, now())?;
    let index = app
        .config
        .bindings
        .iter()
        .position(|b| std::ptr::eq(b, selected))?;
    let scopes = status
        .scopes()?
        .iter()
        .map(|s| s.as_str().to_owned())
        .collect();
    Some(Authorization {
        index,
        scopes,
        bearer: bearer.to_owned(),
    })
}
async fn authenticate(app: &App, headers: &HeaderMap) -> Result<Authorization, Response> {
    if headers
        .get("mcp-protocol-version")
        .is_some_and(|v| v != PROTOCOL)
    {
        return Err(StatusCode::BAD_REQUEST.into_response());
    }
    if !valid_headers(&app.config, headers) {
        return Err(StatusCode::FORBIDDEN.into_response());
    }
    let bearer = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.split_once(' '))
        .filter(|(scheme, token)| {
            scheme.eq_ignore_ascii_case("Bearer") && !token.is_empty() && token.len() <= 8192
        })
        .map(|(_, t)| t)
        .ok_or_else(|| unauthorized(&app.config))?;
    introspect(app, bearer)
        .await
        .ok_or_else(|| unauthorized(&app.config))
}
async fn bounded_json(mut response: reqwest::Response) -> Result<Value, StatusCode> {
    if !response.status().is_success() {
        return Err(response.status());
    }
    if response.content_length().is_some_and(|n| n > 1024 * 1024) {
        return Err(StatusCode::BAD_GATEWAY);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?
    {
        if chunk.len() > 1024 * 1024 - bytes.len() {
            return Err(StatusCode::BAD_GATEWAY);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| StatusCode::BAD_GATEWAY)
}
async fn desktop(app: &App, grant: &Binding, msg: &Value) -> Result<reqwest::Response, StatusCode> {
    app.http
        .post(&app.config.desktop_url)
        .bearer_auth(&grant.desktop_token)
        .header("Accept", "application/json, text/event-stream")
        .header("MCP-Protocol-Version", PROTOCOL)
        .json(msg)
        .send()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)
}
fn boundary_value(value: &Value, scope: &str) -> bool {
    let v = &value["result"]["structuredContent"];
    value["result"]["isError"] != true
        && v["protocol"] == PROTOCOL
        && v["executionIsolation"]["available"] == true
        && v["executionIsolation"]["backend"] == "macos-seatbelt-broker-v1"
        && v["executionIsolation"]["workerWireVersion"] == 1
        && v["remoteGateway"]["boundaryVersion"] == 1
        && v["remoteGateway"][if scope == TRANSFER {
            "transferEnabled"
        } else {
            "writeEnabled"
        }] == true
}
async fn boundary(app: &App, index: usize, scope: &str) -> bool {
    if (scope == WRITE && !app.config.enable_remote_write)
        || (scope == TRANSFER && !app.config.enable_artifact_transfer)
    {
        return false;
    }
    let msg = json!({"jsonrpc":"2.0","id":"gateway-boundary","method":"tools/call","params":{"name":"omniget_capabilities","arguments":{}}});
    let check = async {
        let response = desktop(app, &app.config.bindings[index], &msg).await.ok()?;
        bounded_json(response).await.ok()
    };
    tokio::time::timeout(Duration::from_secs(5), check)
        .await
        .ok()
        .flatten()
        .is_some_and(|v| boundary_value(&v, scope))
}
fn permits(c: &Config, auth: &Authorization, tool: &str) -> bool {
    c.bindings[auth.index].tools.iter().any(|t| t == tool)
        && tool_scope(tool).is_some_and(|scope| {
            auth.scopes.contains(scope) && (scope != WRITE || c.enable_remote_write)
        })
}
fn sanitize(value: &mut Value, app: &App, auth: &Authorization, transfer: bool) {
    match value {
        Value::Object(map) => {
            for key in ["desktop_token", "authorization", "bearer", "bridge_url"] {
                map.remove(key);
            }
            if !transfer {
                map.remove("access");
                map.remove("download_path");
                if map.contains_key("transferAllowed") {
                    map.insert("transferAllowed".into(), json!(false));
                }
                if map.contains_key("transfer_allowed") {
                    map.insert("transfer_allowed".into(), json!(false));
                }
            }
            for v in map.values_mut() {
                sanitize(v, app, auth, transfer)
            }
        }
        Value::Array(values) => {
            for v in values {
                sanitize(v, app, auth, transfer)
            }
        }
        Value::String(s) => {
            for secret in [
                &app.config.bindings[auth.index].desktop_token,
                &app.config.introspection_client_secret,
                &auth.bearer,
            ] {
                if !secret.is_empty() {
                    *s = s.replace(secret, "[REDACTED]");
                }
            }
            let origin = reqwest::Url::parse(&app.config.desktop_url)
                .unwrap()
                .origin()
                .ascii_serialization();
            *s = s.replace(&origin, "[LOCAL_ENDPOINT]");
        }
        _ => {}
    }
}
async fn mcp(State(app): State<App>, headers: HeaderMap, body: Bytes) -> Response {
    let Ok(_permit) = app.limits.requests.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    if !app.limits.admit(None) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let auth = match authenticate(&app, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !app.limits.admit(Some(auth.index)) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if !msg.is_object() || msg["jsonrpc"] != "2.0" {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let method = msg["method"].as_str().unwrap_or("");
    if ![
        "initialize",
        "notifications/initialized",
        "ping",
        "tools/list",
        "tools/call",
        "resources/list",
        "prompts/list",
    ]
    .contains(&method)
    {
        return Json(json!({"jsonrpc":"2.0","id":msg["id"],"error":{"code":-32601,"message":"Method not found"}})).into_response();
    }
    if method == "tools/call" {
        let tool = msg["params"]["name"].as_str().unwrap_or("");
        if !permits(&app.config, &auth, tool) {
            return StatusCode::FORBIDDEN.into_response();
        }
        if tool_scope(tool) == Some(WRITE) && !boundary(&app, auth.index, WRITE).await {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    let response = match desktop(&app, &app.config.bindings[auth.index], &msg).await {
        Ok(r) => r,
        Err(c) => return c.into_response(),
    };
    if response.status() == StatusCode::ACCEPTED {
        return StatusCode::ACCEPTED.into_response();
    }
    let mut value = match bounded_json(response).await {
        Ok(v) => v,
        Err(c) => return c.into_response(),
    };
    if method == "tools/list" {
        let writable = auth.scopes.contains(WRITE) && boundary(&app, auth.index, WRITE).await;
        if let Some(tools) = value["result"]["tools"].as_array_mut() {
            tools.retain(|t| {
                t["name"].as_str().is_some_and(|n| {
                    permits(&app.config, &auth, n) && (tool_scope(n) != Some(WRITE) || writable)
                })
            });
        }
    }
    let transfer = auth.scopes.contains(TRANSFER)
        && app.config.enable_artifact_transfer
        && boundary(&app, auth.index, TRANSFER).await;
    sanitize(&mut value, &app, &auth, transfer);
    // Desktop content.text mirrors structuredContent; replace it after redaction so
    // transfer descriptors cannot survive in an independently serialized copy.
    if method == "tools/call" && value["result"].get("structuredContent").is_some() {
        value["result"]["content"] =
            json!([{"type":"text","text":value["result"]["structuredContent"].to_string()}]);
    }
    (
        StatusCode::OK,
        [("cache-control", "private, no-store")],
        Json(value),
    )
        .into_response()
}
fn valid_artifact_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
fn transfer_headers(headers: &HeaderMap) -> Option<(String, Option<String>)> {
    let raw = headers.get("if-match")?.to_str().ok()?;
    let digest = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(raw);
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let range = match headers.get("range") {
        None => None,
        Some(v) => {
            let s = v.to_str().ok()?;
            if s.len() > 80 {
                return None;
            }
            let (start, end) = s.strip_prefix("bytes=")?.split_once('-')?;
            if start.is_empty() && end.is_empty() {
                return None;
            }
            for part in [start, end] {
                if !part.is_empty()
                    && (!part.bytes().all(|b| b.is_ascii_digit()) || part.parse::<u64>().is_err())
                {
                    return None;
                }
            }
            Some(s.to_owned())
        }
    };
    Some((format!("\"{digest}\""), range))
}
struct Relay {
    app: App,
    auth: Authorization,
    response: reqwest::Response,
    pending: Bytes,
    left: u64,
    deadline: tokio::time::Instant,
    checked: tokio::time::Instant,
    _permit: tokio::sync::OwnedSemaphorePermit,
}
fn relay_error() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::PermissionDenied, "TRANSFER_STOPPED")
}
async fn relay_next(mut state: Relay) -> Result<Option<(Bytes, Relay)>, std::io::Error> {
    if state.left == 0 {
        return Ok(None);
    }
    let operation = async {
        if state.checked.elapsed() >= Duration::from_secs(1) {
            let renewed = introspect(&state.app, &state.auth.bearer)
                .await
                .ok_or_else(relay_error)?;
            if renewed.index != state.auth.index
                || !renewed.scopes.contains(TRANSFER)
                || !boundary(&state.app, state.auth.index, TRANSFER).await
            {
                return Err(relay_error());
            }
            state.auth = renewed;
            state.checked = tokio::time::Instant::now();
        }
        if state.pending.is_empty() {
            loop {
                tokio::select! {
                    chunk = state.response.chunk() => {
                        state.pending = chunk.map_err(|_| relay_error())?.ok_or_else(relay_error)?;
                        if state.pending.len() as u64 > state.left { return Err(relay_error()); }
                        break;
                    },
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                        let renewed = introspect(&state.app, &state.auth.bearer).await.ok_or_else(relay_error)?;
                        if renewed.index != state.auth.index || !renewed.scopes.contains(TRANSFER)
                            || !boundary(&state.app, state.auth.index, TRANSFER).await { return Err(relay_error()); }
                        state.auth = renewed; state.checked = tokio::time::Instant::now();
                    }
                }
            }
        }
        // Revalidate again after a slow upstream read, before releasing new bytes.
        if state.checked.elapsed() >= Duration::from_secs(1) {
            let renewed = introspect(&state.app, &state.auth.bearer)
                .await
                .ok_or_else(relay_error)?;
            if renewed.index != state.auth.index
                || !renewed.scopes.contains(TRANSFER)
                || !boundary(&state.app, state.auth.index, TRANSFER).await
            {
                return Err(relay_error());
            }
            state.auth = renewed;
            state.checked = tokio::time::Instant::now();
        }
        let size = state.pending.len().min(65536).min(state.left as usize);
        if size == 0 {
            return Err(relay_error());
        }
        let bytes = state.pending.split_to(size);
        state.left -= size as u64;
        Ok(bytes)
    };
    let chunk = tokio::time::timeout_at(state.deadline, operation)
        .await
        .map_err(|_| relay_error())??;
    Ok(Some((chunk, state)))
}
async fn artifact(State(app): State<App>, Path(id): Path<String>, headers: HeaderMap) -> Response {
    let Ok(_request) = app.limits.requests.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    if !app.config.enable_artifact_transfer || !valid_artifact_id(&id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !app.limits.admit(None) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let auth = match authenticate(&app, &headers).await {
        Ok(a) => a,
        Err(r) => return r,
    };
    if !auth.scopes.contains(TRANSFER)
        || !app.config.bindings[auth.index]
            .tools
            .iter()
            .any(|t| t == "download_artifacts")
        || !boundary(&app, auth.index, TRANSFER).await
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !app.limits.admit(Some(auth.index)) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let Some((digest, range)) = transfer_headers(&headers) else {
        return StatusCode::PRECONDITION_REQUIRED.into_response();
    };
    let Ok(permit) = app.limits.transfers.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let mut url = reqwest::Url::parse(&app.config.desktop_url).unwrap();
    url.set_path(&format!("/mcp/artifacts/{id}"));
    let mut request = app
        .http
        .get(url)
        .bearer_auth(&app.config.bindings[auth.index].desktop_token)
        .header("if-match", &digest)
        .timeout(Duration::from_secs(120));
    if let Some(range) = range {
        request = request.header("range", range);
    }
    let response = match request.send().await {
        Ok(r) => r,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let code = response.status();
    if ![StatusCode::OK, StatusCode::PARTIAL_CONTENT].contains(&code) {
        return code.into_response();
    }
    let Some(length) = response
        .content_length()
        .filter(|n| *n > 0 && *n <= MAX_TRANSFER)
    else {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    if response.headers().get("etag").and_then(|h| h.to_str().ok()) != Some(digest.as_str()) {
        return StatusCode::BAD_GATEWAY.into_response();
    }
    let content_range = response.headers().get("content-range").cloned();
    let state = Relay {
        app,
        auth,
        response,
        pending: Bytes::new(),
        left: length,
        deadline: tokio::time::Instant::now() + Duration::from_secs(120),
        checked: tokio::time::Instant::now() - Duration::from_secs(2),
        _permit: permit,
    };
    let stream = futures::stream::try_unfold(state, relay_next);
    let mut builder = Response::builder()
        .status(code)
        .header("content-type", "application/octet-stream")
        .header("content-length", length)
        .header("etag", digest)
        .header("accept-ranges", "bytes")
        .header("cache-control", "private, no-store")
        .header("x-content-type-options", "nosniff")
        .header("content-disposition", "attachment");
    if let Some(range) = content_range {
        builder = builder.header("content-range", range);
    }
    builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}
#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    fn config() -> Config {
        serde_json::from_value(json!({"listen":"127.0.0.1:47740","resource":"https://mcp.example.com/mcp","issuer":"https://auth.example.com","introspection_url":"https://auth.example.com/introspect","introspection_client_id":"gateway","introspection_client_secret":"synthetic-client-secret","desktop_url":"http://127.0.0.1:47720/mcp","bindings":[{"client_id":"test","subject":"user","desktop_token":"a".repeat(64),"tools":["download_status","download_artifacts"]}]})).unwrap()
    }
    fn status(c: &Config, scope: &str) -> BasicTokenIntrospectionResponse {
        serde_json::from_value(json!({"active":true,"exp":now()+3600,"aud":[c.resource],"iss":c.issuer,"scope":scope,"client_id":"test","sub":"user"})).unwrap()
    }
    fn capabilities(write: bool, transfer: bool) -> Value {
        json!({"jsonrpc":"2.0","result":{"isError":false,"structuredContent":{"protocol":PROTOCOL,"executionIsolation":{"available":true,"backend":"macos-seatbelt-broker-v1","workerWireVersion":1},"remoteGateway":{"boundaryVersion":1,"writeEnabled":write,"transferEnabled":transfer}}}})
    }
    #[test]
    fn defaults_and_scopes_fail_closed() {
        let mut c = config();
        assert!(!c.enable_remote_write && !c.enable_artifact_transfer);
        assert!(validate(&c).is_ok());
        c.bindings[0].tools.push("download_enqueue".into());
        assert!(validate(&c).is_err());
        c.enable_remote_write = true;
        assert!(validate(&c).is_ok());
        c.bindings[0].tools.push("run_shell".into());
        assert!(validate(&c).is_err());
        let read = Authorization {
            index: 0,
            scopes: [READ.to_owned()].into(),
            bearer: "synthetic".into(),
        };
        assert!(!permits(&c, &read, "download_enqueue"));
        assert!(permits(&c, &read, "download_status"));
        assert!(binding(&c, &status(&c, TRANSFER), now()).is_some());
        assert!(binding(&c, &status(&c, "other"), now()).is_none());
        assert!(!boundary_value(&json!({}), WRITE));
        assert!(!boundary_value(&capabilities(false, true), WRITE));
        assert!(boundary_value(&capabilities(true, true), WRITE));
        let mut v = capabilities(true, true);
        v["result"]["structuredContent"]["executionIsolation"]["available"] = json!(false);
        assert!(!boundary_value(&v, TRANSFER));
    }
    #[test]
    fn validates_identity_expiry_audience_and_rate() {
        let c = config();
        let base = serde_json::to_value(status(&c, READ)).unwrap();
        for (key, v) in [
            ("active", json!(false)),
            ("exp", json!(1)),
            ("nbf", json!(now() + 100)),
            ("aud", json!(["elsewhere"])),
            ("iss", json!("wrong")),
            ("client_id", json!("other")),
            ("sub", json!("other")),
        ] {
            let mut value = base.clone();
            value[key] = v;
            let t = serde_json::from_value(value).unwrap();
            assert!(binding(&c, &t, now()).is_none(), "{key}");
        }
        let limits = Limits::new(1);
        for _ in 0..60 {
            assert!(limits.admit(Some(0)))
        }
        assert!(!limits.admit(Some(0)));
        for _ in 0..120 {
            assert!(limits.admit(None))
        }
        assert!(!limits.admit(None));
    }
    #[test]
    fn transfer_id_headers_and_tunnel_are_narrow() {
        assert!(valid_artifact_id("12345678-1234-1234-1234-123456789abc"));
        assert!(!valid_artifact_id("../secrets"));
        let mut h = HeaderMap::new();
        h.insert(
            "if-match",
            format!("\"{}\"", "b".repeat(64)).parse().unwrap(),
        );
        h.insert("range", "bytes=0-99".parse().unwrap());
        assert!(transfer_headers(&h).is_some());
        h.insert("range", "bytes=0-2,5-8".parse().unwrap());
        assert!(transfer_headers(&h).is_none());
        let yaml = tunnel_config(
            &config(),
            &Cloudflare {
                tunnel_id: "fixture".into(),
                credentials_file: "/tmp/synthetic.json".into(),
            },
        )
        .unwrap();
        assert!(yaml.contains("127.0.0.1:47740"));
        assert!(!yaml.contains("47720"));
        assert!(yaml.ends_with("http_status:404\n"));
    }
    struct Fixture {
        active: AtomicBool,
        write: AtomicBool,
        transfer: AtomicBool,
        writes: AtomicUsize,
        scope: Mutex<String>,
    }
    async fn controlled_fixture(scope: &str) -> (App, Arc<Fixture>, tokio::task::JoinHandle<()>) {
        let fixture = Arc::new(Fixture {
            active: AtomicBool::new(true),
            write: AtomicBool::new(true),
            transfer: AtomicBool::new(true),
            writes: AtomicUsize::new(0),
            scope: Mutex::new(scope.to_owned()),
        });
        let router=Router::new().route("/introspect",post(|State(f):State<Arc<Fixture>>|async move{Json(json!({"active":f.active.load(Ordering::SeqCst),"exp":now()+3600,"aud":["https://mcp.example.com/mcp"],"iss":"https://auth.example.com","scope":f.scope.lock().unwrap().clone(),"client_id":"test","sub":"user"}))}))
            .route("/mcp",post(|State(f):State<Arc<Fixture>>,headers:HeaderMap,Json(msg):Json<Value>|async move{
                if headers.get("authorization").and_then(|h|h.to_str().ok())!=Some(format!("Bearer {}","a".repeat(64)).as_str()){return StatusCode::UNAUTHORIZED.into_response()}
                let result=if msg["method"]=="tools/list"{json!({"result":{"tools":[{"name":"download_status"},{"name":"download_enqueue"},{"name":"run_shell"}]}})}
                else{match msg["params"]["name"].as_str().unwrap_or(""){
                    "omniget_capabilities"=>capabilities(f.write.load(Ordering::SeqCst),f.transfer.load(Ordering::SeqCst)),
                    "download_enqueue"=>{f.writes.fetch_add(1,Ordering::SeqCst);json!({"result":{"structuredContent":{"accepted":true}}})},
                    "download_artifacts"=>json!({"result":{"structuredContent":{"artifacts":[{"transferAllowed":true,"access":{"download_path":"/mcp/artifacts/12345678-1234-1234-1234-123456789abc"},"unsafeEcho":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}]},"content":[{"type":"text","text":"unfiltered access descriptor"}]}}),
                    _=>json!({"result":{"structuredContent":{"ok":true}}})
                }};Json(result).into_response()
            }))
            .route("/mcp/artifacts/{id}",get(||async{
                let stream=futures::stream::unfold(0u8,|i|async move{if i>=2{return None}if i==1{tokio::time::sleep(Duration::from_millis(1300)).await;}Some((Ok::<_,std::io::Error>(vec![b'x';65536]),i+1))});
                Response::builder().header("content-length",131072).header("etag",format!("\"{}\"","b".repeat(64))).body(Body::from_stream(stream)).unwrap()
            })).with_state(fixture.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        // Local HTTP exists only for deterministic test transports; validate() rejects
        // this introspection endpoint in production configuration.
        let mut c = config();
        c.introspection_url = format!("http://{address}/introspect");
        c.desktop_url = format!("http://{address}/mcp");
        c.enable_remote_write = true;
        c.enable_artifact_transfer = true;
        c.bindings[0].tools.push("download_enqueue".into());
        let app = App {
            config: Arc::new(c),
            http: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .timeout(Duration::from_secs(10))
                .build()
                .unwrap(),
            limits: Arc::new(Limits::new(1)),
        };
        (app, fixture, handle)
    }
    fn headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("host", "mcp.example.com".parse().unwrap());
        h.insert(
            "authorization",
            "Bearer external-synthetic-token".parse().unwrap(),
        );
        h
    }
    async fn call(app: &App, name: &str) -> Response {
        mcp(State(app.clone()),headers(),Bytes::from(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":{}}}).to_string())).await
    }
    #[tokio::test]
    async fn controlled_oauth_scope_boundary_revocation_and_redaction() {
        let (app, fixture, server) = controlled_fixture(READ).await;
        assert_eq!(
            call(&app, "download_enqueue").await.status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(fixture.writes.load(Ordering::SeqCst), 0);
        let response = call(&app, "download_artifacts").await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        let text = value.to_string();
        assert!(
            !text.contains("download_path")
                && !text.contains(&"a".repeat(64))
                && !text.contains("unfiltered access descriptor")
        );
        *fixture.scope.lock().unwrap() = WRITE.into();
        assert_eq!(
            call(&app, "download_enqueue").await.status(),
            StatusCode::OK
        );
        assert_eq!(fixture.writes.load(Ordering::SeqCst), 1);
        fixture.write.store(false, Ordering::SeqCst);
        assert_eq!(
            call(&app, "download_enqueue").await.status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(fixture.writes.load(Ordering::SeqCst), 1);
        fixture.active.store(false, Ordering::SeqCst);
        assert_eq!(
            call(&app, "download_enqueue").await.status(),
            StatusCode::UNAUTHORIZED
        );
        server.abort();
    }
    async fn stream_revocation(revoke_oauth: bool) {
        let (app, fixture, server) = controlled_fixture(TRANSFER).await;
        let mut h = headers();
        h.insert(
            "if-match",
            format!("\"{}\"", "b".repeat(64)).parse().unwrap(),
        );
        let response = artifact(
            State(app),
            Path("12345678-1234-1234-1234-123456789abc".to_owned()),
            h,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let mut stream = response.into_body().into_data_stream();
        let mut received = 0;
        while received < 65536 {
            received += stream.next().await.unwrap().unwrap().len();
        }
        if revoke_oauth {
            fixture.active.store(false, Ordering::SeqCst)
        } else {
            fixture.transfer.store(false, Ordering::SeqCst)
        }
        assert!(stream.next().await.unwrap().is_err());
        server.abort();
    }
    #[tokio::test]
    async fn controlled_transfer_limit_is_released_when_client_drops() {
        let (app, _, server) = controlled_fixture(TRANSFER).await;
        let mut h = headers();
        h.insert(
            "if-match",
            format!("\"{}\"", "b".repeat(64)).parse().unwrap(),
        );
        let id = "12345678-1234-1234-1234-123456789abc".to_owned();
        let first = artifact(State(app.clone()), Path(id.clone()), h.clone()).await;
        let second = artifact(State(app.clone()), Path(id.clone()), h.clone()).await;
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(second.status(), StatusCode::OK);
        assert_eq!(
            artifact(State(app.clone()), Path(id.clone()), h.clone())
                .await
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        drop(first);
        assert_eq!(
            artifact(State(app), Path(id), h).await.status(),
            StatusCode::OK
        );
        drop(second);
        server.abort();
    }
    #[tokio::test]
    async fn controlled_stream_stops_after_oauth_revocation() {
        stream_revocation(true).await;
    }
    #[tokio::test]
    async fn controlled_stream_stops_after_desktop_grant_revocation() {
        stream_revocation(false).await;
    }
}
