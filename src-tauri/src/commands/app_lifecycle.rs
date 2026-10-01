use serde::Serialize;
use tauri::AppHandle;

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

#[derive(Debug, Serialize)]
pub struct PortableInfo {
    pub is_portable: bool,
    pub data_dir: Option<String>,
    // WHY: wry ignores data_directory on WKWebView, so on macOS session data stays under ~/Library.
    pub macos_webview_notice: bool,
}

#[tauri::command]
pub fn get_portable_info() -> PortableInfo {
    let is_portable = std::env::var("OMNIGET_PORTABLE").ok().as_deref() == Some("1");

    PortableInfo {
        is_portable,
        data_dir: std::env::var("OMNIGET_DATA_DIR").ok(),
        macos_webview_notice: cfg!(target_os = "macos") && is_portable,
    }
}
