use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use anyhow::Context;
use serde::{Deserialize, Serialize};

pub const CHROME_HOST_NAME: &str = "wtf.tonho.omniget";
pub const CHROME_EXTENSION_IDS: &[&str] = &[
    // Unpacked development extension ID derived from browser-extension/chrome/manifest.json.
    "dkjelkhaaakffpghdfalobccaaipajip",
    // Add the Chrome Web Store extension ID here once it is assigned.
];

#[cfg(target_os = "windows")]
const HOST_COPY_NAME: &str = "omniget-native-host.exe";
#[cfg(not(target_os = "windows"))]
const HOST_COPY_NAME: &str = "omniget-native-host";
const HOST_BINARY_STEM: &str = "omniget-native-host";
const HOST_CONFIG_NAME: &str = "native-host-config.json";
const HOST_MANIFEST_NAME: &str = "wtf.tonho.omniget.json";

#[derive(Debug, Deserialize)]
struct NativeHostRequest {
    #[serde(rename = "type")]
    kind: String,
    url: String,
    #[serde(default)]
    cookies: Option<Vec<NativeCookie>>,
    #[serde(default)]
    referer: Option<String>,
    #[serde(default)]
    headers: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct NativeCookie {
    domain: String,
    #[serde(rename = "httpOnly")]
    http_only: bool,
    path: String,
    secure: bool,
    expires: i64,
    name: String,
    value: String,
}

#[derive(Debug, Serialize)]
struct NativeHostResponse {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct NativeHostConfig {
    app_path: String,
}

pub fn should_run_as_native_host() -> bool {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.file_stem().map(|stem| stem.to_string_lossy().to_string()))
        .map(|stem| stem.eq_ignore_ascii_case(HOST_BINARY_STEM))
        .unwrap_or(false)
}

pub fn run_native_host() -> anyhow::Result<()> {
    detect_portable_mode();
    let request = read_message()?;
    let response = handle_request(request);
    write_message(&response)?;
    Ok(())
}

/// Detect portable mode by reading the native-host config to locate the main
/// app executable, then checking its parent directory for portable markers.
/// This mirrors the `check_portable_mode()` logic in `main.rs` so that
/// `app_data_dir()` resolves to the portable data directory when the native
/// host process is launched by Chrome.
fn detect_portable_mode() {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return,
    };
    let host_dir = match exe.parent() {
        Some(d) => d,
        None => return,
    };
    let config_bytes = match fs::read(host_dir.join(HOST_CONFIG_NAME)) {
        Ok(b) => b,
        Err(_) => return,
    };
    let config: NativeHostConfig = match serde_json::from_slice(&config_bytes) {
        Ok(c) => c,
        Err(_) => return,
    };
    let app_dir = match Path::new(&config.app_path).parent() {
        Some(d) => d,
        None => return,
    };
    if app_dir.join("portable.txt").exists() || app_dir.join(".portable").exists() {
        let data_dir = app_dir.join("data");
        let _ = fs::create_dir_all(&data_dir);
        std::env::set_var("OMNIGET_PORTABLE", "1");
        std::env::set_var("OMNIGET_DATA_DIR", data_dir.to_string_lossy().to_string());
    }
}

pub fn ensure_registered() -> anyhow::Result<()> {
    let current_exe = std::env::current_exe()?;
    let integration_dir = chrome_integration_dir();
    fs::create_dir_all(&integration_dir)?;

    let host_exe = integration_dir.join(HOST_COPY_NAME);
    copy_host_exe(&current_exe, &host_exe)?;

    let config_path = integration_dir.join(HOST_CONFIG_NAME);
    write_host_config(&config_path, &current_exe)?;

    let manifest_path = chrome_manifest_path(&integration_dir)?;
    if let Some(parent) = manifest_path.parent() {
        fs::create_dir_all(parent)?;
    }

    write_host_manifest(&manifest_path, &host_exe)?;
    register_host_manifest(&manifest_path)?;

    Ok(())
}

fn chrome_integration_dir() -> PathBuf {
    crate::core::paths::app_data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("chrome-native-host")
}

fn copy_host_exe(source: &Path, dest: &Path) -> anyhow::Result<()> {
    if source != dest && should_copy_exe(source, dest) {
        fs::copy(source, dest)?;
        sync_host_permissions(source, dest)?;
    }
    Ok(())
}

fn write_host_config(config_path: &Path, app_path: &Path) -> anyhow::Result<()> {
    let config = NativeHostConfig {
        app_path: app_path.to_string_lossy().to_string(),
    };
    fs::write(config_path, serde_json::to_vec_pretty(&config)?)?;
    Ok(())
}

fn write_host_manifest(manifest_path: &Path, host_exe: &Path) -> anyhow::Result<()> {
    fs::write(manifest_path, serde_json::to_vec_pretty(&build_host_manifest(host_exe))?)?;
    Ok(())
}

fn build_host_manifest(host_exe: &Path) -> serde_json::Value {
    serde_json::json!({
        "name": CHROME_HOST_NAME,
        "description": "OmniGet native host for Chrome",
        "path": host_exe.to_string_lossy().to_string(),
        "type": "stdio",
        "allowed_origins": chrome_allowed_origins()
    })
}

fn chrome_allowed_origins() -> Vec<String> {
    CHROME_EXTENSION_IDS
        .iter()
        .map(|extension_id| format!("chrome-extension://{extension_id}/"))
        .collect()
}

#[cfg(target_os = "windows")]
fn chrome_manifest_path(integration_dir: &Path) -> anyhow::Result<PathBuf> {
    Ok(chrome_manifest_path_from_base(integration_dir))
}

#[cfg(target_os = "linux")]
fn chrome_manifest_path(_integration_dir: &Path) -> anyhow::Result<PathBuf> {
    let base_dir = dirs::config_dir()
        .context("Could not resolve the Linux config directory for Chrome native host registration")?;
    Ok(chrome_manifest_path_from_base(&base_dir))
}

#[cfg(target_os = "macos")]
fn chrome_manifest_path(_integration_dir: &Path) -> anyhow::Result<PathBuf> {
    let base_dir = dirs::data_dir().context(
        "Could not resolve the macOS Application Support directory for Chrome native host registration",
    )?;
    Ok(chrome_manifest_path_from_base(&base_dir))
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn chrome_manifest_path(_integration_dir: &Path) -> anyhow::Result<PathBuf> {
    anyhow::bail!("Chrome native host registration is unsupported on this platform");
}

#[cfg(target_os = "windows")]
fn chrome_manifest_path_from_base(base: &Path) -> PathBuf {
    base.join(HOST_MANIFEST_NAME)
}

#[cfg(target_os = "linux")]
fn chrome_manifest_path_from_base(base: &Path) -> PathBuf {
    base.join("google-chrome")
        .join("NativeMessagingHosts")
        .join(HOST_MANIFEST_NAME)
}

#[cfg(target_os = "macos")]
fn chrome_manifest_path_from_base(base: &Path) -> PathBuf {
    base.join("Google")
        .join("Chrome")
        .join("NativeMessagingHosts")
        .join(HOST_MANIFEST_NAME)
}

#[cfg(target_os = "windows")]
fn register_host_manifest(manifest_path: &Path) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let status = std::process::Command::new("reg")
        .args([
            "add",
            &format!(
                r"HKCU\Software\Google\Chrome\NativeMessagingHosts\{}",
                CHROME_HOST_NAME
            ),
            "/ve",
            "/t",
            "REG_SZ",
            "/d",
            &manifest_path.to_string_lossy(),
            "/f",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status()?;

    if !status.success() {
        anyhow::bail!("Failed to register Chrome native host");
    }

    Ok(())
}

// On Linux/macOS, placing the manifest JSON in the well-known directory is
// sufficient — no registry step is needed (unlike Windows which requires a
// registry key under HKCU\Software\Google\Chrome\NativeMessagingHosts).
#[cfg(not(target_os = "windows"))]
fn register_host_manifest(_manifest_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_host_permissions(source: &Path, dest: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = fs::metadata(source)?.permissions().mode();
    fs::set_permissions(dest, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_host_permissions(_source: &Path, _dest: &Path) -> anyhow::Result<()> {
    Ok(())
}

fn should_copy_exe(source: &Path, dest: &Path) -> bool {
    let Ok(src_meta) = fs::metadata(source) else {
        return true;
    };
    let Ok(dst_meta) = fs::metadata(dest) else {
        return true;
    };
    src_meta.len() != dst_meta.len()
}

pub fn extension_cookie_file_path() -> PathBuf {
    crate::core::paths::app_data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("chrome-extension-cookies.txt")
}

fn sanitize_cookie_field(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '\n' && *c != '\r' && *c != '\t')
        .collect()
}

fn write_extension_cookies(cookies: &[NativeCookie]) -> anyhow::Result<()> {
    let path = extension_cookie_file_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Session cookies (expires == 0) must be given a future TTL.
    // Python's MozillaCookieJar treats 0 as "expired at epoch" and discards
    // the cookie, which strips auth context and breaks yt-dlp downloads.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let session_ttl = now + 86400; // 24 h, same approach as Cookie-Editor

    let mut content = String::from("# Netscape HTTP Cookie File\n");
    for c in cookies {
        let domain = sanitize_cookie_field(&c.domain);
        let path_field = sanitize_cookie_field(&c.path);
        let name = sanitize_cookie_field(&c.name);
        let value = sanitize_cookie_field(&c.value);
        let http_only_prefix = if c.http_only { "#HttpOnly_" } else { "" };
        let include_subdomains = if domain.starts_with('.') { "TRUE" } else { "FALSE" };
        let secure = if c.secure { "TRUE" } else { "FALSE" };
        let expires = if c.expires == 0 { session_ttl } else { c.expires as u64 };
        content.push_str(&format!(
            "{}{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            http_only_prefix,
            domain,
            include_subdomains,
            path_field,
            secure,
            expires,
            name,
            value,
        ));
    }

    fs::write(&path, content)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

fn extension_metadata_path() -> PathBuf {
    crate::core::paths::app_data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("extension-metadata.json")
}

fn write_extension_metadata(request: &NativeHostRequest) -> anyhow::Result<()> {
    let path = extension_metadata_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let meta = serde_json::json!({
        "url": request.url,
        "referer": request.referer,
        "headers": request.headers,
        "timestamp": now,
    });

    fs::write(&path, serde_json::to_string(&meta)?)?;
    Ok(())
}

pub fn read_extension_metadata(url: &str) -> Option<String> {
    let path = extension_metadata_path();
    let content = fs::read_to_string(&path).ok()?;
    let meta: serde_json::Value = serde_json::from_str(&content).ok()?;

    let meta_url = meta.get("url")?.as_str()?;
    if meta_url != url {
        return None;
    }

    let timestamp = meta.get("timestamp")?.as_u64()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if now.saturating_sub(timestamp) > 60 {
        return None;
    }

    let referer = meta.get("referer").and_then(|v| v.as_str()).map(String::from);

    let _ = fs::remove_file(&path);

    referer
}

fn handle_request(request: NativeHostRequest) -> NativeHostResponse {
    if request.kind != "enqueue" {
        return NativeHostResponse {
            ok: false,
            code: Some("INVALID_URL"),
            message: Some("Unsupported native host message".to_string()),
        };
    }

    if !crate::external_url::is_external_url(&request.url) {
        return NativeHostResponse {
            ok: false,
            code: Some("INVALID_URL"),
            message: Some("The requested URL is invalid".to_string()),
        };
    }

    if let Some(ref cookies) = request.cookies {
        if !cookies.is_empty() {
            if let Err(e) = write_extension_cookies(cookies) {
                eprintln!("[OmniGet] Warning: failed to write extension cookies: {e}");
            }
        }
    }

    if request.referer.is_some() || request.headers.is_some() {
        if let Err(e) = write_extension_metadata(&request) {
            eprintln!("[OmniGet] Warning: failed to write extension metadata: {e}");
        }
    }

    match launch_omniget(&request.url) {
        Ok(()) => NativeHostResponse {
            ok: true,
            code: None,
            message: None,
        },
        Err(error) => NativeHostResponse {
            ok: false,
            code: Some("LAUNCH_FAILED"),
            message: Some(error.to_string()),
        },
    }
}

fn launch_omniget(url: &str) -> anyhow::Result<()> {
    let current_exe = std::env::current_exe()?;
    let config_path = current_exe
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(HOST_CONFIG_NAME);
    let config: NativeHostConfig = serde_json::from_slice(&fs::read(config_path)?)?;

    let mut command = std::process::Command::new(config.app_path);
    command.arg(url);

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    command.spawn()?;
    Ok(())
}

fn read_message() -> anyhow::Result<NativeHostRequest> {
    const MAX_MESSAGE_LENGTH: usize = 1_048_576; // 1 MB — Chrome's own limit

    let mut length_bytes = [0u8; 4];
    std::io::stdin().read_exact(&mut length_bytes)?;
    let length = u32::from_le_bytes(length_bytes) as usize;

    if length > MAX_MESSAGE_LENGTH {
        anyhow::bail!(
            "Native message too large ({length} bytes, max {MAX_MESSAGE_LENGTH})"
        );
    }

    let mut payload = vec![0u8; length];
    std::io::stdin().read_exact(&mut payload)?;
    Ok(serde_json::from_slice(&payload)?)
}

fn write_message(response: &NativeHostResponse) -> anyhow::Result<()> {
    let payload = serde_json::to_vec(response)?;
    let length = (payload.len() as u32).to_le_bytes();

    let mut stdout = std::io::stdout();
    stdout.write_all(&length)?;
    stdout.write_all(&payload)?;
    stdout.flush()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_copy_name_matches_platform() {
        #[cfg(target_os = "windows")]
        assert_eq!(HOST_COPY_NAME, "omniget-native-host.exe");

        #[cfg(not(target_os = "windows"))]
        assert_eq!(HOST_COPY_NAME, "omniget-native-host");
    }

    #[test]
    fn manifest_path_from_base_matches_platform_location() {
        #[cfg(target_os = "windows")]
        let base = Path::new(r"C:\Users\test\AppData\Roaming\omniget\chrome-native-host");

        #[cfg(not(target_os = "windows"))]
        let base = Path::new("/tmp/chrome-base");

        let manifest_path = chrome_manifest_path_from_base(base);

        #[cfg(target_os = "windows")]
        assert_eq!(manifest_path, base.join(HOST_MANIFEST_NAME));

        #[cfg(target_os = "linux")]
        assert_eq!(
            manifest_path,
            base.join("google-chrome")
                .join("NativeMessagingHosts")
                .join(HOST_MANIFEST_NAME)
        );

        #[cfg(target_os = "macos")]
        assert_eq!(
            manifest_path,
            base.join("Google")
                .join("Chrome")
                .join("NativeMessagingHosts")
                .join(HOST_MANIFEST_NAME)
        );
    }

    #[test]
    fn build_host_manifest_contains_expected_fields() {
        #[cfg(target_os = "windows")]
        let host_exe = Path::new(r"C:\tmp\omniget-native-host.exe");

        #[cfg(not(target_os = "windows"))]
        let host_exe = Path::new("/tmp/omniget-native-host");

        let manifest = build_host_manifest(host_exe);

        assert_eq!(manifest["name"].as_str(), Some(CHROME_HOST_NAME));
        assert_eq!(manifest["description"].as_str(), Some("OmniGet native host for Chrome"));
        assert_eq!(manifest["path"].as_str(), Some(host_exe.to_string_lossy().as_ref()));
        assert_eq!(manifest["type"].as_str(), Some("stdio"));
        let allowed_origins = manifest["allowed_origins"]
            .as_array()
            .expect("allowed_origins should be an array");

        assert_eq!(allowed_origins.len(), CHROME_EXTENSION_IDS.len());

        for (origin, extension_id) in allowed_origins.iter().zip(CHROME_EXTENSION_IDS.iter()) {
            assert_eq!(
                origin.as_str(),
                Some(format!("chrome-extension://{extension_id}/").as_str())
            );
        }
    }
}
