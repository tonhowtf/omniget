use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

const MAX_COOKIES_PER_REQUEST: usize = 500;
const METADATA_TTL_SECS: u64 = 60;

static METADATA_LOCK: Mutex<()> = Mutex::new(());
static METADATA_TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn lock_metadata() -> MutexGuard<'static, ()> {
    METADATA_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

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
    #[serde(default, rename = "manifestText")]
    pub manifest_text: Option<String>,
}

pub fn cookie_limit() -> usize {
    MAX_COOKIES_PER_REQUEST
}

pub fn extension_cookie_file_path() -> PathBuf {
    crate::core::paths::app_data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("chrome-extension-cookies.txt")
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

fn prune_expired_metadata(map: &mut serde_json::Map<String, serde_json::Value>, now: u64) {
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
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "extension-metadata.json".to_string());
    let tmp = path.with_file_name(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        METADATA_TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    if let Err(e) = fs::write(&tmp, serialized).and_then(|_| fs::rename(&tmp, path)) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

pub fn write_extension_metadata(payload: &ExtensionPayload) -> anyhow::Result<()> {
    let _guard = lock_metadata();
    write_extension_metadata_at(&extension_metadata_path(), payload)
}

fn write_extension_metadata_at(
    path: &std::path::Path,
    payload: &ExtensionPayload,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let now = current_unix_timestamp();
    let mut map = load_metadata_map(path);
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
    write_metadata_map(path, &map)?;
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
    let _guard = lock_metadata();
    take_extension_metadata_at(&extension_metadata_path(), url)
}

/// Drops the entry for `url` (e.g. headers that were staged for a queue
/// attempt that never consumed them) so credentials do not linger on disk.
pub fn remove_extension_metadata(url: &str) {
    let _ = read_extension_metadata(url);
}

fn take_extension_metadata_at(path: &std::path::Path, url: &str) -> Option<ExtensionMetadata> {
    let now = current_unix_timestamp();
    let mut map = load_metadata_map(path);
    prune_expired_metadata(&mut map, now);

    let entry = map.remove(url)?;
    let result = parse_metadata_entry(&entry);

    let _ = write_metadata_map(path, &map);

    Some(result)
}

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
    let _guard = lock_metadata();
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

    fn payload(url: &str, title: &str) -> ExtensionPayload {
        ExtensionPayload {
            url: url.to_string(),
            cookies: None,
            referer: Some("https://ref.example/".to_string()),
            headers: Some(HashMap::from([("cookie".to_string(), "a=1".to_string())])),
            media_type: None,
            content_type: None,
            title: Some(title.to_string()),
            thumbnail: None,
            open_app: None,
            page_url: None,
            user_agent: None,
            manifest_text: None,
        }
    }

    #[test]
    fn concurrent_writes_and_takes_keep_every_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extension-metadata.json");
        let handles: Vec<_> = (0..16)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let url = format!("https://h.example/{i}");
                    {
                        let _guard = lock_metadata();
                        write_extension_metadata_at(&path, &payload(&url, "t")).unwrap();
                    }
                    let _guard = lock_metadata();
                    take_extension_metadata_at(&path, &url).map(|m| m.title)
                })
            })
            .collect();
        for h in handles {
            assert_eq!(h.join().unwrap(), Some(Some("t".to_string())));
        }
        assert!(!path.exists());
        let leftovers = fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn take_removes_only_the_requested_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extension-metadata.json");
        write_extension_metadata_at(&path, &payload("https://a/", "A")).unwrap();
        write_extension_metadata_at(&path, &payload("https://b/", "B")).unwrap();
        let a = take_extension_metadata_at(&path, "https://a/").unwrap();
        assert_eq!(a.headers.unwrap().get("cookie").unwrap(), "a=1");
        assert!(take_extension_metadata_at(&path, "https://a/").is_none());
        let raw = fs::read_to_string(&path).unwrap();
        assert!(raw.contains("https://b/") && !raw.contains("https://a/"));
    }
}
