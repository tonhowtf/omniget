use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[cfg(any(target_os = "linux", target_os = "macos"))]
use anyhow::Context;
use serde::{Deserialize, Serialize};

pub const CHROME_HOST_NAME: &str = "wtf.tonho.omniget";
pub const CHROME_EXTENSION_IDS: &[&str] = &["dkjelkhaaakffpghdfalobccaaipajip"];
const FIREFOX_EXTENSION_ID: &str = "omniget@tonho.wtf";
const HOST_MAX_PROTOCOL_VERSION: u32 = 1;
const MAX_MESSAGE_LENGTH: usize = 1_048_576;
const MAX_COOKIES_PER_REQUEST: usize = 500;

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
    #[serde(default)]
    url: String,
    #[serde(default, rename = "protocolVersion")]
    protocol_version: Option<u32>,
    #[serde(default)]
    cookies: Option<Vec<NativeCookie>>,
    #[serde(default)]
    referer: Option<String>,
    #[serde(default)]
    headers: Option<std::collections::HashMap<String, String>>,
    #[serde(default, rename = "mediaType")]
    media_type: Option<String>,
    #[serde(default, rename = "contentType")]
    content_type: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default, rename = "openApp")]
    open_app: Option<bool>,
    #[serde(default, rename = "pageUrl")]
    page_url: Option<String>,
    #[serde(default, rename = "userAgent")]
    user_agent: Option<String>,
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
    #[serde(default, rename = "hostOnly")]
    host_only: Option<bool>,
    #[serde(default, rename = "sameSite")]
    #[allow(dead_code)]
    same_site: Option<String>,
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
        .and_then(|path| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
        })
        .map(|stem| stem.eq_ignore_ascii_case(HOST_BINARY_STEM))
        .unwrap_or(false)
}

pub fn run_native_host() -> anyhow::Result<()> {
    detect_portable_mode();
    let response = match read_message()? {
        ReadOutcome::Ok(request) => handle_request(request),
        ReadOutcome::TooLarge(length) => {
            log_host_event(
                "PAYLOAD_TOO_LARGE",
                &format!("payload_len={length} limit={MAX_MESSAGE_LENGTH}"),
            );
            NativeHostResponse {
                ok: false,
                code: Some("PAYLOAD_TOO_LARGE"),
                message: Some(format!(
                    "Native message ({length} bytes) exceeds {MAX_MESSAGE_LENGTH} bytes limit"
                )),
            }
        }
        ReadOutcome::MalformedJson(details) => {
            log_host_event("INVALID_PAYLOAD", &details);
            NativeHostResponse {
                ok: false,
                code: Some("INVALID_PAYLOAD"),
                message: Some(format!("Failed to parse native message: {details}")),
            }
        }
    };
    write_message(&response)?;
    Ok(())
}

enum ReadOutcome {
    Ok(NativeHostRequest),
    TooLarge(usize),
    MalformedJson(String),
}

fn host_log_path() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|dir| dir.join("native-host.log"))
}

fn log_host_event(kind: &str, detail: &str) {
    let ts = current_unix_timestamp();
    let line = format!("{ts} [{kind}] {detail}\n");
    eprintln!("[native-host] {kind} {detail}");
    if let Some(path) = host_log_path() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| f.write_all(line.as_bytes()));
    }
}

pub(crate) fn safe_payload_summary(payload: &[u8]) -> String {
    let payload_len = payload.len();
    let parsed: Result<serde_json::Value, _> = serde_json::from_slice(payload);
    match parsed {
        Ok(serde_json::Value::Object(map)) => {
            let type_ = map
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("<missing>");
            let has_url = map.get("url").is_some();
            let cookie_count = map
                .get("cookies")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let protocol_version = map
                .get("protocolVersion")
                .and_then(|v| v.as_u64())
                .map(|v| v.to_string())
                .unwrap_or_else(|| "<none>".to_string());
            let mut keys: Vec<&str> = map.keys().map(|s| s.as_str()).collect();
            keys.sort_unstable();
            format!(
                "payload_len={payload_len} type=\"{type_}\" has_url={has_url} cookie_count={cookie_count} protocolVersion={protocol_version} keys={keys:?}"
            )
        }
        Ok(_) => format!("payload_len={payload_len} not_a_json_object"),
        Err(_) => format!("payload_len={payload_len} unparseable"),
    }
}

fn build_deserialize_error_detail(payload: &[u8], err: &serde_json::Error) -> String {
    format!(
        "err=\"{}\" line={} column={} category={:?} | {}",
        err,
        err.line(),
        err.column(),
        err.classify(),
        safe_payload_summary(payload)
    )
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

    if let Ok(ff_path) = firefox_manifest_path(&integration_dir) {
        if let Some(parent) = ff_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = write_firefox_manifest(&ff_path, &host_exe);
        let _ = register_firefox_manifest(&ff_path);
    }

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
    fs::write(
        manifest_path,
        serde_json::to_vec_pretty(&build_host_manifest(host_exe))?,
    )?;
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
    let base_dir = dirs::config_dir().context(
        "Could not resolve the Linux config directory for Chrome native host registration",
    )?;
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

fn build_firefox_manifest(host_exe: &Path) -> serde_json::Value {
    serde_json::json!({
        "name": CHROME_HOST_NAME,
        "description": "OmniGet native host for Firefox",
        "path": host_exe.to_string_lossy().to_string(),
        "type": "stdio",
        "allowed_extensions": [FIREFOX_EXTENSION_ID]
    })
}

fn write_firefox_manifest(manifest_path: &Path, host_exe: &Path) -> anyhow::Result<()> {
    fs::write(
        manifest_path,
        serde_json::to_vec_pretty(&build_firefox_manifest(host_exe))?,
    )?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn firefox_manifest_path(integration_dir: &Path) -> anyhow::Result<PathBuf> {
    Ok(integration_dir.join(format!("{}.firefox.json", CHROME_HOST_NAME)))
}

#[cfg(target_os = "linux")]
fn firefox_manifest_path(_integration_dir: &Path) -> anyhow::Result<PathBuf> {
    let home = dirs::home_dir()
        .context("Could not resolve home directory for Firefox native host registration")?;
    Ok(home
        .join(".mozilla")
        .join("native-messaging-hosts")
        .join(format!("{}.json", CHROME_HOST_NAME)))
}

#[cfg(target_os = "macos")]
fn firefox_manifest_path(_integration_dir: &Path) -> anyhow::Result<PathBuf> {
    let home = dirs::home_dir()
        .context("Could not resolve home directory for Firefox native host registration")?;
    Ok(home
        .join("Library")
        .join("Application Support")
        .join("Mozilla")
        .join("NativeMessagingHosts")
        .join(format!("{}.json", CHROME_HOST_NAME)))
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
fn firefox_manifest_path(_integration_dir: &Path) -> anyhow::Result<PathBuf> {
    anyhow::bail!("Firefox native host registration is unsupported on this platform");
}

#[cfg(target_os = "windows")]
fn register_firefox_manifest(manifest_path: &Path) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let _ = std::process::Command::new("reg")
        .args([
            "add",
            &format!(
                r"HKCU\Software\Mozilla\NativeMessagingHosts\{}",
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
        .status();
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn register_firefox_manifest(_manifest_path: &Path) -> anyhow::Result<()> {
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

fn root_domain_of(host: &str) -> String {
    let h = host.trim_start_matches('.').to_lowercase();
    let parts: Vec<&str> = h.split('.').collect();
    if parts.len() >= 2 {
        parts[parts.len() - 2..].join(".")
    } else {
        h
    }
}

fn format_cookie_line(c: &NativeCookie, session_ttl: u64) -> String {
    let raw_domain = sanitize_cookie_field(&c.domain);
    let path_field = sanitize_cookie_field(&c.path);
    let name = sanitize_cookie_field(&c.name);
    let value = sanitize_cookie_field(&c.value);
    let http_only_prefix = if c.http_only { "#HttpOnly_" } else { "" };
    let is_host_only = c
        .host_only
        .unwrap_or_else(|| !raw_domain.starts_with('.'));
    let (domain, include_subdomains) = if is_host_only {
        let stripped = raw_domain
            .strip_prefix('.')
            .unwrap_or(&raw_domain)
            .to_string();
        (stripped, "FALSE")
    } else if raw_domain.starts_with('.') {
        (raw_domain.clone(), "TRUE")
    } else {
        (format!(".{}", raw_domain), "TRUE")
    };
    let secure = if c.secure { "TRUE" } else { "FALSE" };
    let expires = if c.expires == 0 {
        session_ttl
    } else {
        c.expires as u64
    };
    format!(
        "{}{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        http_only_prefix, domain, include_subdomains, path_field, secure, expires, name, value,
    )
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

    // Merge with existing file: keep cookies whose root domain is NOT in the
    // incoming batch. Each platform capture only ships its own domains, so
    // overwriting the file would wipe other platforms' auth.
    let incoming_roots: std::collections::HashSet<String> = cookies
        .iter()
        .map(|c| root_domain_of(&c.domain))
        .collect();

    let mut preserved: Vec<String> = Vec::new();
    if let Ok(existing) = fs::read_to_string(&path) {
        for line in existing.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let effective = if let Some(rest) = trimmed.strip_prefix("#HttpOnly_") {
                rest
            } else if trimmed.starts_with('#') {
                continue; // header / comment
            } else {
                trimmed
            };
            let parts: Vec<&str> = effective.split('\t').collect();
            if parts.len() < 7 {
                continue;
            }
            let domain = parts[0];
            let root = root_domain_of(domain);
            if !incoming_roots.contains(&root) {
                preserved.push(line.to_string());
            }
        }
    }

    let mut content = String::from("# Netscape HTTP Cookie File\n");
    for line in &preserved {
        content.push_str(line);
        content.push('\n');
    }
    for c in cookies {
        content.push_str(&format_cookie_line(c, session_ttl));
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

const METADATA_TTL_SECS: u64 = 60;

fn current_unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn load_metadata_map(path: &std::path::Path) -> serde_json::Map<String, serde_json::Value> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return serde_json::Map::new(),
    };
    let parsed: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return serde_json::Map::new(),
    };
    match parsed {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    }
}

fn prune_expired_metadata(
    map: &mut serde_json::Map<String, serde_json::Value>,
    now: u64,
) {
    map.retain(|_, v| {
        v.get("timestamp")
            .and_then(|t| t.as_u64())
            .map(|ts| now.saturating_sub(ts) <= METADATA_TTL_SECS)
            .unwrap_or(false)
    });
}

fn write_metadata_map(
    path: &std::path::Path,
    map: &serde_json::Map<String, serde_json::Value>,
) -> anyhow::Result<()> {
    if map.is_empty() {
        let _ = fs::remove_file(path);
        return Ok(());
    }
    let serialized = serde_json::to_string(&serde_json::Value::Object(map.clone()))?;
    fs::write(path, serialized)?;
    Ok(())
}

fn write_extension_metadata(request: &NativeHostRequest) -> anyhow::Result<()> {
    let path = extension_metadata_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let now = current_unix_timestamp();
    let mut map = load_metadata_map(&path);
    prune_expired_metadata(&mut map, now);

    let entry = serde_json::json!({
        "referer": request.referer,
        "headers": request.headers,
        "mediaType": request.media_type,
        "contentType": request.content_type,
        "title": request.title,
        "thumbnail": request.thumbnail,
        "openApp": request.open_app,
        "pageUrl": request.page_url,
        "userAgent": request.user_agent,
        "timestamp": now,
    });

    map.insert(request.url.clone(), entry);
    write_metadata_map(&path, &map)?;
    Ok(())
}

pub struct ExtensionMetadata {
    pub referer: Option<String>,
    pub headers: Option<std::collections::HashMap<String, String>>,
    pub media_type: Option<String>,
    pub content_type: Option<String>,
    pub title: Option<String>,
    pub thumbnail: Option<String>,
    pub open_app: Option<bool>,
    pub page_url: Option<String>,
    pub user_agent: Option<String>,
}

fn parse_metadata_entry(meta: &serde_json::Value) -> ExtensionMetadata {
    let headers = meta.get("headers").and_then(|v| v.as_object()).map(|obj| {
        obj.iter()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
            .collect::<std::collections::HashMap<String, String>>()
    });

    ExtensionMetadata {
        referer: meta
            .get("referer")
            .and_then(|v| v.as_str())
            .map(String::from),
        headers,
        media_type: meta
            .get("mediaType")
            .and_then(|v| v.as_str())
            .map(String::from),
        content_type: meta
            .get("contentType")
            .and_then(|v| v.as_str())
            .map(String::from),
        title: meta
            .get("title")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        thumbnail: meta
            .get("thumbnail")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        open_app: meta.get("openApp").and_then(|v| v.as_bool()),
        page_url: meta
            .get("pageUrl")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
        user_agent: meta
            .get("userAgent")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from),
    }
}

pub fn read_extension_metadata(url: &str) -> Option<ExtensionMetadata> {
    let path = extension_metadata_path();
    let now = current_unix_timestamp();
    let mut map = load_metadata_map(&path);
    prune_expired_metadata(&mut map, now);

    let entry = map.remove(url)?;
    let result = parse_metadata_entry(&entry);

    let _ = write_metadata_map(&path, &map);

    Some(result)
}

pub fn peek_extension_open_app(url: &str) -> Option<bool> {
    let path = extension_metadata_path();
    let now = current_unix_timestamp();
    let mut map = load_metadata_map(&path);
    prune_expired_metadata(&mut map, now);

    let entry = map.get(url)?;
    let timestamp = entry.get("timestamp").and_then(|v| v.as_u64())?;
    if now.saturating_sub(timestamp) > METADATA_TTL_SECS {
        return None;
    }
    entry.get("openApp").and_then(|v| v.as_bool())
}

fn handle_request(request: NativeHostRequest) -> NativeHostResponse {
    if let Some(client_version) = request.protocol_version {
        if client_version > HOST_MAX_PROTOCOL_VERSION {
            log_host_event(
                "UNSUPPORTED_PROTOCOL",
                &format!(
                    "client_version={client_version} host_max_version={HOST_MAX_PROTOCOL_VERSION}"
                ),
            );
            return NativeHostResponse {
                ok: false,
                code: Some("UNSUPPORTED_PROTOCOL"),
                message: Some(format!(
                    "Extension protocol v{} is newer than host v{}. Please update OmniGet.",
                    client_version, HOST_MAX_PROTOCOL_VERSION
                )),
            };
        }
    }

    if request.kind == "cookies:export" {
        if let Some(ref cookies) = request.cookies {
            if cookies.len() > MAX_COOKIES_PER_REQUEST {
                return NativeHostResponse {
                    ok: false,
                    code: Some("TOO_MANY_COOKIES"),
                    message: Some(format!(
                        "cookies:export contains {} cookies; max is {MAX_COOKIES_PER_REQUEST}",
                        cookies.len()
                    )),
                };
            }
            if !cookies.is_empty() {
                if let Err(e) = write_extension_cookies(cookies) {
                    return NativeHostResponse {
                        ok: false,
                        code: Some("WRITE_FAILED"),
                        message: Some(format!("write extension cookies: {}", e)),
                    };
                }
            }
        }
        return NativeHostResponse {
            ok: true,
            code: None,
            message: None,
        };
    }

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
        if cookies.len() > MAX_COOKIES_PER_REQUEST {
            return NativeHostResponse {
                ok: false,
                code: Some("TOO_MANY_COOKIES"),
                message: Some(format!(
                    "Request contains {} cookies; max allowed is {MAX_COOKIES_PER_REQUEST}",
                    cookies.len()
                )),
            };
        }
        if !cookies.is_empty() {
            if let Err(e) = write_extension_cookies(cookies) {
                eprintln!("[OmniGet] Warning: failed to write extension cookies: {e}");
            }
        }
    }

    if request.referer.is_some()
        || request.headers.is_some()
        || request.media_type.is_some()
        || request.title.is_some()
        || request.thumbnail.is_some()
        || request.open_app.is_some()
        || request.page_url.is_some()
        || request.user_agent.is_some()
    {
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

fn read_message() -> anyhow::Result<ReadOutcome> {
    let mut length_bytes = [0u8; 4];
    std::io::stdin().read_exact(&mut length_bytes)?;
    let length = u32::from_le_bytes(length_bytes) as usize;

    if length > MAX_MESSAGE_LENGTH {
        return Ok(ReadOutcome::TooLarge(length));
    }

    let mut payload = vec![0u8; length];
    std::io::stdin().read_exact(&mut payload)?;
    match serde_json::from_slice(&payload) {
        Ok(request) => Ok(ReadOutcome::Ok(request)),
        Err(err) => Ok(ReadOutcome::MalformedJson(build_deserialize_error_detail(
            &payload, &err,
        ))),
    }
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
    fn rejects_protocol_version_newer_than_host() {
        let payload = serde_json::json!({
            "type": "enqueue",
            "url": "https://example.com/video",
            "protocolVersion": HOST_MAX_PROTOCOL_VERSION + 1,
        });
        let request: NativeHostRequest = serde_json::from_value(payload).unwrap();
        let response = handle_request(request);
        assert!(!response.ok);
        assert_eq!(response.code, Some("UNSUPPORTED_PROTOCOL"));
    }

    #[test]
    fn accepts_missing_protocol_version_for_backwards_compat() {
        let payload = serde_json::json!({
            "type": "enqueue",
            "url": "not a url",
        });
        let request: NativeHostRequest = serde_json::from_value(payload).unwrap();
        assert!(request.protocol_version.is_none());
    }

    #[test]
    fn accepts_current_protocol_version() {
        let payload = serde_json::json!({
            "type": "enqueue",
            "url": "not a url",
            "protocolVersion": HOST_MAX_PROTOCOL_VERSION,
        });
        let request: NativeHostRequest = serde_json::from_value(payload).unwrap();
        assert_eq!(request.protocol_version, Some(HOST_MAX_PROTOCOL_VERSION));
    }

    #[test]
    fn build_host_manifest_contains_expected_fields() {
        #[cfg(target_os = "windows")]
        let host_exe = Path::new(r"C:\tmp\omniget-native-host.exe");

        #[cfg(not(target_os = "windows"))]
        let host_exe = Path::new("/tmp/omniget-native-host");

        let manifest = build_host_manifest(host_exe);

        assert_eq!(manifest["name"].as_str(), Some(CHROME_HOST_NAME));
        assert_eq!(
            manifest["description"].as_str(),
            Some("OmniGet native host for Chrome")
        );
        assert_eq!(
            manifest["path"].as_str(),
            Some(host_exe.to_string_lossy().as_ref())
        );
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

    #[test]
    fn prune_expired_metadata_drops_entries_past_ttl() {
        let mut map = serde_json::Map::new();
        map.insert(
            "https://a".to_string(),
            serde_json::json!({"timestamp": 100_u64, "referer": "r1"}),
        );
        map.insert(
            "https://b".to_string(),
            serde_json::json!({"timestamp": 200_u64, "referer": "r2"}),
        );
        let now = 100 + METADATA_TTL_SECS + 1;
        prune_expired_metadata(&mut map, now);
        assert!(!map.contains_key("https://a"));
        assert!(map.contains_key("https://b"));
    }

    #[test]
    fn metadata_map_roundtrip_preserves_multiple_urls() {
        let tmp = std::env::temp_dir().join(format!(
            "omniget-meta-test-{}.json",
            current_unix_timestamp()
        ));
        let _ = fs::remove_file(&tmp);

        let mut map = serde_json::Map::new();
        map.insert(
            "https://a".to_string(),
            serde_json::json!({"timestamp": 1_u64, "referer": "ra"}),
        );
        map.insert(
            "https://b".to_string(),
            serde_json::json!({"timestamp": 2_u64, "referer": "rb"}),
        );
        write_metadata_map(&tmp, &map).unwrap();
        let loaded = load_metadata_map(&tmp);
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded.get("https://a").and_then(|v| v.get("referer")).and_then(|v| v.as_str()),
            Some("ra")
        );
        assert_eq!(
            loaded.get("https://b").and_then(|v| v.get("referer")).and_then(|v| v.as_str()),
            Some("rb")
        );

        let _ = fs::remove_file(&tmp);
    }

    fn sample_cookie(name: &str) -> serde_json::Value {
        serde_json::json!({
            "domain": ".example.com",
            "httpOnly": false,
            "path": "/",
            "secure": true,
            "expires": 0,
            "name": name,
            "value": "v",
        })
    }

    #[test]
    fn rejects_request_with_too_many_cookies() {
        let cookies: Vec<_> = (0..MAX_COOKIES_PER_REQUEST + 1)
            .map(|i| sample_cookie(&format!("c{i}")))
            .collect();
        let payload = serde_json::json!({
            "type": "enqueue",
            "url": "https://example.com/v",
            "cookies": cookies,
        });
        let request: NativeHostRequest = serde_json::from_value(payload).unwrap();
        let response = handle_request(request);
        assert!(!response.ok);
        assert_eq!(response.code, Some("TOO_MANY_COOKIES"));
    }

    #[test]
    fn accepts_request_at_cookie_limit() {
        let cookies: Vec<_> = (0..MAX_COOKIES_PER_REQUEST)
            .map(|i| sample_cookie(&format!("c{i}")))
            .collect();
        let payload = serde_json::json!({
            "type": "enqueue",
            "url": "not a real url",
            "cookies": cookies,
        });
        let request: NativeHostRequest = serde_json::from_value(payload).unwrap();
        let response = handle_request(request);
        assert_ne!(response.code, Some("TOO_MANY_COOKIES"));
    }

    #[test]
    fn write_empty_metadata_map_removes_file() {
        let tmp = std::env::temp_dir().join(format!(
            "omniget-meta-empty-{}.json",
            current_unix_timestamp()
        ));
        fs::write(&tmp, "{}").unwrap();
        assert!(tmp.exists());
        let empty: serde_json::Map<String, serde_json::Value> = serde_json::Map::new();
        write_metadata_map(&tmp, &empty).unwrap();
        assert!(!tmp.exists());
    }

    #[test]
    fn safe_payload_summary_extracts_non_sensitive_fields() {
        let payload = serde_json::to_vec(&serde_json::json!({
            "type": "enqueue",
            "url": "https://secret.example.com/path?token=SECRET_TOKEN_VALUE",
            "protocolVersion": 1,
            "cookies": [
                {"domain": "a", "httpOnly": false, "path": "/", "secure": true, "expires": 0, "name": "session", "value": "SECRET_VALUE"},
                {"domain": "b", "httpOnly": false, "path": "/", "secure": true, "expires": 0, "name": "csrf", "value": "TOP_SECRET"},
            ],
        })).unwrap();

        let summary = safe_payload_summary(&payload);

        assert!(summary.contains("type=\"enqueue\""));
        assert!(summary.contains("has_url=true"));
        assert!(summary.contains("cookie_count=2"));
        assert!(summary.contains("protocolVersion=1"));
    }

    #[test]
    fn safe_payload_summary_never_leaks_cookie_values() {
        let payload = serde_json::to_vec(&serde_json::json!({
            "type": "enqueue",
            "url": "https://example.com/v",
            "cookies": [
                {"domain": "a", "httpOnly": false, "path": "/", "secure": true, "expires": 0, "name": "session", "value": "SECRET_COOKIE_VALUE_XYZ"},
            ],
        })).unwrap();

        let summary = safe_payload_summary(&payload);

        assert!(!summary.contains("SECRET_COOKIE_VALUE_XYZ"));
        assert!(!summary.contains("session"));
    }

    #[test]
    fn safe_payload_summary_never_leaks_url_query_contents() {
        let payload = serde_json::to_vec(&serde_json::json!({
            "type": "enqueue",
            "url": "https://example.com/?access_token=AAAA_SECRET_TOKEN",
        })).unwrap();

        let summary = safe_payload_summary(&payload);

        assert!(!summary.contains("AAAA_SECRET_TOKEN"));
        assert!(!summary.contains("example.com"));
    }

    #[test]
    fn safe_payload_summary_handles_unparseable_input() {
        let summary = safe_payload_summary(b"{not valid json");

        assert!(summary.contains("unparseable"));
        assert!(summary.contains("payload_len=15"));
    }

    #[test]
    fn safe_payload_summary_handles_non_object_json() {
        let summary = safe_payload_summary(b"[1,2,3]");

        assert!(summary.contains("not_a_json_object"));
    }

    #[test]
    fn safe_payload_summary_marks_missing_type_as_placeholder() {
        let payload = serde_json::to_vec(&serde_json::json!({
            "url": "https://example.com/",
        })).unwrap();

        let summary = safe_payload_summary(&payload);

        assert!(summary.contains("type=\"<missing>\""));
        assert!(summary.contains("has_url=true"));
    }

    #[test]
    fn safe_payload_summary_reports_cookie_count_zero_when_absent() {
        let payload = serde_json::to_vec(&serde_json::json!({
            "type": "enqueue",
            "url": "https://example.com/",
        })).unwrap();

        let summary = safe_payload_summary(&payload);

        assert!(summary.contains("cookie_count=0"));
        assert!(summary.contains("protocolVersion=<none>"));
    }
}
