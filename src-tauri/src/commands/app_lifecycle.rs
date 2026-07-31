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
    /// Whether the UI should warn that the WebView is not portable here.
    ///
    /// Only macOS: wry does not read `data_directory` on WKWebView, so the
    /// session data stays under `~/Library` no matter where the app runs from.
    /// Windows and Linux redirect it in `lib.rs`, so there is nothing to warn
    /// about there. See issue #227.
    pub macos_webview_notice: bool,
}

/// Reports what portable mode actually covers on this platform.
///
/// Reads the same environment `main.rs` publishes rather than re-detecting
/// `portable.txt`, so there is a single source of truth for the mode.
#[tauri::command]
pub fn get_portable_info() -> PortableInfo {
    let is_portable = std::env::var("OMNIGET_PORTABLE").ok().as_deref() == Some("1");

    PortableInfo {
        is_portable,
        data_dir: std::env::var("OMNIGET_DATA_DIR").ok(),
        macos_webview_notice: cfg!(target_os = "macos") && is_portable,
    }
}
