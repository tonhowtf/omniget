use anyhow::anyhow;
use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::core::direct_downloader;
use crate::models::media::{
    DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality,
};
use crate::platforms::traits::PlatformDownloader;

const GQL_URL: &str = "https://gql.twitch.tv/gql";
const CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
const TOKEN_HASH: &str = "36b89d2507fce29e5ca551df756d27c1cfe079e2609642b4390aa4c35796eb11";

#[derive(Deserialize)]
struct GqlMetadataResponse {
    data: GqlMetadataData,
}

#[derive(Deserialize)]
struct GqlMetadataData {
    clip: Option<ClipData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClipData {
    id: String,
    title: String,
    duration_seconds: f64,
    #[serde(rename = "medium")]
    thumbnail_url: Option<String>,
    broadcaster: Option<Broadcaster>,
    curator: Option<Curator>,
    video_qualities: Option<Vec<TwitchVideoQuality>>,
}

#[derive(Deserialize)]
struct Broadcaster {
    login: String,
}

#[derive(Deserialize)]
struct Curator {
    login: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TwitchVideoQuality {
    quality: String,
    source_url: String,
}

#[derive(Deserialize)]
struct GqlTokenResponse {
    data: GqlTokenData,
}

#[derive(Deserialize)]
struct GqlTokenData {
    clip: Option<TokenClipData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TokenClipData {
    playback_access_token: Option<PlaybackAccessToken>,
}

#[derive(Deserialize)]
struct PlaybackAccessToken {
    signature: String,
    value: String,
}

pub struct TwitchClipsDownloader {
    client: reqwest::Client,
}

impl TwitchClipsDownloader {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
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

    async fn fetch_clip_metadata(&self, slug: &str) -> anyhow::Result<ClipData> {
        let query = format!(
            r#"{{ clip(slug: "{}") {{ broadcaster {{ login }} curator {{ login }} createdAt durationSeconds id medium: thumbnailURL(width: 480, height: 272) title videoQualities {{ quality sourceURL }} }} }}"#,
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

        let gql: GqlMetadataResponse = response.json().await?;

        gql.data
            .clip
            .ok_or_else(|| anyhow!("Clip não encontrado: {}", slug))
    }

    async fn fetch_access_token(&self, slug: &str) -> anyhow::Result<PlaybackAccessToken> {
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

        let gql: Vec<GqlTokenResponse> = response.json().await?;

        gql.into_iter()
            .next()
            .and_then(|r| r.data.clip)
            .and_then(|c| c.playback_access_token)
            .ok_or_else(|| anyhow!("Token de acesso não disponível para clip: {}", slug))
    }

    fn build_authenticated_url(source_url: &str, token: &PlaybackAccessToken) -> String {
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

        let raw_qualities = clip
            .video_qualities
            .as_ref()
            .ok_or_else(|| anyhow!("Nenhuma qualidade de vídeo disponível"))?;

        if clip.broadcaster.is_none() {
            return Err(anyhow!("Dados do clip incompletos"));
        }

        let token = self.fetch_access_token(&slug).await?;

        let broadcaster = clip.broadcaster.as_ref().unwrap();
        let curator_name = clip
            .curator
            .as_ref()
            .map(|c| c.login.clone())
            .unwrap_or_default();

        let clip_title = clip.title.trim().to_string();
        let author = if curator_name.is_empty() {
            format!("{} - @{}", clip_title, broadcaster.login)
        } else {
            format!("{} - @{}, clipped by @{}", clip_title, broadcaster.login, curator_name)
        };

        let available_qualities: Vec<VideoQuality> = raw_qualities
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
        )
        .await?;

        Ok(DownloadResult {
            file_path: output_path,
            file_size_bytes: total_bytes,
            duration_seconds: info.duration_seconds.unwrap_or(0.0),
        })
    }
}
