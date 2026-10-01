use serde::Serialize;

#[cfg(not(target_os = "android"))]
use crate::core::ytdlp;
#[cfg(not(target_os = "android"))]
use crate::storage::config;

pub const SIZE_CHECK_LIMIT: u32 = 50;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SizeCheck {
    pub items: Vec<omniget_core::core::ytdlp::SizeItem>,
    pub total_bytes: u64,
    pub unknown: usize,
    pub estimated: bool,
    pub limit_reached: bool,
    pub limit: u32,
    pub output_dir: String,
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
