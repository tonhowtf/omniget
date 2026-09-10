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

// The tray menu is built natively in Rust, so $t is unavailable there. The
// frontend resolves the labels with $t and pushes them here; the backend just
// rebuilds the submenu with whatever localized strings it receives.
#[tauri::command]
pub fn sync_channels_tray(
    app: tauri::AppHandle,
    header: String,
    channels: Vec<(String, String)>,
) -> Result<(), String> {
    crate::tray::rebuild_menu(&app, header, channels).map_err(|e| e.to_string())
}

// Same push-based pattern for the static tray items and tooltip formats: the
// frontend owns the translations, the backend only stores the strings and
// applies them to the live menu right away.
#[tauri::command]
pub fn sync_tray_strings(
    quit: String,
    downloads_none: String,
    downloads_active: String,
    channels: String,
    tooltip_active: String,
    tooltip_speed: String,
) -> Result<(), String> {
    let strings = crate::tray::TrayStrings {
        quit,
        downloads_none,
        downloads_active,
        channels,
        tooltip_active,
        tooltip_speed,
    };
    crate::tray::apply_strings(strings).map_err(|e| e.to_string())
}
