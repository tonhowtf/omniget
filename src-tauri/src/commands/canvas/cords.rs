use serde_json::Value;
use tauri::AppHandle;

use crate::canvas::cords::{host, NodeEnv};

#[tauri::command]
pub async fn canvas_terminal_env(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    node_id: String,
) -> Result<NodeEnv, String> {
    tauri::async_runtime::spawn_blocking(move || {
        host().terminal_env(&app, &workspace_id, &floor_id, &node_id)
    })
    .await
    .map_err(|e| format!("CANVAS_TASK: {e}"))?
}

#[tauri::command]
pub async fn canvas_cord_test(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    connection_id: String,
) -> Result<Value, String> {
    let h = host().bind(&app);
    tauri::async_runtime::spawn_blocking(move || {
        h.cord_test(&app, &workspace_id, &floor_id, &connection_id)
    })
    .await
    .map_err(|e| format!("CANVAS_TASK: {e}"))?
}

#[tauri::command]
pub fn canvas_asks_list(app: AppHandle, workspace_id: Option<String>) -> Vec<Value> {
    host().bind(&app).asks_list(workspace_id.as_deref())
}

#[tauri::command]
pub fn canvas_inbox(app: AppHandle, workspace_id: Option<String>) -> Vec<Value> {
    host().bind(&app).inbox(workspace_id.as_deref())
}

#[tauri::command]
pub fn canvas_mark_seen(app: AppHandle, node_id: String) {
    host().bind(&app).mark_seen(&node_id);
}

#[tauri::command]
pub async fn canvas_cord_link(
    app: AppHandle,
    from_node_id: String,
    to_node_id: String,
    perms: Option<Vec<String>>,
) -> Result<Value, String> {
    crate::canvas::doc::host().bind(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let s = crate::canvas::doc::host().store().clone();
        let end = |id: &str| -> Result<crate::canvas::link::End, String> {
            let (ws, floor) = s
                .locate_node(id)
                .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {id}"))?;
            let node = s
                .read(&ws, &floor, |d| d.node(id).cloned())?
                .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {id}"))?;
            Ok(crate::canvas::link::End { ws, floor, node })
        };
        let (a, b) = (end(&from_node_id)?, end(&to_node_id)?);
        crate::canvas::link::link(&a, &b, &perms.unwrap_or_default(), "user")
    })
    .await
    .map_err(|e| format!("CANVAS_TASK: {e}"))?
}
