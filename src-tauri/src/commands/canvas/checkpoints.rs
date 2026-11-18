use serde_json::Value;
use tauri::AppHandle;

use crate::canvas::{checkpoints, cords};

fn folder(app: &AppHandle, node_id: &str) -> Result<std::path::PathBuf, String> {
    cords::host()
        .bind(app)
        .node_cwd(node_id)
        .ok_or_else(|| format!("CANVAS_NOT_FOUND: node {node_id} has no folder"))
}

#[tauri::command]
pub async fn canvas_checkpoints(app: AppHandle, node_id: String) -> Result<Value, String> {
    let cwd = folder(&app, &node_id)?;
    checkpoints::list(&node_id, &cwd).await
}

#[tauri::command]
pub async fn canvas_checkpoint_changes(
    app: AppHandle,
    node_id: String,
    turn: u32,
) -> Result<Value, String> {
    let cwd = folder(&app, &node_id)?;
    checkpoints::changes(&node_id, &cwd, turn).await
}

#[tauri::command]
pub async fn canvas_checkpoint_restore(
    app: AppHandle,
    node_id: String,
    turn: u32,
) -> Result<Value, String> {
    let cwd = folder(&app, &node_id)?;
    let done = checkpoints::restore(&node_id, &cwd, turn).await?;
    serde_json::to_value(done).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn canvas_checkpoint_undo(app: AppHandle, node_id: String) -> Result<Value, String> {
    let cwd = folder(&app, &node_id)?;
    let done = checkpoints::undo_restore(&node_id, &cwd).await?;
    serde_json::to_value(done).map_err(|e| e.to_string())
}
