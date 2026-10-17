use std::sync::Arc;

use base64::Engine;
use omniget_core::core::canvas::chat::{self, ChatEntry, HistoryPage};
use omniget_core::core::canvas::store::CanvasStore;
use tauri::AppHandle;

fn store(app: &AppHandle) -> Arc<CanvasStore> {
    crate::canvas::chat::bind(app);
    crate::canvas::doc::host().bind(app).store().clone()
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("CANVAS_TASK: {e}"))?
}

#[tauri::command]
pub async fn canvas_chat_history(
    app: AppHandle,
    workspace_id: String,
    node_id: String,
    thread: Option<u8>,
    before: Option<String>,
    limit: Option<usize>,
    peek: Option<bool>,
) -> Result<HistoryPage, String> {
    let s = store(&app);
    blocking(move || {
        let peek = peek.unwrap_or(false);
        let page = chat::history_opts(
            &s,
            &workspace_id,
            &node_id,
            thread,
            before.as_deref(),
            limit,
            peek,
        )?;
        if !peek && page.unread[page.thread as usize] > 0 {
            crate::canvas::chat::emit_state(
                &workspace_id,
                &node_id,
                &chat::load_state(&s, &workspace_id, &node_id),
            );
        }
        Ok(page)
    })
    .await
}

#[tauri::command]
pub async fn canvas_chat_select_thread(
    app: AppHandle,
    workspace_id: String,
    node_id: String,
    thread: u8,
) -> Result<chat::ChatState, String> {
    let _ = store(&app);
    blocking(move || crate::canvas::chat::select(&workspace_id, &node_id, thread)).await
}

#[tauri::command]
pub async fn canvas_chat_mark_read(
    app: AppHandle,
    workspace_id: String,
    node_id: String,
    thread: Option<u8>,
) -> Result<chat::ChatState, String> {
    let _ = store(&app);
    blocking(move || crate::canvas::chat::mark_read(&workspace_id, &node_id, thread)).await
}

#[tauri::command]
pub async fn canvas_chat_interrupt(
    app: AppHandle,
    workspace_id: String,
    node_id: String,
) -> Result<Option<ChatEntry>, String> {
    let _ = store(&app);
    blocking(move || crate::canvas::chat::interrupt(&workspace_id, &node_id)).await
}

#[tauri::command]
pub async fn canvas_chat_search(
    app: AppHandle,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<chat::SearchHit>, String> {
    let _ = store(&app);
    blocking(move || Ok(crate::canvas::chat::search(&query, limit))).await
}

#[tauri::command]
pub async fn canvas_chat_send(
    app: AppHandle,
    workspace_id: String,
    node_id: String,
    thread: u8,
    text: String,
    attachments: Option<Vec<String>>,
    client_message_id: Option<String>,
) -> Result<ChatEntry, String> {
    let _ = store(&app);
    blocking(move || {
        crate::canvas::chat::send(
            &workspace_id,
            &node_id,
            thread,
            &text,
            &attachments.unwrap_or_default(),
            client_message_id,
        )
    })
    .await
}

#[tauri::command]
pub async fn canvas_chat_clear(
    app: AppHandle,
    workspace_id: String,
    node_id: String,
    thread: u8,
) -> Result<ChatEntry, String> {
    let s = store(&app);
    blocking(move || {
        let e = chat::clear(&s, &workspace_id, &node_id, thread)?;
        crate::canvas::chat::emit_entry(&workspace_id, &node_id, &e);
        Ok(e)
    })
    .await
}

#[tauri::command]
pub async fn canvas_chat_delete_entry(
    app: AppHandle,
    workspace_id: String,
    node_id: String,
    entry_id: String,
    thread: Option<u8>,
) -> Result<chat::Removed, String> {
    let _ = store(&app);
    blocking(move || crate::canvas::chat::delete_entry(&workspace_id, &node_id, &entry_id, thread))
        .await
}

#[tauri::command]
pub async fn canvas_chat_wipe(
    app: AppHandle,
    workspace_id: String,
    node_id: String,
    thread: Option<u8>,
) -> Result<chat::Removed, String> {
    let _ = store(&app);
    blocking(move || crate::canvas::chat::wipe(&workspace_id, &node_id, thread)).await
}

#[tauri::command]
pub async fn canvas_attachment_copy(
    app: AppHandle,
    workspace_id: String,
    path: String,
) -> Result<String, String> {
    let s = store(&app);
    blocking(move || {
        let src = std::path::PathBuf::from(&path);
        let meta = std::fs::metadata(&src).map_err(|e| format!("CANVAS_NOT_FOUND: {path}: {e}"))?;
        if !meta.is_file() {
            return Err(format!("CANVAS_INVALID: not a file: {path}"));
        }
        if meta.len() > chat::MAX_ATTACHMENT {
            return Err("CANVAS_INVALID: attachment larger than 25 MB".into());
        }
        let bytes = std::fs::read(&src).map_err(|e| format!("CANVAS_IO: {e}"))?;
        let name = src
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        let out = chat::save_inbox_attachment(&s, &workspace_id, &name, &bytes)?;
        Ok(out.to_string_lossy().into_owned())
    })
    .await
}

#[tauri::command]
pub async fn canvas_attachment_save(
    app: AppHandle,
    workspace_id: String,
    name: String,
    data_base64: String,
) -> Result<String, String> {
    let s = store(&app);
    blocking(move || {
        let raw = data_base64
            .split_once("base64,")
            .map(|(_, b)| b)
            .unwrap_or(&data_base64);
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(raw.trim())
            .map_err(|e| format!("CANVAS_INVALID: base64: {e}"))?;
        let path = chat::save_inbox_attachment(&s, &workspace_id, &name, &bytes)?;
        Ok(path.to_string_lossy().into_owned())
    })
    .await
}
