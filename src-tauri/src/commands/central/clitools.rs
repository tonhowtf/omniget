use omniget_core::core::clitools::{self as ct, acp_registry, detect};
use serde_json::{json, Value};

fn to_value<T: serde::Serialize>(v: &T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| format!("{}: {e}", ct::ERR_FAILED))
}

#[tauri::command]
pub async fn clitools_list() -> Result<Value, String> {
    to_value(&json!({
        "tools": ct::list(),
        "platform": acp_registry::platform_key(),
        "detected": detect::cached().is_some(),
    }))
}

#[tauri::command]
pub async fn clitools_detect(force: Option<bool>) -> Result<Value, String> {
    let force = force.unwrap_or(false);
    let list = tauri::async_runtime::spawn_blocking(move || {
        tauri::async_runtime::block_on(detect::detect_all(force))
    })
    .await
    .map_err(|e| e.to_string())?;
    to_value(&*list)
}
