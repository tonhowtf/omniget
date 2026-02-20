use std::collections::HashSet;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::core::ytdlp;
use crate::models::media::{
    DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality as MediaVideoQuality,
};
use crate::platforms::traits::PlatformDownloader;

pub struct VimeoDownloader;

impl VimeoDownloader {
    pub fn new() -> Self {
        Self
    }

    fn extract_quality_height(quality_str: &str) -> Option<u32> {
        let s = quality_str.trim().to_lowercase();
        if s == "best" || s == "highest" {
            return None;
        }
        s.trim_end_matches('p').parse::<u32>().ok()
    }

    fn parse_video_info(json: &serde_json::Value) -> anyhow::Result<MediaInfo> {
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

        let webpage_url = json
            .get("webpage_url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let mut qualities: Vec<MediaVideoQuality> = Vec::new();
        let mut seen_heights: HashSet<u32> = HashSet::new();

        if let Some(formats) = json.get("formats").and_then(|v| v.as_array()) {
            for f in formats {
                let height = f.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let width = f.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let vcodec = f
                    .get("vcodec")
                    .and_then(|v| v.as_str())
                    .unwrap_or("none");

                if vcodec == "none" || height == 0 {
                    continue;
                }

                if seen_heights.insert(height) {
                    qualities.push(MediaVideoQuality {
                        label: format!("{}p", height),
                        width,
                        height,
                        url: webpage_url.clone(),
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
                url: webpage_url,
                format: "ytdlp".to_string(),
            });
        }

        Ok(MediaInfo {
            title,
            author,
            platform: "vimeo".to_string(),
            duration_seconds: duration,
            thumbnail_url: thumbnail,
            available_qualities: qualities,
            media_type: MediaType::Video,
            file_size_bytes: None,
        })
    }
}

#[async_trait]
impl PlatformDownloader for VimeoDownloader {
    fn name(&self) -> &str {
        "vimeo"
    }

    fn can_handle(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let host = host.to_lowercase();
                return host == "vimeo.com"
                    || host.ends_with(".vimeo.com");
            }
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let ytdlp_path = ytdlp::ensure_ytdlp().await.map_err(|e| {
            anyhow!(
                "Vimeo requer yt-dlp para funcionar. Falha ao obter yt-dlp: {}",
                e
            )
        })?;

        let json = ytdlp::get_video_info(&ytdlp_path, url).await?;
        Self::parse_video_info(&json)
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<f64>,
    ) -> anyhow::Result<DownloadResult> {
        let _ = progress.send(0.0).await;

        let ytdlp_path = ytdlp::ensure_ytdlp().await?;

        let first = info
            .available_qualities
            .first()
            .ok_or_else(|| anyhow!("Nenhuma qualidade disponível"))?;

        let selected = if let Some(ref wanted) = opts.quality {
            info.available_qualities
                .iter()
                .find(|q| q.label == *wanted)
                .unwrap_or(first)
        } else {
            first
        };

        let quality_height = Self::extract_quality_height(&selected.label);
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
            opts.referer.as_deref(),
            opts.cancel_token.clone(),
            None,
            opts.concurrent_fragments,
            false,
        )
        .await
    }
}
