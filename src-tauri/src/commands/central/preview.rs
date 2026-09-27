//! Tauri commands of the thread Browser tab and the SnapShot capture. The
//! logic lives in `crate::preview`; see that module for the window model.

use serde_json::Value;
use tauri::{AppHandle, Manager, Webview};

use crate::preview::{self, capture, ports, PreviewState, Rect};

/// Dev servers listening from inside `cwd` (with `all`, every local listener,
/// flagged `inWorkspace`). Each one probed for HTML.
#[tauri::command]
pub async fn preview_discover_ports(
    cwd: Option<String>,
    all: Option<bool>,
) -> Result<Vec<ports::DevServer>, String> {
    let root = cwd
        .filter(|c| !c.trim().is_empty())
        .map(std::path::PathBuf::from);
    ports::discover(root.as_deref(), all.unwrap_or(false)).await
}

/// Opens or reuses the thread's preview; with `rect` it is shown over it.
#[tauri::command]
pub fn preview_open(
    app: AppHandle,
    thread_id: String,
    url: Option<String>,
    rect: Option<Rect>,
) -> Result<PreviewState, String> {
    preview::open(&app, &thread_id, url.as_deref(), rect)
}

/// The tab moved or resized (CSS px of the main webview + devicePixelRatio).
#[tauri::command]
pub fn preview_set_bounds(app: AppHandle, thread_id: String, rect: Rect) -> Result<(), String> {
    preview::set_bounds(&app, &thread_id, rect)
}

#[tauri::command]
pub fn preview_hide(app: AppHandle, thread_id: Option<String>) -> Result<(), String> {
    match thread_id {
        Some(t) => preview::hide(&app, &t),
        None => {
            preview::hide_all(&app);
            Ok(())
        }
    }
}

#[tauri::command]
pub fn preview_close(app: AppHandle, thread_id: String) -> Result<(), String> {
    preview::close(&app, &thread_id)
}

#[tauri::command]
pub async fn preview_navigate(
    app: AppHandle,
    thread_id: String,
    url: String,
) -> Result<PreviewState, String> {
    let label = preview::label_for(&thread_id);
    if app.get_webview_window(&label).is_none() {
        return preview::open(&app, &thread_id, Some(&url), None);
    }
    preview::navigate(&app, &label, &url).await
}

/// `action`: back | forward | reload.
#[tauri::command]
pub async fn preview_history(
    app: AppHandle,
    thread_id: String,
    action: String,
) -> Result<PreviewState, String> {
    preview::history(&app, &preview::label_for(&thread_id), &action).await
}

#[tauri::command]
pub fn preview_state(thread_id: Option<String>) -> Result<Vec<PreviewState>, String> {
    let m = preview::manager();
    Ok(match thread_id {
        Some(t) => m.state(&t).into_iter().collect(),
        None => m.states(),
    })
}

/// PNG of the thread's preview, for the composer.
#[tauri::command]
pub async fn preview_screenshot(
    app: AppHandle,
    thread_id: String,
) -> Result<capture::Captured, String> {
    preview::screenshot(&app, &preview::label_for(&thread_id)).await
}

/// Answers from the script injected in a preview page. Only preview webviews
/// are heard; the label comes from Tauri, not from the page.
#[tauri::command]
pub fn preview_report(
    app: AppHandle,
    webview: Webview,
    kind: String,
    id: Option<String>,
    data: Option<Value>,
) {
    preview::report(
        &app,
        webview.label(),
        &kind,
        id,
        data.unwrap_or(Value::Null),
    );
}

/// Windows of other apps for SnapShot (`screen` first).
#[tauri::command]
pub async fn snapshot_list_windows() -> Result<capture::WindowList, String> {
    capture::list_windows().await
}

/// Captures a window (or `screen`) to `<app_data>/snapshots/*.png`.
#[tauri::command]
pub async fn snapshot_capture(window_id: String) -> Result<capture::Captured, String> {
    capture::capture_window(&window_id).await
}

/// macOS: asks for Screen Recording. Returns whether it is granted now.
#[tauri::command]
pub fn snapshot_request_permission() -> bool {
    capture::request_permission()
}
