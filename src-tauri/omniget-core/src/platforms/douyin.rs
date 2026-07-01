use crate::models::progress::ProgressUpdate;
use std::collections::HashSet;
use std::time::Duration;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::core::http_client;
use crate::core::ytdlp;
use crate::models::media::{
    DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality as MediaVideoQuality,
};
use crate::platforms::traits::PlatformDownloader;

const DOUYIN_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/139.0.0.0 Safari/537.36";
const DOUYIN_REFERER: &str = "https://www.douyin.com/?recommend=1";

pub struct DouyinDownloader;

impl Default for DouyinDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl DouyinDownloader {
    pub fn new() -> Self {
        Self
    }

    fn extra_flags() -> Vec<String> {
        vec![
            "--referer".to_string(),
            DOUYIN_REFERER.to_string(),
            "--user-agent".to_string(),
            DOUYIN_UA.to_string(),
        ]
    }

    fn host_matches(host: &str) -> bool {
        let host = host.to_lowercase();
        host == "douyin.com"
            || host.ends_with(".douyin.com")
            || host == "iesdouyin.com"
            || host.ends_with(".iesdouyin.com")
            || host == "amemv.com"
            || host.ends_with(".amemv.com")
    }

    fn is_short_link(url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let host = host.to_lowercase();
                return host == "v.douyin.com"
                    || host == "amemv.com"
                    || host.ends_with(".amemv.com")
                    || host == "iesdouyin.com"
                    || host.ends_with(".iesdouyin.com");
            }
        }
        false
    }

    fn canonicalize(url: &str) -> String {
        let Ok(parsed) = url::Url::parse(url) else {
            return url.to_string();
        };
        let host = parsed.host_str().unwrap_or("").to_lowercase();
        if !Self::host_matches(&host) {
            return url.to_string();
        }

        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
        let target = match segments.as_slice() {
            ["share", kind @ ("video" | "note" | "slides"), id, ..] => Some((*kind, *id)),
            [kind @ ("video" | "note"), id, ..] => Some((*kind, *id)),
            _ => None,
        };
        if let Some((kind, id)) = target {
            if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
                let kind = if kind == "slides" { "note" } else { kind };
                return format!("https://www.douyin.com/{}/{}", kind, id);
            }
        }

        if let Some(id) = parsed
            .query_pairs()
            .find(|(k, _)| k == "modal_id")
            .map(|(_, v)| v.to_string())
        {
            if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
                return format!("https://www.douyin.com/video/{}", id);
            }
        }

        url.to_string()
    }

    async fn resolve_url(url: &str) -> String {
        if !Self::is_short_link(url) {
            return Self::canonicalize(url);
        }
        let builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::limited(10))
            .user_agent(DOUYIN_UA);
        let client = match http_client::apply_global_proxy(builder).build() {
            Ok(c) => c,
            Err(_) => return Self::canonicalize(url),
        };
        match client.get(url).send().await {
            Ok(resp) => Self::canonicalize(resp.url().as_str()),
            Err(e) => {
                tracing::warn!("[douyin] short-link resolve failed: {}", e);
                Self::canonicalize(url)
            }
        }
    }

    fn build_qualities(formats: &[serde_json::Value], url: &str) -> Vec<MediaVideoQuality> {
        let mut seen_heights = HashSet::new();
        let mut qualities = Vec::new();

        for fmt in formats {
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

        qualities
    }
}

#[async_trait]
impl PlatformDownloader for DouyinDownloader {
    fn name(&self) -> &str {
        "douyin"
    }

    fn can_handle(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                return Self::host_matches(host);
            }
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let ytdlp_path = ytdlp::find_ytdlp_cached()
            .await
            .ok_or_else(|| anyhow!("yt-dlp not found"))?;

        let resolved = Self::resolve_url(url).await;
        let extra = Self::extra_flags();
        let json = ytdlp::get_video_info(&ytdlp_path, &resolved, &extra).await?;

        let title = json
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Douyin Video")
            .to_string();

        let author = json
            .get("uploader")
            .and_then(|v| v.as_str())
            .or_else(|| json.get("channel").and_then(|v| v.as_str()))
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

        let qualities = Self::build_qualities(&formats, &resolved);

        let has_video = formats
            .iter()
            .any(|f| f.get("vcodec").and_then(|v| v.as_str()).unwrap_or("none") != "none");

        Ok(MediaInfo {
            title,
            author,
            platform: "douyin".to_string(),
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
        progress: mpsc::Sender<ProgressUpdate>,
    ) -> anyhow::Result<DownloadResult> {
        let _ = progress.send(ProgressUpdate::percent(0.0)).await;

        let ytdlp_path = match &opts.ytdlp_path {
            Some(p) => p.clone(),
            None => ytdlp::find_ytdlp_cached()
                .await
                .ok_or_else(|| anyhow!("yt-dlp not found"))?,
        };

        let canonical = info
            .available_qualities
            .first()
            .map(|q| q.url.as_str())
            .unwrap_or("");
        if canonical.is_empty() {
            return Err(anyhow!("No URL available"));
        }
        let canonical = Self::resolve_url(canonical).await;

        let quality_height = opts
            .quality
            .as_ref()
            .and_then(|q| q.trim_end_matches('p').parse::<u32>().ok());

        let extra = Self::extra_flags();

        ytdlp::download_video(
            &ytdlp_path,
            &canonical,
            &opts.output_dir,
            quality_height,
            progress,
            opts.download_mode.as_deref(),
            opts.format_id.as_deref(),
            opts.filename_template.as_deref(),
            opts.referer.as_deref().or(Some(DOUYIN_REFERER)),
            opts.cancel_token.clone(),
            None,
            opts.concurrent_fragments,
            opts.download_subtitles,
            &extra,
            opts.audio_format.as_deref(),
        )
        .await
    }
}
