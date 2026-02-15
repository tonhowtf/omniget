use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::core::direct_downloader;
use crate::models::media::{
    DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality,
};
use crate::platforms::traits::PlatformDownloader;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const SHORT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko)";

pub struct TikTokDownloader {
    client: reqwest::Client,
    short_client: reqwest::Client,
}

impl TikTokDownloader {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .build()
            .unwrap_or_default();

        let short_client = reqwest::Client::builder()
            .user_agent(SHORT_USER_AGENT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_default();

        Self {
            client,
            short_client,
        }
    }

    fn extract_post_id(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();

        if segments.len() >= 3
            && segments[0].starts_with('@')
            && (segments[1] == "video" || segments[1] == "photo")
        {
            let id = segments[2];
            if id.chars().all(|c| c.is_ascii_digit()) {
                return Some(id.to_string());
            }
        }

        None
    }

    async fn resolve_short_link(&self, url: &str) -> anyhow::Result<String> {
        let response = self.short_client.get(url).send().await?;

        if let Some(location) = response
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
        {
            let clean = location.split('?').next().unwrap_or(location).to_string();
            return Ok(clean);
        }

        let html = response.text().await?;

        if html.starts_with("<a href=\"https://") {
            if let Some(url_part) = html.split("<a href=\"").nth(1) {
                let full_url = url_part.split('"').next().unwrap_or(url_part);
                let clean = full_url.split('?').next().unwrap_or(full_url).to_string();
                return Ok(clean);
            }
        }

        Err(anyhow!("Não foi possível resolver o short link"))
    }

    async fn fetch_detail(&self, post_id: &str) -> anyhow::Result<serde_json::Value> {
        let url = format!("https://www.tiktok.com/@i/video/{}", post_id);

        let response = self
            .client
            .get(&url)
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("Accept-Language", "en-US,en;q=0.9")
            .send()
            .await?;

        let html = response.text().await?;

        let json_str = html
            .split(
                "<script id=\"__UNIVERSAL_DATA_FOR_REHYDRATION__\" type=\"application/json\">",
            )
            .nth(1)
            .and_then(|s| s.split("</script>").next())
            .ok_or_else(|| anyhow!("Não foi possível extrair dados do TikTok"))?;

        let data: serde_json::Value = serde_json::from_str(json_str)?;

        let video_detail = data
            .get("__DEFAULT_SCOPE__")
            .and_then(|s| s.get("webapp.video-detail"))
            .ok_or_else(|| anyhow!("Não foi possível extrair dados do TikTok"))?;

        if video_detail
            .get("statusMsg")
            .and_then(|v| v.as_str())
            .is_some()
        {
            return Err(anyhow!("Post não disponível"));
        }

        let detail = video_detail
            .pointer("/itemInfo/itemStruct")
            .ok_or_else(|| anyhow!("Não foi possível extrair dados do TikTok"))?
            .clone();

        if detail
            .get("isContentClassified")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Err(anyhow!("Conteúdo restrito por idade"));
        }

        if detail.get("author").is_none() {
            return Err(anyhow!("Post não disponível"));
        }

        Ok(detail)
    }

    fn extract_author(detail: &serde_json::Value) -> String {
        detail
            .pointer("/author/uniqueId")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string()
    }

    fn extract_video_url(detail: &serde_json::Value) -> Option<String> {
        let play_addr = detail.pointer("/video/playAddr")?;

        if let Some(url) = play_addr.as_str() {
            if !url.is_empty() {
                return Some(url.to_string());
            }
        }

        None
    }

    fn extract_image_urls(detail: &serde_json::Value) -> Option<Vec<String>> {
        let images = detail
            .pointer("/imagePost/images")
            .and_then(|v| v.as_array())?;

        let urls: Vec<String> = images
            .iter()
            .filter_map(|img| {
                let url_list = img.pointer("/imageURL/urlList")?.as_array()?;
                url_list
                    .iter()
                    .filter_map(|u| u.as_str())
                    .find(|u| u.contains(".jpeg"))
                    .map(|u| u.to_string())
            })
            .collect();

        if urls.is_empty() {
            return None;
        }

        Some(urls)
    }

    fn extract_music_url(detail: &serde_json::Value) -> Option<String> {
        detail
            .pointer("/music/playUrl")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    }

    fn extract_duration(detail: &serde_json::Value) -> Option<f64> {
        detail.pointer("/video/duration").and_then(|v| v.as_f64())
    }
}

#[async_trait]
impl PlatformDownloader for TikTokDownloader {
    fn name(&self) -> &str {
        "tiktok"
    }

    fn can_handle(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let host = host.to_lowercase();
                return host == "tiktok.com" || host.ends_with(".tiktok.com");
            }
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let post_id = match Self::extract_post_id(url) {
            Some(id) => id,
            None => {
                let canonical = self.resolve_short_link(url).await?;
                Self::extract_post_id(&canonical)
                    .ok_or_else(|| anyhow!("Não foi possível extrair o ID do post"))?
            }
        };

        let detail = self.fetch_detail(&post_id).await?;
        let author = Self::extract_author(&detail);
        let filename_base = format!(
            "tiktok_{}_{}",
            sanitize_filename::sanitize(&author),
            post_id
        );

        if let Some(image_urls) = Self::extract_image_urls(&detail) {
            let media_type = if image_urls.len() == 1 {
                MediaType::Photo
            } else {
                MediaType::Carousel
            };

            let qualities: Vec<VideoQuality> = image_urls
                .iter()
                .enumerate()
                .map(|(i, u)| VideoQuality {
                    label: format!("photo_{}", i + 1),
                    width: 0,
                    height: 0,
                    url: u.clone(),
                    format: "jpg".to_string(),
                })
                .collect();

            return Ok(MediaInfo {
                title: filename_base,
                author,
                platform: "tiktok".to_string(),
                duration_seconds: None,
                thumbnail_url: None,
                available_qualities: qualities,
                media_type,
            });
        }

        if let Some(video_url) = Self::extract_video_url(&detail) {
            return Ok(MediaInfo {
                title: filename_base,
                author,
                platform: "tiktok".to_string(),
                duration_seconds: Self::extract_duration(&detail),
                thumbnail_url: None,
                available_qualities: vec![VideoQuality {
                    label: "original".to_string(),
                    width: 0,
                    height: 0,
                    url: video_url,
                    format: "mp4".to_string(),
                }],
                media_type: MediaType::Video,
            });
        }

        if let Some(music_url) = Self::extract_music_url(&detail) {
            return Ok(MediaInfo {
                title: filename_base,
                author,
                platform: "tiktok".to_string(),
                duration_seconds: None,
                thumbnail_url: None,
                available_qualities: vec![VideoQuality {
                    label: "audio".to_string(),
                    width: 0,
                    height: 0,
                    url: music_url,
                    format: "mp3".to_string(),
                }],
                media_type: MediaType::Audio,
            });
        }

        Err(anyhow!("Nenhuma mídia encontrada no post"))
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<f64>,
    ) -> anyhow::Result<DownloadResult> {
        match info.media_type {
            MediaType::Video => {
                let quality = info
                    .available_qualities
                    .first()
                    .ok_or_else(|| anyhow!("Nenhum URL de vídeo disponível"))?;

                let filename = format!("{}.mp4", sanitize_filename::sanitize(&info.title));
                let output = opts.output_dir.join(&filename);

                let bytes = direct_downloader::download_direct(
                    &self.client,
                    &quality.url,
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
            MediaType::Photo | MediaType::Carousel => {
                let mut total_bytes = 0u64;
                let count = info.available_qualities.len();
                let mut last_path = opts.output_dir.clone();

                for (i, quality) in info.available_qualities.iter().enumerate() {
                    let filename = if count == 1 {
                        format!("{}.jpg", sanitize_filename::sanitize(&info.title))
                    } else {
                        format!(
                            "{}_photo_{}.jpg",
                            sanitize_filename::sanitize(&info.title),
                            i + 1
                        )
                    };
                    let output = opts.output_dir.join(&filename);
                    let (tx, _rx) = mpsc::channel(8);

                    let bytes = direct_downloader::download_direct(
                        &self.client,
                        &quality.url,
                        &output,
                        tx,
                    )
                    .await?;

                    total_bytes += bytes;
                    last_path = output;

                    let percent = ((i + 1) as f64 / count as f64) * 100.0;
                    let _ = progress.send(percent).await;
                }

                Ok(DownloadResult {
                    file_path: last_path,
                    file_size_bytes: total_bytes,
                    duration_seconds: 0.0,
                })
            }
            MediaType::Audio => {
                let quality = info
                    .available_qualities
                    .first()
                    .ok_or_else(|| anyhow!("Nenhum URL de áudio disponível"))?;

                let filename = format!("{}.mp3", sanitize_filename::sanitize(&info.title));
                let output = opts.output_dir.join(&filename);

                let bytes = direct_downloader::download_direct(
                    &self.client,
                    &quality.url,
                    &output,
                    progress,
                )
                .await?;

                Ok(DownloadResult {
                    file_path: output,
                    file_size_bytes: bytes,
                    duration_seconds: 0.0,
                })
            }
            _ => Err(anyhow!("Tipo de mídia não suportado para download")),
        }
    }
}
