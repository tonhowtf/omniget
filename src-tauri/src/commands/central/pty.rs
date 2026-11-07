use tauri::AppHandle;

use crate::pty::{self, Attached, OpenRequest, SessionInfo};

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
