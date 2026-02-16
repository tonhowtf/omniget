use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::core::direct_downloader;
use crate::core::ffmpeg;
use crate::core::redirect;
use crate::models::media::{
    DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality,
};
use crate::platforms::traits::PlatformDownloader;

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

pub struct RedditDownloader {
    client: reqwest::Client,
}

enum RedditMedia {
    Video {
        video_url: String,
        duration: Option<f64>,
    },
    Gif {
        url: String,
    },
    Image {
        url: String,
    },
}

impl RedditDownloader {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(std::time::Duration::from_secs(120))
            .connect_timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();
        Self { client }
    }

    fn extract_post_id(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();

        if segments.len() >= 4
            && segments[0] == "r"
            && segments[2] == "comments"
        {
            return Some(segments[3].to_string());
        }

        if segments.first() == Some(&"comments") {
            return segments.get(1).map(|s| s.to_string());
        }

        if segments.first() == Some(&"video") {
            return segments.get(1).map(|s| s.to_string());
        }

        None
    }

    fn extract_subreddit(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
        if segments.first() == Some(&"r") {
            return segments.get(1).map(|s| s.to_string());
        }
        None
    }

    fn is_short_link(url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let host = host.to_lowercase();
                return host == "v.redd.it" || host == "redd.it";
            }
        }
        false
    }

    fn is_share_link(url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
            return segments.len() >= 4 && segments[0] == "r" && segments[2] == "s";
        }
        false
    }

    async fn resolve_to_canonical(&self, url: &str) -> anyhow::Result<String> {
        if Self::is_short_link(url) {
            return redirect::resolve_redirect(&self.client, url).await;
        }

        if Self::is_share_link(url) {
            return redirect::resolve_redirect(&self.client, url).await;
        }

        Ok(url.to_string())
    }

    async fn fetch_post_data(&self, post_id: &str) -> anyhow::Result<serde_json::Value> {
        let url = format!("https://www.reddit.com/comments/{}.json", post_id);

        let response = self
            .client
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(anyhow!("Reddit retornou HTTP {}", response.status()));
        }

        let json: serde_json::Value = response.json().await?;

        if !json.is_array() {
            return Err(anyhow!("Post não encontrado"));
        }

        json.as_array()
            .and_then(|arr| arr.first())
            .and_then(|listing| listing.pointer("/data/children/0/data"))
            .cloned()
            .ok_or_else(|| anyhow!("Post não encontrado"))
    }

    fn construct_audio_url(fallback_url: &str) -> Vec<String> {
        let video = fallback_url.split('?').next().unwrap_or(fallback_url);
        let mut candidates = Vec::new();

        if video.contains(".mp4") {
            if let Some(base) = video.split('_').next() {
                candidates.push(format!("{}_audio.mp4", base));
                candidates.push(format!("{}_AUDIO_128.mp4", base));
            }
        }

        if let Some(dash_pos) = video.find("DASH") {
            candidates.push(format!("{}audio", &video[..dash_pos]));
        }

        candidates
    }

    async fn find_audio_url(&self, fallback_url: &str) -> Option<String> {
        let candidates = Self::construct_audio_url(fallback_url);

        for candidate in candidates {
            let resp = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                self.client.head(&candidate).send(),
            )
            .await;

            if let Ok(Ok(r)) = resp {
                if r.status().is_success() {
                    return Some(candidate);
                }
            }
        }

        None
    }

    fn parse_media(data: &serde_json::Value) -> Option<RedditMedia> {
        if let Some(url) = data.get("url").and_then(|v| v.as_str()) {
            if url.ends_with(".gif") {
                return Some(RedditMedia::Gif {
                    url: url.to_string(),
                });
            }
        }

        if let Some(reddit_video) = data.pointer("/secure_media/reddit_video") {
            let fallback = reddit_video
                .get("fallback_url")
                .and_then(|v| v.as_str())?;
            let duration = reddit_video
                .get("duration")
                .and_then(|v| v.as_f64());
            let video_url = fallback.split('?').next().unwrap_or(fallback).to_string();

            return Some(RedditMedia::Video {
                video_url,
                duration,
            });
        }

        if let Some(url) = data.get("url").and_then(|v| v.as_str()) {
            let is_media = data
                .get("is_reddit_media_domain")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if is_media
                || url.contains("i.redd.it")
                || url.ends_with(".jpg")
                || url.ends_with(".png")
                || url.ends_with(".jpeg")
            {
                return Some(RedditMedia::Image {
                    url: url.to_string(),
                });
            }
        }

        None
    }
}

#[async_trait]
impl PlatformDownloader for RedditDownloader {
    fn name(&self) -> &str {
        "reddit"
    }

    fn can_handle(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let host = host.to_lowercase();
                return host == "reddit.com"
                    || host.ends_with(".reddit.com")
                    || host == "v.redd.it"
                    || host == "redd.it";
            }
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let canonical = self.resolve_to_canonical(url).await?;

        let post_id = Self::extract_post_id(&canonical)
            .ok_or_else(|| anyhow!("Não foi possível extrair o ID do post"))?;

        let subreddit = Self::extract_subreddit(&canonical).unwrap_or_default();

        let data = self.fetch_post_data(&post_id).await?;

        let media = Self::parse_media(&data)
            .ok_or_else(|| anyhow!("Nenhuma mídia encontrada no post"))?;

        let source_id = if subreddit.is_empty() {
            post_id.clone()
        } else {
            format!("{}_{}", subreddit.to_lowercase(), post_id)
        };

        let title = format!("reddit_{}", source_id);

        match media {
            RedditMedia::Video {
                video_url,
                duration,
            } => {
                let audio = self.find_audio_url(&video_url).await;
                let mut qualities = vec![VideoQuality {
                    label: "video".to_string(),
                    width: 0,
                    height: 0,
                    url: video_url,
                    format: "mp4".to_string(),
                }];

                if let Some(audio_url) = audio {
                    qualities.push(VideoQuality {
                        label: "audio".to_string(),
                        width: 0,
                        height: 0,
                        url: audio_url,
                        format: "mp4_audio".to_string(),
                    });
                }

                Ok(MediaInfo {
                    title,
                    author: subreddit,
                    platform: "reddit".to_string(),
                    duration_seconds: duration,
                    thumbnail_url: None,
                    available_qualities: qualities,
                    media_type: MediaType::Video,
                })
            }
            RedditMedia::Gif { url: gif_url } => Ok(MediaInfo {
                title,
                author: subreddit,
                platform: "reddit".to_string(),
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
            RedditMedia::Image { url: image_url } => {
                let ext = if image_url.ends_with(".png") {
                    "png"
                } else {
                    "jpg"
                };
                Ok(MediaInfo {
                    title,
                    author: subreddit,
                    platform: "reddit".to_string(),
                    duration_seconds: None,
                    thumbnail_url: None,
                    available_qualities: vec![VideoQuality {
                        label: "original".to_string(),
                        width: 0,
                        height: 0,
                        url: image_url,
                        format: ext.to_string(),
                    }],
                    media_type: MediaType::Photo,
                })
            }
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
                let video_quality = info
                    .available_qualities
                    .iter()
                    .find(|q| q.label == "video")
                    .ok_or_else(|| anyhow!("Nenhum URL de vídeo"))?;

                let audio_quality = info
                    .available_qualities
                    .iter()
                    .find(|q| q.label == "audio");

                let has_audio = audio_quality.is_some();

                if has_audio {
                    let video_tmp = opts.output_dir.join(format!(
                        "{}_video_tmp.mp4",
                        sanitize_filename::sanitize(&info.title)
                    ));
                    let audio_tmp = opts.output_dir.join(format!(
                        "{}_audio_tmp.mp4",
                        sanitize_filename::sanitize(&info.title)
                    ));
                    let output = opts.output_dir.join(format!(
                        "{}.mp4",
                        sanitize_filename::sanitize(&info.title)
                    ));

                    let _ = progress.send(0.0).await;

                    let (vtx, _vrx) = mpsc::channel(8);
                    let video_bytes = direct_downloader::download_direct(
                        &self.client,
                        &video_quality.url,
                        &video_tmp,
                        vtx,
                        None,
                    )
                    .await?;

                    let _ = progress.send(50.0).await;

                    let audio_url = &audio_quality.unwrap().url;
                    let (atx, _arx) = mpsc::channel(8);
                    let audio_ok = match direct_downloader::download_direct(
                        &self.client,
                        audio_url,
                        &audio_tmp,
                        atx,
                        None,
                    )
                    .await
                    {
                        Ok(_) => true,
                        Err(e) => {
                            tracing::warn!("Reddit audio download failed: {}", e);
                            false
                        }
                    };

                    let _ = progress.send(75.0).await;

                    if audio_ok && ffmpeg::is_ffmpeg_available().await {
                        ffmpeg::mux_video_audio(&video_tmp, &audio_tmp, &output).await?;
                        let _ = tokio::fs::remove_file(&video_tmp).await;
                        let _ = tokio::fs::remove_file(&audio_tmp).await;
                        let _ = progress.send(100.0).await;

                        let file_size = tokio::fs::metadata(&output).await?.len();
                        Ok(DownloadResult {
                            file_path: output,
                            file_size_bytes: file_size,
                            duration_seconds: info.duration_seconds.unwrap_or(0.0),
                        })
                    } else {
                        if !audio_ok {
                            tracing::warn!("Reddit: audio indisponível, salvando só vídeo");
                        } else {
                            tracing::warn!("ffmpeg não disponível, salvando vídeo e áudio separados");
                        }
                        let video_final = opts.output_dir.join(format!(
                            "{}{}.mp4",
                            sanitize_filename::sanitize(&info.title),
                            if !audio_ok { "" } else { "_noaudio" }
                        ));
                        let _ = tokio::fs::rename(&video_tmp, &video_final).await;

                        if audio_ok {
                            let audio_final = opts.output_dir.join(format!(
                                "{}_audio.mp4",
                                sanitize_filename::sanitize(&info.title)
                            ));
                            let _ = tokio::fs::rename(&audio_tmp, &audio_final).await;
                        } else {
                            let _ = tokio::fs::remove_file(&audio_tmp).await;
                        }

                        let _ = progress.send(100.0).await;

                        Ok(DownloadResult {
                            file_path: video_final,
                            file_size_bytes: video_bytes,
                            duration_seconds: info.duration_seconds.unwrap_or(0.0),
                        })
                    }
                } else {
                    let output = opts.output_dir.join(format!(
                        "{}.mp4",
                        sanitize_filename::sanitize(&info.title)
                    ));
                    let bytes = direct_downloader::download_direct(
                        &self.client,
                        &video_quality.url,
                        &output,
                        progress,
                        None,
                    )
                    .await?;

                    Ok(DownloadResult {
                        file_path: output,
                        file_size_bytes: bytes,
                        duration_seconds: info.duration_seconds.unwrap_or(0.0),
                    })
                }
            }
            MediaType::Gif => {
                let url = &info
                    .available_qualities
                    .first()
                    .ok_or_else(|| anyhow!("Nenhum URL GIF"))?
                    .url;
                let output = opts.output_dir.join(format!(
                    "{}.gif",
                    sanitize_filename::sanitize(&info.title)
                ));
                let bytes =
                    direct_downloader::download_direct(&self.client, url, &output, progress, None)
                        .await?;

                Ok(DownloadResult {
                    file_path: output,
                    file_size_bytes: bytes,
                    duration_seconds: 0.0,
                })
            }
            MediaType::Photo => {
                let quality = info
                    .available_qualities
                    .first()
                    .ok_or_else(|| anyhow!("Nenhum URL de imagem"))?;
                let ext = &quality.format;
                let output = opts.output_dir.join(format!(
                    "{}.{}",
                    sanitize_filename::sanitize(&info.title),
                    ext
                ));
                let bytes =
                    direct_downloader::download_direct(&self.client, &quality.url, &output, progress, None)
                        .await?;

                Ok(DownloadResult {
                    file_path: output,
                    file_size_bytes: bytes,
                    duration_seconds: 0.0,
                })
            }
            _ => Err(anyhow!("Tipo de mídia não suportado")),
        }
    }
}
