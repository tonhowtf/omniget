//! Disk-backed storage for cookies and per-URL metadata supplied by the browser
//! extension.
//!
//! The extension communicates with the desktop app over the local HTTP bridge
//! (see `local_bridge.rs`); this module is the only place that knows how to
//! turn the extension's payloads into the on-disk artefacts that yt-dlp and
//! `external_url::queue_url_with_defaults` expect:
//!
//! * a Netscape-format cookie file consumed by yt-dlp's `--cookies` flag,
//! * a small JSON map keyed by URL with metadata (referer, headers, title, …)
//!   that survives the very short window between the extension request and the
//!   moment the queue worker picks the URL up.
//!
//! There is intentionally no networking code here: the bridge module owns the
//! HTTP listener and calls into the helpers below.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const MAX_COOKIES_PER_REQUEST: usize = 500;
const METADATA_TTL_SECS: u64 = 60;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExtensionCookie {
    pub domain: String,
    #[serde(rename = "httpOnly")]
    pub http_only: bool,
    pub path: String,
    pub secure: bool,
    pub expires: i64,
    pub name: String,
    pub value: String,
    #[serde(default, rename = "hostOnly")]
    pub host_only: Option<bool>,
    #[serde(default, rename = "sameSite")]
    pub same_site: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ExtensionPayload {
    pub url: String,
    #[serde(default)]
    pub cookies: Option<Vec<ExtensionCookie>>,
    #[serde(default)]
    pub referer: Option<String>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default, rename = "mediaType")]
    pub media_type: Option<String>,
    #[serde(default, rename = "contentType")]
    pub content_type: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default, rename = "openApp")]
    pub open_app: Option<bool>,
    #[serde(default, rename = "pageUrl")]
    pub page_url: Option<String>,
    #[serde(default, rename = "userAgent")]
    pub user_agent: Option<String>,
}

pub fn cookie_limit() -> usize {
    MAX_COOKIES_PER_REQUEST
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

fn format_cookie_line(c: &ExtensionCookie, session_ttl: u64) -> String {
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

pub fn write_extension_cookies(cookies: &[ExtensionCookie]) -> anyhow::Result<()> {
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

pub fn write_extension_metadata(payload: &ExtensionPayload) -> anyhow::Result<()> {
    let path = extension_metadata_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let now = current_unix_timestamp();
    let mut map = load_metadata_map(&path);
    prune_expired_metadata(&mut map, now);

    let entry = serde_json::json!({
        "referer": payload.referer,
        "headers": payload.headers,
        "mediaType": payload.media_type,
        "contentType": payload.content_type,
        "title": payload.title,
        "thumbnail": payload.thumbnail,
        "openApp": payload.open_app,
        "pageUrl": payload.page_url,
        "userAgent": payload.user_agent,
        "timestamp": now,
    });

    map.insert(payload.url.clone(), entry);
    write_metadata_map(&path, &map)?;
    Ok(())
}

pub struct ExtensionMetadata {
    pub referer: Option<String>,
    pub headers: Option<HashMap<String, String>>,
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
            .collect::<HashMap<String, String>>()
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

/// Best-effort removal of the native-messaging artefacts left behind by the
/// pre-0.5.3 builds: the Chrome / Firefox host manifests and the copied
/// `omniget-native-host` binary that lived under `chrome-native-host/`.
///
/// We can't unregister the manifest atomically across all browsers — the
/// best we can do is delete the files and let Chrome / Firefox skip them
/// silently the next time they look for a host.
pub fn cleanup_legacy_native_messaging() {
    let host_name = "wtf.tonho.omniget";

    if let Some(data_dir) = crate::core::paths::app_data_dir() {
        let dir = data_dir.join("chrome-native-host");
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(config_dir) = dirs::config_dir() {
            let chrome = config_dir
                .join("google-chrome")
                .join("NativeMessagingHosts")
                .join(format!("{host_name}.json"));
            let _ = std::fs::remove_file(chrome);
        }
        if let Some(home) = dirs::home_dir() {
            let firefox = home
                .join(".mozilla")
                .join("native-messaging-hosts")
                .join(format!("{host_name}.json"));
            let _ = std::fs::remove_file(firefox);
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(data_dir) = dirs::data_dir() {
            let chrome = data_dir
                .join("Google")
                .join("Chrome")
                .join("NativeMessagingHosts")
                .join(format!("{host_name}.json"));
            let _ = std::fs::remove_file(chrome);
        }
        if let Some(home) = dirs::home_dir() {
            let firefox = home
                .join("Library")
                .join("Application Support")
                .join("Mozilla")
                .join("NativeMessagingHosts")
                .join(format!("{host_name}.json"));
            let _ = std::fs::remove_file(firefox);
        }
    }

    #[cfg(target_os = "windows")]
    {
        // Remove the registry keys created by the legacy `ensure_registered`
        // path. Failure (key absent, no permissions) is fine — we ignore it.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        for vendor in ["Google\\Chrome", "Mozilla"] {
            let _ = std::process::Command::new("reg")
                .args([
                    "delete",
                    &format!("HKCU\\Software\\{vendor}\\NativeMessagingHosts\\{host_name}"),
                    "/f",
                ])
                .creation_flags(CREATE_NO_WINDOW)
                .status();
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_limit_is_reasonable() {
        assert_eq!(cookie_limit(), 500);
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
            loaded
                .get("https://a")
                .and_then(|v| v.get("referer"))
                .and_then(|v| v.as_str()),
            Some("ra")
        );
        assert_eq!(
            loaded
                .get("https://b")
                .and_then(|v| v.get("referer"))
                .and_then(|v| v.as_str()),
            Some("rb")
        );

        let _ = fs::remove_file(&tmp);
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
}
