use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize)]
pub struct BrowserExtensionStatus {
    pub browser: String,
    pub supported: bool,
    pub bundled_version: Option<String>,
    pub installable: bool,
    pub install_hint_key: String,
    pub store_url: Option<String>,
}

fn read_bundled_manifest_version(app: &AppHandle, browser: &str) -> Option<String> {
    let resource = format!("browser-extension/{}/manifest.json", browser);
    let path = app
        .path()
        .resolve(resource, tauri::path::BaseDirectory::Resource)
        .ok()?;
    let raw = std::fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v.get("version")?.as_str().map(|s| s.to_string())
}

fn extension_export_dir(app: &AppHandle, browser: &str) -> Option<PathBuf> {
    let base = app.path().app_data_dir().ok()?;
    Some(base.join("browser-extension").join(browser))
}

fn copy_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    if !src.exists() {
        return Ok(());
    }
    if src.is_file() {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(src, dst)?;
        return Ok(());
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        copy_recursive(&from, &to)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn browser_extension_status(
    app: AppHandle,
) -> Result<Vec<BrowserExtensionStatus>, String> {
    let chrome_v = read_bundled_manifest_version(&app, "chrome");
    let firefox_v = read_bundled_manifest_version(&app, "firefox");
    Ok(vec![
        BrowserExtensionStatus {
            browser: "chrome".into(),
            supported: true,
            bundled_version: chrome_v,
            installable: true,
            install_hint_key: "chrome".into(),
            store_url: None,
        },
        BrowserExtensionStatus {
            browser: "firefox".into(),
            supported: true,
            bundled_version: firefox_v,
            installable: true,
            install_hint_key: "firefox".into(),
            store_url: None,
        },
        BrowserExtensionStatus {
            browser: "safari".into(),
            supported: false,
            bundled_version: None,
            installable: false,
            install_hint_key: "safari".into(),
            store_url: None,
        },
    ])
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtensionExportResult {
    pub path: String,
    pub version: Option<String>,
    pub browser: String,
}

#[tauri::command]
pub async fn browser_extension_export(
    app: AppHandle,
    browser: String,
) -> Result<ExtensionExportResult, String> {
    if browser != "chrome" && browser != "firefox" {
        return Err(format!("browser '{}' não suportado pra export", browser));
    }
    let resource = format!("browser-extension/{}", browser);
    let src = app
        .path()
        .resolve(resource, tauri::path::BaseDirectory::Resource)
        .map_err(|e| format!("resolve resource: {}", e))?;
    let dst = extension_export_dir(&app, &browser)
        .ok_or_else(|| "could not derive extension dir".to_string())?;
    if dst.exists() {
        let _ = std::fs::remove_dir_all(&dst);
    }
    std::fs::create_dir_all(&dst).map_err(|e| format!("mkdir: {}", e))?;
    copy_recursive(&src, &dst).map_err(|e| format!("copy extension: {}", e))?;

    let version = read_bundled_manifest_version(&app, &browser);

    Ok(ExtensionExportResult {
        path: dst.to_string_lossy().into_owned(),
        version,
        browser,
    })
}

#[tauri::command]
pub async fn browser_extension_open_folder(path: String) -> Result<(), String> {
    let path = PathBuf::from(&path);
    if !path.exists() {
        return Err(format!("path not found: {}", path.display()));
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("explorer: {}", e))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("open: {}", e))?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("xdg-open: {}", e))?;
    }
    Ok(())
}
