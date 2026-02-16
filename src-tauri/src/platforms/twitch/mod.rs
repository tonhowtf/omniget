use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::core::direct_downloader;
use crate::models::media::{
    DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality,
};
use crate::platforms::traits::PlatformDownloader;

const GQL_URL: &str = "https://gql.twitch.tv/gql";
const CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
const TOKEN_HASH: &str = "36b89d2507fce29e5ca551df756d27c1cfe079e2609642b4390aa4c35796eb11";

struct ClipMetadata {
    id: String,
    title: String,
    duration_seconds: f64,
    thumbnail_url: Option<String>,
    broadcaster_login: Option<String>,
    curator_login: Option<String>,
    video_qualities: Vec<ClipQuality>,
}

struct ClipQuality {
    quality: String,
    source_url: String,
}

struct AccessToken {
    signature: String,
    value: String,
}

pub struct TwitchClipsDownloader {
    client: reqwest::Client,
}

impl TwitchClipsDownloader {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();

        Self { client }
    }

    fn extract_clip_slug(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let host = parsed.host_str()?.to_lowercase();
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();

        if host == "clips.twitch.tv" || host.ends_with(".clips.twitch.tv") {
            return segments.first().map(|s| s.to_string());
        }

        if segments.len() >= 3 && segments.get(1) == Some(&"clip") {
            return segments.get(2).map(|s| s.to_string());
        }

        None
    }

    async fn fetch_clip_metadata(&self, slug: &str) -> anyhow::Result<ClipMetadata> {
        let query = format!(
            r#"{{ clip(slug: "{}") {{ broadcaster {{ login }} curator {{ login }} durationSeconds id medium: thumbnailURL(width: 480, height: 272) title videoQualities {{ quality sourceURL }} }} }}"#,
            slug
        );

        let body = serde_json::json!({ "query": query });

        let response = self
            .client
            .post(GQL_URL)
            .header("client-id", CLIENT_ID)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow!("Twitch GQL retornou HTTP {}", response.status()));
        }

        let json: serde_json::Value = response.json().await?;

        let clip = json
            .pointer("/data/clip")
            .ok_or_else(|| anyhow!("Clip não encontrado: {}", slug))?;

        if clip.is_null() {
            return Err(anyhow!("Clip não encontrado: {}", slug));
        }

        let id = clip.get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Clip sem ID"))?
            .to_string();

        let title = clip.get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled")
            .to_string();

        let duration_seconds = clip.get("durationSeconds")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let thumbnail_url = clip.get("medium")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let broadcaster_login = clip.pointer("/broadcaster/login")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let curator_login = clip.pointer("/curator/login")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let video_qualities = clip.get("videoQualities")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|q| {
                        let quality = q.get("quality")?.as_str()?.to_string();
                        let source_url = q.get("sourceURL")?.as_str()?.to_string();
                        Some(ClipQuality { quality, source_url })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(ClipMetadata {
            id,
            title,
            duration_seconds,
            thumbnail_url,
            broadcaster_login,
            curator_login,
            video_qualities,
        })
    }

    async fn fetch_access_token(&self, slug: &str) -> anyhow::Result<AccessToken> {
        let body = serde_json::json!([{
            "operationName": "VideoAccessToken_Clip",
            "variables": { "slug": slug },
            "extensions": {
                "persistedQuery": {
                    "version": 1,
                    "sha256Hash": TOKEN_HASH
                }
            }
        }]);

        let response = self
            .client
            .post(GQL_URL)
            .header("client-id", CLIENT_ID)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow!("Twitch GQL token retornou HTTP {}", response.status()));
        }

        let json: serde_json::Value = response.json().await?;

        let token_obj = json
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|r| r.pointer("/data/clip/playbackAccessToken"))
            .ok_or_else(|| anyhow!("Token de acesso não disponível para clip: {}", slug))?;

        let signature = token_obj.get("signature")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Token sem signature"))?
            .to_string();

        let value = token_obj.get("value")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Token sem value"))?
            .to_string();

        Ok(AccessToken { signature, value })
    }

    fn build_authenticated_url(source_url: &str, token: &AccessToken) -> String {
        format!(
            "{}?sig={}&token={}",
            source_url,
            urlencoding::encode(&token.signature),
            urlencoding::encode(&token.value),
        )
    }
}

#[async_trait]
impl PlatformDownloader for TwitchClipsDownloader {
    fn name(&self) -> &str {
        "twitch"
    }

    fn can_handle(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let host = host.to_lowercase();
                let is_twitch = host == "twitch.tv"
                    || host.ends_with(".twitch.tv");

                if !is_twitch {
                    return false;
                }

                return Self::extract_clip_slug(url).is_some();
            }
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let slug = Self::extract_clip_slug(url)
            .ok_or_else(|| anyhow!("Não foi possível extrair o slug do clip"))?;

        let clip = self.fetch_clip_metadata(&slug).await?;

        if clip.video_qualities.is_empty() {
            return Err(anyhow!("Nenhuma qualidade de vídeo disponível"));
        }

        let broadcaster = clip.broadcaster_login.as_deref()
            .ok_or_else(|| anyhow!("Dados do clip incompletos"))?;

        let token = self.fetch_access_token(&slug).await?;

        let clip_title = clip.title.trim().to_string();
        let author = match clip.curator_login.as_deref() {
            Some(curator) if !curator.is_empty() => {
                format!("{} - @{}, clipped by @{}", clip_title, broadcaster, curator)
            }
            _ => format!("{} - @{}", clip_title, broadcaster),
        };

        let available_qualities: Vec<VideoQuality> = clip
            .video_qualities
            .iter()
            .map(|q| {
                let height: u32 = q.quality.parse().unwrap_or(0);
                let authenticated_url = Self::build_authenticated_url(&q.source_url, &token);
                VideoQuality {
                    label: format!("{}p", q.quality),
                    width: 0,
                    height,
                    url: authenticated_url,
                    format: "mp4".to_string(),
                }
            })
            .collect();

        Ok(MediaInfo {
            title: format!("twitchclip_{}", clip.id),
            author,
            platform: "twitch".to_string(),
            duration_seconds: Some(clip.duration_seconds),
            thumbnail_url: clip.thumbnail_url,
            available_qualities,
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
            .ok_or_else(|| anyhow!("Nenhum URL de mídia disponível"))?;

        let selected = if let Some(ref wanted) = opts.quality {
            info.available_qualities
                .iter()
                .find(|q| q.label == *wanted)
                .unwrap_or(first)
        } else {
            first
        };

        let filename = format!(
            "{}_{}.mp4",
            sanitize_filename::sanitize(&info.title),
            selected.label
        );
        let output_path = opts.output_dir.join(&filename);

        let total_bytes = direct_downloader::download_direct(
            &self.client,
            &selected.url,
            &output_path,
            progress,
            None,
        )
        .await?;

        Ok(DownloadResult {
            file_path: output_path,
            file_size_bytes: total_bytes,
            duration_seconds: info.duration_seconds.unwrap_or(0.0),
        })
    }
}
