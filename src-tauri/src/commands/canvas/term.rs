//! Tauri commands of the canvas terminals. Frames go through the `Channel`
//! given to `term_subscribe` (format `OGT1`, see
//! `omniget_core::core::term::wire`); input comes back as small JSON
//! commands. The subscription is keyed by the calling webview's label.

use omniget_core::core::term::input::{KeyInput, MouseInput, ScrollInput, SelectInput};
use omniget_core::core::term::screen::ScreenSnapshot;
use omniget_core::core::term::session::{SearchHit, TermInfo, TermOpen};
use omniget_core::core::term::wire::{SlotInfo, Visible};
use std::sync::Arc;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Webview};

use crate::canvas::term::{host, ChannelSink};

#[tauri::command]
pub async fn term_open(app: AppHandle, req: TermOpen) -> Result<TermInfo, String> {
    let m = host(&app);
    tauri::async_runtime::spawn_blocking(move || m.open(req))
        .await
        .map_err(|e| format!("PTY_OPEN: {e}"))?
}

#[tauri::command]
pub fn term_close(app: AppHandle, id: String, delete_history: Option<bool>) -> Result<(), String> {
    host(&app).close(&id, delete_history.unwrap_or(false))
}

#[tauri::command]
pub fn term_list(app: AppHandle) -> Vec<TermInfo> {
    host(&app).list()
}

#[tauri::command]
pub fn term_subscribe(
    app: AppHandle,
    webview: Webview,
    channel: Channel<InvokeResponseBody>,
    visible: Vec<Visible>,
) -> Vec<SlotInfo> {
    host(&app).subscribe_frames(webview.label(), Arc::new(ChannelSink(channel)), visible)
}

#[tauri::command]
pub fn term_set_visible(app: AppHandle, webview: Webview, visible: Vec<Visible>) -> Vec<SlotInfo> {
    host(&app).set_visible(webview.label(), visible)
}

#[tauri::command]
pub fn term_unsubscribe(app: AppHandle, webview: Webview) {
    host(&app).unsubscribe_frames(webview.label());
}

#[tauri::command]
pub fn term_frame_done(app: AppHandle, webview: Webview, seq: u32) {
    host(&app).frame_done(webview.label(), seq);
}

#[tauri::command]
pub async fn term_key(app: AppHandle, id: String, key: KeyInput) -> Result<(), String> {
    let m = host(&app);
    tauri::async_runtime::spawn_blocking(move || m.key(&id, &key))
        .await
        .map_err(|e| format!("PTY_WRITE: {e}"))?
}

#[tauri::command]
pub async fn term_text(app: AppHandle, id: String, text: String) -> Result<(), String> {
    let m = host(&app);
    tauri::async_runtime::spawn_blocking(move || m.text(&id, &text))
        .await
        .map_err(|e| format!("PTY_WRITE: {e}"))?
}

#[tauri::command]
pub async fn term_paste(app: AppHandle, id: String, text: String) -> Result<(), String> {
    let m = host(&app);
    tauri::async_runtime::spawn_blocking(move || m.paste(&id, &text))
        .await
        .map_err(|e| format!("PTY_WRITE: {e}"))?
}

#[tauri::command]
pub fn term_resize(app: AppHandle, id: String, cols: u16, rows: u16) -> Result<(), String> {
    host(&app).resize(&id, cols, rows)
}

#[tauri::command]
pub fn term_scroll(app: AppHandle, id: String, scroll: ScrollInput) -> Result<(), String> {
    host(&app).scroll(&id, &scroll)
}

#[tauri::command]
pub fn term_mouse(app: AppHandle, id: String, mouse: MouseInput) -> Result<bool, String> {
    host(&app).mouse(&id, &mouse)
}

#[tauri::command]
pub fn term_select(app: AppHandle, id: String, sel: SelectInput) -> Result<(), String> {
    host(&app).select(&id, &sel)
}

#[tauri::command]
pub fn term_copy(app: AppHandle, id: String) -> Result<Option<String>, String> {
    host(&app).copy(&id)
}

#[tauri::command]
pub fn term_search(
    app: AppHandle,
    id: String,
    query: String,
    backwards: Option<bool>,
) -> Result<Option<SearchHit>, String> {
    host(&app).search(&id, &query, backwards.unwrap_or(false))
}

#[tauri::command]
pub fn term_focus(app: AppHandle, id: String, focused: bool) -> Result<(), String> {
    host(&app).focus(&id, focused)
}

#[tauri::command]
pub fn term_link(app: AppHandle, id: String, link_id: u32) -> Option<String> {
    host(&app).link_uri(&id, link_id)
}

#[tauri::command]
pub fn term_screen(
    app: AppHandle,
    id: String,
    last_lines: Option<usize>,
) -> Result<ScreenSnapshot, String> {
    host(&app).screen_text(&id, last_lines.unwrap_or(0))
}

/// Opens (or attaches to) the terminal of a canvas node with the whole launch
/// recipe (role, cords env, floor context; `crate::canvas::launch`). The UI
/// passes its measured grid and, for a node it just created, the node itself.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn canvas_term_ensure(
    app: AppHandle,
    workspace_id: String,
    floor_id: Option<String>,
    node_id: String,
    node: Option<omniget_core::core::canvas::model::CanvasNode>,
    cols: Option<u16>,
    rows: Option<u16>,
    restore: Option<bool>,
) -> Result<TermInfo, String> {
    crate::canvas::routines::start(&app);
    let floor = floor_id
        .filter(|f| !f.is_empty())
        .unwrap_or_else(|| omniget_core::core::canvas::model::GROUND_FLOOR_ID.to_string());
    crate::canvas::launch::ensure(
        &workspace_id,
        &floor,
        &node_id,
        crate::canvas::launch::LaunchOpts {
            node,
            cols,
            rows,
            restore: restore.unwrap_or(false),
        },
    )
    .await
    .map(|l| l.info)
}

/// ⌘K: wipes the screen and scrollback of a terminal.
#[tauri::command]
pub fn term_clear(app: AppHandle, id: String) -> Result<(), String> {
    host(&app).clear_screen(&id)
}

/// ⌘⇧B: scroll lock on/off (toggles when `on` is absent); returns the state.
#[tauri::command]
pub fn term_scroll_lock(app: AppHandle, id: String, on: Option<bool>) -> Result<bool, String> {
    host(&app).set_scroll_lock(&id, on)
}

/// Number of matches of a find query (plain text, smart case).
#[tauri::command]
pub fn term_search_count(app: AppHandle, id: String, query: String) -> Result<usize, String> {
    host(&app).search_count(&id, &query)
}

/// Ends a find: no highlight, view back at the bottom.
#[tauri::command]
pub fn term_search_clear(app: AppHandle, id: String) -> Result<(), String> {
    host(&app).search_clear(&id)
}

/// The user's terminal themes (`~/.omniget/terminal/themes`, Ghostty format).
#[tauri::command]
pub fn term_themes() -> Vec<crate::canvas::term::UserTheme> {
    crate::canvas::term::user_themes()
}
