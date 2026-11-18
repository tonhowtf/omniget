use crate::core::channels::{self, ChannelFollow};

#[tauri::command]
pub fn channels_list() -> Vec<ChannelFollow> {
    channels::list()
}

#[tauri::command]
pub fn channel_add(url: String, title: Option<String>) -> Result<ChannelFollow, String> {
    let url = url.trim().to_string();
    if url.is_empty() {
        return Err("Empty URL".to_string());
    }
    let title = title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| url.clone());
    Ok(channels::add(url, title))
}

#[tauri::command]
pub fn channel_remove(id: String) -> Result<(), String> {
    if channels::remove(&id) {
        Ok(())
    } else {
        Err("Channel not found".to_string())
    }
}

#[tauri::command]
pub fn channel_update(
    id: String,
    enabled: Option<bool>,
    auto_download: Option<bool>,
    interval_minutes: Option<u32>,
) -> Result<ChannelFollow, String> {
    channels::update(&id, enabled, auto_download, interval_minutes)
        .ok_or_else(|| "Channel not found".to_string())
}

#[tauri::command]
pub async fn channel_check_now(app: tauri::AppHandle, id: String) -> Result<usize, String> {
    crate::core::channel_poller::check_now(&app, &id).await
}

#[tauri::command]
pub fn sync_channels_tray(
    app: tauri::AppHandle,
    header: String,
    channels: Vec<(String, String)>,
) -> Result<(), String> {
    crate::tray::rebuild_menu(&app, header, channels).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn sync_tray_strings(
    app: tauri::AppHandle,
    strings: crate::tray::TrayStrings,
) -> Result<(), String> {
    crate::tray::apply_strings(&app, strings).map_err(|e| e.to_string())
}
