//! Downloads › Tools › Size check: how much space a video or a playlist will
//! take at a chosen quality, and whether it fits in the download folder.
//!
//! The idea comes from PyTube Downloader, which prints the size next to every
//! quality and every playlist video before anything is downloaded.
//! Reimplemented on top of yt-dlp (`omniget_core::core::ytdlp::measure_sizes`);
//! no code was copied.

use serde::Serialize;

#[cfg(not(target_os = "android"))]
use crate::core::ytdlp;
#[cfg(not(target_os = "android"))]
use crate::storage::config;

/// Playlists are read up to this many videos: each one is a full metadata
/// request, and YouTube throttles long bursts.
pub const SIZE_CHECK_LIMIT: u32 = 50;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SizeCheck {
    pub items: Vec<omniget_core::core::ytdlp::SizeItem>,
    /// Sum of the sizes that are known.
    pub total_bytes: u64,
    /// Items whose size the site does not report.
    pub unknown: usize,
    /// True when at least one size is yt-dlp's estimate.
    pub estimated: bool,
    /// True when the playlist may have more than `limit` videos.
    pub limit_reached: bool,
    pub limit: u32,
    pub output_dir: String,
    /// Free space in the download folder; `None` when it could not be read.
    pub free_bytes: Option<u64>,
}

pub fn summarize(
    items: Vec<omniget_core::core::ytdlp::SizeItem>,
    limit: u32,
    output_dir: String,
    free_bytes: Option<u64>,
) -> SizeCheck {
    let total_bytes = items.iter().filter_map(|i| i.bytes).sum();
    let unknown = items.iter().filter(|i| i.bytes.is_none()).count();
    let estimated = items.iter().any(|i| i.estimate);
    let limit_reached = items.len() as u32 >= limit;
    SizeCheck {
        items,
        total_bytes,
        unknown,
        estimated,
        limit_reached,
        limit,
        output_dir,
        free_bytes,
    }
}

/// `quality`: the maximum height (`1080`), `0` for audio only, or none for the best.
#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn check_download_size(
    app: tauri::AppHandle,
    url: String,
    quality: Option<u32>,
) -> Result<SizeCheck, String> {
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Paste a web address that starts with http:// or https://".to_string());
    }
    omniget_core::core::platform_optout::ensure_allowed(&url)?;
    let ytdlp_path = ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| format!("yt-dlp unavailable: {}", e))?;
    let ffmpeg = crate::core::ffmpeg::is_ffmpeg_available().await;
    let items = ytdlp::measure_sizes(&ytdlp_path, &url, quality, ffmpeg, SIZE_CHECK_LIMIT)
        .await
        .map_err(|e| e.to_string())?;
    let output_dir = config::load_settings(&app).download.default_output_dir;
    let free_bytes = crate::core::preflight::available_space(&output_dir);
    Ok(summarize(
        items,
        SIZE_CHECK_LIMIT,
        output_dir.to_string_lossy().into_owned(),
        free_bytes,
    ))
}
