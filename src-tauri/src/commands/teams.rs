//! Microsoft 365 (Teams) in the Meetings screen: set the app registration,
//! sign in with a device code, export chats, list calendar meetings and
//! import official transcripts. The work is `omniget_core::core::msgraph`,
//! shared with `omniget teams` and the MCP tools.

use std::sync::Mutex;

use omniget_core::core::msgraph::{self, calendar, chats, transcripts, DeviceCode};
use tauri::{AppHandle, Emitter};

/// The device code between "show it" and "wait for it".
static PENDING: Mutex<Option<DeviceCode>> = Mutex::new(None);

fn e(err: anyhow::Error) -> String {
    format!("{err:#}")
}

#[tauri::command]
pub fn teams_status() -> msgraph::Status {
    msgraph::status()
}

#[tauri::command]
pub fn teams_configure(
    client_id: String,
    tenant: String,
    admin_scopes: bool,
) -> Result<msgraph::Status, String> {
    msgraph::configure(&client_id, &tenant, admin_scopes).map_err(e)?;
    Ok(msgraph::status())
}

/// Step 1: the code and the page to type it in.
#[tauri::command]
pub async fn teams_sign_in_start() -> Result<DeviceCode, String> {
    let code = msgraph::start_sign_in().await.map_err(e)?;
    *PENDING.lock().unwrap_or_else(|p| p.into_inner()) = Some(code.clone());
    Ok(code)
}

/// Step 2: waits until the person finished in the browser.
#[tauri::command]
pub async fn teams_sign_in_finish() -> Result<msgraph::Status, String> {
    let code = PENDING
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
        .ok_or("Start the sign-in first")?;
    msgraph::finish_sign_in(&code).await.map_err(e)?;
    Ok(msgraph::status())
}

#[tauri::command]
pub fn teams_sign_out() -> Result<msgraph::Status, String> {
    msgraph::sign_out().map_err(e)?;
    Ok(msgraph::status())
}

#[tauri::command]
pub async fn teams_chats(
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<chats::ChatInfo>, String> {
    chats::list_chats(limit.unwrap_or(50), query.as_deref())
        .await
        .map_err(e)
}

#[derive(serde::Serialize)]
pub struct ExportResult {
    pub folder: String,
    pub exported: Vec<chats::Exported>,
    pub errors: Vec<String>,
}

/// Exports the chosen chats (Markdown, or JSON) into the export folder.
#[tauri::command]
pub async fn teams_export(
    chat_ids: Vec<String>,
    since: Option<String>,
    json: bool,
) -> Result<ExportResult, String> {
    let since = since
        .filter(|s| !s.trim().is_empty())
        .map(|s| msgraph::parse_since(&s))
        .transpose()
        .map_err(e)?;
    let dir = chats::export_dir().map_err(e)?;
    let mut exported = Vec::new();
    let mut errors = Vec::new();
    for id in chat_ids {
        match chats::export_chat(&id, since, json, &dir).await {
            Ok(x) => exported.push(x),
            Err(err) => errors.push(format!("{id}: {err:#}")),
        }
    }
    Ok(ExportResult {
        folder: dir.to_string_lossy().to_string(),
        exported,
        errors,
    })
}

#[tauri::command]
pub async fn teams_calendar(
    days: Option<i64>,
    past_days: Option<i64>,
) -> Result<Vec<calendar::CalendarMeeting>, String> {
    calendar::meetings(past_days.unwrap_or(1), days.unwrap_or(7), true)
        .await
        .map_err(e)
}

#[tauri::command]
pub async fn teams_transcript_import(
    app: AppHandle,
    join_url: String,
    title: Option<String>,
) -> Result<omniget_core::core::meetings::Meeting, String> {
    let app2 = app.clone();
    let progress: omniget_core::core::tools::ProgressFn =
        std::sync::Arc::new(move |p: omniget_core::core::tools::ToolProgress| {
            if p.stage == "status" {
                let _ = app2.emit("meetings://changed", serde_json::json!({ "id": p.id }));
            }
        });
    let m = transcripts::import(&join_url, title.as_deref(), progress)
        .await
        .map_err(e)?;
    let _ = app.emit("meetings://changed", serde_json::json!({ "id": m.id }));
    Ok(m)
}
