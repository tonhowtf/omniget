//! Tauri commands of the canvas clipboard (⌘C / ⌘X / ⌘V and the menus).
//! Logic in `crate::canvas::clipboard`.

use omniget_core::core::canvas::clipboard::ClipSelection;
use omniget_core::core::canvas::model::Point;
use serde_json::Value;
use tauri::AppHandle;

use crate::canvas::clipboard::{self, Where};

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("CANVAS_TASK: {e}"))?
}

#[tauri::command]
pub async fn canvas_clipboard_copy(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    node_ids: Vec<String>,
    drawing_ids: Option<Vec<String>>,
    cut: Option<bool>,
) -> Result<Value, String> {
    let sel = ClipSelection {
        node_ids,
        drawing_ids: drawing_ids.unwrap_or_default(),
    };
    blocking(move || clipboard::copy(&app, &workspace_id, &floor_id, &sel, cut.unwrap_or(false)))
        .await
}

/// `at`: top-left of the paste (right-click Paste). `center`: the view centre
/// (⌘V), with the cascade of repeated pastes.
#[tauri::command]
pub async fn canvas_clipboard_paste(
    app: AppHandle,
    workspace_id: String,
    floor_id: String,
    at: Option<Point>,
    center: Option<Point>,
) -> Result<Value, String> {
    blocking(move || clipboard::paste(&app, &workspace_id, &floor_id, Where { at, center })).await
}

#[tauri::command]
pub async fn canvas_clipboard_read(app: AppHandle) -> Result<Value, String> {
    blocking(move || Ok(clipboard::read(&app))).await
}
