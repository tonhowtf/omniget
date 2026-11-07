#[cfg(not(windows))]
use tauri_plugin_autostart::ManagerExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutostartStatus {
    pub supported: bool,
    pub registered: bool,
    pub blocked_by_system: bool,
}

fn is_portable() -> bool {
    std::env::var("OMNIGET_PORTABLE").is_ok()
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn run_value(exe: &str) -> String {
    let exe = exe.strip_prefix(r"\\?\").unwrap_or(exe);
    format!(
        "\"{exe}\" {}",
        omniget_core::core::app_launch::AUTOSTART_FLAG
    )
}

#[cfg(windows)]
mod windows_run {
    use std::io::ErrorKind;
    use winreg::enums::{RegType, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
    use winreg::{RegKey, RegValue};

    const RUN: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Run";
    const APPROVED: &str =
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
    const APPROVED_ON: [u8; 12] = [2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

    fn hkcu() -> RegKey {
        RegKey::predef(HKEY_CURRENT_USER)
    }

    pub fn write(name: &str, value: &str) -> std::io::Result<()> {
        hkcu()
            .open_subkey_with_flags(RUN, KEY_SET_VALUE)?
            .set_value(name, &value.to_string())
    }

    pub fn approve(name: &str) -> std::io::Result<()> {
        match hkcu().open_subkey_with_flags(APPROVED, KEY_SET_VALUE) {
            Ok(key) => key.set_raw_value(
                name,
                &RegValue {
                    vtype: RegType::REG_BINARY,
                    bytes: APPROVED_ON.to_vec(),
                },
            ),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub fn remove(name: &str) -> std::io::Result<()> {
        match hkcu()
            .open_subkey_with_flags(RUN, KEY_SET_VALUE)?
            .delete_value(name)
        {
            Err(e) if e.kind() != ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    pub fn registered(name: &str) -> bool {
        hkcu()
            .open_subkey_with_flags(RUN, KEY_READ)
            .and_then(|key| key.get_value::<String, _>(name))
            .is_ok()
    }

    pub fn blocked(name: &str) -> bool {
        hkcu()
            .open_subkey_with_flags(APPROVED, KEY_READ)
            .and_then(|key| key.get_raw_value(name))
            .map(|v| v.bytes.first().is_some_and(|b| b & 1 == 1))
            .unwrap_or(false)
    }
}

#[cfg(windows)]
fn entry(app: &tauri::AppHandle) -> Result<(String, String), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok((
        app.package_info().name.clone(),
        run_value(&exe.display().to_string()),
    ))
}

pub fn apply_autostart(app: &tauri::AppHandle, enabled: bool) -> Result<(), String> {
    if is_portable() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        let (name, value) = entry(app)?;
        let done = if enabled {
            windows_run::write(&name, &value).and_then(|()| windows_run::approve(&name))
        } else {
            windows_run::remove(&name)
        };
        done.map_err(|e| format!("Failed to update autostart: {e}"))
    }
    #[cfg(not(windows))]
    {
        let manager = app.autolaunch();
        if enabled {
            manager
                .enable()
                .map_err(|e| format!("Failed to enable autostart: {e}"))
        } else if manager.is_enabled().unwrap_or(false) {
            manager
                .disable()
                .map_err(|e| format!("Failed to disable autostart: {e}"))
        } else {
            Ok(())
        }
    }
}

pub fn sync_at_startup(app: &tauri::AppHandle, wanted: bool) {
    if is_portable() || cfg!(debug_assertions) {
        return;
    }
    #[cfg(windows)]
    let result = entry(app).and_then(|(name, value)| {
        if wanted {
            windows_run::write(&name, &value)
        } else {
            windows_run::remove(&name)
        }
        .map_err(|e| e.to_string())
    });
    #[cfg(not(windows))]
    let result = apply_autostart(app, wanted);
    if let Err(e) = result {
        tracing::warn!("[autostart] could not sync the login entry: {e}");
    }
}

#[tauri::command]
pub fn autostart_status(app: tauri::AppHandle) -> AutostartStatus {
    if is_portable() {
        return AutostartStatus {
            supported: false,
            registered: false,
            blocked_by_system: false,
        };
    }
    #[cfg(windows)]
    {
        let name = app.package_info().name.clone();
        AutostartStatus {
            supported: true,
            registered: windows_run::registered(&name),
            blocked_by_system: windows_run::blocked(&name),
        }
    }
    #[cfg(not(windows))]
    AutostartStatus {
        supported: true,
        registered: app.autolaunch().is_enabled().unwrap_or(false),
        blocked_by_system: false,
    }
}

#[cfg(test)]
mod tests {
    use super::run_value;

    #[test]
    fn the_login_entry_quotes_a_path_with_spaces() {
        assert_eq!(
            run_value(r"C:\Users\Ana Maria\AppData\Local\OmniGet\omniget.exe"),
            r#""C:\Users\Ana Maria\AppData\Local\OmniGet\omniget.exe" --autostart"#
        );
    }

    #[test]
    fn the_login_entry_drops_the_extended_path_prefix() {
        assert_eq!(
            run_value(r"\\?\C:\OmniGet\omniget.exe"),
            r#""C:\OmniGet\omniget.exe" --autostart"#
        );
    }
}
