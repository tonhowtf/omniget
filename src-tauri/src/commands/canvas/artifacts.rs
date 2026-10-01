use omniget_core::core::canvas::model::Point;
use serde_json::Value;

use crate::canvas::artifacts;

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("CANVAS_TASK: {e}"))?
}

#[tauri::command]
pub async fn canvas_artifact_list(workspace_id: String) -> Result<Vec<Value>, String> {
    blocking(move || {
        let list = artifacts::list(&workspace_id)?;
        Ok(list
            .iter()
            .map(|m| artifacts::view(&workspace_id, m))
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn canvas_artifact_open(
    workspace_id: String,
    floor_id: String,
    artifact_id: String,
    at: Option<Point>,
) -> Result<Value, String> {
    blocking(move || {
        let m = artifacts::get(&workspace_id, &artifact_id)?;
        artifacts::place(&workspace_id, &floor_id, &m, at, None)
    })
    .await
}
