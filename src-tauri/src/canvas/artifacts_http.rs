use std::net::SocketAddr;
use std::path::{Component, PathBuf};
use std::sync::{Mutex, OnceLock};

use axum::body::Body;
use axum::extract::Path as UrlPath;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};

const CONFIG_FILE: &str = "canvas-artifacts-server.json";
const MAX_FILE: u64 = 20 * 1024 * 1024;
const CSP: &str = "default-src 'self' https: data: blob: 'unsafe-inline' 'unsafe-eval'; connect-src 'self' https: data: blob:; img-src 'self' https: data: blob:; media-src 'self' https: data: blob:; frame-ancestors 'none'; form-action 'self' https:; base-uri 'self'";

#[derive(Clone, Serialize, Deserialize)]
struct Config {
    port: u16,
    token: String,
}

static RUNNING: OnceLock<Config> = OnceLock::new();
static START: Mutex<()> = Mutex::new(());

fn config_path() -> Option<PathBuf> {
    omniget_core::core::paths::app_data_dir().map(|d| d.join(CONFIG_FILE))
}

fn saved() -> Option<Config> {
    let raw = std::fs::read(config_path()?).ok()?;
    let c: Config = serde_json::from_slice(&raw).ok()?;
    (c.token.len() == 32 && c.token.bytes().all(|b| b.is_ascii_hexdigit())).then_some(c)
}

fn persist(c: &Config) {
    let Some(p) = config_path() else { return };
    if let Ok(body) = serde_json::to_vec(c) {
        if std::fs::write(&p, body).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
            }
        }
    }
}

fn bind(port: u16) -> Option<std::net::TcpListener> {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let l = std::net::TcpListener::bind(addr).ok()?;
    l.set_nonblocking(true).ok()?;
    Some(l)
}

pub fn ensure() -> Result<(u16, String), String> {
    if let Some(c) = RUNNING.get() {
        return Ok((c.port, c.token.clone()));
    }
    let _g = START.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = RUNNING.get() {
        return Ok((c.port, c.token.clone()));
    }
    let previous = saved();
    let (listener, token) = match previous
        .as_ref()
        .and_then(|c| bind(c.port).map(|l| (l, c.token.clone())))
    {
        Some(x) => x,
        None => (
            bind(0).ok_or("CANVAS_IO: cannot open the artifact server")?,
            previous
                .map(|c| c.token)
                .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string()),
        ),
    };
    let port = listener
        .local_addr()
        .map_err(|e| format!("CANVAS_IO: {e}"))?
        .port();
    let cfg = Config {
        port,
        token: token.clone(),
    };
    persist(&cfg);
    let router = Router::new().route("/a/{token}/{ws}/{id}/{*file}", get(serve));
    tauri::async_runtime::spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("[artifacts] server failed to start: {e}");
                return;
            }
        };
        if let Err(e) = axum::serve(listener, router).await {
            tracing::warn!("[artifacts] server stopped: {e}");
        }
    });
    let _ = RUNNING.set(cfg);
    tracing::info!("[artifacts] serving workspace artifacts on http://127.0.0.1:{port}");
    Ok((port, token))
}

pub fn url_for(ws: &str, id: &str, file: &str) -> Result<String, String> {
    let (port, token) = ensure()?;
    Ok(format!(
        "http://127.0.0.1:{port}/a/{token}/{}/{id}/{file}",
        urlencoding::encode(ws)
    ))
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, "not found").into_response()
}

fn mime(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("md" | "txt") => "text/plain; charset=utf-8",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

async fn serve(
    headers: HeaderMap,
    UrlPath((token, ws, id, file)): UrlPath<(String, String, String, String)>,
) -> Response {
    let Some(cfg) = RUNNING.get() else {
        return not_found();
    };
    if !same(&token, &cfg.token) {
        return not_found();
    }
    let host_ok = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|h| {
            h == format!("127.0.0.1:{}", cfg.port) || h == format!("localhost:{}", cfg.port)
        });
    if !host_ok {
        return not_found();
    }
    if !omniget_core::core::canvas::model::is_safe_id(&ws) || !super::artifacts::valid_id(&id) {
        return not_found();
    }
    let rel = PathBuf::from(&file);
    let clean = rel.components().all(|c| match c {
        Component::Normal(p) => !p.to_string_lossy().starts_with('.'),
        _ => false,
    });
    if !clean || file == "meta.json" || file.len() > 512 {
        return not_found();
    }
    let s = crate::canvas::doc::host().store().clone();
    if s.workspace(&ws).is_err() {
        return not_found();
    }
    let base = super::artifacts::root(&s, &ws).join(&id);
    let found = tokio::task::spawn_blocking(move || {
        let base = base.canonicalize().ok()?;
        let target = base.join(rel).canonicalize().ok()?;
        if !target.starts_with(&base) {
            return None;
        }
        let meta = std::fs::metadata(&target).ok()?;
        if !meta.is_file() || meta.len() > MAX_FILE {
            return None;
        }
        let bytes = std::fs::read(&target).ok()?;
        Some((mime(&target), bytes))
    })
    .await
    .ok()
    .flatten();
    let Some((ct, bytes)) = found else {
        return not_found();
    };
    let mut r = Response::new(Body::from(bytes));
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    r
}
