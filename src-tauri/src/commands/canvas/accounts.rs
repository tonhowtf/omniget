use serde_json::{json, Value};
use tauri::AppHandle;

use crate::portal::sessions::{self, PortalSession};

#[tauri::command]
pub async fn canvas_accounts_overview() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        json!({
            "accounts": crate::canvas::accounts::refs(),
            "sessions": sessions::list(),
            "namedSessionsSupported": sessions::supported(),
            "running": crate::canvas::accounts::running(),
        })
    })
    .await
    .map_err(|e| format!("CANVAS_TASK: {e}"))
}

#[tauri::command]
pub fn canvas_portal_session_create(
    label: String,
    color: Option<String>,
) -> Result<PortalSession, String> {
    sessions::create(&label, color)
}

#[tauri::command]
pub fn canvas_portal_session_update(
    id: String,
    label: Option<String>,
    color: Option<String>,
) -> Result<PortalSession, String> {
    sessions::update(&id, label.as_deref(), color.map(Some))
}

#[tauri::command]
pub fn canvas_portal_session_usage(id: String) -> Vec<Value> {
    sessions::users(&id)
}

#[tauri::command]
pub async fn canvas_portal_session_delete(app: AppHandle, id: String) -> Result<Value, String> {
    crate::portal::bind(&app);
    sessions::delete(&app, &id).await
}
