use crate::models::progress::ProgressUpdate;
use std::sync::LazyLock;

use anyhow::anyhow;
use async_trait::async_trait;
use regex::Regex;
use tokio::sync::mpsc;

use crate::core::direct_downloader;
use crate::core::redirect;
use crate::models::media::{DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality};
use crate::platforms::traits::PlatformDownloader;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

static PIN_NOT_FOUND_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""__typename"\s*:\s*"PinNotFound""#).expect("valid PIN_NOT_FOUND_RE")
});

static VIDEO_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#""url":"(https://v1\.pinimg\.com/videos/.*?)""#).expect("valid VIDEO_URL_RE")
});

static IMAGE_URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"src="(https://i\.pinimg\.com/.*?\.(jpg|gif))""#).expect("valid IMAGE_URL_RE")
});

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
        let mut builder = crate::core::http_client::apply_global_proxy(reqwest::Client::builder())
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(15));

        if let Some(jar) =
            crate::core::cookie_parser::load_extension_cookies_for_domain("pinterest.com")
        {
            builder = builder.cookie_provider(jar);
        }

        let client = builder.build().unwrap_or_default();
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
            return Err(anyhow!(
                "HTTP {} ao acessar pin {}",
                response.status(),
                pin_id
            ));
        }

        response.text().await.map_err(Into::into)
    }

    /// gallery-dl pinterest.py:526-527 (`/resource/PinResource/get/`, no
    /// login) with its API headers (pinterest.py:419-432).
    async fn fetch_pin_resource(&self, pin_id: &str) -> anyhow::Result<serde_json::Value> {
        let data = serde_json::json!({"options": {"id": pin_id, "field_set_key": "detailed"}});
        let response = self
            .client
            .get("https://www.pinterest.com/resource/PinResource/get/")
            .query(&[("data", data.to_string()), ("source_url", String::new())])
            .header("Accept", "application/json, text/javascript, */*, q=0.01")
            .header("X-Requested-With", "XMLHttpRequest")
            .header("X-Pinterest-AppState", "active")
            .header("X-Pinterest-PWS-Handler", "www/[username].js")
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(anyhow!(
                "Pinterest PinResource returned HTTP {}",
                response.status()
            ));
        }
        Ok(response.json().await?)
    }

    /// Best progressive MP4 of a video_list: V_720P first (gallery-dl
    /// pinterest.py:203-204 keeps it as the HLS fallback), else the widest
    /// non-HLS .mp4. HLS entries are left to yt-dlp.
    fn progressive_from_video_list(list: &serde_json::Value) -> Option<(String, Option<f64>, u32)> {
        let map = list.as_object()?;
        let is_mp4 = |v: &serde_json::Value| {
            v.get("url")
                .and_then(|u| u.as_str())
                .is_some_and(|u| u.split('?').next().unwrap_or(u).ends_with(".mp4"))
        };
        let chosen = map.get("V_720P").filter(|v| is_mp4(v)).or_else(|| {
            map.values()
                .filter(|v| is_mp4(v))
                .max_by_key(|v| v.get("width").and_then(|w| w.as_u64()).unwrap_or(0))
        })?;
        let url = chosen.get("url")?.as_str()?.to_string();
        let duration = chosen
            .get("duration")
            .and_then(|d| d.as_f64())
            .map(|ms| ms / 1000.0);
        let height = height_bound(&url, chosen);
        Some((url, duration, height))
    }

    fn video_from_pin_resource(json: &serde_json::Value) -> Option<(String, Option<f64>, u32)> {
        let pin = json.pointer("/resource_response/data")?;
        if let Some(found) = pin
            .pointer("/videos/video_list")
            .and_then(Self::progressive_from_video_list)
        {
            return Some(found);
        }
        pin.pointer("/story_pin_data/pages")
            .and_then(|p| p.as_array())?
            .iter()
            .filter_map(|page| page.get("blocks").and_then(|b| b.as_array()))
            .flatten()
            .find_map(|block| {
                block
                    .pointer("/video/video_list")
                    .and_then(Self::progressive_from_video_list)
            })
    }

    fn image_from_pin_resource(json: &serde_json::Value) -> Option<String> {
        json.pointer("/resource_response/data/images/orig/url")
            .and_then(|u| u.as_str())
            .map(str::to_string)
    }

    fn check_pin_not_found(html: &str) -> bool {
        PIN_NOT_FOUND_RE.is_match(html)
    }

    fn extract_video_url(html: &str) -> Option<String> {
        VIDEO_URL_RE
            .captures_iter(html)
            .filter_map(|cap| cap.get(1).map(|m| m.as_str().to_string()))
            .find(|url| url.ends_with(".mp4"))
    }

    fn extract_image_url(html: &str) -> Option<(String, bool)> {
        let mut best: Option<(String, bool)> = None;

        for cap in IMAGE_URL_RE.captures_iter(html) {
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
        let native = self.native_get_media_info(url).await;
        crate::platforms::generic_ytdlp::native_then_ytdlp("pinterest", native, || {
            self.fallback_ytdlp(url)
        })
        .await
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
                    opts.referer
                        .as_deref()
                        .or(Some("https://www.pinterest.com/")),
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
            Some(&opts.cancel_token),
        )
        .await?;

        Ok(DownloadResult {
            file_path: output_path,
            file_size_bytes: total_bytes,
            duration_seconds: 0.0,
            torrent_id: None,
        })
    }
}

impl PinterestDownloader {
    async fn fallback_ytdlp(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let ytdlp_path = crate::core::ytdlp::ensure_ytdlp().await?;
        let json = crate::core::ytdlp::get_video_info(&ytdlp_path, url, &[]).await?;
        crate::platforms::generic_ytdlp::GenericYtdlpDownloader::parse_video_info(&json)
    }

    async fn native_get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let canonical = self.resolve_pin_url(url).await?;

        let pin_id =
            Self::extract_pin_id(&canonical).ok_or_else(|| anyhow!("Could not extract pin ID"))?;

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
                available_qualities: vec![
                    VideoQuality {
                        label: "original".to_string(),
                        width: 0,
                        height: 0,
                        url: video_url,
                        format: "mp4".to_string(),
                    },
                    ytdlp_alternative(&pin_id),
                ],
                media_type: MediaType::Video,
                file_size_bytes: None,
            });
        }

        // No .mp4 in the HTML: ask the Pin API before settling for an image,
        // otherwise a video pin yields its thumbnail or falls to HLS.
        let resource = match self.fetch_pin_resource(&pin_id).await {
            Ok(json) => Some(json),
            Err(e) => {
                tracing::warn!("[pinterest] PinResource failed for {}: {}", pin_id, e);
                None
            }
        };
        if let Some((video_url, duration, height)) =
            resource.as_ref().and_then(Self::video_from_pin_resource)
        {
            return Ok(MediaInfo {
                title: format!("pinterest_{}", pin_id),
                author: String::new(),
                platform: "pinterest".to_string(),
                duration_seconds: duration,
                thumbnail_url: None,
                available_qualities: vec![
                    VideoQuality {
                        label: "720p".to_string(),
                        width: 0,
                        height,
                        url: video_url,
                        format: "mp4".to_string(),
                    },
                    ytdlp_alternative(&pin_id),
                ],
                media_type: MediaType::Video,
                file_size_bytes: None,
            });
        }
        // A video pin whose API answer has only HLS: its HTML images are the
        // cover, not the media. Leave it to yt-dlp.
        if resource
            .as_ref()
            .and_then(|r| r.pointer("/resource_response/data/videos/video_list"))
            .is_some()
        {
            return Err(anyhow!("Pin {} only has HLS video", pin_id));
        }

        if let Some((image_url, is_gif)) = Self::extract_image_url(&html) {
            let media_type = if is_gif {
                MediaType::Gif
            } else {
                MediaType::Photo
            };
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

        if let Some(image_url) = resource.as_ref().and_then(Self::image_from_pin_resource) {
            let is_gif = image_url.ends_with(".gif");
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
                    format: if is_gif { "gif" } else { "jpg" }.to_string(),
                }],
                media_type: if is_gif {
                    MediaType::Gif
                } else {
                    MediaType::Photo
                },
                file_size_bytes: None,
            });
        }

        Err(anyhow!("No media found in pin {}", pin_id))
    }
}

/// Upper bound on the height of a `/720p/` MP4. Pinterest scales it to 720 px
/// wide (V_720P of a 1080x1920 source measured 720x1280 on 26/09); the entry's
/// `width`/`height` are the source's. Bound = max(720, 720*h/w), which covers
/// both a width-720 and a short-side-720 scaling. Other URLs: unknown (0).
fn height_bound(url: &str, entry: &serde_json::Value) -> u32 {
    if !url.contains("/720p/") {
        return 0;
    }
    let dim = |k: &str| entry.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
    let (w, h) = (dim("width"), dim("height"));
    if w == 0 || h == 0 {
        return 0;
    }
    u32::try_from((720 * h).div_ceil(w).max(720)).unwrap_or(0)
}

/// yt-dlp on the pin page: the MCP worker takes it under a height ceiling
/// when the native MP4 cannot prove it fits (HLS renditions go down to 240p).
fn ytdlp_alternative(pin_id: &str) -> VideoQuality {
    VideoQuality {
        label: "auto".to_string(),
        width: 0,
        height: 0,
        url: format!("https://www.pinterest.com/pin/{pin_id}/"),
        format: "ytdlp".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // PinResource answer captured 26/09 for the corpus pin
    // /pin/dive-into-serenity-...--2885187256207927 (trimmed). The pin HTML
    // had no .mp4, so the adapter fell back to yt-dlp's HLS, which failed with
    // "fragment failure" in the baseline (pinterest-1, BROKEN_SOURCE).
    const PIN_RESOURCE_VIDEO: &str = r#"{"resource_response": {"status": "success", "data": {"id": "2885187256207927", "videos": {"video_list": {"V_720P": {"url": "https://v1.pinimg.com/videos/iht/720p/ec/e1/c5/ece1c59e8cc370b60000396c5952a72d.mp4", "width": 1080, "height": 1920, "duration": 7967}, "V_HLSV4": {"url": "https://v1.pinimg.com/videos/iht/hls/ec/e1/c5/ece1c59e8cc370b60000396c5952a72d.m3u8", "width": 1080, "height": 1920, "duration": 7967}, "V_HLSV3_MOBILE": {"url": "https://v1.pinimg.com/videos/iht/hls/ec/e1/c5/ece1c59e8cc370b60000396c5952a72d.m3u8", "width": 1080, "height": 1920, "duration": 7967}}}, "images": {"orig": {"width": 736, "height": 1308, "url": "https://i.pinimg.com/originals/c0/0a/2e/c00a2e5b706c1cfb9d35d2874ebd822a.jpg"}}, "story_pin_data": null}}}"#;

    #[test]
    fn pin_resource_prefers_progressive_720p_over_hls() {
        let json: serde_json::Value = serde_json::from_str(PIN_RESOURCE_VIDEO).unwrap();
        let (url, duration, height) = PinterestDownloader::video_from_pin_resource(&json).unwrap();
        assert_eq!(
            url,
            "https://v1.pinimg.com/videos/iht/720p/ec/e1/c5/ece1c59e8cc370b60000396c5952a72d.mp4"
        );
        assert_eq!(duration, Some(7.967));
        // Measured 26/09 (ffprobe): this V_720P is 720x1280.
        assert_eq!(height, 1280);
    }

    /// Round-1 regression (26/09): pinterest-1 (story pin, source 640x1138,
    /// V_EXP7 under /720p/) and pinterest-2 (V_720P of a 1080x1920 source =
    /// 720x1280) were downloaded natively and refused at maxHeight=720. Their
    /// HLS has 360x640, so yt-dlp's ceiling selector fits.
    #[test]
    fn portrait_720p_is_bounded_and_ytdlp_is_offered() {
        let v = serde_json::json!({"url":"https://v1.pinimg.com/videos/mc/720p/c8/a.mp4","width":640,"height":1138});
        assert_eq!(
            height_bound("https://v1.pinimg.com/videos/mc/720p/c8/a.mp4", &v),
            1281
        );
        let landscape = serde_json::json!({"width":1920,"height":1080});
        assert_eq!(
            height_bound("https://v1.pinimg.com/videos/iht/720p/a.mp4", &landscape),
            720
        );
        assert_eq!(
            height_bound("https://v1.pinimg.com/videos/mc/expMp4/a_t1.mp4", &v),
            0
        );
        assert_eq!(
            height_bound(
                "https://v1.pinimg.com/videos/iht/720p/a.mp4",
                &serde_json::json!({})
            ),
            0
        );
        let alt = ytdlp_alternative("2885187256207927");
        assert_eq!(alt.format, "ytdlp");
        assert_eq!(alt.height, 0);
        assert_eq!(alt.url, "https://www.pinterest.com/pin/2885187256207927/");
    }

    #[test]
    fn pin_resource_with_only_hls_has_no_progressive_video() {
        let json = serde_json::json!({"resource_response":{"data":{"videos":{"video_list":{
            "V_HLSV4":{"url":"https://v1.pinimg.com/videos/x.m3u8","width":720}}}}}});
        assert!(PinterestDownloader::video_from_pin_resource(&json).is_none());
    }

    // Idea pins keep the video under story_pin_data.pages[].blocks[].video.
    #[test]
    fn story_pin_video_block_is_found() {
        let json = serde_json::json!({"resource_response":{"data":{"videos":null,
            "story_pin_data":{"pages":[{"blocks":[{"type":"story_pin_image_block"},
                {"video":{"video_list":{
                    "V_EXP4":{"url":"https://v1.pinimg.com/videos/mc/expMp4/a.mp4","width":360},
                    "V_EXP7":{"url":"https://v1.pinimg.com/videos/mc/expMp4/b.mp4","width":720},
                    "V_HLSV3_MOBILE":{"url":"https://v1.pinimg.com/videos/mc/hls/b.m3u8","width":720}}}}]}]}}}});
        let (url, _, _) = PinterestDownloader::video_from_pin_resource(&json).unwrap();
        assert_eq!(url, "https://v1.pinimg.com/videos/mc/expMp4/b.mp4");
    }

    #[test]
    fn image_pin_has_no_video_but_has_original_image() {
        let json = serde_json::json!({"resource_response":{"data":{"videos":null,"story_pin_data":null,
            "images":{"orig":{"url":"https://i.pinimg.com/originals/a/b.jpg"}}}}});
        assert!(PinterestDownloader::video_from_pin_resource(&json).is_none());
        assert_eq!(
            PinterestDownloader::image_from_pin_resource(&json).as_deref(),
            Some("https://i.pinimg.com/originals/a/b.jpg")
        );
    }

    #[test]
    fn slugged_pin_url_keeps_only_the_id() {
        assert_eq!(
            PinterestDownloader::extract_pin_id(
                "https://pinterest.com/pin/dive-into-serenity-video-in-2024--2885187256207927"
            )
            .as_deref(),
            Some("2885187256207927")
        );
    }
}
