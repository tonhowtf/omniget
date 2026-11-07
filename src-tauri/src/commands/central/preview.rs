use serde_json::Value;
use tauri::{AppHandle, Webview};

use crate::preview::{self};

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
