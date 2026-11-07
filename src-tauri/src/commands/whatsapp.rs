use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use serde_json::Value;
use tauri::{AppHandle, Emitter};

use omniget_whatsapp::{
    ChatRow, DownloadJob, ExportFormat, ExportLabels, MediaKind, MediaStats, MessageRow, SavedFile,
    Status, WhatsApp,
};

struct AppHost {
    app: AppHandle,
}

impl omniget_whatsapp::Host for AppHost {
    fn emit(&self, event: &str, payload: Value) {
        let _ = self.app.emit(event, payload);
    }
}

static INSTANCE: OnceLock<Arc<WhatsApp>> = OnceLock::new();

fn wa(app: &AppHandle) -> Result<Arc<WhatsApp>, String> {
    if let Some(w) = INSTANCE.get() {
        return Ok(w.clone());
    }
    let dir = omniget_core::core::paths::app_data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("whatsapp");
    let w =
        WhatsApp::new(dir, Arc::new(AppHost { app: app.clone() })).map_err(|e| format!("{e:#}"))?;
    Ok(INSTANCE.get_or_init(|| w).clone())
}

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

fn base_dir(app: &AppHandle, out_dir: Option<String>) -> PathBuf {
    match out_dir.filter(|d| !d.trim().is_empty()) {
        Some(d) => PathBuf::from(d),
        None => {
            crate::storage::config::load_settings(app)
                .download
                .default_output_dir
        }
    }
}

#[tauri::command]
pub async fn whatsapp_status(app: AppHandle) -> Result<Status, String> {
    Ok(wa(&app)?.status())
}

#[tauri::command]
pub async fn whatsapp_connect(app: AppHandle) -> Result<Status, String> {
    wa(&app)?.connect().await.map_err(err)
}

#[tauri::command]
pub async fn whatsapp_disconnect(app: AppHandle) -> Result<Status, String> {
    wa(&app)?.disconnect().await.map_err(err)
}

#[tauri::command]
pub async fn whatsapp_pair_phone(app: AppHandle, phone: String) -> Result<String, String> {
    wa(&app)?.pair_phone(&phone).await.map_err(err)
}

#[tauri::command]
pub async fn whatsapp_logout(app: AppHandle) -> Result<Status, String> {
    wa(&app)?.logout().await.map_err(err)
}

#[tauri::command]
pub async fn whatsapp_chats(app: AppHandle, query: Option<String>) -> Result<Vec<ChatRow>, String> {
    wa(&app)?.chats(query.as_deref()).map_err(err)
}

#[tauri::command]
pub async fn whatsapp_messages(
    app: AppHandle,
    chat: String,
    before: Option<i64>,
    limit: Option<u32>,
) -> Result<Vec<MessageRow>, String> {
    wa(&app)?.messages(&chat, before, limit).map_err(err)
}

#[tauri::command]
pub async fn whatsapp_mark_read(app: AppHandle, chat: String) -> Result<usize, String> {
    wa(&app)?.mark_read(&chat).await.map_err(err)
}

#[tauri::command]
pub async fn whatsapp_send_text(
    app: AppHandle,
    chat: String,
    text: String,
) -> Result<MessageRow, String> {
    wa(&app)?.send_text(&chat, &text).await.map_err(err)
}

#[tauri::command]
pub async fn whatsapp_request_older(app: AppHandle, chat: String) -> Result<(), String> {
    wa(&app)?.request_older(&chat).await.map_err(err)
}

#[tauri::command]
pub async fn whatsapp_media_stats(app: AppHandle, chat: String) -> Result<MediaStats, String> {
    wa(&app)?.media_stats(&chat).map_err(err)
}

#[tauri::command]
pub async fn whatsapp_default_dir(app: AppHandle) -> Result<String, String> {
    Ok(base_dir(&app, None).to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn whatsapp_download_media(
    app: AppHandle,
    chat: String,
    message_id: String,
    out_dir: Option<String>,
) -> Result<SavedFile, String> {
    let base = base_dir(&app, out_dir);
    wa(&app)?
        .download_message(&chat, &message_id, &base)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn whatsapp_download_chat_media(
    app: AppHandle,
    chat: String,
    kinds: Vec<MediaKind>,
    out_dir: Option<String>,
) -> Result<DownloadJob, String> {
    let base = base_dir(&app, out_dir);
    wa(&app)?
        .download_chat(&chat, &kinds, &base)
        .await
        .map_err(err)
}

#[tauri::command]
pub async fn whatsapp_cancel_download(app: AppHandle, job_id: String) -> Result<bool, String> {
    Ok(wa(&app)?.cancel_download(&job_id))
}

#[tauri::command]
pub async fn whatsapp_export_name(
    app: AppHandle,
    chat: String,
    format: ExportFormat,
) -> Result<String, String> {
    wa(&app)?.suggested_export_name(&chat, format).map_err(err)
}

#[tauri::command]
pub async fn whatsapp_export(
    app: AppHandle,
    chat: String,
    format: ExportFormat,
    path: String,
    labels: Option<ExportLabels>,
) -> Result<usize, String> {
    let labels = labels.unwrap_or_default();
    let w = wa(&app)?;
    let target = PathBuf::from(path);
    tokio::task::spawn_blocking(move || w.export(&chat, format, &target, &labels))
        .await
        .map_err(|e| e.to_string())?
        .map_err(err)
}
