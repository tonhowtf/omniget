use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;

use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::mpsc;

use crate::core::ffmpeg;
use crate::models::media::{
    DownloadOptions, DownloadResult, MediaInfo, MediaType, VideoQuality as MediaVideoQuality,
};
use crate::platforms::traits::PlatformDownloader;

pub struct YouTubeDownloader;

impl YouTubeDownloader {
    pub fn new() -> Self {
        Self
    }

    fn extract_video_id(url: &str) -> Option<String> {
        let parsed = url::Url::parse(url).ok()?;
        let host = parsed.host_str()?.to_lowercase();

        if host.contains("youtu.be") {
            let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();
            return segments.first().map(|s| s.to_string());
        }

        if host.contains("youtube.com") || host.contains("youtube-nocookie.com") {
            let segments: Vec<&str> = parsed.path().split('/').filter(|s| !s.is_empty()).collect();

            if segments.first() == Some(&"shorts") {
                return segments.get(1).map(|s| s.to_string());
            }

            return parsed
                .query_pairs()
                .find(|(k, _)| k == "v")
                .map(|(_, v)| v.to_string());
        }

        None
    }

    fn is_bot_detection_error(err: &rusty_ytdl::VideoError) -> bool {
        let msg = format!("{}", err);
        msg.contains("Sign in to confirm")
            || msg.contains("bot")
            || msg.contains("429")
            || msg.contains("consent")
            || msg.contains("CAPTCHA")
    }

    async fn is_ytdlp_available() -> bool {
        tokio::process::Command::new("yt-dlp")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    async fn get_info_ytdlp(video_id: &str) -> anyhow::Result<MediaInfo> {
        let output = tokio::process::Command::new("yt-dlp")
            .args([
                "--dump-json",
                "--no-playlist",
                &format!("https://www.youtube.com/watch?v={}", video_id),
            ])
            .output()
            .await
            .map_err(|e| anyhow!("Falha ao executar yt-dlp: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!("yt-dlp falhou: {}", stderr.trim()));
        }

        let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;

        let title = json.get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let author = json.get("uploader")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let duration = json.get("duration").and_then(|v| v.as_f64());

        let thumbnail = json.get("thumbnail")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // yt-dlp handles quality selection itself, just offer "best"
        let qualities = vec![MediaVideoQuality {
            label: "best".to_string(),
            width: 0,
            height: 0,
            url: video_id.to_string(),
            format: "ytdlp".to_string(),
        }];

        Ok(MediaInfo {
            title: format!("youtube_{}", video_id),
            author: format!("{} - {}", title, author),
            platform: "youtube".to_string(),
            duration_seconds: duration,
            thumbnail_url: thumbnail,
            available_qualities: qualities,
            media_type: MediaType::Video,
        })
    }
}

#[async_trait]
impl PlatformDownloader for YouTubeDownloader {
    fn name(&self) -> &str {
        "youtube"
    }

    fn can_handle(&self, url: &str) -> bool {
        if let Ok(parsed) = url::Url::parse(url) {
            if let Some(host) = parsed.host_str() {
                let host = host.to_lowercase();
                return host == "youtube.com"
                    || host.ends_with(".youtube.com")
                    || host == "youtu.be"
                    || host == "youtube-nocookie.com"
                    || host.ends_with(".youtube-nocookie.com");
            }
        }
        false
    }

    async fn get_media_info(&self, url: &str) -> anyhow::Result<MediaInfo> {
        let video_id = Self::extract_video_id(url)
            .ok_or_else(|| anyhow!("Não foi possível extrair o ID do vídeo YouTube"))?;

        let video = rusty_ytdl::Video::new(&video_id).map_err(|e| anyhow!("{}", e))?;

        match video.get_basic_info().await {
            Ok(info) => {
                let details = &info.video_details;

                if details.is_private {
                    return Err(anyhow!("Vídeo privado"));
                }

                if details.age_restricted {
                    return Err(anyhow!("Conteúdo restrito por idade"));
                }

                let title = details.title.clone();
                let author = details
                    .author
                    .as_ref()
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| details.owner_channel_name.clone());

                let duration = details.length_seconds.parse::<f64>().ok();

                let thumbnail = details.thumbnails.last().map(|t| t.url.clone());

                let mut qualities: Vec<MediaVideoQuality> = Vec::new();
                let mut seen_heights: HashSet<u32> = HashSet::new();

                let has_live_only = info
                    .formats
                    .iter()
                    .all(|f| f.is_live || (!f.has_video && !f.has_audio));
                if has_live_only && details.is_live_content {
                    return Err(anyhow!("Livestreams não suportados"));
                }

                let mut combined: Vec<&rusty_ytdl::VideoFormat> = info
                    .formats
                    .iter()
                    .filter(|f| f.has_video && f.has_audio && !f.is_live)
                    .collect();
                combined.sort_by(|a, b| b.height.unwrap_or(0).cmp(&a.height.unwrap_or(0)));

                for f in &combined {
                    let height = f.height.unwrap_or(0) as u32;
                    let width = f.width.unwrap_or(0) as u32;
                    let label = f
                        .quality_label
                        .clone()
                        .unwrap_or_else(|| format!("{}p", height));

                    if height > 0 && seen_heights.insert(height) {
                        qualities.push(MediaVideoQuality {
                            label: label.clone(),
                            width,
                            height,
                            url: video_id.clone(),
                            format: "mp4".to_string(),
                        });
                    }
                }

                let mut adaptive_video: Vec<&rusty_ytdl::VideoFormat> = info
                    .formats
                    .iter()
                    .filter(|f| f.has_video && !f.has_audio && !f.is_live)
                    .filter(|f| {
                        f.mime_type.container == "mp4"
                            || f.mime_type.container == "webm"
                    })
                    .collect();
                adaptive_video.sort_by(|a, b| b.height.unwrap_or(0).cmp(&a.height.unwrap_or(0)));

                for f in &adaptive_video {
                    let height = f.height.unwrap_or(0) as u32;
                    let width = f.width.unwrap_or(0) as u32;
                    let label = f
                        .quality_label
                        .clone()
                        .unwrap_or_else(|| format!("{}p", height));

                    if height > 0 && !seen_heights.contains(&height) {
                        seen_heights.insert(height);
                        qualities.push(MediaVideoQuality {
                            label: format!("{} (HD)", label),
                            width,
                            height,
                            url: video_id.clone(),
                            format: "mp4+mux".to_string(),
                        });
                    }
                }

                qualities.sort_by(|a, b| b.height.cmp(&a.height));

                if qualities.is_empty() {
                    qualities.push(MediaVideoQuality {
                        label: "best".to_string(),
                        width: 0,
                        height: 0,
                        url: video_id.clone(),
                        format: "mp4".to_string(),
                    });
                }

                Ok(MediaInfo {
                    title: format!("youtube_{}", video_id),
                    author: format!("{} - {}", title, author),
                    platform: "youtube".to_string(),
                    duration_seconds: duration,
                    thumbnail_url: thumbnail,
                    available_qualities: qualities,
                    media_type: MediaType::Video,
                })
            }
            Err(e) => {
                // Map known errors
                match &e {
                    rusty_ytdl::VideoError::VideoIsPrivate => return Err(anyhow!("Vídeo privado")),
                    rusty_ytdl::VideoError::VideoNotFound => return Err(anyhow!("Vídeo não encontrado")),
                    rusty_ytdl::VideoError::LiveStreamNotSupported => return Err(anyhow!("Livestreams não suportados")),
                    _ => {}
                }

                // Try yt-dlp fallback for bot detection errors
                if Self::is_bot_detection_error(&e) && Self::is_ytdlp_available().await {
                    tracing::info!("YouTube bot detection, falling back to yt-dlp");
                    return Self::get_info_ytdlp(&video_id).await;
                }

                Err(anyhow!("YouTube: {}", e))
            }
        }
    }

    async fn download(
        &self,
        info: &MediaInfo,
        opts: &DownloadOptions,
        progress: mpsc::Sender<f64>,
    ) -> anyhow::Result<DownloadResult> {
        let _ = progress.send(0.0).await;

        let video_id = info
            .available_qualities
            .first()
            .map(|q| q.url.clone())
            .ok_or_else(|| anyhow!("Nenhuma qualidade disponível"))?;

        let first = info.available_qualities.first().unwrap();
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
            sanitize_filename::sanitize(&info.author),
            sanitize_filename::sanitize(&selected.label)
        );
        let output_path = opts.output_dir.join(&filename);

        tokio::fs::create_dir_all(&opts.output_dir).await?;

        // yt-dlp download path
        if selected.format == "ytdlp" {
            return download_ytdlp(&video_id, &output_path, progress.clone()).await;
        }

        let needs_mux =
            selected.format == "mp4+mux" && ffmpeg::is_ffmpeg_available().await;

        let file_size = if needs_mux {
            download_muxed(&video_id, &output_path, progress.clone()).await?
        } else {
            match download_combined(&video_id, &output_path, progress.clone()).await {
                Ok(size) => size,
                Err(e) => {
                    // If download fails and yt-dlp is available, try fallback
                    if YouTubeDownloader::is_ytdlp_available().await {
                        tracing::info!("rusty_ytdl download failed ({}), trying yt-dlp", e);
                        return download_ytdlp(&video_id, &output_path, progress).await;
                    }
                    return Err(e);
                }
            }
        };

        Ok(DownloadResult {
            file_path: output_path,
            file_size_bytes: file_size,
            duration_seconds: info.duration_seconds.unwrap_or(0.0),
        })
    }
}

async fn download_combined(
    video_id: &str,
    output: &PathBuf,
    progress: mpsc::Sender<f64>,
) -> anyhow::Result<u64> {
    let options = rusty_ytdl::VideoOptions {
        quality: rusty_ytdl::VideoQuality::Highest,
        filter: rusty_ytdl::VideoSearchOptions::VideoAudio,
        ..Default::default()
    };

    let video = rusty_ytdl::Video::new_with_options(video_id, options)
        .map_err(|e| anyhow!("{}", e))?;

    let stream = video
        .stream()
        .await
        .map_err(|e| anyhow!("Erro ao iniciar stream: {}", e))?;

    let content_length = stream.content_length() as u64;

    let mut file =
        std::fs::File::create(output).map_err(|e| anyhow!("Erro ao criar arquivo: {}", e))?;

    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream
        .chunk()
        .await
        .map_err(|e| anyhow!("Erro no stream: {}", e))?
    {
        file.write_all(&chunk)
            .map_err(|e| anyhow!("Erro ao escrever: {}", e))?;
        downloaded += chunk.len() as u64;

        if content_length > 0 {
            let percent = (downloaded as f64 / content_length as f64) * 100.0;
            let _ = progress.send(percent.min(99.0)).await;
        }
    }

    let _ = progress.send(100.0).await;
    Ok(downloaded)
}

async fn download_muxed(
    video_id: &str,
    output: &PathBuf,
    progress: mpsc::Sender<f64>,
) -> anyhow::Result<u64> {
    let temp_dir = tempfile::tempdir()?;
    let video_path = temp_dir.path().join("video.mp4");
    let audio_path = temp_dir.path().join("audio.m4a");

    // Download video stream
    let video_opts = rusty_ytdl::VideoOptions {
        quality: rusty_ytdl::VideoQuality::HighestVideo,
        filter: rusty_ytdl::VideoSearchOptions::Video,
        ..Default::default()
    };

    let video = rusty_ytdl::Video::new_with_options(video_id, video_opts)
        .map_err(|e| anyhow!("{}", e))?;

    let stream = video
        .stream()
        .await
        .map_err(|e| anyhow!("Erro ao iniciar stream de vídeo: {}", e))?;

    let video_cl = stream.content_length() as u64;
    let mut file = std::fs::File::create(&video_path)?;
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream
        .chunk()
        .await
        .map_err(|e| anyhow!("Erro no stream de vídeo: {}", e))?
    {
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;

        if video_cl > 0 {
            let percent = (downloaded as f64 / video_cl as f64) * 45.0;
            let _ = progress.send(percent).await;
        }
    }

    let _ = progress.send(45.0).await;

    // Download audio stream
    let audio_opts = rusty_ytdl::VideoOptions {
        quality: rusty_ytdl::VideoQuality::HighestAudio,
        filter: rusty_ytdl::VideoSearchOptions::Audio,
        ..Default::default()
    };

    let audio = rusty_ytdl::Video::new_with_options(video_id, audio_opts)
        .map_err(|e| anyhow!("{}", e))?;

    let stream = audio
        .stream()
        .await
        .map_err(|e| anyhow!("Erro ao iniciar stream de áudio: {}", e))?;

    let audio_cl = stream.content_length() as u64;
    let mut file = std::fs::File::create(&audio_path)?;
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream
        .chunk()
        .await
        .map_err(|e| anyhow!("Erro no stream de áudio: {}", e))?
    {
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;

        if audio_cl > 0 {
            let percent = 45.0 + (downloaded as f64 / audio_cl as f64) * 45.0;
            let _ = progress.send(percent).await;
        }
    }

    let _ = progress.send(90.0).await;

    ffmpeg::mux_video_audio(&video_path, &audio_path, output).await?;

    let _ = progress.send(100.0).await;

    let meta = tokio::fs::metadata(output).await?;
    Ok(meta.len())
}

async fn download_ytdlp(
    video_id: &str,
    output: &PathBuf,
    progress: mpsc::Sender<f64>,
) -> anyhow::Result<DownloadResult> {
    let output_template = output.to_string_lossy().to_string();

    let result = tokio::process::Command::new("yt-dlp")
        .args([
            "-f", "bv*+ba/b",
            "--merge-output-format", "mp4",
            "--no-playlist",
            "-o", &output_template,
            &format!("https://www.youtube.com/watch?v={}", video_id),
        ])
        .output()
        .await
        .map_err(|e| anyhow!("Falha ao executar yt-dlp: {}", e))?;

    if !result.status.success() {
        let stderr = String::from_utf8_lossy(&result.stderr);
        return Err(anyhow!("yt-dlp falhou: {}", stderr.trim()));
    }

    let _ = progress.send(100.0).await;

    let meta = tokio::fs::metadata(output).await?;
    Ok(DownloadResult {
        file_path: output.clone(),
        file_size_bytes: meta.len(),
        duration_seconds: 0.0,
    })
}
