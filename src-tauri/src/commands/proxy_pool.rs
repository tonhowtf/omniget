use serde_json::{json, Value};

use omniget_core::core::devtools::DevError;
use omniget_core::core::proxy_pool::{self, ProxyError, Rotation};

pub const DEFAULT_ROWS: usize = 100;

pub fn wire(error: ProxyError) -> String {
    match error {
        ProxyError::Save(message) => DevError::Write {
            path: "network/proxies.json".into(),
            message,
        },
        ProxyError::Read { path, message } => DevError::Read { path, message },
        other => DevError::InvalidInput(other.to_string()),
    }
    .wire()
}

async fn stored<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ProxyError> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
        .map_err(wire)
}

pub fn status(limit: usize) -> Value {
    serde_json::to_value(proxy_pool::view(limit)).unwrap_or_default()
}

fn with_pool(mut extra: Value, limit: usize) -> Value {
    if let (Some(out), Value::Object(view)) = (extra.as_object_mut(), status(limit)) {
        out.extend(view);
    }
    extra
}

pub async fn add(inputs: Vec<String>, limit: usize) -> Result<Value, String> {
    let added = stored(move || proxy_pool::add(&inputs)).await?;
    Ok(with_pool(json!(added), limit))
}

pub async fn import(file: String, limit: usize) -> Result<Value, String> {
    let added = stored(move || proxy_pool::import(std::path::Path::new(&file))).await?;
    Ok(with_pool(json!(added), limit))
}

pub async fn remove(ids: Vec<String>, failing: bool, limit: usize) -> Result<Value, String> {
    let removed = stored(move || {
        let mut removed = proxy_pool::remove(&ids)?;
        if failing {
            removed += proxy_pool::remove_failing()?;
        }
        Ok(removed)
    })
    .await?;
    Ok(with_pool(json!({ "removed": removed }), limit))
}

pub async fn set(
    enabled: Option<bool>,
    rotation: Option<Rotation>,
    limit: usize,
) -> Result<Value, String> {
    stored(move || proxy_pool::configure(enabled, rotation)).await?;
    Ok(status(limit))
}

pub async fn check(ids: Vec<String>, url: Option<String>, limit: usize) -> Result<Value, String> {
    let target = match url.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        Some(raw) => {
            Some(omniget_core::core::devtools::http::checked_url(raw).map_err(|e| e.wire())?)
        }
        None => None,
    };
    let run = proxy_pool::check(&ids, target).await;
    let mut out = with_pool(json!(run), limit);
    if run.unchecked > 0 && ids.is_empty() {
        out["next_step"] = json!(
            "one batch was tested; call op=check again to test the proxies that were never tested"
        );
    }
    Ok(out)
}

pub fn rotate(limit: usize) -> Value {
    with_pool(json!({ "movedTo": proxy_pool::rotate() }), limit)
}

pub async fn detect() -> Value {
    let found = proxy_pool::detect().await;
    let mut out = json!({ "found": found, "total": found.len() });
    if found.is_empty() {
        out["reason"] = json!("no proxy app answered on this computer and no system or environment proxy is set; start Clash, v2rayN or a similar app, or add a proxy address by hand");
    }
    out
}

#[tauri::command]
pub async fn proxy_pool_status() -> Value {
    status(proxy_pool::MAX_PROXIES)
}

#[tauri::command]
pub async fn proxy_pool_add(text: String) -> Result<Value, String> {
    let lines = text.lines().map(str::to_string).collect();
    add(lines, proxy_pool::MAX_PROXIES).await
}

#[tauri::command]
pub async fn proxy_pool_remove(ids: Vec<String>, failing: bool) -> Result<Value, String> {
    remove(ids, failing, proxy_pool::MAX_PROXIES).await
}

#[tauri::command]
pub async fn proxy_pool_set(
    enabled: Option<bool>,
    rotation: Option<Rotation>,
) -> Result<Value, String> {
    set(enabled, rotation, proxy_pool::MAX_PROXIES).await
}

#[tauri::command]
pub async fn proxy_pool_check(ids: Vec<String>, url: Option<String>) -> Result<Value, String> {
    check(ids, url, proxy_pool::MAX_PROXIES).await
}

#[tauri::command]
pub async fn proxy_pool_detect() -> Value {
    detect().await
}
