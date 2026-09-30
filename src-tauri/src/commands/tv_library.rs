//! Downloads › Tools › TV library: turns a channel or playlist into a TV
//! show that Jellyfin, Emby, Kodi and Plex recognise on their own, and keeps
//! it in sync when run again.
//!
//! The layout comes from ytdl-sub's "TV Show by Date" presets (GPL-3.0;
//! reimplemented here, no code copied): one folder per channel, a
//! `Season YYYY` folder per upload year, episodes named
//! `sYYYY.eMMDDNN - Title.ext` (NN = order of the video on that day), a
//! `-thumb.jpg` and an `episodedetails` NFO next to each episode, a
//! `tvshow.nfo` at the top, and a download archive so a second run only
//! fetches what is new. Plex reads no NFO, so it gets the files and the
//! metadata embedded in the MP4 instead.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;
use tauri::Emitter;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio_util::sync::CancellationToken;

#[cfg(not(target_os = "android"))]
use crate::core::ytdlp;

static CURRENT: Mutex<Option<CancellationToken>> = Mutex::new(None);
const VIDEO_EXTS: &[&str] = &["mp4", "mkv", "webm", "m4v", "mov"];

#[derive(Debug, Clone, Serialize)]
pub struct TvLibraryResult {
    pub show_dir: String,
    pub show: String,
    pub added: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct TvLibraryProgress {
    item: u32,
    total: u32,
    percent: f64,
}

pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Kodi cannot show characters outside the Basic Multilingual Plane
/// (emoji); ytdl-sub swaps them for a box, and so does this.
pub fn kodi_safe(s: &str) -> String {
    s.chars()
        .map(|c| if (c as u32) > 0xFFFF { '□' } else { c })
        .collect()
}

/// Tags in alphabetical order, empty ones left out, like ytdl-sub writes them.
fn nfo(root: &str, mut tags: Vec<(&str, String)>, kodi: bool) -> String {
    tags.retain(|(_, v)| !v.trim().is_empty());
    tags.sort_by(|a, b| a.0.cmp(b.0));
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    out.push_str(&format!("<{root}>\n"));
    for (k, v) in tags {
        let v = if kodi { kodi_safe(&v) } else { v };
        out.push_str(&format!("  <{k}>{}</{k}>\n", xml_escape(&v)));
    }
    out.push_str(&format!("</{root}>\n"));
    out
}

/// `20240315` → `("2024", "03", "15")`.
pub fn split_date(date: &str) -> Option<(&str, &str, &str)> {
    if date.len() == 8 && date.bytes().all(|b| b.is_ascii_digit()) {
        Some((&date[..4], &date[4..6], &date[6..]))
    } else {
        None
    }
}

/// The next free day index (01, 02...) for `date` among the episode files
/// already in the season folder.
pub fn next_day_index(existing: &[String], date: &str) -> u32 {
    let Some((y, m, d)) = split_date(date) else {
        return 1;
    };
    let prefix = format!("s{y}.e{m}{d}");
    let used = existing
        .iter()
        .filter(|n| n.starts_with(&prefix))
        .filter(|n| {
            Path::new(n.as_str())
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| VIDEO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
                .unwrap_or(false)
        })
        .count() as u32;
    used + 1
}

/// `s2024.e031501 - Title`
pub fn episode_stem(date: &str, index: u32, title: &str) -> String {
    let (y, m, d) = split_date(date).unwrap_or(("0000", "00", "00"));
    let clean = omniget_core::core::filename::sanitize_path_component(title);
    let clean: String = clean.chars().take(120).collect();
    format!("s{y}.e{m}{d}{index:02} - {}", clean.trim())
}

pub fn episode_nfo(
    info: &Value,
    date: &str,
    index: u32,
    genre: &str,
    rating: &str,
    kodi: bool,
) -> String {
    let (y, m, d) = split_date(date).unwrap_or(("0000", "00", "00"));
    let title = info.get("title").and_then(|v| v.as_str()).unwrap_or("");
    let url = info
        .get("webpage_url")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let description = info
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let episode = format!("{}{d}{index:02}", m.trim_start_matches('0'));
    nfo(
        "episodedetails",
        vec![
            ("aired", format!("{y}-{m}-{d}")),
            ("episode", episode),
            ("genre", genre.to_string()),
            ("mpaa", rating.to_string()),
            ("plot", format!("{url}\n\n{description}").trim().to_string()),
            ("season", y.to_string()),
            ("title", format!("{y}-{m}-{d} - {title}")),
            ("year", y.to_string()),
        ],
        kodi,
    )
}

pub fn tvshow_nfo(show: &str, genre: &str, rating: &str, kodi: bool) -> String {
    nfo(
        "tvshow",
        vec![
            ("genre", genre.to_string()),
            ("mpaa", rating.to_string()),
            ("title", show.to_string()),
        ],
        kodi,
    )
}

/// The show name: the channel, else the uploader, else the playlist.
pub fn show_name(info: &Value) -> String {
    ["channel", "uploader", "playlist_uploader", "playlist_title"]
        .iter()
        .find_map(|k| {
            info.get(*k)
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or("Videos")
        .to_string()
}

/// A stable name for the download archive of one link.
fn archive_name(url: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.trim().trim_end_matches('/').hash(&mut h);
    format!(".omniget-tv-{:016x}.txt", h.finish())
}

#[allow(clippy::too_many_arguments)]
pub fn download_args(
    url: &str,
    staging: &Path,
    archive: &Path,
    max_items: Option<u32>,
    max_height: Option<u32>,
    plex: bool,
) -> Vec<String> {
    let h = max_height.unwrap_or(1080);
    let mut a = vec![
        "--no-colors".to_string(),
        "--newline".to_string(),
        "--ignore-errors".to_string(),
        "-f".to_string(),
        format!("bv*[height<={h}]+ba/b[height<={h}]/bv*+ba/b"),
        "--merge-output-format".to_string(),
        "mp4".to_string(),
        "-o".to_string(),
        format!("{}/%(id)s.%(ext)s", staging.to_string_lossy()),
        "--write-info-json".to_string(),
        "--no-write-playlist-metafiles".to_string(),
        "--write-thumbnail".to_string(),
        "--convert-thumbnails".to_string(),
        "jpg".to_string(),
        "--download-archive".to_string(),
        archive.to_string_lossy().into_owned(),
        // ytdl-sub's default: no lives, no premieres waiting to start.
        "--match-filter".to_string(),
        "!is_live & live_status!=is_upcoming".to_string(),
    ];
    if let Some(n) = max_items.filter(|n| *n > 0) {
        a.push("--playlist-end".to_string());
        a.push(n.to_string());
    }
    if plex {
        a.push("--embed-metadata".to_string());
    }
    a.push("--".to_string());
    a.push(url.to_string());
    a
}

/// `[download] Downloading item 3 of 10`
pub fn parse_item(line: &str) -> Option<(u32, u32)> {
    let rest = line.split("Downloading item ").nth(1)?;
    let mut it = rest.split(" of ");
    let a = it.next()?.trim().parse().ok()?;
    let b = it.next()?.split_whitespace().next()?.parse().ok()?;
    Some((a, b))
}

fn parse_percent(line: &str) -> Option<f64> {
    let rest = line.trim().strip_prefix("[download]")?.trim_start();
    rest.split('%').next()?.trim().parse().ok()
}

/// Moves what yt-dlp left in `staging` into the show folder. Returns the
/// show folder, the show name and the episode files added.
pub fn arrange(
    staging: &Path,
    library: &Path,
    server: &str,
    genre: &str,
    rating: &str,
) -> std::io::Result<(PathBuf, String, Vec<String>)> {
    let plex = server == "plex";
    let kodi = server == "kodi";
    let mut infos: Vec<(Value, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(staging)?.flatten() {
        let p = entry.path();
        if p.to_string_lossy().ends_with(".info.json") {
            if let Ok(text) = std::fs::read_to_string(&p) {
                if let Ok(v) = serde_json::from_str::<Value>(&text) {
                    infos.push((v, p));
                }
            }
        }
    }
    // Oldest first, so the day index follows the upload order.
    infos.sort_by_key(|(v, _)| {
        (
            v.get("upload_date")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string(),
            v.get("timestamp").and_then(|t| t.as_i64()).unwrap_or(0),
        )
    });
    let mut added = Vec::new();
    let mut show_dir = library.to_path_buf();
    let mut show = String::new();
    for (info, info_path) in infos {
        let id = info
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let video = VIDEO_EXTS
            .iter()
            .map(|e| staging.join(format!("{id}.{e}")))
            .find(|p| p.exists());
        let Some(video) = video else {
            continue;
        };
        show = show_name(&info);
        show_dir = library.join(omniget_core::core::filename::sanitize_path_component(&show));
        let date = info
            .get("upload_date")
            .or_else(|| info.get("release_date"))
            .and_then(|v| v.as_str())
            .unwrap_or("19700101")
            .to_string();
        let (y, _, _) = split_date(&date).unwrap_or(("1970", "01", "01"));
        let season = show_dir.join(format!("Season {y}"));
        std::fs::create_dir_all(&season)?;
        let existing: Vec<String> = std::fs::read_dir(&season)?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let index = next_day_index(&existing, &date);
        let title = info
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or(&id)
            .to_string();
        let stem = episode_stem(&date, index, &title);
        let ext = video.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
        let target = season.join(format!("{stem}.{ext}"));
        std::fs::rename(&video, &target).or_else(|_| std::fs::copy(&video, &target).map(|_| ()))?;
        let thumb = staging.join(format!("{id}.jpg"));
        if thumb.exists() {
            let _ = std::fs::rename(&thumb, season.join(format!("{stem}-thumb.jpg")));
        }
        if !plex {
            std::fs::write(
                season.join(format!("{stem}.nfo")),
                episode_nfo(&info, &date, index, genre, rating, kodi),
            )?;
        }
        let _ = std::fs::remove_file(&info_path);
        added.push(target.to_string_lossy().into_owned());
    }
    if !added.is_empty() && !plex {
        let show_nfo = show_dir.join("tvshow.nfo");
        if !show_nfo.exists() {
            std::fs::write(show_nfo, tvshow_nfo(&show, genre, rating, kodi))?;
        }
    }
    Ok((show_dir, show, added))
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn tv_library_sync(
    app: tauri::AppHandle,
    url: String,
    library_dir: String,
    server: String,
    genre: Option<String>,
    max_items: Option<u32>,
    max_height: Option<u32>,
) -> Result<TvLibraryResult, String> {
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("Paste a web address that starts with http:// or https://".to_string());
    }
    omniget_core::core::platform_optout::ensure_allowed(&url)?;
    if library_dir.trim().is_empty() {
        return Err("Choose a download folder first".to_string());
    }
    let library = PathBuf::from(&library_dir);
    std::fs::create_dir_all(&library).map_err(|e| e.to_string())?;
    let staging = library.join(format!(".omniget-tv-staging-{}", std::process::id()));
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let archive = library.join(archive_name(&url));
    let plex = server == "plex";
    let mut args = download_args(&url, &staging, &archive, max_items, max_height, plex);
    let bin = ytdlp::ensure_ytdlp()
        .await
        .map_err(|e| format!("yt-dlp unavailable: {e}"))?;
    if let Ok(ffmpeg) = omniget_core::core::dependencies::ensure_ffmpeg().await {
        args.insert(0, ffmpeg.to_string_lossy().into_owned());
        args.insert(0, "--ffmpeg-location".to_string());
    }

    let token = CancellationToken::new();
    {
        let mut cur = CURRENT.lock().map_err(|e| e.to_string())?;
        if cur.is_some() {
            return Err("A library sync is already running".to_string());
        }
        *cur = Some(token.clone());
    }
    let ran = run(&app, &bin, args, token).await;
    if let Ok(mut cur) = CURRENT.lock() {
        *cur = None;
    }
    // Whatever finished is filed, even after a stop or an error on one video.
    let genre = genre
        .filter(|g| !g.trim().is_empty())
        .unwrap_or_else(|| "YouTube".to_string());
    let arranged = arrange(&staging, &library, &server, &genre, "TV-14");
    let _ = std::fs::remove_dir_all(&staging);
    let (show_dir, show, added) = arranged.map_err(|e| e.to_string())?;
    match ran {
        Err(e) if added.is_empty() => Err(e),
        _ => Ok(TvLibraryResult {
            show_dir: show_dir.to_string_lossy().into_owned(),
            show,
            added,
        }),
    }
}

#[cfg(not(target_os = "android"))]
async fn run(
    app: &tauri::AppHandle,
    bin: &Path,
    args: Vec<String>,
    token: CancellationToken,
) -> Result<(), String> {
    let mut child = omniget_core::core::process::command(bin)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Failed to start yt-dlp: {e}"))?;
    let stdout = child.stdout.take().ok_or("No output from yt-dlp")?;
    let stderr = child.stderr.take().ok_or("No output from yt-dlp")?;
    let app2 = app.clone();
    tokio::spawn(async move {
        let (mut item, mut total) = (1u32, 1u32);
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some((a, b)) = parse_item(&line) {
                item = a;
                total = b;
            } else if let Some(p) = parse_percent(&line) {
                let _ = app2.emit(
                    "tv-library-progress",
                    TvLibraryProgress {
                        item,
                        total,
                        percent: p,
                    },
                );
            }
        }
    });
    let err_reader = tokio::spawn(async move {
        let mut buf = String::new();
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });
    let status = tokio::select! {
        s = child.wait() => s.map_err(|e| e.to_string())?,
        _ = token.cancelled() => {
            let _ = child.kill().await;
            return Err("Cancelled".to_string());
        }
    };
    let err = err_reader.await.unwrap_or_default();
    if status.success() {
        return Ok(());
    }
    Err(err
        .lines()
        .rev()
        .find(|l| l.starts_with("ERROR:"))
        .map(|l| l.trim_start_matches("ERROR:").trim().to_string())
        .unwrap_or_else(|| "yt-dlp could not read this channel".to_string()))
}

#[tauri::command]
pub fn tv_library_cancel() -> bool {
    match CURRENT.lock() {
        Ok(cur) => cur.as_ref().map(|t| t.cancel()).is_some(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn names_episodes_by_date_and_order_of_the_day() {
        assert_eq!(
            episode_stem("20240315", 1, "Spring: Part 1"),
            "s2024.e031501 - Spring꞉ Part 1"
        );
        let existing = vec![
            "s2024.e031501 - A.mp4".to_string(),
            "s2024.e031501 - A.nfo".to_string(),
            "s2024.e031501 - A-thumb.jpg".to_string(),
            "s2024.e031601 - B.mp4".to_string(),
        ];
        assert_eq!(next_day_index(&existing, "20240315"), 2);
        assert_eq!(next_day_index(&existing, "20240317"), 1);
    }

    #[test]
    fn writes_the_episode_nfo_in_ytdl_sub_order() {
        let info = json!({"title": "Sprite <Fright> & more", "webpage_url": "https://v.example/w/1", "description": "A film."});
        let x = episode_nfo(&info, "20240315", 1, "Animation", "TV-14", false);
        assert!(x.starts_with("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<episodedetails>\n  <aired>2024-03-15</aired>\n  <episode>31501</episode>\n"));
        assert!(x.contains("<title>2024-03-15 - Sprite &lt;Fright&gt; &amp; more</title>"));
        assert!(x.contains("<plot>https://v.example/w/1\n\nA film.</plot>"));
        let order: Vec<usize> = [
            "<aired>",
            "<episode>",
            "<genre>",
            "<mpaa>",
            "<plot>",
            "<season>",
            "<title>",
            "<year>",
        ]
        .iter()
        .map(|t| x.find(t).unwrap())
        .collect();
        assert!(order.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn tvshow_nfo_and_kodi_emoji() {
        let x = tvshow_nfo("Blender 🎬", "Animation", "TV-14", true);
        assert!(x.contains("<title>Blender □</title>"));
        assert!(x.contains("<tvshow>") && x.contains("<genre>Animation</genre>"));
    }

    #[test]
    fn arranges_a_staged_download_into_seasons() {
        let root = std::env::temp_dir().join(format!("omniget-tv-{}", std::process::id()));
        let staging = root.join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        for (id, date) in [("a1", "20240315"), ("b2", "20240315"), ("c3", "20231201")] {
            std::fs::write(staging.join(format!("{id}.mp4")), b"v").unwrap();
            std::fs::write(staging.join(format!("{id}.jpg")), b"j").unwrap();
            std::fs::write(
                staging.join(format!("{id}.info.json")),
                json!({"id": id, "title": format!("T {id}"), "upload_date": date, "channel": "Blender Studio", "timestamp": if id == "a1" {1} else {2}}).to_string(),
            )
            .unwrap();
        }
        let (show_dir, show, added) =
            arrange(&staging, &root.join("TV"), "jellyfin", "Animation", "TV-14").unwrap();
        assert_eq!(show, "Blender Studio");
        assert_eq!(added.len(), 3);
        assert!(show_dir.join("tvshow.nfo").exists());
        assert!(show_dir
            .join("Season 2024/s2024.e031501 - T a1.mp4")
            .exists());
        assert!(show_dir
            .join("Season 2024/s2024.e031502 - T b2.mp4")
            .exists());
        assert!(show_dir
            .join("Season 2024/s2024.e031502 - T b2-thumb.jpg")
            .exists());
        assert!(show_dir
            .join("Season 2023/s2023.e120101 - T c3.nfo")
            .exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reads_the_item_counter() {
        assert_eq!(
            parse_item("[download] Downloading item 3 of 10"),
            Some((3, 10))
        );
        assert_eq!(parse_item("[download]  5.0% of 1MiB"), None);
    }

    #[test]
    fn download_args_keep_an_archive_and_skip_lives() {
        let a = download_args(
            "https://v.example/c/x",
            Path::new("/tmp/s"),
            Path::new("/tmp/a.txt"),
            Some(5),
            None,
            true,
        );
        assert!(a
            .windows(2)
            .any(|w| w == ["--download-archive", "/tmp/a.txt"]));
        assert!(a.windows(2).any(|w| w == ["--playlist-end", "5"]));
        assert!(a.iter().any(|x| x == "--embed-metadata"));
        assert_eq!(a.last().unwrap(), "https://v.example/c/x");
    }
}
