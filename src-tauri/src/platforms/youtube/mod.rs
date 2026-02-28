use std::collections::HashSet;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::core::ytdlp;
use crate::models::media::{
    DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality as MediaVideoQuality,
};
use crate::platforms::traits::PlatformDownloader;

pub struct YouTubeDownloader;

impl Default for YouTubeDownloader {
    fn default() -> Self {
        Self::new()
    }
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

            if segments.first() == Some(&"shorts") {
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
            if parsed.query_pairs().any(|(k, _)| k == "list") {
                return true;
            }
        }
        false
    }

    pub async fn fetch_with_ytdlp(
        url: &str,
        ytdlp_path: &std::path::Path,
    ) -> anyhow::Result<MediaInfo> {
        if Self::is_playlist_url(url) {
            let (playlist_title, entries) = ytdlp::get_playlist_info(ytdlp_path, url, &[]).await?;

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

            return Ok(MediaInfo {
                title: sanitize_filename::sanitize(&playlist_title),
                author: playlist_title,
                platform: "youtube".to_string(),
                duration_seconds: None,
                thumbnail_url: None,
                available_qualities: qualities,
                media_type: MediaType::Playlist,
                file_size_bytes: None,
            });
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

        qualities.sort_by(|a, b| b.height.cmp(&a.height));

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
        let ytdlp_path = ytdlp::ensure_ytdlp().await.map_err(|e| {
            anyhow!(
                "YouTube requer yt-dlp para funcionar. Falha ao obter yt-dlp: {}",
                e
            )
        })?;

        if Self::is_playlist_url(url) {
            let (playlist_title, entries) = ytdlp::get_playlist_info(&ytdlp_path, url, &[]).await?;

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

            return Ok(MediaInfo {
                title: sanitize_filename::sanitize(&playlist_title),
                author: playlist_title,
                platform: "youtube".to_string(),
                duration_seconds: None,
                thumbnail_url: None,
                available_qualities: qualities,
                media_type: MediaType::Playlist,
                file_size_bytes: None,
            });
        }

        let _video_id = Self::extract_video_id(url)
            .ok_or_else(|| anyhow!("Could not extract YouTube video ID"))?;

        let json = ytdlp::get_video_info(&ytdlp_path, url, &[]).await?;
        Self::parse_video_info(&json)
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<f64>,
    ) -> anyhow::Result<DownloadResult> {
        let _ = progress.send(0.0).await;

        let ytdlp_path = if let Some(ref p) = opts.ytdlp_path {
            p.clone()
        } else {
            ytdlp::ensure_ytdlp().await?
        };

        if info.media_type == MediaType::Playlist {
            return self
                .download_playlist(info, opts, progress, &ytdlp_path)
                .await;
        }

        let first = info
            .available_qualities
            .first()
            .ok_or_else(|| anyhow!("No quality available"))?;

        let quality_height = if let Some(ref wanted) = opts.quality {
            if wanted == "best" {
                None
            } else {
                Self::extract_quality_height(wanted)
            }
        } else {
            None
        };

        let selected = if let Some(ref wanted) = opts.quality {
            if wanted == "best" {
                first
            } else {
                info.available_qualities
                    .iter()
                    .find(|q| q.label == *wanted)
                    .unwrap_or(first)
            }
        } else {
            first
        };
        let video_url = &selected.url;

        ytdlp::download_video(
            &ytdlp_path,
            video_url,
            &opts.output_dir,
            quality_height,
            progress,
            opts.download_mode.as_deref(),
            opts.format_id.as_deref(),
            opts.filename_template.as_deref(),
            None,
            opts.cancel_token.clone(),
            None,
            opts.concurrent_fragments,
            opts.download_subtitles,
            &[],
        )
        .await
    }
}

impl YouTubeDownloader {
    async fn download_playlist(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<f64>,
        ytdlp_path: &std::path::Path,
    ) -> anyhow::Result<DownloadResult> {
        let playlist_dir = opts
            .output_dir
            .join(sanitize_filename::sanitize(&info.title));
        tokio::fs::create_dir_all(&playlist_dir).await?;

        let total = info.available_qualities.len();
        let mut total_bytes = 0u64;
        let mut last_path = playlist_dir.clone();

        for (i, entry) in info.available_qualities.iter().enumerate() {
            if opts.cancel_token.is_cancelled() {
                anyhow::bail!("Download cancelado");
            }

            let (video_tx, mut video_rx) = mpsc::channel::<f64>(16);
            let progress_tx = progress.clone();
            let video_idx = i;
            let video_total = total;
            let forwarder = tokio::spawn(async move {
                let mut max_pct = 0.0_f64;
                while let Some(pct) = video_rx.recv().await {
                    max_pct = max_pct.max(pct);
                    let overall = (video_idx as f64 / video_total as f64) * 100.0
                        + (max_pct / video_total as f64);
                    let _ = progress_tx.send(overall).await;
                }
            });

            match ytdlp::download_video(
                ytdlp_path,
                &entry.url,
                &playlist_dir,
                None,
                video_tx,
                opts.download_mode.as_deref(),
                None,
                opts.filename_template.as_deref(),
                None,
                opts.cancel_token.clone(),
                None,
                opts.concurrent_fragments,
                opts.download_subtitles,
                &[],
            )
            .await
            {
                Ok(result) => {
                    total_bytes += result.file_size_bytes;
                    last_path = result.file_path;
                }
                Err(e) => {
                    tracing::warn!("Playlist video {} falhou: {}", i + 1, e);
                }
            }

            let _ = forwarder.await;
        }

        let _ = progress.send(100.0).await;

        Ok(DownloadResult {
            file_path: last_path,
            file_size_bytes: total_bytes,
            duration_seconds: 0.0,
        })
    }
}
