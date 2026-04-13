use serde::Serialize;
use tauri::AppHandle;

#[tauri::command]
pub fn get_active_download_count(app: AppHandle) -> u32 {
    crate::tray::compute_total_active(&app)
}

#[tauri::command]
pub fn request_app_quit(app: AppHandle) {
    crate::tray::request_quit(&app);
}

#[tauri::command]
pub fn force_exit_app(app: AppHandle) {
    app.exit(0);
}

#[derive(Debug, Serialize)]
pub struct DebugInfo {
    pub os: String,
    pub arch: String,
    pub os_family: String,
    pub proxy_enabled: bool,
}

#[tauri::command]
pub fn get_debug_info(app: AppHandle) -> DebugInfo {
    let settings = crate::storage::config::load_settings(&app);
    DebugInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        os_family: std::env::consts::FAMILY.to_string(),
        proxy_enabled: settings.proxy.enabled,
    }
}
