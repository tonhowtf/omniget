//! Downloads › Tools › Record a live: records a live stream from now on, for
//! a chosen time or until stopped, in the quality asked for, and keeps what
//! was recorded when it is stopped.
//!
//! The idea comes from Streamlink (`-o`, `--stream-segmented-duration`,
//! `best`/`720p` quality names). The loop itself is in
//! `omniget_core::core::live_record`; this file resolves a page link to its
//! live playlist through yt-dlp and reports progress to the panel.

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use omniget_core::core::live_record::{self, LiveRecordOptions, LiveRecordResult};
use serde_json::Value;
use tauri::Emitter;
use tokio_util::sync::CancellationToken;

static CURRENT: Mutex<Option<CancellationToken>> = Mutex::new(None);

/// A link that already is an HLS playlist, recorded without asking yt-dlp.
pub fn is_playlist_url(url: &str) -> bool {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    path.ends_with(".m3u8") || path.ends_with(".m3u")
}

/// The live HLS format of a yt-dlp JSON: the tallest at or under
/// `max_height` (the smallest when all are taller), preferring a higher
/// bitrate at the same height. Returns its playlist URL and headers.
pub fn pick_live_format(
    json: &Value,
    max_height: Option<u32>,
) -> Option<(String, Vec<(String, String)>)> {
    let formats = json.get("formats")?.as_array()?;
    let hls: Vec<&Value> = formats
        .iter()
        .filter(|f| {
            f.get("protocol")
                .and_then(|p| p.as_str())
                .map(|p| p.starts_with("m3u8"))
                .unwrap_or(false)
                && f.get("vcodec").and_then(|v| v.as_str()) != Some("none")
        })
        .collect();
    let height = |f: &Value| f.get("height").and_then(|h| h.as_u64()).unwrap_or(0);
    let tbr = |f: &Value| (f.get("tbr").and_then(|t| t.as_f64()).unwrap_or(0.0) * 1000.0) as u64;
    let fits: Vec<&Value> = hls
        .iter()
        .copied()
        .filter(|f| max_height.map(|m| height(f) <= m as u64).unwrap_or(true))
        .collect();
    let chosen = if fits.is_empty() {
        hls.iter().copied().min_by_key(|f| (height(f), tbr(f)))?
    } else {
        fits.iter().copied().max_by_key(|f| (height(f), tbr(f)))?
    };
    let url = chosen.get("url")?.as_str()?.to_string();
    let headers = chosen
        .get("http_headers")
        .and_then(|h| h.as_object())
        .map(|h| {
            h.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                .collect()
        })
        .unwrap_or_default();
    Some((url, headers))
}

fn file_stem(title: &str) -> String {
    let when = chrono::Local::now().format("%Y-%m-%d %H-%M");
    let clean = omniget_core::core::filename::sanitize_path_component(title.trim());
    let clean: String = clean.chars().take(80).collect();
    if clean.trim().is_empty() {
        format!("Live {when}")
    } else {
        format!("{} {when}", clean.trim())
    }
}

#[cfg(not(target_os = "android"))]
async fn resolve_page(
    url: &str,
    max_height: Option<u32>,
) -> Result<(String, Vec<(String, String)>, String), String> {
    let ytdlp = omniget_core::core::ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| format!("yt-dlp unavailable: {e}"))?;
    let json = omniget_core::core::ytdlp::get_video_info(&ytdlp, url, &[])
        .await
        .map_err(|e| format!("Could not read this page: {e}"))?;
    let live = json
        .get("is_live")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || json.get("live_status").and_then(|v| v.as_str()) == Some("is_live");
    if !live {
        return Err("This page is not live right now. To save a finished video, paste the link in the download bar".to_string());
    }
    let (playlist, headers) = pick_live_format(&json, max_height)
        .ok_or_else(|| "This live has no HLS stream to record".to_string())?;
    let title = json
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("Live")
        .to_string();
    Ok((playlist, headers, title))
}

#[cfg(target_os = "android")]
async fn resolve_page(
    _url: &str,
    _max_height: Option<u32>,
) -> Result<(String, Vec<(String, String)>, String), String> {
    Err("Paste the address of an .m3u8 playlist".to_string())
}

#[tauri::command]
pub async fn live_record_start(
    app: tauri::AppHandle,
    url: String,
    output_dir: String,
    max_height: Option<u32>,
    max_minutes: Option<f64>,
) -> Result<LiveRecordResult, String> {
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Paste a web address that starts with http:// or https://".to_string());
    }
    omniget_core::core::platform_optout::ensure_allowed(&url)?;
    if output_dir.trim().is_empty() {
        return Err("Choose a download folder first".to_string());
    }
    let token = CancellationToken::new();
    {
        let mut cur = CURRENT.lock().map_err(|e| e.to_string())?;
        if cur.is_some() {
            return Err("A live is already being recorded".to_string());
        }
        *cur = Some(token.clone());
    }
    let result = async {
        let (playlist, headers, title) = if is_playlist_url(&url) {
            (url.clone(), Vec::new(), "Live".to_string())
        } else {
            tokio::select! {
                r = resolve_page(&url, max_height) => r?,
                _ = token.cancelled() => return Err("Cancelled".to_string()),
            }
        };
        let mut opts =
            LiveRecordOptions::new(playlist, PathBuf::from(&output_dir).join(file_stem(&title)));
        opts.max_height = max_height;
        opts.max_seconds = max_minutes.filter(|m| *m > 0.0).map(|m| m * 60.0);
        opts.headers = headers;
        opts.stall_timeout = Duration::from_secs(60);
        let app2 = app.clone();
        live_record::record(opts, token.clone(), move |p| {
            let _ = app2.emit("live-record-progress", p);
        })
        .await
        .map_err(|e| e.to_string())
    }
    .await;
    if let Ok(mut cur) = CURRENT.lock() {
        *cur = None;
    }
    result
}

/// Stops the recording; what was recorded so far is saved.
#[tauri::command]
pub fn live_record_stop() -> bool {
    match CURRENT.lock() {
        Ok(cur) => cur.as_ref().map(|t| t.cancel()).is_some(),
        Err(_) => false,
    }
}
