use std::collections::HashSet;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::core::ytdlp;
use crate::models::media::{
    DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality as MediaVideoQuality,
};
use crate::platforms::traits::PlatformDownloader;

pub struct BilibiliDownloader;

impl Default for BilibiliDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl BilibiliDownloader {
    pub fn new() -> Self {
        Self
    }

    fn is_playlist_or_series(url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            let path = parsed.path();
            if path.starts_with("/bangumi/") || path.starts_with("/cheese/") {
                return true;
            }
            if path.contains("/channel/") || path.contains("/favlist/") || path.contains("/medialist/") {
                return true;
            }
        }
        false
    }

    fn bilibili_extra_flags() -> Vec<String> {
        vec![
            "--referer".to_string(),
            "https://www.bilibili.com".to_string(),
        ]
    }
}

#[async_trait]
impl PlatformDownloader for BilibiliDownloader {
    fn name(&self) -> &str {
        "bilibili"
    }

    fn can_handle(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let host = host.to_lowercase();
                return host.contains("bilibili.com") || host.contains("bilibili.tv") || host == "b23.tv";
            }
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let ytdlp_path = ytdlp::find_ytdlp_cached()
            .await
            .ok_or_else(|| anyhow!("yt-dlp not found"))?;

        let extra = Self::bilibili_extra_flags();

        if Self::is_playlist_or_series(url) {
            let (title, entries) = ytdlp::get_playlist_info(&ytdlp_path, url, &extra).await?;

            if entries.is_empty() {
                return Err(anyhow!("Playlist empty or unavailable"));
            }

            let qualities: Vec<MediaVideoQuality> = entries
                .iter()
                .enumerate()
                .map(|(i, e)| MediaVideoQuality {
                    label: format!("{}. {}", i + 1, e.title),
                    width: 0,
                    height: 0,
                    url: e.url.clone(),
                    format: "mp4".to_string(),
                })
                .collect();

            return Ok(MediaInfo {
                title,
                author: "Bilibili".to_string(),
                platform: "bilibili".to_string(),
                duration_seconds: None,
                thumbnail_url: None,
                available_qualities: qualities,
                media_type: MediaType::Playlist,
                file_size_bytes: None,
            });
        }

        let json = ytdlp::get_video_info(&ytdlp_path, url, &extra).await?;

        let title = json
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Bilibili Video")
            .to_string();

        let author = json
            .get("uploader")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();

        let duration = json.get("duration").and_then(|v| v.as_f64());

        let thumbnail = json
            .get("thumbnail")
            .and_then(|v| v.as_str())
            .map(String::from);

        let formats = json
            .get("formats")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut seen_heights = HashSet::new();
        let mut qualities = Vec::new();

        for fmt in &formats {
            let height = fmt.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let vcodec = fmt.get("vcodec").and_then(|v| v.as_str()).unwrap_or("none");

            if height == 0 || vcodec == "none" {
                continue;
            }
            if !seen_heights.insert(height) {
                continue;
            }

            let label = match height {
                2160 => "4K".to_string(),
                1440 => "2K".to_string(),
                1080 => "1080p".to_string(),
                720 => "720p".to_string(),
                480 => "480p".to_string(),
                360 => "360p".to_string(),
                _ => format!("{}p", height),
            };

            qualities.push(MediaVideoQuality {
                label,
                width: fmt.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                height,
                url: url.to_string(),
                format: "mp4".to_string(),
            });
        }

        qualities.sort_by(|a, b| b.height.cmp(&a.height));

        if qualities.is_empty() {
            qualities.push(MediaVideoQuality {
                label: "best".to_string(),
                width: 0,
                height: 0,
                url: url.to_string(),
                format: "mp4".to_string(),
            });
        }

        let has_video = formats
            .iter()
            .any(|f| f.get("vcodec").and_then(|v| v.as_str()).unwrap_or("none") != "none");

        Ok(MediaInfo {
            title,
            author,
            platform: "bilibili".to_string(),
            duration_seconds: duration,
            thumbnail_url: thumbnail,
            available_qualities: qualities,
            media_type: if has_video {
                MediaType::Video
            } else {
                MediaType::Audio
            },
            file_size_bytes: None,
        })
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<f64>,
    ) -> anyhow::Result<DownloadResult> {
        let _ = progress.send(0.0).await;

        let ytdlp_path = match &opts.ytdlp_path {
            Some(p) => p.clone(),
            None => ytdlp::find_ytdlp_cached()
                .await
                .ok_or_else(|| anyhow!("yt-dlp not found"))?,
        };

        let url = info
            .available_qualities
            .first()
            .map(|q| q.url.as_str())
            .unwrap_or("");

        if url.is_empty() {
            return Err(anyhow!("No URL available"));
        }

        if info.media_type == MediaType::Playlist {
            return self
                .download_playlist(info, opts, progress, &ytdlp_path)
                .await;
        }

        let quality_height = opts
            .quality
            .as_ref()
            .and_then(|q| {
                q.trim_end_matches('p')
                    .parse::<u32>()
                    .ok()
            });

        let mut extra = Self::bilibili_extra_flags();
        extra.push("--no-playlist".to_string());

        ytdlp::download_video(
            &ytdlp_path,
            url,
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
            &extra,
        )
        .await
    }
}

impl BilibiliDownloader {
    async fn download_playlist(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<f64>,
        ytdlp_path: &std::path::Path,
    ) -> anyhow::Result<DownloadResult> {
        let total = info.available_qualities.len().max(1);
        let mut last_result = DownloadResult {
            file_path: opts.output_dir.clone(),
            file_size_bytes: 0,
            duration_seconds: 0.0,
            torrent_id: None,
        };

        for (i, quality) in info.available_qualities.iter().enumerate() {
            if opts.cancel_token.is_cancelled() {
                return Err(anyhow!("Download cancelled"));
            }

            let (entry_tx, mut entry_rx) = mpsc::channel::<f64>(16);
            let progress_clone = progress.clone();
            let total_f = total as f64;
            let idx = i as f64;

            tokio::spawn(async move {
                while let Some(p) = entry_rx.recv().await {
                    let overall = (idx + p / 100.0) / total_f * 100.0;
                    let _ = progress_clone.send(overall).await;
                }
            });

            let mut extra = Self::bilibili_extra_flags();
            extra.push("--no-playlist".to_string());

            match ytdlp::download_video(
                ytdlp_path,
                &quality.url,
                &opts.output_dir,
                None,
                entry_tx,
                opts.download_mode.as_deref(),
                None,
                opts.filename_template.as_deref(),
                None,
                opts.cancel_token.clone(),
                None,
                opts.concurrent_fragments,
                opts.download_subtitles,
                &extra,
            )
            .await
            {
                Ok(result) => {
                    last_result.file_size_bytes += result.file_size_bytes;
                    last_result.duration_seconds += result.duration_seconds;
                    last_result.file_path = result.file_path;
                }
                Err(e) => {
                    tracing::error!("[bilibili] playlist item {} failed: {}", i + 1, e);
                }
            }
        }

        let _ = progress.send(100.0).await;
        Ok(last_result)
    }
}
