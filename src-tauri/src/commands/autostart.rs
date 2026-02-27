#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[cfg(windows)]
const REG_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(windows)]
const APP_NAME: &str = "OmniGet";

pub fn apply_autostart(enabled: bool) -> Result<(), String> {
    if std::env::var("OMNIGET_PORTABLE").is_ok() { return Ok(()); }
    #[cfg(windows)]
    {
        if enabled {
            let exe = std::env::current_exe()
                .map_err(|e| format!("Failed to get exe path: {e}"))?;
            let output = std::process::Command::new("reg")
                .args(["add", REG_KEY, "/v", APP_NAME, "/d", &exe.to_string_lossy(), "/f"])
                .creation_flags(CREATE_NO_WINDOW)
                .output()
                .map_err(|e| format!("reg add failed: {e}"))?;
            if !output.status.success() {
                return Err(format!(
                    "reg add failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
        } else {
            let _ = std::process::Command::new("reg")
                .args(["delete", REG_KEY, "/v", APP_NAME, "/f"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
        }
    }
    #[cfg(not(windows))]
    let _ = enabled;
    Ok(())
}

pub fn get_autostart_enabled() -> Result<bool, String> {
    #[cfg(windows)]
    {
        let output = std::process::Command::new("reg")
            .args(["query", REG_KEY, "/v", APP_NAME])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("reg query failed: {e}"))?;
        Ok(output.status.success())
    }
    #[cfg(not(windows))]
    Ok(false)
}

#[tauri::command]
pub fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<(), String> {
    apply_autostart(enabled)?;
    let mut current = crate::storage::config::load_settings(&app);
    current.start_with_windows = enabled;
    crate::storage::config::save_settings(&app, &current)
        .map_err(|e| format!("Save: {e}"))?;
    Ok(())
}

#[tauri::command]
pub fn get_autostart_status() -> Result<bool, String> {
    get_autostart_enabled()
}
