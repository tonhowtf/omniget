use std::sync::Arc;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::{mpsc, Mutex};
use tokio_util::sync::CancellationToken;

use crate::core::direct_downloader;
use crate::core::hls_downloader::HlsDownloader;
use crate::models::media::{
    DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality,
};
use crate::platforms::traits::PlatformDownloader;

const VIMEO_OAUTH_URL: &str = "https://api.vimeo.com/oauth/authorize/client";
const VIMEO_API_BASE: &str = "https://api.vimeo.com/videos";
const VIMEO_AUTH_BASIC: &str = "Basic NzRmYTg5YjgxMWExY2JiNzUwZDg1MjhkMTYzZjQ4YWYyOGEyZGJlMTp4OGx2NFd3QnNvY1lkamI2UVZsdjdDYlNwSDUrdm50YzdNNThvWDcwN1JrenJGZC9tR1lReUNlRjRSVklZeWhYZVpRS0tBcU9YYzRoTGY2Z1dlVkJFYkdJc0dMRHpoZWFZbU0reDRqZ1dkZ1diZmdIdGUrNUM5RVBySlM0VG1qcw==";
const VIMEO_USER_AGENT: &str = "com.vimeo.android.videoapp (Google, Pixel 7a, google, Android 16/36 Version 11.8.1) Kotlin VimeoNetworking/3.12.0";
const VIMEO_ACCEPT: &str = "application/vnd.vimeo.*+json; version=3.4.10";

pub struct VimeoDownloader {
    client: reqwest::Client,
    bearer: Arc<Mutex<Option<String>>>,
}

impl VimeoDownloader {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(VIMEO_USER_AGENT)
            .build()
            .unwrap_or_default();
        Self {
            client,
            bearer: Arc::new(Mutex::new(None)),
        }
    }

    fn extract_video_id_and_password(url: &str) -> Option<(String, Option<String>)> {
        let parsed = url::Url::parse(url).ok()?;
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();

        let (id, password_from_path) = if segments.first() == Some(&"video") {
            let id = segments.get(1)?;
            let pw = parsed
                .query_pairs()
                .find(|(k, _)| k == "h")
                .map(|(_, v)| v.to_string());
            (id.to_string(), pw)
        } else {
            let id = segments.first()?;
            if !id.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let pw = segments
                .get(1)
                .filter(|s| !s.is_empty() && !s.chars().all(|c| c.is_ascii_digit()))
                .map(|s| s.to_string());
            (id.to_string(), pw)
        };

        if !id.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }

        Some((id, password_from_path))
    }

    async fn get_bearer(&self) -> anyhow::Result<String> {
        {
            let cached = self.bearer.lock().await;
            if let Some(ref token) = *cached {
                return Ok(token.clone());
            }
        }

        let token = self.fetch_new_bearer().await?;
        let mut cached = self.bearer.lock().await;
        *cached = Some(token.clone());
        Ok(token)
    }

    async fn refresh_bearer(&self) -> anyhow::Result<String> {
        {
            let mut cached = self.bearer.lock().await;
            *cached = None;
        }
        self.get_bearer().await
    }

    async fn fetch_new_bearer(&self) -> anyhow::Result<String> {
        let response = self
            .client
            .post(VIMEO_OAUTH_URL)
            .header("Authorization", VIMEO_AUTH_BASIC)
            .header("Accept", VIMEO_ACCEPT)
            .header("Accept-Language", "en")
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body("scope=private public create edit delete interact upload purchased stats&grant_type=client_credentials")
            .send()
            .await?;

        let json: serde_json::Value = response.json().await?;

        json.get("access_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| anyhow!("Falha ao obter token Vimeo"))
    }

    async fn fetch_video_info(
        &self,
        video_id: &str,
        password: Option<&str>,
    ) -> anyhow::Result<serde_json::Value> {
        let bearer = self.get_bearer().await?;
        let api_id = match password {
            Some(pw) => format!("{}:{}", video_id, pw),
            None => video_id.to_string(),
        };

        let url = format!("{}/{}", VIMEO_API_BASE, api_id);
        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", bearer))
            .header("Accept", VIMEO_ACCEPT)
            .header("Accept-Language", "en")
            .send()
            .await?;

        let json: serde_json::Value = response.json().await?;

        if json.get("error_code").and_then(|v| v.as_u64()) == Some(8003) {
            let new_bearer = self.refresh_bearer().await?;
            let response = self
                .client
                .get(&url)
                .header("Authorization", format!("Bearer {}", new_bearer))
                .header("Accept", VIMEO_ACCEPT)
                .header("Accept-Language", "en")
                .send()
                .await?;
            return response.json().await.map_err(Into::into);
        }

        if json.get("error_code").is_some() || json.get("error").is_some() {
            return Err(anyhow!("Vídeo Vimeo não encontrado"));
        }

        Ok(json)
    }

    fn extract_progressive_files(info: &serde_json::Value) -> Vec<VideoQuality> {
        let files = match info.get("files").and_then(|f| f.as_array()) {
            Some(f) => f,
            None => return Vec::new(),
        };

        files
            .iter()
            .filter_map(|f| {
                let rendition = f.get("rendition")?.as_str()?;
                if !rendition.ends_with('p') {
                    return None;
                }
                let link = f.get("link")?.as_str()?;
                let width = f.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let height = f.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

                Some(VideoQuality {
                    label: rendition.to_string(),
                    width,
                    height,
                    url: link.to_string(),
                    format: "mp4".to_string(),
                })
            })
            .collect()
    }

    async fn extract_hls_url(&self, info: &serde_json::Value) -> Option<String> {
        let config_url = info.get("config_url")?.as_str()?;

        let response = self.client.get(config_url).send().await.ok()?;
        let config: serde_json::Value = response.json().await.ok()?;

        config
            .pointer("/request/files/hls/cdns/akfire_interconnect_quic/url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
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
        let (video_id, password) = Self::extract_video_id_and_password(url)
            .ok_or_else(|| anyhow!("Não foi possível extrair o ID do vídeo Vimeo"))?;

        let info = self
            .fetch_video_info(&video_id, password.as_deref())
            .await?;

        let title = info
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("vimeo_video")
            .to_string();

        let author = info
            .pointer("/user/name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let duration = info.get("duration").and_then(|v| v.as_f64());

        let mut qualities = Self::extract_progressive_files(&info);

        if qualities.is_empty() {
            if let Some(hls_url) = self.extract_hls_url(&info).await {
                qualities.push(VideoQuality {
                    label: "best".to_string(),
                    width: 0,
                    height: 0,
                    url: hls_url,
                    format: "hls".to_string(),
                });
            }
        }

        if qualities.is_empty() {
            return Err(anyhow!("Nenhum link de download disponível"));
        }

        qualities.sort_by(|a, b| b.height.cmp(&a.height));

        Ok(MediaInfo {
            title: format!("vimeo_{}", video_id),
            author: format!("{} - {}", title, author),
            platform: "vimeo".to_string(),
            duration_seconds: duration,
            thumbnail_url: None,
            available_qualities: qualities,
            media_type: MediaType::Video,
        })
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<f64>,
    ) -> anyhow::Result<DownloadResult> {
        let first = info
            .available_qualities
            .first()
            .ok_or_else(|| anyhow!("Nenhum URL disponível"))?;

        let selected = if let Some(ref wanted) = opts.quality {
            info.available_qualities
                .iter()
                .find(|q| q.label == *wanted)
                .unwrap_or(first)
        } else {
            first
        };

        if selected.format == "hls" {
            let filename = format!("{}.mp4", sanitize_filename::sanitize(&info.title));
            let output_path = opts.output_dir.join(&filename);
            let output_str = output_path.to_string_lossy().to_string();

            let downloader = HlsDownloader::new();
            let cancel = CancellationToken::new();
            let _ = progress.send(0.0).await;

            let result = downloader
                .download(
                    &selected.url,
                    &output_str,
                    "https://vimeo.com",
                    None,
                    cancel,
                    20,
                    3,
                )
                .await?;

            let _ = progress.send(100.0).await;

            Ok(DownloadResult {
                file_path: result.path,
                file_size_bytes: result.file_size,
                duration_seconds: info.duration_seconds.unwrap_or(0.0),
            })
        } else {
            let filename = format!(
                "{}_{}.mp4",
                sanitize_filename::sanitize(&info.title),
                selected.label
            );
            let output = opts.output_dir.join(&filename);

            let bytes = direct_downloader::download_direct(
                &self.client,
                &selected.url,
                &output,
                progress,
            )
            .await?;

            Ok(DownloadResult {
                file_path: output,
                file_size_bytes: bytes,
                duration_seconds: info.duration_seconds.unwrap_or(0.0),
            })
        }
    }
}
