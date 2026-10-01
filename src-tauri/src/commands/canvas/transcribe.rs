use serde_json::{json, Value};
use tauri::AppHandle;

#[tauri::command]
pub async fn canvas_audio_extract(
    app: AppHandle,
    workspace_id: String,
    path: String,
    format: Option<String>,
) -> Result<Value, String> {
    let mut a = json!({ "workspaceId": workspace_id, "path": path });
    if let Some(f) = format {
        a["format"] = json!(f);
    }
    crate::mcp::canvas::media_tool(&app, "canvas_media_to_audio", a).await
}

#[tauri::command]
pub async fn canvas_transcribe_start(
    app: AppHandle,
    workspace_id: String,
    path: String,
    summarize: Option<bool>,
) -> Result<Value, String> {
    let a = json!({
        "workspaceId": workspace_id,
        "path": path,
        "summarize": if summarize.unwrap_or(true) { "agent" } else { "none" },
    });
    crate::mcp::canvas::media_tool(&app, "canvas_transcribe", a).await
}

#[tauri::command]
pub async fn canvas_transcribe_job(app: AppHandle, job_id: String) -> Result<Value, String> {
    crate::mcp::canvas::media_tool(&app, "canvas_transcribe_status", json!({ "jobId": job_id }))
        .await
}
