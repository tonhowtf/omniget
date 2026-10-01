use std::path::Path;
use std::process::Stdio;

use anyhow::anyhow;
use serde::Serialize;

use super::*;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ListedVideo {
    pub id: String,
    pub title: String,
    pub url: String,
    pub duration: Option<f64>,
    pub view_count: Option<u64>,
    pub is_short: bool,
    pub live_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ListedPage {
    pub title: String,
    pub uploader: Option<String>,
    pub entries: Vec<ListedVideo>,
    pub has_more: bool,
}

pub fn channel_videos_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let lower = trimmed.to_ascii_lowercase();
    if !(lower.contains("youtube.com/")) || lower.contains("list=") {
        return trimmed.to_string();
    }
    let path = lower
        .split("youtube.com/")
        .nth(1)
        .unwrap_or_default()
        .split(['?', '#'])
        .next()
        .unwrap_or_default()
        .to_string();
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let is_root = match parts.as_slice() {
        [handle] => handle.starts_with('@'),
        ["channel" | "c" | "user", _] => true,
        _ => false,
    };
    if is_root {
        format!("{}/videos", trimmed)
    } else {
        trimmed.to_string()
    }
}

fn text(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub fn parse_listing(json: &serde_json::Value, want: usize) -> ListedPage {
    let mut entries: Vec<ListedVideo> = json
        .get("entries")
        .and_then(|e| e.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|e| {
                    let id = text(e, "id")?;
                    let url = text(e, "url")
                        .or_else(|| text(e, "webpage_url"))
                        .unwrap_or_else(|| id.clone());
                    if e.get("_type").and_then(|t| t.as_str()) == Some("playlist")
                        || url.contains("/playlist?")
                    {
                        return None;
                    }
                    Some(ListedVideo {
                        is_short: url.contains("/shorts/"),
                        title: text(e, "title").unwrap_or_else(|| id.clone()),
                        duration: e.get("duration").and_then(|d| d.as_f64()),
                        view_count: e.get("view_count").and_then(|d| d.as_u64()),
                        live_status: text(e, "live_status"),
                        id,
                        url,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let has_more = entries.len() > want;
    entries.truncate(want);
    ListedPage {
        title: text(json, "title").unwrap_or_default(),
        uploader: text(json, "uploader").or_else(|| text(json, "channel")),
        entries,
        has_more,
    }
}

pub async fn list_videos_page(
    ytdlp: &Path,
    url: &str,
    start: u32,
    count: u32,
) -> anyhow::Result<ListedPage> {
    let url = channel_videos_url(url);
    if is_youtube_url(&url) {
        yt_rate_limiter().acquire().await;
    }
    let start = start.max(1);
    let count = count.clamp(1, 500);
    let mut args = vec![
        "--flat-playlist".to_string(),
        "-J".to_string(),
        "--no-warnings".to_string(),
        "--encoding".to_string(),
        "utf-8".to_string(),
        "--socket-timeout".to_string(),
        "30".to_string(),
        "--extractor-retries".to_string(),
        "3".to_string(),
        "--playlist-items".to_string(),
        format!("{}:{}", start, start + count),
        "--user-agent".to_string(),
        CHROME_UA.to_string(),
    ];
    args.extend(js_runtime_args());
    append_metadata_cookie_args(&mut args, &url, &[], "channel list");
    args.extend(proxy_args());
    args.push(url.clone());

    note_operator_command(ytdlp, &args, "channel list");
    let _slot = acquire_ytdlp_slot("channel list").await;
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(180),
        ytdlp_command(ytdlp)
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow!("Timeout listing the videos (180s)"))?
    .map_err(|e| anyhow!("Failed to run yt-dlp: {}", e))?;

    if !output.status.success() && output.stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last = stderr
            .lines()
            .rev()
            .find(|l| l.contains("ERROR"))
            .unwrap_or("yt-dlp could not list this link");
        return Err(anyhow!("{}", last.trim()));
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| anyhow!("yt-dlp returned invalid JSON: {}", e))?;
    Ok(parse_listing(&json, count as usize))
}

#[cfg(test)]
mod channel_list_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_bare_channel_opens_its_videos_tab() {
        assert_eq!(
            channel_videos_url("https://www.youtube.com/@BlenderStudio"),
            "https://www.youtube.com/@BlenderStudio/videos"
        );
        assert_eq!(
            channel_videos_url("https://www.youtube.com/channel/UCz75RVbH8q2jiCKTmKx6ZJA/"),
            "https://www.youtube.com/channel/UCz75RVbH8q2jiCKTmKx6ZJA/videos"
        );
        assert_eq!(
            channel_videos_url("https://www.youtube.com/@BlenderStudio/shorts"),
            "https://www.youtube.com/@BlenderStudio/shorts"
        );
        assert_eq!(
            channel_videos_url("https://www.youtube.com/playlist?list=PLx"),
            "https://www.youtube.com/playlist?list=PLx"
        );
        assert_eq!(
            channel_videos_url("https://archive.org/details/The_Public_Domain_EP"),
            "https://archive.org/details/The_Public_Domain_EP"
        );
    }

    #[test]
    fn keeps_duration_views_and_the_next_page_hint() {
        let j = json!({
            "title": "Blender Studio - Videos", "channel": "Blender Studio",
            "entries": [
                {"id": "a", "title": "One", "url": "https://www.youtube.com/watch?v=a", "duration": 282.0, "view_count": 29000},
                {"id": "b", "title": "Two", "url": "https://www.youtube.com/shorts/b", "duration": 40.0},
                {"id": "c", "title": "Three", "url": "https://www.youtube.com/watch?v=c", "live_status": "is_upcoming"}
            ]
        });
        let p = parse_listing(&j, 2);
        assert_eq!(p.title, "Blender Studio - Videos");
        assert_eq!(p.uploader.as_deref(), Some("Blender Studio"));
        assert!(p.has_more);
        assert_eq!(p.entries.len(), 2);
        assert_eq!(p.entries[0].duration, Some(282.0));
        assert_eq!(p.entries[0].view_count, Some(29000));
        assert!(!p.entries[0].is_short);
        assert!(p.entries[1].is_short);

        let all = parse_listing(&j, 3);
        assert!(!all.has_more);
        assert_eq!(all.entries[2].live_status.as_deref(), Some("is_upcoming"));
    }

    #[test]
    fn skips_nested_tabs() {
        let j = json!({"entries": [
            {"_type": "playlist", "id": "UCx-videos", "url": "https://www.youtube.com/@x/videos"},
            {"id": "PL1", "url": "https://www.youtube.com/playlist?list=PL1"},
            {"id": "v", "url": "https://www.youtube.com/watch?v=v"}
        ]});
        let p = parse_listing(&j, 10);
        assert_eq!(p.entries.len(), 1);
        assert_eq!(p.entries[0].id, "v");
    }
}
