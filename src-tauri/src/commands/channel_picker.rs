//! Downloads › Tools › Channel picker: list a channel or playlist page by
//! page, filter by title and length, and download only the videos you tick.
//!
//! The idea comes from YT Channel Downloader, which lists a channel in
//! windows ("Fetch Next N") with a duration column and a checkbox per video
//! and greys out what was already downloaded. Reimplemented on top of yt-dlp
//! (`omniget_core::core::ytdlp::list_videos_page`); no code was copied.
//! "Already downloaded" here comes from OmniGet's own history, by video id.

use std::collections::HashSet;

use serde::Serialize;

#[cfg(not(target_os = "android"))]
use crate::core::ytdlp;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PickerVideo {
    #[serde(flatten)]
    pub video: omniget_core::core::ytdlp::ListedVideo,
    /// A finished download in the history has the same video id or URL.
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

/// The id a video URL carries: `v=` on YouTube, the last path part elsewhere
/// (`/shorts/<id>`, `youtu.be/<id>`, `/video/<id>`).
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

/// Ids and URLs of every successful download in the history.
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

#[cfg(test)]
mod tests {
    use super::*;
    use omniget_core::core::ytdlp::{ListedPage, ListedVideo};

    #[test]
    fn keys_from_common_video_urls() {
        assert_eq!(
            video_key("https://www.youtube.com/watch?v=aqz-KE-bpKQ&t=3").as_deref(),
            Some("aqz-KE-bpKQ")
        );
        assert_eq!(
            video_key("https://youtu.be/aqz-KE-bpKQ").as_deref(),
            Some("aqz-KE-bpKQ")
        );
        assert_eq!(
            video_key("https://www.youtube.com/shorts/abc123/").as_deref(),
            Some("abc123")
        );
        assert_eq!(
            video_key("https://www.bilibili.com/video/BV1eycyeyEec").as_deref(),
            Some("BV1eycyeyEec")
        );
        // A file name is not an id.
        assert_eq!(video_key("https://download.blender.org/a/file.mov"), None);
    }

    #[test]
    fn marks_what_the_history_already_has() {
        let v = |id: &str| ListedVideo {
            id: id.to_string(),
            title: id.to_string(),
            url: format!("https://www.youtube.com/watch?v={id}"),
            duration: None,
            view_count: None,
            is_short: false,
            live_status: None,
        };
        let page = ListedPage {
            title: "Channel".into(),
            uploader: None,
            entries: vec![v("aaa"), v("bbb")],
            has_more: true,
        };
        let keys = downloaded_keys(["https://youtu.be/bbb"].into_iter());
        let p = mark_downloaded(page, 101, &keys);
        assert_eq!(p.start, 101);
        assert!(!p.entries[0].downloaded);
        assert!(p.entries[1].downloaded);
        assert!(p.has_more);
    }
}
