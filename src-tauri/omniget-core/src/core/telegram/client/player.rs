//! Player support: stream URLs with the file's real size and type, where
//! the person stopped watching, and handing a stream to VLC/IINA/mpv.

use std::collections::HashMap;
use std::path::PathBuf;

use grammers_client::grammers_tl_types as tl;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::auth::TelegramSessionHandle;
use super::stream_server;
use crate::core::telegram::host;

// ---- stream URL ------------------------------------------------------------

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// `{url, mime, file_name, file_size, duration_sec?, width?, height?}` for
/// one message's file. The URL ends with the file name so external players
/// can tell the format.
pub async fn stream_url(
    handle: &TelegramSessionHandle,
    chat_id: i64,
    chat_type: &str,
    message_id: i32,
) -> Result<serde_json::Value, String> {
    let (base, token) = super::stream_endpoint().await?;
    let (_, media) = stream_server::media_for(handle, chat_id, chat_type, message_id)
        .await
        .map_err(|e| e.to_string())?;

    let mut duration: Option<f64> = None;
    let mut width: Option<i32> = None;
    let mut height: Option<i32> = None;
    match &media.media {
        tl::enums::MessageMedia::Document(d) => {
            if let Some(tl::enums::Document::Document(doc)) = d.document.as_ref() {
                for attr in &doc.attributes {
                    match attr {
                        tl::enums::DocumentAttribute::Video(v) => {
                            duration = Some(v.duration);
                            width = Some(v.w);
                            height = Some(v.h);
                        }
                        tl::enums::DocumentAttribute::Audio(a) if duration.is_none() => {
                            duration = Some(a.duration as f64);
                        }
                        _ => {}
                    }
                }
            }
        }
        tl::enums::MessageMedia::Photo(p) => {
            if let Some(tl::enums::Photo::Photo(photo)) = p.photo.as_ref() {
                if let Some(ps) = photo
                    .sizes
                    .iter()
                    .filter_map(|s| match s {
                        tl::enums::PhotoSize::Size(ps) => Some(ps),
                        _ => None,
                    })
                    .max_by_key(|ps| ps.size)
                {
                    width = Some(ps.w);
                    height = Some(ps.h);
                }
            }
        }
        _ => {}
    }

    let url = format!(
        "{}/stream/{}/{}/{}/{}?token={}",
        base,
        chat_type,
        chat_id,
        message_id,
        url_encode(&media.file_name),
        token
    );
    // MKV/AVI/WebM don't open in the WKWebView: offer the HLS remux.
    let playable = !stream_server::needs_remux(&media.file_name, &media.mime);
    let (remux_url, transcode_url) = if playable {
        (None, None)
    } else {
        let base_remux = format!(
            "{}/remux/{}/{}/{}/index.m3u8?token={}",
            base, chat_type, chat_id, message_id, token
        );
        (
            Some(format!("{}&mode=aac", base_remux)),
            Some(format!("{}&mode=transcode", base_remux)),
        )
    };
    Ok(json!({
        "url": url,
        "mime": media.mime,
        "file_name": media.file_name,
        "file_size": media.file_size,
        "media_type": media.media_type,
        "duration_sec": duration,
        "width": width,
        "height": height,
        "playable": playable,
        "container": super::template::ext_of(&media.file_name),
        "remux_url": remux_url,
        "transcode_url": transcode_url,
    }))
}

// ---- playback positions ----------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackEntry {
    pub chat_id: i64,
    pub chat_type: String,
    pub message_id: i32,
    pub position_sec: f64,
    pub duration_sec: f64,
    pub finished: bool,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_title: Option<String>,
}

const MAX_ENTRIES: usize = 500;

fn playback_path() -> PathBuf {
    host::data_dir().join("playback.json")
}

fn lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn load() -> HashMap<String, PlaybackEntry> {
    std::fs::read(playback_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save(map: &HashMap<String, PlaybackEntry>) -> Result<(), String> {
    let path = playback_path();
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(map).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

fn key(chat_id: i64, message_id: i32) -> String {
    format!("{}:{}", chat_id, message_id)
}

/// Watched to the end: within the last 5% or the last 10 seconds.
fn is_finished(position: f64, duration: f64) -> bool {
    duration > 0.0 && (position >= duration * 0.95 || duration - position <= 10.0)
}

pub fn playback_get(chat_id: i64, message_id: i32) -> serde_json::Value {
    let _g = lock().lock();
    match load().get(&key(chat_id, message_id)) {
        Some(e) => json!({
            "position_sec": e.position_sec,
            "duration_sec": e.duration_sec,
            "finished": e.finished,
        }),
        None => serde_json::Value::Null,
    }
}

#[allow(clippy::too_many_arguments)]
pub fn playback_set(
    chat_id: i64,
    chat_type: &str,
    message_id: i32,
    position_sec: f64,
    duration_sec: f64,
    file_name: Option<String>,
    chat_title: Option<String>,
) -> Result<serde_json::Value, String> {
    let _g = lock().lock();
    let mut map = load();
    let k = key(chat_id, message_id);
    let prev = map.get(&k).cloned();
    let entry = PlaybackEntry {
        chat_id,
        chat_type: chat_type.to_string(),
        message_id,
        position_sec: position_sec.max(0.0),
        duration_sec: duration_sec.max(0.0),
        finished: is_finished(position_sec, duration_sec),
        updated_at: super::now_secs(),
        file_name: file_name.or_else(|| prev.as_ref().and_then(|p| p.file_name.clone())),
        chat_title: chat_title.or_else(|| prev.as_ref().and_then(|p| p.chat_title.clone())),
    };
    let out = json!({
        "position_sec": entry.position_sec,
        "duration_sec": entry.duration_sec,
        "finished": entry.finished,
    });
    map.insert(k, entry);
    if map.len() > MAX_ENTRIES {
        let mut by_age: Vec<(String, i64)> =
            map.iter().map(|(k, e)| (k.clone(), e.updated_at)).collect();
        by_age.sort_by_key(|(_, t)| *t);
        for (k, _) in by_age.into_iter().take(map.len() - MAX_ENTRIES) {
            map.remove(&k);
        }
    }
    save(&map)?;
    Ok(out)
}

/// "Continue watching": started (5 s or more), not finished, newest first.
pub fn playback_recent(limit: usize) -> Vec<PlaybackEntry> {
    let _g = lock().lock();
    let mut list: Vec<PlaybackEntry> = load()
        .into_values()
        .filter(|e| !e.finished && e.position_sec >= 5.0)
        .collect();
    list.sort_by_key(|x| std::cmp::Reverse(x.updated_at));
    list.truncate(limit.max(1));
    list
}

// ---- external players --------------------------------------------------------

fn on_path(name: &str) -> Option<PathBuf> {
    let path = crate::core::process::enhanced_path().or_else(|| std::env::var("PATH").ok())?;
    let sep = if cfg!(windows) { ';' } else { ':' };
    for dir in path.split(sep).filter(|d| !d.is_empty()) {
        let base = PathBuf::from(dir);
        let candidates = if cfg!(windows) {
            vec![base.join(format!("{}.exe", name))]
        } else {
            vec![base.join(name)]
        };
        if let Some(p) = candidates.into_iter().find(|p| p.is_file()) {
            return Some(p);
        }
    }
    None
}

#[cfg(target_os = "macos")]
fn mac_app(name: &str) -> Option<PathBuf> {
    let mut dirs = vec![PathBuf::from("/Applications")];
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join("Applications"));
    }
    dirs.into_iter()
        .map(|d| d.join(format!("{}.app", name)))
        .find(|p| p.is_dir())
}

#[cfg(target_os = "windows")]
fn windows_vlc() -> Option<PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .map(|d| {
            PathBuf::from(d)
                .join("VideoLAN")
                .join("VLC")
                .join("vlc.exe")
        })
        .find(|p| p.is_file())
        .or_else(|| on_path("vlc"))
}

fn installed(player: &str) -> bool {
    match player {
        "default" => true,
        #[cfg(target_os = "macos")]
        "vlc" => mac_app("VLC").is_some() || on_path("vlc").is_some(),
        #[cfg(target_os = "macos")]
        "iina" => mac_app("IINA").is_some(),
        #[cfg(target_os = "macos")]
        "mpv" => on_path("mpv").is_some() || mac_app("mpv").is_some(),
        #[cfg(target_os = "windows")]
        "vlc" => windows_vlc().is_some(),
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        "vlc" => on_path("vlc").is_some(),
        #[cfg(not(target_os = "macos"))]
        "mpv" => on_path("mpv").is_some(),
        _ => false,
    }
}

/// Players this machine has, `default` first.
pub fn external_players() -> Vec<&'static str> {
    ["default", "vlc", "iina", "mpv"]
        .into_iter()
        .filter(|p| installed(p))
        .collect()
}

fn spawn(program: &std::ffi::OsStr, args: &[&str]) -> Result<(), String> {
    let mut cmd = crate::core::process::std_command(program);
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    cmd.spawn()
        .map(|_| ())
        .map_err(|e| format!("Could not start {}: {}", program.to_string_lossy(), e))
}

/// Opens one of our stream URLs in an external player. Only URLs of the
/// local stream server are accepted.
pub fn open_external(url: &str, player: &str) -> Result<(), String> {
    if !url.starts_with("http://127.0.0.1:") || url.contains(char::is_whitespace) {
        return Err("Only Telegram stream URLs can be opened".into());
    }
    let player = if player.trim().is_empty() {
        "default"
    } else {
        player
    };
    if player != "default" && !installed(player) {
        return Err(format!("{} is not installed", player));
    }

    #[cfg(target_os = "macos")]
    {
        let open = std::ffi::OsStr::new("open");
        match player {
            "default" => spawn(open, &[url]),
            "vlc" => match mac_app("VLC") {
                Some(_) => spawn(open, &["-a", "VLC", url]),
                None => spawn(std::ffi::OsStr::new("vlc"), &[url]),
            },
            "iina" => {
                let link = format!("iina://weblink?url={}", url_encode(url));
                spawn(open, &[&link])
            }
            "mpv" => match on_path("mpv") {
                Some(p) => spawn(p.as_os_str(), &["--force-window=immediate", url]),
                None => spawn(open, &["-a", "mpv", "--args", url]),
            },
            _ => Err(format!("Unknown player: {}", player)),
        }
    }

    #[cfg(target_os = "windows")]
    {
        match player {
            "default" => spawn(
                std::ffi::OsStr::new("rundll32"),
                &["url.dll,FileProtocolHandler", url],
            ),
            "vlc" => {
                let exe = windows_vlc().ok_or("VLC is not installed")?;
                spawn(exe.as_os_str(), &[url])
            }
            "mpv" => {
                let exe = on_path("mpv").ok_or("mpv is not installed")?;
                spawn(exe.as_os_str(), &["--force-window=immediate", url])
            }
            _ => Err(format!("Unknown player: {}", player)),
        }
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        match player {
            "default" => spawn(std::ffi::OsStr::new("xdg-open"), &[url]),
            "vlc" => spawn(std::ffi::OsStr::new("vlc"), &[url]),
            "mpv" => spawn(
                std::ffi::OsStr::new("mpv"),
                &["--force-window=immediate", url],
            ),
            _ => Err(format!("Unknown player: {}", player)),
        }
    }
}
