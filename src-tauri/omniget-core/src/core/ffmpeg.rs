use std::path::Path;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

static FFMPEG_AVAILABLE_CACHE: std::sync::RwLock<Option<bool>> = std::sync::RwLock::new(None);

pub async fn is_ffmpeg_available() -> bool {
    if let Ok(cache) = FFMPEG_AVAILABLE_CACHE.read() {
        if let Some(val) = *cache {
            return val;
        }
    }
    let available = crate::core::dependencies::find_tool("ffmpeg")
        .await
        .is_some();
    if let Ok(mut cache) = FFMPEG_AVAILABLE_CACHE.write() {
        *cache = Some(available);
    }
    available
}

pub fn reset_ffmpeg_available_cache() {
    if let Ok(mut cache) = FFMPEG_AVAILABLE_CACHE.write() {
        *cache = None;
    }
}

pub async fn mux_video_audio(video: &Path, audio: &Path, output: &Path) -> anyhow::Result<()> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let status = crate::core::process::command("ffmpeg")
        .args([
            "-y",
            "-i",
            &video.to_string_lossy(),
            "-i",
            &audio.to_string_lossy(),
            "-c",
            "copy",
            &output.to_string_lossy(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|e| anyhow!("Failed to run ffmpeg: {}", e))?;

    if !status.success() {
        return Err(anyhow!("ffmpeg returned code {}", status));
    }

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaProbeInfo {
    pub duration_seconds: f64,
    pub format_name: String,
    pub format_long_name: String,
    pub file_size_bytes: u64,
    pub bit_rate: u64,
    pub streams: Vec<StreamInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamInfo {
    pub index: u32,
    pub codec_type: String,
    pub codec_name: String,
    pub codec_long_name: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub bit_rate: Option<u64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub duration_seconds: Option<f64>,
}

pub async fn probe(path: &Path) -> anyhow::Result<MediaProbeInfo> {
    let output = crate::core::process::command("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            &path.to_string_lossy(),
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| anyhow!("Failed to run ffprobe: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("ffprobe failed: {}", stderr));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| anyhow!("Failed to parse ffprobe JSON: {}", e))?;

    let format = json
        .get("format")
        .ok_or_else(|| anyhow!("Missing 'format' field"))?;

    let duration_seconds = format
        .get("duration")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);

    let format_name = format
        .get("format_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let format_long_name = format
        .get("format_long_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let file_size_bytes = format
        .get("size")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let bit_rate = format
        .get("bit_rate")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let streams = json
        .get("streams")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().map(parse_stream_info).collect())
        .unwrap_or_default();

    Ok(MediaProbeInfo {
        duration_seconds,
        format_name,
        format_long_name,
        file_size_bytes,
        bit_rate,
        streams,
    })
}

fn parse_stream_info(s: &serde_json::Value) -> StreamInfo {
    let index = s.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u32;

    let codec_type = s
        .get("codec_type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let codec_name = s
        .get("codec_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let codec_long_name = s
        .get("codec_long_name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let width = s.get("width").and_then(|v| v.as_u64()).map(|v| v as u32);
    let height = s.get("height").and_then(|v| v.as_u64()).map(|v| v as u32);

    let fps = s
        .get("r_frame_rate")
        .and_then(|v| v.as_str())
        .and_then(parse_frame_rate);

    let bit_rate = s
        .get("bit_rate")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok());

    let sample_rate = s
        .get("sample_rate")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u32>().ok());

    let channels = s.get("channels").and_then(|v| v.as_u64()).map(|v| v as u32);

    let duration_seconds = s
        .get("duration")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<f64>().ok());

    StreamInfo {
        index,
        codec_type,
        codec_name,
        codec_long_name,
        width,
        height,
        fps,
        bit_rate,
        sample_rate,
        channels,
        duration_seconds,
    }
}

fn parse_frame_rate(s: &str) -> Option<f64> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() == 2 {
        let num = parts[0].parse::<f64>().ok()?;
        let den = parts[1].parse::<f64>().ok()?;
        if den > 0.0 {
            return Some(num / den);
        }
    }
    s.parse::<f64>().ok()
}

pub async fn get_duration_us(path: &Path) -> anyhow::Result<u64> {
    let info = probe(path).await?;
    Ok((info.duration_seconds * 1_000_000.0) as u64)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MetadataEmbed {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_number: Option<String>,
    pub genre: Option<String>,
    pub year: Option<String>,
    pub comment: Option<String>,
    pub thumbnail_url: Option<String>,
}

pub async fn embed_metadata(
    file: &Path,
    metadata: &MetadataEmbed,
    embed_thumbnail: bool,
    http_client: &reqwest::Client,
) -> anyhow::Result<()> {
    if !is_ffmpeg_available().await {
        return Err(anyhow!("ffmpeg not available"));
    }

    let temp_dir = file.parent().unwrap_or(Path::new("."));
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("mp4");
    let temp_output = temp_dir.join(format!(".omniget_meta_{}.{}", uuid::Uuid::new_v4(), ext));

    let is_audio_only = matches!(
        ext.to_lowercase().as_str(),
        "mp3" | "m4a" | "aac" | "ogg" | "opus" | "flac" | "wav" | "wma"
    );

    let thumbnail_path = if embed_thumbnail && is_audio_only {
        if let Some(ref url) = metadata.thumbnail_url {
            match download_thumbnail(http_client, url, temp_dir).await {
                Ok(p) => Some(p),
                Err(e) => {
                    tracing::warn!("Failed to download thumbnail: {}", e);
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    let mut args: Vec<String> = vec![
        "-y".to_string(),
        "-i".to_string(),
        file.to_string_lossy().to_string(),
    ];

    if let Some(ref thumb) = thumbnail_path {
        args.extend(["-i".to_string(), thumb.to_string_lossy().to_string()]);
    }

    if let Some(ref thumb) = thumbnail_path {
        let _ = thumb;
        args.extend([
            "-map".to_string(),
            "0:a".to_string(),
            "-map".to_string(),
            "1:v".to_string(),
            "-c".to_string(),
            "copy".to_string(),
            "-disposition:v:0".to_string(),
            "attached_pic".to_string(),
        ]);
    } else {
        args.extend(["-c".to_string(), "copy".to_string()]);
    }

    if let Some(ref v) = metadata.title {
        args.extend(["-metadata".to_string(), format!("title={}", v)]);
    }
    if let Some(ref v) = metadata.artist {
        args.extend(["-metadata".to_string(), format!("artist={}", v)]);
    }
    if let Some(ref v) = metadata.album {
        args.extend(["-metadata".to_string(), format!("album={}", v)]);
    }
    if let Some(ref v) = metadata.track_number {
        args.extend(["-metadata".to_string(), format!("track={}", v)]);
    }
    if let Some(ref v) = metadata.genre {
        args.extend(["-metadata".to_string(), format!("genre={}", v)]);
    }
    if let Some(ref v) = metadata.year {
        args.extend(["-metadata".to_string(), format!("date={}", v)]);
    }
    if let Some(ref v) = metadata.comment {
        args.extend(["-metadata".to_string(), format!("comment={}", v)]);
    }

    args.push(temp_output.to_string_lossy().to_string());

    let output = crate::core::process::command("ffmpeg")
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| anyhow!("Failed to run ffmpeg: {}", e))?;

    if let Some(ref thumb) = thumbnail_path {
        let _ = std::fs::remove_file(thumb);
    }

    if !output.status.success() {
        let _ = std::fs::remove_file(&temp_output);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("ffmpeg metadata failed: {}", stderr));
    }

    let mut rename_ok = false;
    for attempt in 0..3 {
        match std::fs::rename(&temp_output, file) {
            Ok(()) => {
                rename_ok = true;
                break;
            }
            Err(e) if attempt < 2 => {
                tracing::warn!(
                    "Failed to replace file (attempt {}): {}, retrying...",
                    attempt + 1,
                    e
                );
                tokio::time::sleep(std::time::Duration::from_millis(500 * (attempt as u64 + 1)))
                    .await;
            }
            Err(e) => {
                let _ = std::fs::remove_file(&temp_output);
                return Err(anyhow!("Failed to replace file after 3 attempts: {}", e));
            }
        }
    }
    if !rename_ok {
        let _ = std::fs::remove_file(&temp_output);
        return Err(anyhow!("Failed to replace file"));
    }

    Ok(())
}

async fn download_thumbnail(
    client: &reqwest::Client,
    url: &str,
    dest_dir: &Path,
) -> anyhow::Result<std::path::PathBuf> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| anyhow!("Failed to download thumbnail: {}", e))?;

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_string();

    let bytes = response
        .bytes()
        .await
        .map_err(|e| anyhow!("Failed to read thumbnail: {}", e))?;

    let ext = if content_type.contains("png") {
        "png"
    } else {
        "jpg"
    };

    let thumb_path = dest_dir.join(format!(".omniget_thumb_{}.{}", uuid::Uuid::new_v4(), ext));
    std::fs::write(&thumb_path, &bytes)?;

    if ext == "png" {
        let jpg_path = dest_dir.join(format!(".omniget_thumb_{}.jpg", uuid::Uuid::new_v4()));
        let convert_result = crate::core::process::command("ffmpeg")
            .args([
                "-y",
                "-i",
                &thumb_path.to_string_lossy(),
                &jpg_path.to_string_lossy(),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;

        let _ = std::fs::remove_file(&thumb_path);

        if let Ok(status) = convert_result {
            if status.success() {
                return Ok(jpg_path);
            }
        }
        let _ = std::fs::remove_file(&jpg_path);
        return Err(anyhow!("Failed to convert thumbnail to JPEG"));
    }

    Ok(thumb_path)
}
