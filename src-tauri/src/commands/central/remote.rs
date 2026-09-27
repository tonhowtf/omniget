//! `remote_*` commands of the `/llm/remote` page (T11): turn the remote
//! listener on/off on a chosen interface, pair a device with a QR code, list
//! and revoke devices, read the access log, and the Tailscale/SSH helpers.
//! The logic lives in `crate::local_bridge_remote`; nothing here polls.

use tauri::AppHandle;

use crate::local_bridge_remote::{
    self as remote, AccessEntry, CommandRun, DeviceView, PairLink, RemoteStatus, Scope, SshInfo,
    TailscaleInfo,
};

#[tauri::command]
pub async fn remote_status(app: AppHandle) -> Result<RemoteStatus, String> {
    let h = remote::hub();
    h.set_app(&app);
    Ok(h.status())
}

/// The explicit opt-in: starts the second listener on `bindIp` (one of
/// `remote_status().interfaces`, or any local address) and remembers it for
/// the next launch.
#[tauri::command]
pub async fn remote_start(
    app: AppHandle,
    bind_ip: String,
    port: Option<u16>,
) -> Result<RemoteStatus, String> {
    remote::start_for_app(&app, &bind_ip, port).await
}

#[tauri::command]
pub async fn remote_stop(app: AppHandle) -> Result<RemoteStatus, String> {
    let h = remote::hub();
    h.set_app(&app);
    h.stop();
    Ok(h.status())
}

/// A one-time pairing link (10 min) for `scope` (`read|drive`). `base` is
/// the origin the phone will use (an endpoint of `remote_status`, the
/// `tailscale serve` URL or the SSH base); default: the listener itself.
#[tauri::command]
pub async fn remote_pair_create(
    app: AppHandle,
    scope: String,
    base: Option<String>,
    label: Option<String>,
) -> Result<PairLink, String> {
    let h = remote::hub();
    h.set_app(&app);
    let scope = Scope::parse(&scope)?;
    let base = match base.filter(|b| !b.trim().is_empty()) {
        Some(b) => b,
        None => {
            let addr = h
                .bound_addr()
                .ok_or_else(|| "REMOTE_NOT_RUNNING: turn remote access on first".to_string())?;
            remote::base_for(addr.ip(), addr.port())
        }
    };
    h.create_pairing(scope, &base, label)
}

#[tauri::command]
pub async fn remote_pair_cancel() -> Result<u32, String> {
    Ok(remote::hub().cancel_pairings())
}

#[tauri::command]
pub async fn remote_devices() -> Result<Vec<DeviceView>, String> {
    Ok(remote::hub().devices())
}

/// Revokes a device: its bearer stops working and its sockets close now.
#[tauri::command]
pub async fn remote_device_revoke(id: String) -> Result<bool, String> {
    Ok(remote::hub().revoke(&id))
}

#[tauri::command]
pub async fn remote_device_rename(id: String, name: String) -> Result<bool, String> {
    Ok(remote::hub().rename(&id, &name))
}

/// Newest first.
#[tauri::command]
pub async fn remote_access_log(limit: Option<usize>) -> Result<Vec<AccessEntry>, String> {
    Ok(remote::hub().access_log(limit.unwrap_or(100).min(300)))
}

#[tauri::command]
pub async fn remote_access_log_clear() -> Result<(), String> {
    remote::hub().clear_access_log();
    Ok(())
}

/// Detects `tailscale` and reads its status (spawns it only on this call).
#[tauri::command]
pub async fn remote_tailscale_status(https_port: Option<u16>) -> Result<TailscaleInfo, String> {
    Ok(remote::tailscale_status(https_port).await)
}

/// Runs `tailscale serve` on (or off) for the remote port. Without
/// `confirm: true` it only answers `REMOTE_CONFIRM_REQUIRED: <command>`.
#[tauri::command]
pub async fn remote_tailscale_serve(
    enable: bool,
    confirm: Option<bool>,
    https_port: Option<u16>,
) -> Result<CommandRun, String> {
    remote::tailscale_serve(enable, confirm.unwrap_or(false), https_port).await
}

/// The `ssh -L` command to reach the remote port through a tunnel.
#[tauri::command]
pub async fn remote_ssh_command(
    local_port: Option<u16>,
    host: Option<String>,
) -> Result<SshInfo, String> {
    Ok(remote::ssh_info(local_port, host))
}
