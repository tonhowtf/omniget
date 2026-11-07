use crate::models::progress::ProgressUpdate;
use crate::platforms::traits::PlatformDownloader;
use std::collections::HashSet;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::core::ytdlp;
use crate::models::media::{
    DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality as MediaVideoQuality,
};

const MIX_ITEM_LIMIT: u32 = 100;

pub struct YouTubeDownloader;

impl Default for YouTubeDownloader {
    fn default() -> Self {
        Self::new()
    }
}

// WHY: reporting a partial playlist as complete hides the videos the user never got. The files that
// did download stay in the folder, and a retry skips them.
fn playlist_failure(
    succeeded: usize,
    total: usize,
    last_err: Option<anyhow::Error>,
    dir: &std::path::Path,
) -> Option<anyhow::Error> {
    if succeeded == total {
        return None;
    }
    let reason = last_err
        .map(|e| e.to_string())
        .unwrap_or_else(|| "Playlist download finished without any files".to_string());
    if succeeded == 0 {
        return Some(anyhow!("{reason}"));
    }
    Some(anyhow!(
        "{} of {total} playlist videos failed. The other {succeeded} are in {}. Last error: {reason}",
        total - succeeded,
        dir.display(),
    ))
}

impl YouTubeDownloader {
    pub fn new() -> Self {
        Self
    }

    fn extract_video_id(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let host = parsed.host_str()?.to_lowercase();

        if host.contains("youtu.be") {
            let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
            return segments.first().map(|s| s.to_string());
        }

        if host.contains("youtube.com") && parsed.path().starts_with("/embed/") {
            let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
            return segments.last().map(|s| s.to_string());
        }

        if host.contains("youtube.com") || host.contains("youtube-nocookie.com") {
            let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();

            if matches!(segments.first(), Some(&"shorts") | Some(&"live")) {
                return segments.get(1).map(|s| s.to_string());
            }

            return parsed
                .query_pairs()
                .find(|(k, _)| k == "v")
                .map(|(_, v)| v.to_string());
        }

        None
    }

    pub fn is_playlist_url(url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if parsed.path().starts_with("/playlist") {
                return true;
            }

            let mut has_list = false;
            let mut has_video = false;
            for (key, value) in parsed.query_pairs() {
                if key == "list" && !value.is_empty() {
                    has_list = true;
                }
                if key == "v" && !value.is_empty() {
                    has_video = true;
                }
            }

            return has_list && !has_video;
        }
        false
    }

    fn playlist_limit_flags(url: &str) -> Vec<String> {
        let is_mix = url::Url::parse(url).ok().is_some_and(|parsed| {
            parsed
                .query_pairs()
                .any(|(k, v)| k == "list" && v.starts_with("RD"))
        });
        if is_mix {
            vec!["--playlist-end".to_string(), MIX_ITEM_LIMIT.to_string()]
        } else {
            Vec::new()
        }
    }

    pub async fn fetch_playlist(
        url: &str,
        ytdlp_path: &std::path::Path,
    ) -> anyhow::Result<MediaInfo> {
        let (playlist_title, entries) =
            ytdlp::get_playlist_info(ytdlp_path, url, &Self::playlist_limit_flags(url)).await?;

        if entries.is_empty() {
            return Err(anyhow!("Playlist empty or unavailable"));
        }

        let qualities: Vec<MediaVideoQuality> = entries
            .into_iter()
            .enumerate()
            .map(|(i, entry)| MediaVideoQuality {
                label: format!("{}. {}", i + 1, entry.title),
                width: 0,
                height: 0,
                url: entry.url,
                format: "ytdlp_playlist".to_string(),
            })
            .collect();

        Ok(MediaInfo {
            title: sanitize_filename::sanitize(&playlist_title),
            author: playlist_title,
            platform: "youtube".to_string(),
            duration_seconds: None,
            thumbnail_url: None,
            available_qualities: qualities,
            media_type: MediaType::Playlist,
            file_size_bytes: None,
        })
    }

    pub async fn fetch_with_ytdlp(
        url: &str,
        ytdlp_path: &std::path::Path,
    ) -> anyhow::Result<MediaInfo> {
        if Self::is_playlist_url(url) {
            return Self::fetch_playlist(url, ytdlp_path).await;
        }

        let _video_id = Self::extract_video_id(url)
            .ok_or_else(|| anyhow!("Could not extract YouTube video ID"))?;

        let json = ytdlp::get_video_info(ytdlp_path, url, &[]).await?;
        Self::parse_video_info(&json)
    }

    fn extract_quality_height(quality_str: &str) -> Option<u32> {
        let s = quality_str.trim().to_lowercase();
        if s == "best" || s == "highest" {
            return None;
        }
        s.trim_end_matches('p').parse::<u32>().ok()
    }

    pub fn parse_video_info(json: &serde_json::Value) -> anyhow::Result<MediaInfo> {
        let video_id = json
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let title = json
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let author = json
            .get("uploader")
            .or_else(|| json.get("channel"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let duration = json.get("duration").and_then(|v| v.as_f64());

        let thumbnail = json
            .get("thumbnail")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let is_live = json
            .get("is_live")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if is_live {
            return Err(anyhow!("Livestreams not supported"));
        }

        let mut qualities: Vec<MediaVideoQuality> = Vec::new();
        let mut seen_heights: HashSet<u32> = HashSet::new();

        if let Some(formats) = json.get("formats").and_then(|v| v.as_array()) {
            for f in formats {
                let height = f.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let width = f.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let vcodec = f.get("vcodec").and_then(|v| v.as_str()).unwrap_or("none");
                let acodec = f.get("acodec").and_then(|v| v.as_str()).unwrap_or("none");

                if vcodec == "none" || height == 0 {
                    continue;
                }

                let has_audio = acodec != "none";

                if seen_heights.insert(height) {
                    let label = if has_audio {
                        format!("{}p", height)
                    } else {
                        format!("{}p (HD)", height)
                    };

                    qualities.push(MediaVideoQuality {
                        label,
                        width,
                        height,
                        url: format!("https://www.youtube.com/watch?v={}", video_id),
                        format: "ytdlp".to_string(),
                    });
                }
            }
        }

        qualities.sort_by_key(|q| std::cmp::Reverse(q.height));

        if qualities.is_empty() {
            qualities.push(MediaVideoQuality {
                label: "best".to_string(),
                width: 0,
                height: 0,
                url: format!("https://www.youtube.com/watch?v={}", video_id),
                format: "ytdlp".to_string(),
            });
        }

        Ok(MediaInfo {
            title,
            author,
            platform: "youtube".to_string(),
            duration_seconds: duration,
            thumbnail_url: thumbnail,
            available_qualities: qualities,
            media_type: MediaType::Video,
            file_size_bytes: None,
        })
    }
}

#[async_trait]
impl PlatformDownloader for YouTubeDownloader {
    fn name(&self) -> &str {
        "youtube"
    }

    fn can_handle(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let host = host.to_lowercase();
                return host == "youtube.com"
                    || host.ends_with(".youtube.com")
                    || host == "youtu.be"
                    || host == "youtube-nocookie.com"
                    || host.ends_with(".youtube-nocookie.com");
            }
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let ytdlp_path = ytdlp::ensure_ytdlp()
            .await
            .map_err(|e| anyhow!("YouTube requires yt-dlp. Failed to get yt-dlp: {}", e))?;

        Self::fetch_with_ytdlp(url, &ytdlp_path).await
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<ProgressUpdate>,
    ) -> anyhow::Result<DownloadResult> {
        let _ = progress.send(ProgressUpdate::percent(0.0)).await;

        let ytdlp_path = if let Some(ref p) = opts.ytdlp_path {
            p.clone()
        } else {
            ytdlp::ensure_ytdlp().await?
        };

        let quality_height = opts
            .quality
            .as_deref()
            .and_then(Self::extract_quality_height);

        if info.media_type == MediaType::Playlist {
            return self
                .download_playlist(info, opts, progress, &ytdlp_path, quality_height)
                .await;
        }

        let first = info
            .available_qualities
            .first()
            .ok_or_else(|| anyhow!("No quality available"))?;

        let selected = match quality_height {
            Some(h) => info
                .available_qualities
                .iter()
                .filter(|q| q.height > 0 && q.height <= h)
                .max_by_key(|q| q.height)
                .unwrap_or(first),
            None => first,
        };
        let video_url = &selected.url;

        let extra_flags: Vec<String> = opts
            .custom_ytdlp_args
            .as_deref()
            .map(|args| args.to_vec())
            .unwrap_or_default();

        ytdlp::download_video(
            &ytdlp_path,
            video_url,
            &opts.output_dir,
            quality_height,
            progress,
            opts.download_mode.as_deref(),
            opts.format_id.as_deref(),
            opts.filename_template.as_deref(),
            opts.referer.as_deref().or(Some("https://www.youtube.com/")),
            opts.cancel_token.clone(),
            None,
            opts.concurrent_fragments,
            opts.download_subtitles,
            &extra_flags,
            opts.audio_format.as_deref(),
        )
        .await
    }
}

impl YouTubeDownloader {
    async fn download_playlist(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<ProgressUpdate>,
        ytdlp_path: &std::path::Path,
        quality_height: Option<u32>,
    ) -> anyhow::Result<DownloadResult> {
        let playlist_dir = opts
            .output_dir
            .join(sanitize_filename::sanitize(&info.title));
        tokio::fs::create_dir_all(&playlist_dir).await?;

        let total = info.available_qualities.len();
        let last_position = ytdlp::last_playlist_position(&info.available_qualities);
        let mut total_bytes = 0u64;
        let mut last_path = playlist_dir.clone();
        let mut success_count = 0usize;
        let mut last_err: Option<anyhow::Error> = None;

        for (i, entry) in info.available_qualities.iter().enumerate() {
            if opts.cancel_token.is_cancelled() {
                anyhow::bail!("Download cancelled");
            }

            let (video_tx, mut video_rx) = mpsc::channel::<ProgressUpdate>(16);
            let progress_tx = progress.clone();
            let video_idx = i;
            let video_total = total;
            let forwarder = tokio::spawn(async move {
                let mut max_pct = 0.0_f64;
                while let Some(pu) = video_rx.recv().await {
                    max_pct = max_pct.max(pu.percent);
                    let overall = (video_idx as f64 / video_total as f64) * 100.0
                        + (max_pct / video_total as f64);
                    let _ = progress_tx
                        .send(ProgressUpdate::rich(
                            overall,
                            None,
                            None,
                            pu.speed_bps,
                            None,
                        ))
                        .await;
                }
            });

            let extra_flags: Vec<String> = opts
                .custom_ytdlp_args
                .as_deref()
                .map(|args| args.to_vec())
                .unwrap_or_default();
            let numbered_template = opts.playlist_number_files.then(|| {
                ytdlp::numbered_filename_template(
                    opts.filename_template.as_deref(),
                    ytdlp::playlist_position(entry, i),
                    last_position,
                )
            });

            match ytdlp::download_video(
                ytdlp_path,
                &entry.url,
                &playlist_dir,
                quality_height,
                video_tx,
                opts.download_mode.as_deref(),
                None,
                numbered_template
                    .as_deref()
                    .or(opts.filename_template.as_deref()),
                opts.referer.as_deref().or(Some("https://www.youtube.com/")),
                opts.cancel_token.clone(),
                None,
                opts.concurrent_fragments,
                opts.download_subtitles,
                &extra_flags,
                opts.audio_format.as_deref(),
            )
            .await
            {
                Ok(result) => {
                    success_count += 1;
                    total_bytes += result.file_size_bytes;
                    last_path = result.file_path;
                }
                Err(e) => {
                    tracing::warn!("Playlist video {} failed: {}", i + 1, e);
                    last_err = Some(e);
                }
            }

            let _ = forwarder.await;
        }

        if let Some(e) = playlist_failure(success_count, total, last_err, &playlist_dir) {
            return Err(e);
        }

        if success_count > 1 {
            last_path = playlist_dir.clone();
        }

        let _ = progress.send(ProgressUpdate::percent(100.0)).await;

        Ok(DownloadResult {
            file_path: last_path,
            file_size_bytes: total_bytes,
            duration_seconds: 0.0,
            torrent_id: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_playlist_with_failures_is_not_reported_as_complete() {
        let dir = std::path::Path::new("/tmp/list");
        assert!(playlist_failure(3, 3, None, dir).is_none());
        let partial = playlist_failure(2, 3, Some(anyhow!("HTTP 403")), dir)
            .unwrap()
            .to_string();
        assert!(
            partial.starts_with("1 of 3 playlist videos failed"),
            "{partial}"
        );
        assert!(
            partial.contains("/tmp/list") && partial.contains("HTTP 403"),
            "{partial}"
        );
        let none = playlist_failure(0, 3, Some(anyhow!("HTTP 403")), dir)
            .unwrap()
            .to_string();
        assert_eq!(none, "HTTP 403");
    }

    use super::YouTubeDownloader;
    use crate::core::ytdlp;
    use crate::models::media::{
        DownloadOptions, MediaInfo, MediaType, VideoQuality as MediaVideoQuality,
    };
    use crate::platforms::traits::PlatformDownloader;
    use tokio::sync::mpsc;

    #[test]
    fn watch_url_with_playlist_param_is_single_video() {
        assert!(!YouTubeDownloader::is_playlist_url(
            "https://www.youtube.com/watch?v=abc123&list=PLxyz&index=2",
        ));
    }

    #[test]
    fn an_endless_mix_is_capped_and_a_normal_playlist_is_not() {
        assert_eq!(
            YouTubeDownloader::playlist_limit_flags(
                "https://www.youtube.com/watch?v=abc&list=RDabc"
            ),
            vec!["--playlist-end".to_string(), "100".to_string()]
        );
        assert!(YouTubeDownloader::playlist_limit_flags(
            "https://www.youtube.com/playlist?list=PL1"
        )
        .is_empty());
    }

    fn network_options(dir: &std::path::Path, mode: Option<&str>) -> DownloadOptions {
        DownloadOptions {
            quality: Some("360p".to_string()),
            output_dir: dir.to_path_buf(),
            filename_template: Some("%(title).200s [%(id)s].%(ext)s".to_string()),
            download_subtitles: false,
            include_auto_subtitles: false,
            download_mode: mode.map(str::to_string),
            audio_format: mode.map(|_| "m4a".to_string()),
            format_id: None,
            referer: None,
            extra_headers: None,
            page_url: None,
            user_agent: None,
            cancel_token: tokio_util::sync::CancellationToken::new(),
            concurrent_fragments: 1,
            ytdlp_path: None,
            torrent_listen_port: None,
            torrent_id_slot: None,
            custom_ytdlp_args: None,
            torrent_files: None,
            torrent_auto_trackers: false,
            torrent_upnp: false,
            torrent_stop_seeding: false,
            playlist_number_files: true,
        }
    }

    fn streams(path: &std::path::Path) -> String {
        let out = std::process::Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "stream=codec_name,codec_type:stream_disposition=attached_pic",
                "-of",
                "compact",
            ])
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn fresh_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("omniget-net-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    #[ignore = "downloads from YouTube"]
    async fn a_selected_audio_playlist_downloads_every_item_numbered_with_cover_art() {
        let dir = fresh_dir("playlist");
        let entry = |n: u32, id: &str| MediaVideoQuality {
            label: format!("{n}. item"),
            width: 0,
            height: 0,
            url: format!("https://www.youtube.com/watch?v={id}"),
            format: "ytdlp_playlist".to_string(),
        };
        let info = MediaInfo {
            title: "List".to_string(),
            author: "List".to_string(),
            platform: "youtube".to_string(),
            duration_seconds: None,
            thumbnail_url: None,
            available_qualities: vec![entry(2, "jNQXAC9IVRw"), entry(5, "dQw4w9WgXcQ")],
            media_type: MediaType::Playlist,
            file_size_bytes: None,
        };
        let (tx, mut rx) = mpsc::channel(64);
        tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let result = YouTubeDownloader::new()
            .download(&info, &network_options(&dir, Some("audio")), tx)
            .await
            .unwrap();
        assert!(result.file_path.is_dir());
        let mut names: Vec<std::path::PathBuf> = std::fs::read_dir(&result.file_path)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        names.sort();
        assert_eq!(names.len(), 2, "{names:?}");
        let first = names[0].file_name().unwrap().to_string_lossy().into_owned();
        let second = names[1].file_name().unwrap().to_string_lossy().into_owned();
        assert!(first.starts_with("02 - "), "{first}");
        assert!(second.starts_with("05 - "), "{second}");
        for path in &names {
            let probe = streams(path);
            assert!(probe.contains("codec_type=audio"), "{probe}");
            assert!(probe.contains("attached_pic=1"), "{probe}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    #[ignore = "downloads from YouTube"]
    async fn a_thumbnails_only_download_saves_an_image_and_no_video() {
        let dir = fresh_dir("thumb");
        let (tx, mut rx) = mpsc::channel(64);
        tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let flags: Vec<String> = [
            "--skip-download",
            "--write-thumbnail",
            "--convert-thumbnails",
            "jpg",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let path = ytdlp::ensure_ytdlp().await.unwrap();
        let result = ytdlp::download_video(
            &path,
            "https://www.youtube.com/watch?v=jNQXAC9IVRw",
            &dir,
            None,
            tx,
            None,
            None,
            None,
            None,
            tokio_util::sync::CancellationToken::new(),
            None,
            1,
            false,
            &flags,
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.file_path.extension().unwrap(), "jpg");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    #[ignore = "downloads from YouTube"]
    async fn the_compatible_choice_saves_h264_and_aac_in_mp4() {
        ytdlp::set_compatible_video_fn(|| true);
        let dir = fresh_dir("compatible");
        let (tx, mut rx) = mpsc::channel(64);
        tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let path = ytdlp::ensure_ytdlp().await.unwrap();
        let result = ytdlp::download_video(
            &path,
            "https://www.youtube.com/live/jNQXAC9IVRw",
            &dir,
            Some(360),
            tx,
            None,
            None,
            None,
            None,
            tokio_util::sync::CancellationToken::new(),
            None,
            1,
            false,
            &[],
            None,
        )
        .await
        .unwrap();
        assert_eq!(result.file_path.extension().unwrap(), "mp4");
        let probe = streams(&result.file_path);
        assert!(probe.contains("codec_name=h264"), "{probe}");
        assert!(probe.contains("codec_name=aac"), "{probe}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn live_format_link_yields_the_video_id() {
        assert_eq!(
            YouTubeDownloader::extract_video_id("https://www.youtube.com/live/jNQXAC9IVRw?si=x")
                .as_deref(),
            Some("jNQXAC9IVRw")
        );
        assert!(!YouTubeDownloader::is_playlist_url(
            "https://www.youtube.com/live/jNQXAC9IVRw"
        ));
    }

    #[test]
    fn playlist_url_is_playlist() {
        assert!(YouTubeDownloader::is_playlist_url(
            "https://www.youtube.com/playlist?list=PLxyz",
        ));
    }

    #[test]
    fn list_only_watch_url_is_playlist() {
        assert!(YouTubeDownloader::is_playlist_url(
            "https://www.youtube.com/watch?list=PLxyz",
        ));
    }
}
