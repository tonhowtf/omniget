use anyhow::anyhow;
use async_trait::async_trait;
use regex::Regex;
use tokio::sync::mpsc;

use crate::core::direct_downloader;
use crate::core::redirect;
use crate::models::media::{
    DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality,
};
use crate::platforms::traits::PlatformDownloader;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub struct PinterestDownloader {
    client: reqwest::Client,
}

impl Default for PinterestDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl PinterestDownloader {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();

        Self { client }
    }

    fn extract_pin_id(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();

        if segments.first() == Some(&"pin") {
            let raw_id = segments.get(1)?;
            if raw_id.contains("--") {
                return raw_id.split("--").last().map(|s| s.to_string());
            }
            return Some(raw_id.to_string());
        }

        if segments.first() == Some(&"url_shortener") {
            return None;
        }

        segments.last().map(|s| {
            if s.contains("--") {
                s.split("--").last().unwrap_or(s).to_string()
            } else {
                s.to_string()
            }
        })
    }

    fn is_short_link(url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                return host == "pin.it";
            }
        }
        false
    }

    async fn resolve_pin_url(&self, url: &str) -> anyhow::Result<String> {
        if Self::is_short_link(url) {
            let canonical = redirect::resolve_redirect(&self.client, url).await?;
            return Ok(canonical);
        }
        Ok(url.to_string())
    }

    async fn fetch_pin_html(&self, pin_id: &str) -> anyhow::Result<String> {
        let url = format!("https://www.pinterest.com/pin/{}/", pin_id);

        let response = self
            .client
            .get(&url)
            .header("Accept", "text/html")
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow!("HTTP {} ao acessar pin {}", response.status(), pin_id));
        }

        response.text().await.map_err(Into::into)
    }

    fn check_pin_not_found(html: &str) -> bool {
        let re = Regex::new(r#""__typename"\s*:\s*"PinNotFound""#).unwrap();
        re.is_match(html)
    }

    fn extract_video_url(html: &str) -> Option<String> {
        let re = Regex::new(r#""url":"(https://v1\.pinimg\.com/videos/.*?)""#).unwrap();
        let result = re
            .captures_iter(html)
            .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
            .find(|url| url.ends_with(".mp4"));
        result
    }

    fn extract_image_url(html: &str) -> Option<(String, bool)> {
        let re = Regex::new(r#"src="(https://i\.pinimg\.com/.*?\.(jpg|gif))""#).unwrap();
        let mut best: Option<(String, bool)> = None;

        for cap in re.captures_iter(html) {
            if let Some(url_match) = cap.get(1) {
                let url = url_match.as_str().to_string();
                let is_gif = url.ends_with(".gif");
                if best.is_none() || url.contains("originals") || url.contains("1200x") {
                    best = Some((url, is_gif));
                }
            }
        }

        best
    }
}

#[async_trait]
impl PlatformDownloader for PinterestDownloader {
    fn name(&self) -> &str {
        "pinterest"
    }

    fn can_handle(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let host = host.to_lowercase();
                return host == "pin.it"
                    || host.contains("pinterest.com")
                    || host.contains("pinterest.ca")
                    || host.contains("pinterest.co.uk")
                    || host.contains("pinterest.fr")
                    || host.contains("pinterest.de")
                    || host.contains("pinterest.es")
                    || host.contains("pinterest.it")
                    || host.contains("pinterest.pt")
                    || host.contains("pinterest.jp")
                    || host.contains("pinterest.kr")
                    || host.contains("pinterest.com.br")
                    || host.contains("pinterest.com.mx")
                    || host.contains("pinterest.co.kr")
                    || host.contains("pinterest.cl")
                    || host.contains("pinterest.at")
                    || host.contains("pinterest.ch")
                    || host.contains("pinterest.com.au")
                    || host.contains("pinterest.co.in")
                    || host.contains("pinterest.nz")
                    || host.contains("pinterest.ph")
                    || host.contains("pinterest.ru")
                    || host.contains("pinterest.se")
                    || host.contains("pinterest.dk")
                    || host.contains("pinterest.");
            }
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let canonical = self.resolve_pin_url(url).await?;

        let pin_id = Self::extract_pin_id(&canonical)
            .ok_or_else(|| anyhow!("Could not extract pin ID"))?;

        let html = self.fetch_pin_html(&pin_id).await?;

        if Self::check_pin_not_found(&html) {
            return Err(anyhow!("Pin not found"));
        }

        if let Some(video_url) = Self::extract_video_url(&html) {
            return Ok(MediaInfo {
                title: format!("pinterest_{}", pin_id),
                author: String::new(),
                platform: "pinterest".to_string(),
                duration_seconds: None,
                thumbnail_url: None,
                available_qualities: vec![VideoQuality {
                    label: "original".to_string(),
                    width: 0,
                    height: 0,
                    url: video_url,
                    format: "mp4".to_string(),
                }],
                media_type: MediaType::Video,
                file_size_bytes: None,
            });
        }

        if let Some((image_url, is_gif)) = Self::extract_image_url(&html) {
            let media_type = if is_gif { MediaType::Gif } else { MediaType::Photo };
            let format = if is_gif { "gif" } else { "jpg" };

            return Ok(MediaInfo {
                title: format!("pinterest_{}", pin_id),
                author: String::new(),
                platform: "pinterest".to_string(),
                duration_seconds: None,
                thumbnail_url: None,
                available_qualities: vec![VideoQuality {
                    label: "original".to_string(),
                    width: 0,
                    height: 0,
                    url: image_url,
                    format: format.to_string(),
                }],
                media_type,
                file_size_bytes: None,
            });
        }

        Err(anyhow!("No media found in pin {}", pin_id))
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<f64>,
    ) -> anyhow::Result<DownloadResult> {
        let quality = info
            .available_qualities
            .first()
            .ok_or_else(|| anyhow!("No media URL available"))?;

        let extension = &quality.format;
        let filename = format!("{}.{}", info.title, extension);
        let safe_filename = sanitize_filename::sanitize(&filename);
        let output_path = opts.output_dir.join(&safe_filename);

        let total_bytes = direct_downloader::download_direct(
            &self.client,
            &quality.url,
            &output_path,
            progress,
            None,
        )
        .await?;

        Ok(DownloadResult {
            file_path: output_path,
            file_size_bytes: total_bytes,
            duration_seconds: 0.0,
        })
    }
}
