//! Tauri commands of the chat face and the Prompt Composer's attachments.
//! Transcript logic: `omniget_core::core::canvas::chat`; delivery and the
//! `canvas://chat` event: `crate::canvas::chat`.

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

/// A page of one thread (newest 80, or 60 before `before`), plus the thread
/// list, the active thread and the unread counts. Reading marks the thread
/// read unless `peek`.
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

/// Selects the thread the next message goes to. `CHAT_TURN_ACTIVE` while
/// the agent answers on another thread.
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

/// Marks one thread (or every thread) of a node as read.
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

/// Esc on the chat face: ESC to the agent and the open turn closed as
/// interrupted (the entry, or null when no turn was open).
#[tauri::command]
pub async fn canvas_chat_interrupt(
    app: AppHandle,
    workspace_id: String,
    node_id: String,
) -> Result<Option<ChatEntry>, String> {
    let _ = store(&app);
    blocking(move || crate::canvas::chat::interrupt(&workspace_id, &node_id)).await
}

/// Palette: entries of every chat (all workspaces, threads and terminals)
/// holding every word of `query`, no case and no accents, newest first.
#[tauri::command]
pub async fn canvas_chat_search(
    app: AppHandle,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<chat::SearchHit>, String> {
    let _ = store(&app);
    blocking(move || Ok(crate::canvas::chat::search(&query, limit))).await
}

/// User → agent: records the message on `thread` and pastes the chat
/// envelope into the node's terminal (bracketed paste + Enter).
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

/// "Clear": a divider on the thread; `recall` stops there unless `--all`.
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

/// Delete Message: removes one transcript entry (and its attachment files
/// when no other entry uses them).
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

/// Clear Chat History: every thread (or `thread`) and the attachments.
/// Refused with `CHAT_TURN_ACTIVE` while the agent is mid-turn.
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

/// Saves a pasted/dropped file (base64) into `<ws>/inbox/attachments` and
/// returns its absolute path (the composer inserts it into the prompt).
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
