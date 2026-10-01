use std::collections::HashSet;

use serde::Serialize;

#[cfg(not(target_os = "android"))]
use crate::core::ytdlp;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PickerVideo {
    #[serde(flatten)]
    pub video: omniget_core::core::ytdlp::ListedVideo,
    pub downloaded: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PickerPage {
    pub title: String,
    pub uploader: Option<String>,
    pub start: u32,
    pub entries: Vec<PickerVideo>,
    pub has_more: bool,
}

pub fn video_key(url: &str) -> Option<String> {
    let no_frag = url.split('#').next().unwrap_or(url);
    if let Some(q) = no_frag.split_once('?').map(|(_, q)| q) {
        if let Some(v) = q
            .split('&')
            .find_map(|kv| kv.strip_prefix("v="))
            .filter(|v| !v.is_empty())
        {
            return Some(v.to_string());
        }
    }
    let path = no_frag.split('?').next().unwrap_or(no_frag);
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty() && !s.contains('.'))
        .map(str::to_string)
}

pub fn downloaded_keys<'a>(done: impl Iterator<Item = &'a str>) -> HashSet<String> {
    let mut keys = HashSet::new();
    for url in done {
        keys.insert(url.trim_end_matches('/').to_string());
        if let Some(k) = video_key(url) {
            keys.insert(k);
        }
    }
    keys
}

pub fn mark_downloaded(
    page: omniget_core::core::ytdlp::ListedPage,
    start: u32,
    keys: &HashSet<String>,
) -> PickerPage {
    let entries = page
        .entries
        .into_iter()
        .map(|video| {
            let downloaded =
                keys.contains(&video.id) || keys.contains(video.url.trim_end_matches('/'));
            PickerVideo { video, downloaded }
        })
        .collect();
    PickerPage {
        title: page.title,
        uploader: page.uploader,
        start,
        entries,
        has_more: page.has_more,
    }
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn list_channel_videos(
    url: String,
    start: Option<u32>,
    count: Option<u32>,
) -> Result<PickerPage, String> {
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Paste a web address that starts with http:// or https://".to_string());
    }
    omniget_core::core::platform_optout::ensure_allowed(&url)?;
    let start = start.unwrap_or(1).max(1);
    let ytdlp_path = ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| format!("yt-dlp unavailable: {}", e))?;
    let page = ytdlp::list_videos_page(&ytdlp_path, &url, start, count.unwrap_or(100))
        .await
        .map_err(|e| e.to_string())?;
    if page.entries.is_empty() && start == 1 {
        return Err("No videos were found at this address".to_string());
    }
    let history = crate::core::queue_history::list();
    let keys = downloaded_keys(history.iter().filter(|h| h.success).map(|h| h.url.as_str()));
    Ok(mark_downloaded(page, start, &keys))
}
