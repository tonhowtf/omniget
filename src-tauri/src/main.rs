#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn check_portable_mode() {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("portable.txt").exists() || dir.join(".portable").exists() {
                let data_dir = dir.join("data");
                let _ = std::fs::create_dir_all(&data_dir);
                std::env::set_var("OMNIGET_PORTABLE", "1");
                std::env::set_var("OMNIGET_DATA_DIR", data_dir.to_string_lossy().to_string());

                #[cfg(windows)]
                if std::env::var("WEBVIEW2_USER_DATA_FOLDER").is_err() {
                    let webview_dir = data_dir.join("webview");
                    let _ = std::fs::create_dir_all(&webview_dir);
                    std::env::set_var(
                        "WEBVIEW2_USER_DATA_FOLDER",
                        webview_dir.to_string_lossy().to_string(),
                    );
                }

                // Older versions resolved the settings store against Tauri's
                // own AppData dir, so portable users' settings landed in the
                // OS profile. Adopt that file once if the portable dir has none.
                let portable_settings = data_dir.join("settings.json");
                if !portable_settings.exists() {
                    if let Some(os_settings) = dirs::data_dir()
                        .map(|d| d.join("wtf.tonho.omniget").join("settings.json"))
                        .filter(|p| p.exists())
                    {
                        let _ = std::fs::copy(&os_settings, &portable_settings);
                    }
                }
            }
        }
    }
}

fn setup_environment() {
    std::env::remove_var("PYTHONHOME");
    std::env::remove_var("PYTHONPATH");

    if let Some(bin_dir) = omniget_lib::core::paths::app_data_dir().map(|d| d.join("bin")) {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let current = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}{}{}", bin_dir.display(), sep, current));
    }

    std::env::set_var("PYTHONIOENCODING", "utf-8");
    std::env::set_var("PYTHONUTF8", "1");
    std::env::set_var("PYTHONLEGACYWINDOWSSTDIO", "0");

    #[cfg(target_os = "linux")]
    if std::env::var("WEBKIT_DISABLE_DMABUF_RENDERER").is_err() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
}

/// Usa um WebView2 Fixed Version Runtime descompactado ao lado do executavel.
///
/// Issue #218. O Tauri so define `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` quando o
/// bundle e compilado em modo `fixedRuntime` (`app.rs`), e nunca a limpa — entao
/// definir aqui funciona, ao contrario do `WEBVIEW2_USER_DATA_FOLDER`, que o
/// Tauri sobrescrevia (ver `core/portable.rs`). Nao mexe se o usuario ja definiu.
#[cfg(windows)]
fn check_fixed_webview_runtime() {
    if std::env::var_os("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER").is_some() {
        return;
    }
    let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|p| p.to_path_buf()))
    else {
        return;
    };
    if let Some(runtime) = omniget_lib::core::portable::find_fixed_runtime(&dir) {
        std::env::set_var("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER", &runtime);
    }
}

fn main() {
    check_portable_mode();
    #[cfg(windows)]
    check_fixed_webview_runtime();
    setup_environment();
    omniget_lib::run()
}
