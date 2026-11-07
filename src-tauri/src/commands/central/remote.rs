use tauri::AppHandle;

use crate::local_bridge_remote::{self as remote, AccessEntry, DeviceView, RemoteStatus};

#[tauri::command]
pub async fn remote_status(app: AppHandle) -> Result<RemoteStatus, String> {
    let h = remote::hub();
    h.set_app(&app);
    Ok(h.status())
}

#[tauri::command]
pub async fn remote_start(
    app: AppHandle,
    bind_ip: String,
    port: Option<u16>,
) -> Result<RemoteStatus, String> {
    remote::start_for_app(&app, &bind_ip, port).await
}

#[tauri::command]
pub async fn remote_private_only_set(app: AppHandle, on: bool) -> Result<RemoteStatus, String> {
    let h = remote::hub();
    h.set_app(&app);
    h.set_private_only(on);
    Ok(h.status())
}

#[tauri::command]
pub async fn remote_stop(app: AppHandle) -> Result<RemoteStatus, String> {
    let h = remote::hub();
    h.set_app(&app);
    h.stop();
    Ok(h.status())
}

#[tauri::command]
pub async fn remote_devices() -> Result<Vec<DeviceView>, String> {
    Ok(remote::hub().devices())
}

#[tauri::command]
pub async fn remote_device_revoke(id: String) -> Result<bool, String> {
    Ok(remote::hub().revoke(&id))
}

#[tauri::command]
pub async fn remote_access_log(limit: Option<usize>) -> Result<Vec<AccessEntry>, String> {
    Ok(remote::hub().access_log(limit.unwrap_or(100).min(300)))
}
