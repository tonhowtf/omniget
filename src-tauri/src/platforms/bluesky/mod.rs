use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::core::direct_downloader;
use crate::core::hls_downloader::HlsDownloader;
use crate::models::media::{
    DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality,
};
use crate::platforms::traits::PlatformDownloader;

const API_BASE: &str = "https://public.api.bsky.app/xrpc/app.bsky.feed.getPostThread";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub struct BlueskyDownloader {
    client: reqwest::Client,
}

impl BlueskyDownloader {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();
        Self { client }
    }

    fn extract_user_and_post(url: &str) -> Option<(String, String)> {
        let parsed = url::Url::parse(url).ok()?;
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
        if segments.len() >= 4 && segments[0] == "profile" && segments[2] == "post" {
            return Some((segments[1].to_string(), segments[3].to_string()));
        }
        None
    }

    async fn fetch_post(&self, user: &str, post_id: &str) -> anyhow::Result<serde_json::Value> {
        let uri = format!("at://{}/app.bsky.feed.post/{}", user, post_id);
        let url = format!(
            "{}?depth=0&parentHeight=0&uri={}",
            API_BASE,
            urlencoding::encode(&uri)
        );

        let response = self.client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!("Bluesky API retornou HTTP {}", response.status()));
        }

        let json: serde_json::Value = response.json().await?;

        if let Some(error) = json.get("error").and_then(|e| e.as_str()) {
            return match error {
                "NotFound" | "InternalServerError" => Err(anyhow!("Post não disponível")),
                "InvalidRequest" => Err(anyhow!("Link não suportado")),
                _ => Err(anyhow!("Erro da API: {}", error)),
            };
        }

        Ok(json)
    }
}

enum BlueskyMedia {
    Video { hls_url: String },
    Images { urls: Vec<String> },
    Gif { url: String },
}

fn extract_media(embed: &serde_json::Value) -> Option<BlueskyMedia> {
    let embed_type = embed.get("$type")?.as_str()?;

    match embed_type {
        "app.bsky.embed.video#view" => {
            let playlist = embed.get("playlist")?.as_str()?;
            let hls_url = playlist.replace("video.bsky.app/watch/", "video.cdn.bsky.app/hls/");
            Some(BlueskyMedia::Video { hls_url })
        }
        "app.bsky.embed.images#view" => {
            let images = embed.get("images")?.as_array()?;
            let urls: Vec<String> = images
                .iter()
                .filter_map(|img| img.get("fullsize")?.as_str().map(|s| s.to_string()))
                .collect();
            if urls.is_empty() {
                return None;
            }
            Some(BlueskyMedia::Images { urls })
        }
        "app.bsky.embed.external#view" => {
            let uri = embed.get("external")?.get("uri")?.as_str()?;
            extract_gif_from_uri(uri)
        }
        "app.bsky.embed.recordWithMedia#view" => {
            let media = embed.get("media")?;
            let media_type = media.get("$type")?.as_str()?;
            if media_type == "app.bsky.embed.external#view" {
                let uri = media.get("external")?.get("uri")?.as_str()?;
                return extract_gif_from_uri(uri);
            }
            if media_type == "app.bsky.embed.video#view" {
                let playlist = media.get("playlist")?.as_str()?;
                let hls_url = playlist.replace("video.bsky.app/watch/", "video.cdn.bsky.app/hls/");
                return Some(BlueskyMedia::Video { hls_url });
            }
            None
        }
        _ => None,
    }
}

fn extract_gif_from_uri(uri: &str) -> Option<BlueskyMedia> {
    let parsed = url::Url::parse(uri).ok()?;
    if parsed.host_str()? == "media.tenor.com" {
        let mut clean = parsed.clone();
        clean.set_query(None);
        return Some(BlueskyMedia::Gif {
            url: clean.to_string(),
        });
    }
    None
}

#[async_trait]
impl PlatformDownloader for BlueskyDownloader {
    fn name(&self) -> &str {
        "bluesky"
    }

    fn can_handle(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let host = host.to_lowercase();
                return host == "bsky.app" || host.ends_with(".bsky.app");
            }
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let (user, post_id) = Self::extract_user_and_post(url)
            .ok_or_else(|| anyhow!("Não foi possível extrair user e post_id da URL"))?;

        let json = self.fetch_post(&user, &post_id).await?;

        let embed = json
            .pointer("/thread/post/embed")
            .ok_or_else(|| anyhow!("Post não contém mídia"))?;

        let media = extract_media(embed).ok_or_else(|| anyhow!("Tipo de mídia não suportado"))?;

        let filename_base = format!(
            "bluesky_{}_{}",
            sanitize_filename::sanitize(&user),
            post_id
        );

        match media {
            BlueskyMedia::Video { hls_url } => Ok(MediaInfo {
                title: filename_base,
                author: user,
                platform: "bluesky".to_string(),
                duration_seconds: None,
                thumbnail_url: None,
                available_qualities: vec![VideoQuality {
                    label: "best".to_string(),
                    width: 0,
                    height: 0,
                    url: hls_url,
                    format: "hls".to_string(),
                }],
                media_type: MediaType::Video,
            }),
            BlueskyMedia::Images { urls } => {
                let media_type = if urls.len() == 1 {
                    MediaType::Photo
                } else {
                    MediaType::Carousel
                };
                let qualities: Vec<VideoQuality> = urls
                    .iter()
                    .enumerate()
                    .map(|(i, u)| VideoQuality {
                        label: format!("{}", i + 1),
                        width: 0,
                        height: 0,
                        url: u.clone(),
                        format: "jpg".to_string(),
                    })
                    .collect();
                Ok(MediaInfo {
                    title: filename_base,
                    author: user,
                    platform: "bluesky".to_string(),
                    duration_seconds: None,
                    thumbnail_url: None,
                    available_qualities: qualities,
                    media_type,
                })
            }
            BlueskyMedia::Gif { url: gif_url } => Ok(MediaInfo {
                title: filename_base,
                author: user,
                platform: "bluesky".to_string(),
                duration_seconds: None,
                thumbnail_url: None,
                available_qualities: vec![VideoQuality {
                    label: "original".to_string(),
                    width: 0,
                    height: 0,
                    url: gif_url,
                    format: "gif".to_string(),
                }],
                media_type: MediaType::Gif,
            }),
        }
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<f64>,
    ) -> anyhow::Result<DownloadResult> {
        match info.media_type {
            MediaType::Video => {
                let hls_url = &info
                    .available_qualities
                    .first()
                    .ok_or_else(|| anyhow!("Nenhum URL HLS disponível"))?
                    .url;

                let filename = format!("{}.mp4", sanitize_filename::sanitize(&info.title));
                let output_path = opts.output_dir.join(&filename);
                let output_str = output_path.to_string_lossy().to_string();

                let downloader = HlsDownloader::new();
                let cancel = CancellationToken::new();
                let _ = progress.send(0.0).await;

                let result = downloader
                    .download(hls_url, &output_str, "https://bsky.app", None, cancel, 20, 3)
                    .await?;

                let _ = progress.send(100.0).await;

                Ok(DownloadResult {
                    file_path: result.path,
                    file_size_bytes: result.file_size,
                    duration_seconds: 0.0,
                })
            }
            MediaType::Photo | MediaType::Carousel => {
                let mut total_bytes = 0u64;
                let count = info.available_qualities.len();
                let mut last_path = opts.output_dir.clone();

                for (i, quality) in info.available_qualities.iter().enumerate() {
                    let ext = &quality.format;
                    let filename = if count == 1 {
                        format!("{}.{}", sanitize_filename::sanitize(&info.title), ext)
                    } else {
                        format!(
                            "{}_{}.{}",
                            sanitize_filename::sanitize(&info.title),
                            i + 1,
                            ext
                        )
                    };
                    let output = opts.output_dir.join(&filename);
                    let (tx, _rx) = mpsc::channel(8);
                    let bytes =
                        direct_downloader::download_direct(&self.client, &quality.url, &output, tx, None)
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
            MediaType::Gif => {
                let gif_url = &info
                    .available_qualities
                    .first()
                    .ok_or_else(|| anyhow!("Nenhum URL GIF disponível"))?
                    .url;

                let filename = format!("{}.gif", sanitize_filename::sanitize(&info.title));
                let output = opts.output_dir.join(&filename);

                let bytes =
                    direct_downloader::download_direct(&self.client, gif_url, &output, progress, None)
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
