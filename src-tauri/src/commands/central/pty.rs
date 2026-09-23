//! Tauri commands of the embedded terminal. The sessions live in
//! `crate::pty::manager()`; see that module for the event contract.

use tauri::AppHandle;

use crate::pty::{self, foreground::Foreground, Attached, OpenRequest, SessionInfo};

#[tauri::command]
pub async fn pty_open(app: AppHandle, req: OpenRequest) -> Result<SessionInfo, String> {
    let m = pty::manager();
    m.bind(&app);
    tauri::async_runtime::spawn_blocking(move || m.open(req))
        .await
        .map_err(|e| format!("PTY_OPEN: {e}"))?
}

#[tauri::command]
pub async fn pty_write(id: String, data: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || pty::manager().write(&id, &data))
        .await
        .map_err(|e| format!("PTY_WRITE: {e}"))?
}

#[tauri::command]
pub fn pty_resize(id: String, cols: u16, rows: u16) -> Result<(), String> {
    pty::manager().resize(&id, cols, rows)
}

#[tauri::command]
pub fn pty_close(id: String, delete_history: Option<bool>) -> Result<(), String> {
    pty::manager().close(&id, delete_history.unwrap_or(false))
}

#[tauri::command]
pub fn pty_list(app: AppHandle) -> Vec<SessionInfo> {
    let m = pty::manager();
    m.bind(&app);
    m.list()
}

#[tauri::command]
pub async fn pty_attach(app: AppHandle, id: String) -> Result<Attached, String> {
    let m = pty::manager();
    m.bind(&app);
    tauri::async_runtime::spawn_blocking(move || m.attach(&id))
        .await
        .map_err(|e| format!("PTY_ATTACH: {e}"))?
}

#[tauri::command]
pub fn pty_detach(id: String) -> Result<(), String> {
    pty::manager().detach(&id)
}

#[tauri::command]
pub fn pty_ack(id: String, seq: u64) -> Result<(), String> {
    pty::manager().ack(&id, seq)
}

#[tauri::command]
pub fn pty_clear(id: String) -> Result<u64, String> {
    pty::manager().clear(&id)
}

#[tauri::command]
pub async fn pty_foreground(id: String) -> Result<Foreground, String> {
    tauri::async_runtime::spawn_blocking(move || pty::manager().foreground(&id))
        .await
        .map_err(|e| format!("PTY_FOREGROUND: {e}"))?
}
