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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallInfo {
    pub kind: String,
    pub self_update: bool,
    pub download_url: String,
}

fn detect_install_info(
    os: &str,
    portable: bool,
    flatpak: bool,
    appimage: bool,
    has_deb_marker: bool,
    has_rpm_marker: bool,
) -> InstallInfo {
    if portable {
        let download_url = match os {
            "windows" => "https://getomniget.com/download/windows",
            "macos" => "https://getomniget.com/download/macos",
            "linux" => "https://getomniget.com/download/linux",
            _ => "https://getomniget.com/download",
        };
        return InstallInfo {
            kind: "portable".into(),
            self_update: false,
            download_url: download_url.into(),
        };
    }

    match os {
        "linux" => {
            if flatpak {
                InstallInfo {
                    kind: "flatpak".into(),
                    self_update: false,
                    download_url: "https://getomniget.com/download/linux".into(),
                }
            } else if appimage {
                InstallInfo {
                    kind: "appimage".into(),
                    self_update: true,
                    download_url: "https://getomniget.com/download/linux".into(),
                }
            } else if has_deb_marker {
                InstallInfo {
                    kind: "deb".into(),
                    self_update: false,
                    download_url: "https://getomniget.com/download/linux".into(),
                }
            } else if has_rpm_marker {
                InstallInfo {
                    kind: "rpm".into(),
                    self_update: false,
                    download_url: "https://getomniget.com/download/linux".into(),
                }
            } else {
                InstallInfo {
                    kind: "package".into(),
                    self_update: false,
                    download_url: "https://getomniget.com/download/linux".into(),
                }
            }
        }
        "macos" => InstallInfo {
            kind: "app".into(),
            self_update: true,
            download_url: "https://getomniget.com/download/macos".into(),
        },
        "windows" => InstallInfo {
            kind: "installer".into(),
            self_update: true,
            download_url: "https://getomniget.com/download/windows".into(),
        },
        _ => InstallInfo {
            kind: "unknown".into(),
            self_update: false,
            download_url: "https://getomniget.com/download".into(),
        },
    }
}

#[cfg(target_os = "linux")]
fn linux_package_markers() -> (bool, bool) {
    use std::path::Path;
    if Path::new("/var/lib/dpkg/info/omniget.list").exists() {
        return (true, false);
    }
    if let Ok(exe) = std::env::current_exe() {
        let exe_s = exe.to_string_lossy().to_string();
        if std::process::Command::new("dpkg")
            .args(["-S", &exe_s])
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return (true, false);
        }
        if std::process::Command::new("rpm")
            .args(["-qf", &exe_s])
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return (false, true);
        }
    }
    let has_deb = Path::new("/etc/debian_version").exists();
    let has_rpm = Path::new("/etc/redhat-release").exists()
        || Path::new("/etc/fedora-release").exists()
        || Path::new("/etc/SuSE-release").exists();
    (has_deb && !has_rpm, has_rpm)
}

#[cfg(not(target_os = "linux"))]
fn linux_package_markers() -> (bool, bool) {
    (false, false)
}

#[tauri::command]
pub fn get_install_info() -> InstallInfo {
    let portable = std::env::var("OMNIGET_PORTABLE").ok().as_deref() == Some("1");
    let flatpak =
        std::env::var_os("FLATPAK_ID").is_some() || std::path::Path::new("/.flatpak-info").exists();
    let appimage = std::env::var_os("APPIMAGE").is_some_and(|v| !v.is_empty());
    let (has_deb, has_rpm) = if cfg!(target_os = "linux") && !flatpak && !appimage && !portable {
        linux_package_markers()
    } else {
        (false, false)
    };
    detect_install_info(
        std::env::consts::OS,
        portable,
        flatpak,
        appimage,
        has_deb,
        has_rpm,
    )
}

#[cfg(test)]
mod tests {
    use super::detect_install_info;

    #[test]
    fn linux_deb_and_rpm_do_not_claim_self_update() {
        let deb = detect_install_info("linux", false, false, false, true, false);
        assert_eq!(deb.kind, "deb");
        assert!(!deb.self_update);
        assert_eq!(deb.download_url, "https://getomniget.com/download/linux");

        let rpm = detect_install_info("linux", false, false, false, false, true);
        assert_eq!(rpm.kind, "rpm");
        assert!(!rpm.self_update);

        let appimage = detect_install_info("linux", false, false, true, true, false);
        assert_eq!(appimage.kind, "appimage");
        assert!(appimage.self_update);

        let flatpak = detect_install_info("linux", false, true, false, false, false);
        assert_eq!(flatpak.kind, "flatpak");
        assert!(!flatpak.self_update);
    }

    #[test]
    fn windows_portable_does_not_self_update_while_installer_does() {
        let portable = detect_install_info("windows", true, false, false, false, false);
        assert_eq!(portable.kind, "portable");
        assert!(!portable.self_update);

        let installer = detect_install_info("windows", false, false, false, false, false);
        assert_eq!(installer.kind, "installer");
        assert!(installer.self_update);
    }
}
