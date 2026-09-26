use crate::models::progress::ProgressUpdate;
use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::core::direct_downloader;
use crate::models::media::{DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality};
use crate::platforms::traits::PlatformDownloader;

const GQL_URL: &str = "https://gql.twitch.tv/gql";
const CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
const TOKEN_HASH: &str = "36b89d2507fce29e5ca551df756d27c1cfe079e2609642b4390aa4c35796eb11";

struct ClipMetadata {
    title: String,
    duration_seconds: f64,
    thumbnail_url: Option<String>,
    broadcaster_login: Option<String>,
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

impl Default for TwitchClipsDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl TwitchClipsDownloader {
    async fn fallback_ytdlp(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let ytdlp_path = crate::core::ytdlp::ensure_ytdlp().await?;
        let json = crate::core::ytdlp::get_video_info(&ytdlp_path, url, &[]).await?;
        crate::platforms::generic_ytdlp::GenericYtdlpDownloader::parse_video_info(&json)
    }

    async fn native_get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let slug =
            Self::extract_clip_slug(url).ok_or_else(|| anyhow!("Could not extract clip slug"))?;

        let clip = self.fetch_clip_metadata(&slug).await?;

        if clip.video_qualities.is_empty() {
            return Err(anyhow!("No video quality available"));
        }

        let broadcaster = clip
            .broadcaster_login
            .as_deref()
            .ok_or_else(|| anyhow!("Dados do clip incompletos"))?;

        let token = self.fetch_access_token(&slug).await?;

        let clip_title = clip.title.trim().to_string();

        let mut available_qualities: Vec<VideoQuality> = clip
            .video_qualities
            .iter()
            .map(|q| {
                let height = clip_height(&q.quality, &q.source_url);
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
        // Unsized renditions cannot prove a height ceiling; the MCP worker
        // then takes this yt-dlp alternative on the page URL instead.
        if available_qualities.iter().any(|q| q.height == 0) {
            available_qualities.push(ytdlp_alternative(url));
        }

        Ok(MediaInfo {
            title: sanitize_filename::sanitize(&clip_title),
            author: broadcaster.to_string(),
            platform: "twitch".to_string(),
            duration_seconds: Some(clip.duration_seconds),
            thumbnail_url: clip.thumbnail_url,
            available_qualities,
            media_type: MediaType::Video,
            file_size_bytes: None,
        })
    }

    pub fn new() -> Self {
        let client = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
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
            .ok_or_else(|| anyhow!("Clip not found: {}", slug))?;

        if clip.is_null() {
            return Err(anyhow!("Clip not found: {}", slug));
        }

        let title = clip
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled")
            .to_string();

        let duration_seconds = clip
            .get("durationSeconds")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let thumbnail_url = clip
            .get("medium")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let broadcaster_login = clip
            .pointer("/broadcaster/login")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let video_qualities = clip
            .get("videoQualities")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|q| {
                        let quality = q.get("quality")?.as_str()?.to_string();
                        let source_url = q.get("sourceURL")?.as_str()?.to_string();
                        Some(ClipQuality {
                            quality,
                            source_url,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(ClipMetadata {
            title,
            duration_seconds,
            thumbnail_url,
            broadcaster_login,
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
            return Err(anyhow!(
                "Twitch GQL token retornou HTTP {}",
                response.status()
            ));
        }

        let json: serde_json::Value = response.json().await?;

        let token_obj = json
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|r| r.pointer("/data/clip/playbackAccessToken"))
            .ok_or_else(|| anyhow!("Access token not available for clip: {}", slug))?;

        let signature = token_obj
            .get("signature")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Token sem signature"))?
            .to_string();

        let value = token_obj
            .get("value")
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
                let is_twitch = host == "twitch.tv" || host.ends_with(".twitch.tv");

                if !is_twitch {
                    return false;
                }

                return Self::extract_clip_slug(url).is_some();
            }
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        match self.native_get_media_info(url).await {
            Ok(info) => Ok(info),
            Err(native_err) => {
                tracing::warn!(
                    "[twitch] native failed: {}, trying yt-dlp fallback",
                    native_err
                );
                self.fallback_ytdlp(url).await.map_err(|_| native_err)
            }
        }
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<ProgressUpdate>,
    ) -> anyhow::Result<DownloadResult> {
        if let Some(quality) = info.available_qualities.first() {
            if quality.format == "ytdlp" {
                let ytdlp_path = crate::core::ytdlp::ensure_ytdlp().await?;
                return crate::core::ytdlp::download_video(
                    &ytdlp_path,
                    &quality.url,
                    &opts.output_dir,
                    None,
                    progress,
                    opts.download_mode.as_deref(),
                    opts.format_id.as_deref(),
                    opts.filename_template.as_deref(),
                    opts.referer.as_deref().or(Some("https://www.twitch.tv/")),
                    opts.cancel_token.clone(),
                    None,
                    opts.concurrent_fragments,
                    false,
                    &[],
                    opts.audio_format.as_deref(),
                )
                .await;
            }
        }

        let first = info
            .available_qualities
            .first()
            .ok_or_else(|| anyhow!("No media URL available"))?;

        let selected =
            select_quality(&info.available_qualities, opts.quality.as_deref()).unwrap_or(first);

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
            Some(&opts.cancel_token),
        )
        .await?;

        Ok(DownloadResult {
            file_path: output_path,
            file_size_bytes: total_bytes,
            duration_seconds: info.duration_seconds.unwrap_or(0.0),
            torrent_id: None,
        })
    }
}

fn ytdlp_alternative(page_url: &str) -> VideoQuality {
    VideoQuality {
        label: "auto".to_string(),
        width: 0,
        height: 0,
        url: page_url.to_string(),
        format: "ytdlp".to_string(),
    }
}

/// `videoQualities.quality` is the frame height of landscape renditions
/// (clip GQL 26/09: 1080/720/480/360 under `/landscape/avc/<q>/`). Portrait
/// renditions name the short side, so their height is left unknown.
fn clip_height(quality: &str, source_url: &str) -> u32 {
    if source_url.contains("/portrait/") {
        return 0;
    }
    quality.parse().unwrap_or(0)
}

/// The requested quality is a height ceiling (`"720"` from the MCP worker, or
/// a `"720p"` label): the tallest variant of known height within it, else an
/// exact label match. The first variant is 1080 on clips, so matching labels
/// alone ignored a `"720"` ceiling (round-1 regression twitch-1).
fn select_quality<'a>(
    qualities: &'a [VideoQuality],
    wanted: Option<&str>,
) -> Option<&'a VideoQuality> {
    let wanted = wanted?;
    if let Ok(ceiling) = wanted.trim_end_matches('p').parse::<u32>() {
        if let Some(q) = qualities
            .iter()
            .filter(|q| q.height > 0 && q.height <= ceiling)
            .max_by_key(|q| q.height)
        {
            return Some(q);
        }
    }
    qualities
        .iter()
        .find(|q| q.label == wanted && q.format != "ytdlp")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture: clip GQL `videoQualities` of corpus twitch-1 (26/09).
    const GQL: &str = r#"[{"quality":"1080","sourceURL":"https://production.assets.clips.twitchcdn.net/v2/media/x/landscape/avc/1080/index.mp4"},{"quality":"720","sourceURL":"https://production.assets.clips.twitchcdn.net/v2/media/x/landscape/avc/720/index.mp4"},{"quality":"480","sourceURL":"https://production.assets.clips.twitchcdn.net/v2/media/x/landscape/avc/480/index.mp4"},{"quality":"360","sourceURL":"https://production.assets.clips.twitchcdn.net/v2/media/x/landscape/avc/360/index.mp4"}]"#;

    fn qualities() -> Vec<VideoQuality> {
        let v: Vec<serde_json::Value> = serde_json::from_str(GQL).unwrap();
        v.iter()
            .map(|q| {
                let quality = q["quality"].as_str().unwrap();
                let url = q["sourceURL"].as_str().unwrap();
                VideoQuality {
                    label: format!("{quality}p"),
                    width: 0,
                    height: clip_height(quality, url),
                    url: url.into(),
                    format: "mp4".into(),
                }
            })
            .collect()
    }

    #[test]
    fn ceiling_picks_tallest_variant_within_it() {
        let q = qualities();
        assert_eq!(select_quality(&q, Some("720")).unwrap().height, 720);
        assert_eq!(select_quality(&q, Some("720p")).unwrap().height, 720);
        assert_eq!(select_quality(&q, Some("600")).unwrap().height, 480);
        assert_eq!(select_quality(&q, Some("2160")).unwrap().height, 1080);
        assert!(select_quality(&q, Some("240")).is_none());
        assert!(select_quality(&q, None).is_none());
    }

    #[test]
    fn portrait_rendition_height_is_unknown() {
        assert_eq!(
            clip_height("1080", "https://x/portrait/avc/1080/index.mp4"),
            0
        );
        assert_eq!(
            clip_height("720", "https://x/landscape/avc/720/index.mp4"),
            720
        );
        assert_eq!(clip_height("source", "https://x/y.mp4"), 0);
    }
}
