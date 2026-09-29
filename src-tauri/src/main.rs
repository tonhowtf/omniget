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

/// `pnpm tauri dev` builds the same identifier as the installed app, so without
/// this a debug build would read and write the real profile: settings, keys,
/// history, databases. Debug builds get `<data dir>/omniget-dev` instead, unless
/// the profile is already chosen (`OMNIGET_DATA_DIR`, as `pnpm dev:app` and the
/// smoke test do, or portable mode) or `OMNIGET_DEV_REAL_PROFILE=1` asks for the
/// real one. `lib.rs` logs where it went. Setting `OMNIGET_DATA_DIR` also makes
/// it a second profile there: own bridge port, no single-instance lock, so it
/// runs next to the installed app.
#[cfg(debug_assertions)]
fn use_dev_profile() {
    use omniget_lib::core::paths;
    let Some(dir) = paths::dev_profile_dir(
        std::env::var_os("OMNIGET_DATA_DIR").is_some(),
        std::env::var("OMNIGET_PORTABLE").ok().as_deref() == Some("1"),
        std::env::var(paths::DEV_REAL_PROFILE_ENV).ok().as_deref(),
        dirs::data_dir().as_deref(),
    ) else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("OMNIGET_DATA_DIR", &dir);
    std::env::set_var("OMNIGET_DEV_PROFILE", "1");
    // The keyring is keyed by service name, not by profile: on Linux (Secret
    // Service is on by default) the dev build would share the installed app's
    // profile seed and provider keys. The encrypted file store lives inside the
    // profile, so it follows the dev dir. An explicit choice still wins.
    if std::env::var_os("OMNIGET_KEYRING").is_none()
        && std::env::var_os("OMNIGET_OMNIDISC_KEYRING").is_none()
    {
        std::env::set_var("OMNIGET_KEYRING", "0");
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

    // Issue #199. AppImages bundle their own libwayland, which conflicts with
    // the system EGL on some Wayland setups (Fedora/KDE, Intel+Mesa) and kills
    // the WebKit process with EGL_BAD_PARAMETER before any window appears
    // (tauri-apps/tauri#11988). Running through XWayland sidesteps the bundled
    // library entirely. Only applies inside an AppImage on a Wayland session,
    // and never overrides an explicit user choice.
    #[cfg(target_os = "linux")]
    if std::env::var_os("APPIMAGE").is_some()
        && std::env::var_os("GDK_BACKEND").is_none()
        && std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland")
    {
        std::env::set_var("GDK_BACKEND", "x11");
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
    #[cfg(debug_assertions)]
    use_dev_profile();
    #[cfg(windows)]
    check_fixed_webview_runtime();
    setup_environment();
    omniget_lib::run()
}
