use std::sync::Arc;

use serde::Serialize;

use crate::core::queue::{self, emit_queue_state_from_state};
use crate::core::url_parser;
use crate::platforms::Platform;
use crate::storage::config;
use crate::AppState;

#[cfg(not(target_os = "android"))]
use crate::core::ytdlp;
#[cfg(not(target_os = "android"))]
use crate::models::media::{FormatInfo, MediaType};

#[derive(Clone, Serialize)]
pub struct PlatformInfo {
    pub platform: String,
    pub supported: bool,
    pub content_id: Option<String>,
    pub content_type: Option<String>,
}

#[tauri::command]
pub fn check_cookie_error() -> bool {
    let has_error = crate::core::ytdlp::has_cookie_error();
    if has_error {
        crate::core::ytdlp::clear_cookie_error();
    }
    has_error
}

#[derive(Clone, Serialize)]
pub struct PathLimitInfo {
    pub limit: usize,
    pub current: usize,
    pub reserve: usize,
    pub ok: bool,
}

#[tauri::command]
pub fn validate_output_path(output_dir: String) -> PathLimitInfo {
    match crate::core::path_limits::validate_output_dir(&output_dir) {
        Ok(()) => PathLimitInfo {
            limit: crate::core::path_limits::MAX_PATH_LEN,
            current: output_dir.chars().count() + crate::core::path_limits::SEPARATOR_RESERVE,
            reserve: crate::core::path_limits::MIN_FILENAME_RESERVE,
            ok: true,
        },
        Err(err) => PathLimitInfo {
            limit: err.limit,
            current: err.current,
            reserve: err.reserve,
            ok: false,
        },
    }
}

#[tauri::command]
pub async fn detect_platform(url: String) -> Result<PlatformInfo, String> {
    let _timer_start = std::time::Instant::now();
    match Platform::from_url(&url) {
        Some(platform) => {
            let parsed = url_parser::parse_url(&url);
            let result = Ok(PlatformInfo {
                platform: platform.to_string(),
                supported: true,
                content_id: parsed.as_ref().and_then(|p| p.content_id.clone()),
                content_type: parsed.map(|p| format!("{:?}", p.content_type).to_lowercase()),
            });
            tracing::debug!("[perf] detect_platform took {:?}", _timer_start.elapsed());
            result
        }
        None => {
            let is_valid_url = url::Url::parse(&url)
                .map(|u| u.scheme() == "http" || u.scheme() == "https")
                .unwrap_or(false);
            let result = Ok(PlatformInfo {
                platform: if is_valid_url {
                    "generic".to_string()
                } else {
                    "unknown".to_string()
                },
                supported: is_valid_url,
                content_id: None,
                content_type: None,
            });
            tracing::debug!("[perf] detect_platform took {:?}", _timer_start.elapsed());
            result
        }
    }
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn get_media_formats(url: String) -> Result<Vec<FormatInfo>, String> {
    let _timer_start = std::time::Instant::now();
    let ytdlp_path = ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| format!("yt-dlp unavailable: {}", e))?;

    let json = ytdlp::get_video_info(&ytdlp_path, &url, &[])
        .await
        .map_err(|e| format!("Failed to get formats: {}", e))?;

    tracing::debug!("[perf] get_media_formats took {:?}", _timer_start.elapsed());
    Ok(ytdlp::parse_formats(&json))
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn prefetch_media_info(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    url: String,
) -> Result<(), String> {
    let platform = Platform::from_url(&url);
    let platform_name = platform
        .map(|p| p.to_string())
        .unwrap_or_else(|| "generic".to_string());

    let downloader = match state.registry.find_platform(&url) {
        Some(d) => d,
        None => return Err("No downloader available".to_string()),
    };

    let ytdlp_path = ytdlp::find_ytdlp_cached().await;

    tokio::spawn(async move {
        queue::prefetch_info_with_emit(
            &url,
            &*downloader,
            &platform_name,
            ytdlp_path.as_deref(),
            Some(app),
        )
        .await;
    });

    Ok(())
}

#[derive(Clone, Serialize)]
pub struct DownloadStarted {
    pub id: u64,
    pub title: String,
}

fn is_valid_time_range(r: &str) -> bool {
    let Some((a, b)) = r.split_once('-') else {
        return false;
    };
    let part_ok = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_digit() || c == ':' || c == '.')
    };
    part_ok(a) && (b == "inf" || part_ok(b))
}

#[derive(Clone, Serialize)]
pub struct PlaylistEntryInfo {
    pub index: u32,
    pub title: String,
    pub url: String,
}

#[derive(Clone, Serialize)]
pub struct MetadataFetchResult {
    pub title: String,
    pub saved: Vec<String>,
}

#[derive(Clone, Serialize)]
pub struct TorrentFileEntry {
    pub index: usize,
    pub path: String,
    pub size_bytes: u64,
}

#[cfg(not(target_os = "android"))]
fn is_torrent_source(url: &str) -> bool {
    url.starts_with("magnet:")
        || url.ends_with(".torrent")
        || (std::path::Path::new(url).exists()
            && std::path::Path::new(url)
                .extension()
                .map(|e| e == "torrent")
                .unwrap_or(false))
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn torrent_contents(url: String) -> Result<Vec<TorrentFileEntry>, String> {
    if !is_torrent_source(&url) {
        return Err("Not a torrent source".to_string());
    }

    let add_torrent = if url.starts_with("magnet:")
        || url.starts_with("http://")
        || url.starts_with("https://")
    {
        librqbit::AddTorrent::from_url(&url)
    } else {
        let path = std::path::Path::new(&url);
        if path.exists() && path.extension().map(|e| e == "torrent").unwrap_or(false) {
            let bytes = tokio::fs::read(path)
                .await
                .map_err(|e| format!("Failed to read .torrent file: {}", e))?;
            librqbit::AddTorrent::from_bytes(bytes)
        } else {
            librqbit::AddTorrent::from_url(&url)
        }
    };

    let tmp_dir = std::env::temp_dir().join("omniget-torrent-list");
    let session = librqbit::Session::new(tmp_dir)
        .await
        .map_err(|e| format!("Failed to init torrent session: {}", e))?;

    let opts = librqbit::AddTorrentOptions {
        list_only: true,
        ..Default::default()
    };

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        session.add_torrent(add_torrent, Some(opts)),
    )
    .await
    .map_err(|_| "Timed out resolving torrent metadata".to_string())?
    .map_err(|e| format!("Failed to resolve torrent: {}", e))?;

    let list = match resp {
        librqbit::AddTorrentResponse::ListOnly(l) => l,
        _ => return Err("Torrent did not return a file listing".to_string()),
    };

    let entries: Vec<TorrentFileEntry> = match list.info.iter_file_details() {
        Ok(iter) => iter
            .enumerate()
            .map(|(index, d)| TorrentFileEntry {
                index,
                path: d
                    .filename
                    .to_string()
                    .unwrap_or_else(|_| format!("file {}", index + 1)),
                size_bytes: d.len,
            })
            .collect(),
        Err(e) => return Err(format!("Failed to read torrent files: {}", e)),
    };

    Ok(entries)
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn metadata_fetch(
    url: String,
    output_dir: String,
) -> Result<MetadataFetchResult, String> {
    let ytdlp_path = ytdlp::find_ytdlp_cached()
        .await
        .ok_or_else(|| "yt-dlp unavailable".to_string())?;
    let json = ytdlp::get_video_info(&ytdlp_path, &url, &[])
        .await
        .map_err(|e| e.to_string())?;

    let raw_title = json
        .get("title")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("metadata");
    let title = sanitize_filename::sanitize(raw_title);
    let dir = std::path::Path::new(&output_dir);
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| e.to_string())?;

    let mut saved = Vec::new();

    let info_name = format!("{title}.info.json");
    let info_body = serde_json::to_vec_pretty(&json).map_err(|e| e.to_string())?;
    tokio::fs::write(dir.join(&info_name), info_body)
        .await
        .map_err(|e| e.to_string())?;
    saved.push(info_name);

    if let Some(desc) = json.get("description").and_then(|v| v.as_str()) {
        if !desc.trim().is_empty() {
            let dname = format!("{title}.description.txt");
            if tokio::fs::write(dir.join(&dname), desc).await.is_ok() {
                saved.push(dname);
            }
        }
    }

    if let Some(thumb) = json.get("thumbnail").and_then(|v| v.as_str()) {
        if let Ok(client) =
            crate::core::http_client::apply_global_proxy(reqwest::Client::builder()).build()
        {
            if let Ok(resp) = client.get(thumb).send().await {
                if resp.status().is_success() {
                    if let Ok(bytes) = resp.bytes().await {
                        let tname = format!("{title}.jpg");
                        if tokio::fs::write(dir.join(&tname), &bytes).await.is_ok() {
                            saved.push(tname);
                        }
                    }
                }
            }
        }
    }

    Ok(MetadataFetchResult { title, saved })
}

#[derive(Clone, Serialize)]
pub struct ThumbnailInfo {
    pub url: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Serialize)]
pub struct ThumbnailListResult {
    pub title: String,
    pub thumbnails: Vec<ThumbnailInfo>,
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn thumbnails_list(url: String) -> Result<ThumbnailListResult, String> {
    let ytdlp_path = ytdlp::find_ytdlp_cached()
        .await
        .ok_or_else(|| "yt-dlp unavailable".to_string())?;
    let json = ytdlp::get_video_info(&ytdlp_path, &url, &[])
        .await
        .map_err(|e| e.to_string())?;

    let raw_title = json
        .get("title")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("thumbnail");
    let title = sanitize_filename::sanitize(raw_title);

    let mut seen: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
    let mut thumbnails: Vec<ThumbnailInfo> = Vec::new();
    if let Some(arr) = json.get("thumbnails").and_then(|v| v.as_array()) {
        for entry in arr {
            let u = entry.get("url").and_then(|v| v.as_str()).unwrap_or("");
            let w = entry.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let h = entry.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            if u.is_empty() || w == 0 || h == 0 {
                continue;
            }
            if seen.insert((w, h)) {
                thumbnails.push(ThumbnailInfo {
                    url: u.to_string(),
                    width: w,
                    height: h,
                });
            }
        }
    }
    thumbnails.sort_by(|a, b| (b.width * b.height).cmp(&(a.width * a.height)));

    if thumbnails.is_empty() {
        if let Some(single) = json.get("thumbnail").and_then(|v| v.as_str()) {
            if !single.is_empty() {
                thumbnails.push(ThumbnailInfo {
                    url: single.to_string(),
                    width: 0,
                    height: 0,
                });
            }
        }
    }

    Ok(ThumbnailListResult { title, thumbnails })
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn thumbnail_save(
    thumb_url: String,
    output_dir: String,
    file_name: String,
) -> Result<String, String> {
    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(&thumb_url)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    let bytes = resp.bytes().await.map_err(|e| e.to_string())?;
    let safe = sanitize_filename::sanitize(&file_name);
    let safe = if safe.to_lowercase().ends_with(".jpg") || safe.to_lowercase().ends_with(".webp") {
        safe
    } else {
        format!("{safe}.jpg")
    };
    let dir = std::path::Path::new(&output_dir);
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::write(dir.join(&safe), &bytes)
        .await
        .map_err(|e| e.to_string())?;
    Ok(safe)
}

#[derive(Clone, Serialize)]
pub struct SubtitleFormat {
    pub ext: String,
    pub url: String,
}

#[derive(Clone, Serialize)]
pub struct SubtitleTrack {
    pub lang: String,
    pub name: String,
    pub auto: bool,
    pub formats: Vec<SubtitleFormat>,
}

#[derive(Clone, Serialize)]
pub struct SubtitleListResult {
    pub title: String,
    pub tracks: Vec<SubtitleTrack>,
}

#[cfg(not(target_os = "android"))]
fn collect_sub_tracks(map: Option<&serde_json::Value>, auto: bool, out: &mut Vec<SubtitleTrack>) {
    let Some(obj) = map.and_then(|v| v.as_object()) else {
        return;
    };
    for (lang, arr) in obj {
        let Some(items) = arr.as_array() else {
            continue;
        };
        let mut formats = Vec::new();
        for f in items {
            let ext = f
                .get("ext")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let url = f
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !url.is_empty() {
                formats.push(SubtitleFormat { ext, url });
            }
        }
        if formats.is_empty() {
            continue;
        }
        let name = items
            .first()
            .and_then(|f| f.get("name"))
            .and_then(|v| v.as_str())
            .unwrap_or(lang)
            .to_string();
        out.push(SubtitleTrack {
            lang: lang.clone(),
            name,
            auto,
            formats,
        });
    }
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn subtitles_list(url: String) -> Result<SubtitleListResult, String> {
    let ytdlp_path = ytdlp::find_ytdlp_cached()
        .await
        .ok_or_else(|| "yt-dlp unavailable".to_string())?;
    let json = ytdlp::get_video_info(&ytdlp_path, &url, &[])
        .await
        .map_err(|e| e.to_string())?;
    let title = sanitize_filename::sanitize(
        json.get("title")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("subtitles"),
    );
    let mut tracks = Vec::new();
    collect_sub_tracks(json.get("subtitles"), false, &mut tracks);
    collect_sub_tracks(json.get("automatic_captions"), true, &mut tracks);
    Ok(SubtitleListResult { title, tracks })
}

#[cfg(not(target_os = "android"))]
async fn fetch_text(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }
    resp.text().await.map_err(|e| e.to_string())
}

#[cfg(not(target_os = "android"))]
fn ensure_ext(name: &str, ext: &str) -> String {
    let safe = sanitize_filename::sanitize(name);
    if safe
        .to_lowercase()
        .ends_with(&format!(".{}", ext.to_lowercase()))
    {
        safe
    } else {
        format!("{safe}.{ext}")
    }
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn subtitles_save(
    sub_url: String,
    ext: String,
    output_dir: String,
    file_name: String,
) -> Result<String, String> {
    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .build()
        .map_err(|e| e.to_string())?;
    let body = fetch_text(&client, &sub_url).await?;
    let dir = std::path::Path::new(&output_dir);
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| e.to_string())?;
    let safe = ensure_ext(&file_name, if ext.is_empty() { "srt" } else { &ext });
    tokio::fs::write(dir.join(&safe), body)
        .await
        .map_err(|e| e.to_string())?;
    Ok(safe)
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn subtitles_merge(
    primary_url: String,
    secondary_url: String,
    format: String,
    output_dir: String,
    file_name: String,
) -> Result<String, String> {
    let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
        .build()
        .map_err(|e| e.to_string())?;
    let primary = fetch_text(&client, &primary_url).await?;
    let secondary = fetch_text(&client, &secondary_url).await?;
    let is_vtt = format.eq_ignore_ascii_case("vtt");
    let merged = if is_vtt {
        omniget_core::core::subtitle_merge::merge_bilingual_vtt(&primary, &secondary)
    } else {
        omniget_core::core::subtitle_merge::merge_bilingual_srt(&primary, &secondary)
    };
    let dir = std::path::Path::new(&output_dir);
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| e.to_string())?;
    let safe = ensure_ext(&file_name, if is_vtt { "vtt" } else { "srt" });
    tokio::fs::write(dir.join(&safe), merged)
        .await
        .map_err(|e| e.to_string())?;
    Ok(safe)
}

#[derive(Clone, Serialize)]
pub struct VideoComment {
    pub id: String,
    pub parent: String,
    pub author: String,
    pub text: String,
    pub timestamp: i64,
    pub like_count: i64,
    pub is_uploader: bool,
}

#[derive(Clone, Serialize)]
pub struct CommentsResult {
    pub title: String,
    pub count: i64,
    pub comments: Vec<VideoComment>,
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn comments_fetch(
    url: String,
    max_comments: u32,
    sort: String,
) -> Result<CommentsResult, String> {
    let ytdlp_path = ytdlp::find_ytdlp_cached()
        .await
        .ok_or_else(|| "yt-dlp unavailable".to_string())?;
    let sort_v = if sort.eq_ignore_ascii_case("new") {
        "new"
    } else {
        "top"
    };
    let ea = format!(
        "youtube:max_comments={};comment_sort={}",
        max_comments.max(1),
        sort_v
    );
    let flags = vec![
        "--write-comments".to_string(),
        "--extractor-args".to_string(),
        ea,
    ];
    let json = ytdlp::get_video_info(&ytdlp_path, &url, &flags)
        .await
        .map_err(|e| e.to_string())?;
    let title = json
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("comments")
        .to_string();
    let count = json
        .get("comment_count")
        .and_then(|v| v.as_i64())
        .unwrap_or(0);
    let mut comments = Vec::new();
    if let Some(arr) = json.get("comments").and_then(|v| v.as_array()) {
        for c in arr {
            comments.push(VideoComment {
                id: c
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                parent: c
                    .get("parent")
                    .and_then(|v| v.as_str())
                    .unwrap_or("root")
                    .to_string(),
                author: c
                    .get("author")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                text: c
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                timestamp: c.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0),
                like_count: c.get("like_count").and_then(|v| v.as_i64()).unwrap_or(0),
                is_uploader: c
                    .get("author_is_uploader")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            });
        }
    }
    Ok(CommentsResult {
        title,
        count,
        comments,
    })
}

#[derive(Clone, Serialize)]
pub struct Chapter {
    pub start: f64,
    pub end: f64,
    pub title: String,
}

#[derive(Clone, Serialize)]
pub struct ChaptersResult {
    pub title: String,
    pub chapters: Vec<Chapter>,
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn chapters_fetch(url: String) -> Result<ChaptersResult, String> {
    let ytdlp_path = ytdlp::find_ytdlp_cached()
        .await
        .ok_or_else(|| "yt-dlp unavailable".to_string())?;
    let json = ytdlp::get_video_info(&ytdlp_path, &url, &[])
        .await
        .map_err(|e| e.to_string())?;
    let title = json
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("chapters")
        .to_string();
    let mut chapters = Vec::new();
    if let Some(arr) = json.get("chapters").and_then(|v| v.as_array()) {
        for ch in arr {
            chapters.push(Chapter {
                start: ch.get("start_time").and_then(|v| v.as_f64()).unwrap_or(0.0),
                end: ch.get("end_time").and_then(|v| v.as_f64()).unwrap_or(0.0),
                title: ch
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
            });
        }
    }
    Ok(ChaptersResult { title, chapters })
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn tools_save_text(
    output_dir: String,
    file_name: String,
    content: String,
) -> Result<String, String> {
    let safe = sanitize_filename::sanitize(&file_name);
    if safe.is_empty() {
        return Err("invalid file name".to_string());
    }
    let dir = std::path::Path::new(&output_dir);
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::write(dir.join(&safe), content)
        .await
        .map_err(|e| e.to_string())?;
    Ok(safe)
}

#[derive(Clone, Serialize)]
pub struct LiveChatResult {
    pub count: usize,
    pub messages: Vec<omniget_core::core::livechat::LiveChatMessage>,
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn livechat_fetch(url: String) -> Result<LiveChatResult, String> {
    let ytdlp_path = ytdlp::find_ytdlp_cached()
        .await
        .ok_or_else(|| "yt-dlp unavailable".to_string())?;

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("omniget-livechat-{stamp}"));
    tokio::fs::create_dir_all(&tmp)
        .await
        .map_err(|e| e.to_string())?;

    let out_template = tmp.join("chat.%(ext)s");
    let result = (|| async {
        let output = omniget_core::core::process::command(&ytdlp_path)
            .arg("--skip-download")
            .arg("--write-subs")
            .arg("--sub-langs")
            .arg("live_chat")
            .arg("--no-warnings")
            .arg("-o")
            .arg(&out_template)
            .arg(&url)
            .output()
            .await
            .map_err(|e| e.to_string())?;
        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            let line = err
                .lines()
                .find(|l| l.contains("ERROR"))
                .unwrap_or("yt-dlp failed");
            return Err(line.chars().take(240).collect::<String>());
        }

        let mut entries = tokio::fs::read_dir(&tmp).await.map_err(|e| e.to_string())?;
        let mut chat_file: Option<std::path::PathBuf> = None;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let p = entry.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.ends_with(".live_chat.json") {
                chat_file = Some(p);
                break;
            }
            if chat_file.is_none() && name.ends_with(".json") {
                chat_file = Some(p);
            }
        }
        let Some(chat_file) = chat_file else {
            return Err("no live chat available for this video".to_string());
        };
        let body = tokio::fs::read_to_string(&chat_file)
            .await
            .map_err(|e| e.to_string())?;
        let messages = omniget_core::core::livechat::parse_live_chat(&body);
        Ok(LiveChatResult {
            count: messages.len(),
            messages,
        })
    })()
    .await;

    let _ = tokio::fs::remove_dir_all(&tmp).await;
    result
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn playlist_entries(url: String) -> Result<Vec<PlaylistEntryInfo>, String> {
    let ytdlp_path = ytdlp::find_ytdlp_cached()
        .await
        .ok_or_else(|| "yt-dlp unavailable".to_string())?;
    let (_title, entries) = ytdlp::get_playlist_info(&ytdlp_path, &url, &[])
        .await
        .map_err(|e| e.to_string())?;
    Ok(entries
        .into_iter()
        .enumerate()
        .map(|(i, e)| PlaylistEntryInfo {
            index: (i + 1) as u32,
            title: e.title,
            url: e.url,
        })
        .collect())
}

#[cfg(not(target_os = "android"))]
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn download_from_url(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    url: String,
    output_dir: String,
    download_mode: Option<String>,
    quality: Option<String>,
    format_id: Option<String>,
    referer: Option<String>,
    cookie_slug: Option<String>,
    time_range: Option<String>,
    playlist_items: Option<Vec<u32>>,
    torrent_files: Option<Vec<usize>>,
    scheduled_at: Option<u64>,
    stop_at: Option<u64>,
) -> Result<DownloadStarted, String> {
    let _timer_start = std::time::Instant::now();
    let platform = Platform::from_url(&url);

    let custom_ytdlp_args = match time_range.as_deref().map(str::trim) {
        Some(r) if !r.is_empty() && is_valid_time_range(r) => {
            Some(vec!["--download-sections".to_string(), format!("*{}", r)])
        }
        _ => None,
    };

    if let Err(err) = crate::core::path_limits::validate_output_dir(&output_dir) {
        return Err(format!(
            "PathTooLong|{}|{}|{}",
            err.limit, err.current, err.reserve
        ));
    }

    let mut download_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let download_queue = state.download_queue.clone();

    {
        let settings = config::load_settings(&app);
        let mut q = download_queue.lock().await;
        q.max_concurrent = settings.advanced.max_concurrent_downloads.max(1);
        q.stagger_delay_ms = settings.advanced.stagger_delay_ms;
        q.default_max_retries = settings.advanced.max_retries;
        if q.has_url(&url) {
            tracing::debug!("[perf] download_from_url took {:?}", _timer_start.elapsed());
            return Err("Download already in progress for this URL".to_string());
        }
        download_id = q.next_available_id(download_id);
    }

    let downloader = match state.registry.find_platform(&url) {
        Some(d) => d,
        None => {
            tracing::debug!("[perf] download_from_url took {:?}", _timer_start.elapsed());
            return Err("No downloader available for this URL".to_string());
        }
    };

    let platform_name = platform
        .map(|p| p.to_string())
        .unwrap_or_else(|| "generic".to_string());
    let title = url.clone();
    let ytdlp_path = ytdlp::find_ytdlp_cached().await;

    let cached_info = {
        let info = queue::try_get_cached_info(&url).await;
        match (info, &playlist_items) {
            (Some(mut info), Some(sel))
                if !sel.is_empty() && info.media_type == MediaType::Playlist =>
            {
                let set: std::collections::HashSet<u32> = sel.iter().copied().collect();
                let filtered: Vec<_> = info
                    .available_qualities
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| set.contains(&((*i as u32) + 1)))
                    .map(|(_, q)| q.clone())
                    .collect();
                if !filtered.is_empty() {
                    info.available_qualities = filtered;
                }
                Some(info)
            }
            (info, _) => info,
        }
    };

    let state_to_emit = {
        let mut q = download_queue.lock().await;
        q.enqueue(
            download_id,
            url,
            platform_name,
            title.clone(),
            output_dir,
            download_mode,
            quality,
            format_id,
            referer,
            None,
            None,
            None,
            cached_info,
            None,
            None,
            downloader,
            ytdlp_path,
            false,
            cookie_slug,
            custom_ytdlp_args,
            torrent_files,
            scheduled_at,
            stop_at,
        );

        let next_ids = q.next_queued_ids();
        for nid in &next_ids {
            q.mark_active(*nid);
        }
        q.get_state()
    };
    emit_queue_state_from_state(&app, state_to_emit);

    let q_clone = download_queue.clone();
    let app_clone = app.clone();
    tokio::spawn(async move {
        let ids_to_start = {
            let q = q_clone.lock().await;
            q.items
                .iter()
                .filter(|i| i.status == queue::QueueStatus::Active)
                .filter(|i| i.id == download_id)
                .map(|i| i.id)
                .collect::<Vec<_>>()
        };

        let stagger = {
            let q = q_clone.lock().await;
            q.stagger_delay_ms
        };

        for (i, nid) in ids_to_start.into_iter().enumerate() {
            if i > 0 && stagger > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(stagger)).await;
            }
            let a = app_clone.clone();
            let qc = q_clone.clone();
            tokio::spawn(async move {
                queue::spawn_download(a, qc, nid).await;
            });
        }
    });

    tracing::debug!("[perf] download_from_url took {:?}", _timer_start.elapsed());
    Ok(DownloadStarted {
        id: download_id,
        title,
    })
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn download_with_custom_args(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    url: String,
    output_dir: String,
    custom_args: Vec<String>,
    cookie_slug: Option<String>,
) -> Result<DownloadStarted, String> {
    if url.trim().is_empty() {
        return Err("URL is required".to_string());
    }
    if let Err(err) = crate::core::path_limits::validate_output_dir(&output_dir) {
        return Err(format!(
            "PathTooLong|{}|{}|{}",
            err.limit, err.current, err.reserve
        ));
    }

    let mut download_id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let download_queue = state.download_queue.clone();
    {
        let settings = config::load_settings(&app);
        let mut q = download_queue.lock().await;
        q.max_concurrent = settings.advanced.max_concurrent_downloads.max(1);
        q.stagger_delay_ms = settings.advanced.stagger_delay_ms;
        q.default_max_retries = settings.advanced.max_retries;
        if q.has_url(&url) {
            return Err("Download already in progress for this URL".to_string());
        }
        download_id = q.next_available_id(download_id);
    }

    let downloader: Arc<dyn crate::platforms::traits::PlatformDownloader> =
        Arc::new(crate::platforms::generic_ytdlp::GenericYtdlpDownloader::new());

    let title = url.clone();
    let ytdlp_path = ytdlp::find_ytdlp_cached().await;

    let state_to_emit = {
        let mut q = download_queue.lock().await;
        q.enqueue(
            download_id,
            url,
            "generic".to_string(),
            title.clone(),
            output_dir,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            downloader,
            ytdlp_path,
            false,
            cookie_slug,
            Some(custom_args),
            None,
            None,
            None,
        );
        let next_ids = q.next_queued_ids();
        for nid in &next_ids {
            q.mark_active(*nid);
        }
        q.get_state()
    };
    emit_queue_state_from_state(&app, state_to_emit);

    let q_clone = download_queue.clone();
    let app_clone = app.clone();
    tokio::spawn(async move {
        let ids_to_start = {
            let q = q_clone.lock().await;
            q.items
                .iter()
                .filter(|i| i.status == queue::QueueStatus::Active)
                .filter(|i| i.id == download_id)
                .map(|i| i.id)
                .collect::<Vec<_>>()
        };
        for nid in ids_to_start {
            let a = app_clone.clone();
            let qc = q_clone.clone();
            tokio::spawn(async move {
                queue::spawn_download(a, qc, nid).await;
            });
        }
    });

    Ok(DownloadStarted {
        id: download_id,
        title,
    })
}

#[tauri::command]
pub async fn cancel_generic_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    download_id: u64,
) -> Result<String, String> {
    let (state_to_emit, seeding_torrent_id) = {
        let mut q = state.download_queue.lock().await;
        let (cancelled, torrent_id) = q.cancel(download_id);
        if cancelled {
            (Some(q.get_state()), torrent_id)
        } else {
            (None, None)
        }
    };
    if let Some(tid) = seeding_torrent_id {
        if let Some(session) = state.torrent_session.lock().await.as_ref() {
            let _ = session
                .delete(librqbit::api::TorrentIdOrHash::Id(tid), false)
                .await;
        }
    }
    if let Some(s) = state_to_emit {
        emit_queue_state_from_state(&app, s);
        queue::try_start_next(app, state.download_queue.clone()).await;
        Ok("Download cancelled".to_string())
    } else {
        Err("No active download for this ID".to_string())
    }
}

#[tauri::command]
pub async fn pause_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    download_id: u64,
) -> Result<String, String> {
    let (state_to_emit, torrent_id) = {
        let mut q = state.download_queue.lock().await;
        if q.pause(download_id) {
            let tid = q
                .items
                .iter()
                .find(|i| i.id == download_id)
                .and_then(|i| i.torrent_id);
            (Some(q.get_state()), tid)
        } else {
            (None, None)
        }
    };
    if let Some(tid) = torrent_id {
        if let Some(session) = state.torrent_session.lock().await.as_ref() {
            if let Some(handle) = session.get(librqbit::api::TorrentIdOrHash::Id(tid)) {
                let _ = session.pause(&handle).await;
            }
        }
    }
    if let Some(s) = state_to_emit {
        emit_queue_state_from_state(&app, s);
        queue::try_start_next(app, state.download_queue.clone()).await;
        Ok("Download paused".to_string())
    } else {
        Err("Download cannot be paused".to_string())
    }
}

#[tauri::command]
pub async fn resume_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    download_id: u64,
) -> Result<String, String> {
    let (state_to_emit, torrent_id) = {
        let mut q = state.download_queue.lock().await;
        if q.resume(download_id) {
            let tid = q
                .items
                .iter()
                .find(|i| i.id == download_id)
                .and_then(|i| i.torrent_id);
            (Some(q.get_state()), tid)
        } else {
            (None, None)
        }
    };
    if let Some(tid) = torrent_id {
        if let Some(session) = state.torrent_session.lock().await.as_ref() {
            if let Some(handle) = session.get(librqbit::api::TorrentIdOrHash::Id(tid)) {
                let _ = session.unpause(&handle).await;
            }
        }
    }
    if let Some(s) = state_to_emit {
        emit_queue_state_from_state(&app, s);
        queue::try_start_next(app, state.download_queue.clone()).await;
        Ok("Download resumed".to_string())
    } else {
        Err("Download cannot be resumed".to_string())
    }
}

#[tauri::command]
pub async fn retry_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    download_id: u64,
) -> Result<String, String> {
    let state_to_emit = {
        let mut q = state.download_queue.lock().await;
        if q.retry(download_id) {
            Some(q.get_state())
        } else {
            None
        }
    };
    if let Some(s) = state_to_emit {
        emit_queue_state_from_state(&app, s);
        queue::try_start_next(app, state.download_queue.clone()).await;
        Ok("Download re-queued".to_string())
    } else {
        Err("Download cannot be retried".to_string())
    }
}

// Deletes only the exact recorded final path (file → unlink, dir → recursive)
// when it exists, plus http_fetcher sidecars derived from that exact path.
// Bounded by construction: every target is derived from the stored file_path,
// so it can never touch an unrelated file. Best-effort: failures here never
// fail the list removal.
fn delete_downloaded_path(path: &str) {
    let p = std::path::Path::new(path);
    if !p.is_absolute() {
        return;
    }
    match p.metadata() {
        Ok(meta) if meta.is_dir() => {
            if let Err(e) = std::fs::remove_dir_all(p) {
                tracing::warn!("[remove] failed to delete directory: {}", e);
            }
        }
        Ok(_) => {
            if let Err(e) = std::fs::remove_file(p) {
                tracing::warn!("[remove] failed to delete file: {}", e);
            }
        }
        Err(_) => {}
    }
    for suffix in [".part", ".resume.json"] {
        let sidecar = format!("{}{}", path, suffix);
        let sp = std::path::Path::new(&sidecar);
        if sp.is_file() {
            let _ = std::fs::remove_file(sp);
        }
    }
}

#[tauri::command]
pub async fn remove_download(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    download_id: u64,
    delete_file: Option<bool>,
) -> Result<String, String> {
    let (state_to_emit, seeding_torrent_id, file_path) = {
        let mut q = state.download_queue.lock().await;
        let path = if delete_file.unwrap_or(false) {
            q.items
                .iter()
                .find(|i| i.id == download_id)
                .and_then(|i| i.file_path.clone())
        } else {
            None
        };
        match q.remove(download_id) {
            Some(torrent_id) => (Some(q.get_state()), torrent_id, path),
            None => (None, None, None),
        }
    };
    if let Some(tid) = seeding_torrent_id {
        if let Some(session) = state.torrent_session.lock().await.as_ref() {
            let _ = session
                .delete(librqbit::api::TorrentIdOrHash::Id(tid), false)
                .await;
        }
    }
    if let Some(path) = file_path {
        delete_downloaded_path(&path);
    }
    if let Some(s) = state_to_emit {
        crate::core::download_log::clear(download_id);
        emit_queue_state_from_state(&app, s);
        queue::try_start_next(app, state.download_queue.clone()).await;
        Ok("Download removed".to_string())
    } else {
        Err("Download not found".to_string())
    }
}

#[tauri::command]
pub fn get_download_log(download_id: u64) -> Vec<String> {
    crate::core::download_log::get(download_id)
}

#[tauri::command]
pub fn get_recovery_items() -> Vec<crate::core::recovery::RecoveryItem> {
    crate::core::recovery::list()
}

#[tauri::command]
pub fn get_download_history() -> Vec<crate::core::queue_history::HistoryEntry> {
    crate::core::queue_history::list()
}

#[tauri::command]
pub fn clear_download_history() {
    crate::core::queue_history::clear_all();
}

#[tauri::command]
pub fn discard_recovery() {
    crate::core::recovery::clear_all();
}

#[tauri::command]
pub async fn restore_recovery(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<u32, String> {
    let items = crate::core::recovery::list();
    crate::core::recovery::clear_all();
    let mut restored: u32 = 0;
    for item in items {
        match download_from_url(
            app.clone(),
            state.clone(),
            item.url,
            item.output_dir,
            item.download_mode,
            item.quality,
            item.format_id,
            item.referer,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        {
            Ok(_) => restored += 1,
            Err(e) => tracing::warn!("[recovery] restore failed: {}", e),
        }
    }
    Ok(restored)
}

#[tauri::command]
pub fn parse_batch_file(path: String) -> Result<Vec<String>, String> {
    let content = std::fs::read_to_string(&path).map_err(|e| format!("Read error: {}", e))?;
    let mut urls = Vec::new();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let candidate = line.split('|').next().unwrap_or(line).trim();
        if candidate.starts_with("http://")
            || candidate.starts_with("https://")
            || candidate.starts_with("magnet:")
            || candidate.starts_with("p2p:")
        {
            urls.push(candidate.to_string());
        }
    }
    Ok(urls)
}

#[tauri::command]
pub async fn update_max_concurrent(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    max: u32,
) -> Result<String, String> {
    if !(1..=10).contains(&max) {
        return Err("Value must be between 1 and 10".to_string());
    }
    let state_to_emit = {
        let mut q = state.download_queue.lock().await;
        q.max_concurrent = max;
        q.get_state()
    };
    emit_queue_state_from_state(&app, state_to_emit);
    queue::try_start_next(app, state.download_queue.clone()).await;
    Ok(format!("Max concurrent set to {}", max))
}

#[tauri::command]
pub async fn pause_all_downloads(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<u32, String> {
    let (state_to_emit, count, paused_torrents) = {
        let mut q = state.download_queue.lock().await;
        let paused = q.pause_all();
        let n = paused.len() as u32;
        let torrents: Vec<usize> = paused.iter().filter_map(|(_, tid)| *tid).collect();
        (q.get_state(), n, torrents)
    };
    if let Some(session) = state.torrent_session.lock().await.as_ref() {
        for tid in paused_torrents {
            if let Some(handle) = session.get(librqbit::api::TorrentIdOrHash::Id(tid)) {
                let _ = session.pause(&handle).await;
            }
        }
    }
    emit_queue_state_from_state(&app, state_to_emit);
    Ok(count)
}

#[tauri::command]
pub async fn resume_all_downloads(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<u32, String> {
    let (state_to_emit, count, resumed_torrents) = {
        let mut q = state.download_queue.lock().await;
        let resumed = q.resume_all();
        let n = resumed.len() as u32;
        let torrents: Vec<usize> = resumed.iter().filter_map(|(_, tid)| *tid).collect();
        (q.get_state(), n, torrents)
    };
    if let Some(session) = state.torrent_session.lock().await.as_ref() {
        for tid in resumed_torrents {
            if let Some(handle) = session.get(librqbit::api::TorrentIdOrHash::Id(tid)) {
                let _ = session.unpause(&handle).await;
            }
        }
    }
    emit_queue_state_from_state(&app, state_to_emit);
    queue::try_start_next(app, state.download_queue.clone()).await;
    Ok(count)
}

#[tauri::command]
pub async fn reorder_queue(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    ids: Vec<u64>,
) -> Result<bool, String> {
    let (changed, state_to_emit) = {
        let mut q = state.download_queue.lock().await;
        let ok = q.reorder(ids);
        (ok, q.get_state())
    };
    if changed {
        emit_queue_state_from_state(&app, state_to_emit);
    }
    Ok(changed)
}

#[tauri::command]
pub async fn clear_finished_downloads(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<String, String> {
    let (state_to_emit, finished_ids) = {
        let mut q = state.download_queue.lock().await;
        let ids = q
            .items
            .iter()
            .filter(|i| {
                matches!(
                    i.status,
                    crate::core::queue::QueueStatus::Complete { .. }
                        | crate::core::queue::QueueStatus::Error { .. }
                )
            })
            .map(|i| i.id)
            .collect::<Vec<_>>();
        q.clear_finished();
        (q.get_state(), ids)
    };
    for id in finished_ids {
        crate::core::download_log::clear(id);
    }
    emit_queue_state_from_state(&app, state_to_emit);
    Ok("Finished downloads cleared".to_string())
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn reveal_file(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path))
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-R", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        use std::path::{Path, PathBuf};
        use std::process::Stdio;

        let file_path = Path::new(&path);
        let abs_path: PathBuf = if file_path.is_absolute() {
            file_path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(file_path))
                .unwrap_or_else(|_| file_path.to_path_buf())
        };

        let dir_path = abs_path.parent().unwrap_or(&abs_path);
        let item_uri = url::Url::from_file_path(&abs_path)
            .or_else(|_| url::Url::from_file_path(file_path))
            .map(|u| u.to_string())
            .unwrap_or_else(|_| format!("file://{}", abs_path.display()));
        let dir_uri = url::Url::from_directory_path(dir_path)
            .map(|u| u.to_string())
            .unwrap_or_else(|_| format!("file://{}", dir_path.display()));

        let gdbus_show_items_arg = format!(
            "[\"{}\"]",
            item_uri.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let show_items_with_gdbus = tokio::process::Command::new("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                "org.freedesktop.FileManager1",
                "--object-path",
                "/org/freedesktop/FileManager1",
                "--method",
                "org.freedesktop.FileManager1.ShowItems",
                &gdbus_show_items_arg,
                "",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);

        let show_items_ok = if show_items_with_gdbus {
            true
        } else {
            let dbus_send_array_arg = format!("array:string:{}", item_uri);
            tokio::process::Command::new("dbus-send")
                .args([
                    "--session",
                    "--dest=org.freedesktop.FileManager1",
                    "--type=method_call",
                    "/org/freedesktop/FileManager1",
                    "org.freedesktop.FileManager1.ShowItems",
                    &dbus_send_array_arg,
                    "string:",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false)
        };

        if !show_items_ok {
            let portal_ok = tokio::process::Command::new("gdbus")
                .args([
                    "call",
                    "--session",
                    "--dest",
                    "org.freedesktop.portal.Desktop",
                    "--object-path",
                    "/org/freedesktop/portal/desktop",
                    "--method",
                    "org.freedesktop.portal.OpenURI.OpenDirectory",
                    "",
                    &dir_uri,
                    "{}",
                ])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .await
                .map(|s| s.success())
                .unwrap_or(false);

            if !portal_ok {
                std::process::Command::new("xdg-open")
                    .arg(dir_path)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|e| e.to_string())?;
            }
        }
    }

    Ok(())
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn open_path_default(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &path])
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}
