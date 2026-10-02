use serde_json::{json, Value};
use tauri::AppHandle;

use crate::device;

#[tauri::command]
pub async fn canvas_device_list() -> Result<Value, String> {
    Ok(device::list().await)
}

#[tauri::command]
pub async fn canvas_device_open(
    app: AppHandle,
    node_id: String,
    platform: String,
    device: Option<String>,
) -> Result<Value, String> {
    let p = device::Platform::parse(&platform)?;
    let s = device::open(&app, &node_id, p, device.as_deref()).await?;
    device::remember(&s);
    Ok(s.info())
}

#[tauri::command]
pub fn canvas_device_state(node_id: String) -> Option<Value> {
    device::get(&node_id).map(|s| s.info())
}

#[tauri::command]
pub async fn canvas_device_watch(node_id: String, on: bool) -> Result<(), String> {
    let Some(s) = device::get(&node_id) else {
        return Ok(());
    };
    s.watch(on);
    if on {
        let _ = s.capture().await;
    }
    Ok(())
}

#[tauri::command]
pub async fn canvas_device_act(
    node_id: String,
    action: String,
    args: Option<Value>,
) -> Result<Value, String> {
    let s = device::need(&node_id)?;
    let mut r = device::act(&s, &action, &args.unwrap_or_else(|| json!({}))).await?;
    r["ok"] = json!(true);
    Ok(r)
}

#[tauri::command]
pub async fn canvas_device_close(node_id: String) -> Result<Value, String> {
    let r = device::close(&node_id, true).await;
    device::mark_closed(&node_id);
    r
}
