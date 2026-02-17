use std::path::Path;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub async fn is_ffmpeg_available() -> bool {
    tokio::process::Command::new("ffmpeg")
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

pub async fn mux_video_audio(video: &Path, audio: &Path, output: &Path) -> anyhow::Result<()> {
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let status = tokio::process::Command::new("ffmpeg")
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
        .map_err(|e| anyhow!("Falha ao executar ffmpeg: {}", e))?;

    if !status.success() {
        return Err(anyhow!("ffmpeg retornou código {}", status));
    }

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionOptions {
    pub input_path: String,
    pub output_path: String,
    pub video_codec: Option<String>,
    pub audio_codec: Option<String>,
    pub resolution: Option<String>,
    pub video_bitrate: Option<String>,
    pub audio_bitrate: Option<String>,
    pub sample_rate: Option<u32>,
    pub fps: Option<f64>,
    pub trim_start: Option<String>,
    pub trim_end: Option<String>,
    pub additional_input_args: Option<Vec<String>>,
    pub additional_output_args: Option<Vec<String>>,
    pub preset: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionResult {
    pub success: bool,
    pub output_path: String,
    pub file_size_bytes: u64,
    pub duration_seconds: f64,
    pub error: Option<String>,
}

pub async fn probe(path: &Path) -> anyhow::Result<MediaProbeInfo> {
    let output = tokio::process::Command::new("ffprobe")
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
        .map_err(|e| anyhow!("Falha ao executar ffprobe: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("ffprobe falhou: {}", stderr));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| anyhow!("Falha ao parsear JSON do ffprobe: {}", e))?;

    let format = json.get("format").ok_or_else(|| anyhow!("Sem campo 'format'"))?;

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

pub async fn convert(
    opts: &ConversionOptions,
    cancel_token: CancellationToken,
    progress_tx: mpsc::Sender<f64>,
) -> anyhow::Result<ConversionResult> {
    let input_path = Path::new(&opts.input_path);
    let output_path = Path::new(&opts.output_path);

    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let total_duration_us = get_duration_us(input_path).await.unwrap_or(0);

    let mut args: Vec<String> = vec!["-y".to_string()];

    if let Some(ref start) = opts.trim_start {
        args.extend(["-ss".to_string(), start.clone()]);
    }

    if let Some(ref extra) = opts.additional_input_args {
        args.extend(extra.clone());
    }

    args.extend(["-i".to_string(), opts.input_path.clone()]);

    if let Some(ref end) = opts.trim_end {
        args.extend(["-to".to_string(), end.clone()]);
    }

    if let Some(ref codec) = opts.video_codec {
        args.extend(["-c:v".to_string(), codec.clone()]);
    }

    if let Some(ref codec) = opts.audio_codec {
        args.extend(["-c:a".to_string(), codec.clone()]);
    }

    if let Some(ref res) = opts.resolution {
        args.extend(["-s".to_string(), res.clone()]);
    }

    if let Some(ref br) = opts.video_bitrate {
        args.extend(["-b:v".to_string(), br.clone()]);
    }

    if let Some(ref br) = opts.audio_bitrate {
        args.extend(["-b:a".to_string(), br.clone()]);
    }

    if let Some(sr) = opts.sample_rate {
        args.extend(["-ar".to_string(), sr.to_string()]);
    }

    if let Some(fps) = opts.fps {
        args.extend(["-r".to_string(), fps.to_string()]);
    }

    if let Some(ref preset) = opts.preset {
        args.extend(["-preset".to_string(), preset.clone()]);
    }

    if let Some(ref extra) = opts.additional_output_args {
        args.extend(extra.clone());
    }

    args.extend([
        "-progress".to_string(),
        "pipe:1".to_string(),
        "-nostats".to_string(),
        opts.output_path.clone(),
    ]);

    let mut child = tokio::process::Command::new("ffmpeg")
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| anyhow!("Falha ao iniciar ffmpeg: {}", e))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Sem stdout do ffmpeg"))?;
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();

    let cancel = cancel_token.clone();
    let progress = progress_tx.clone();
    let line_reader = tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            if cancel.is_cancelled() {
                break;
            }
            if let Some(us) = parse_out_time_us(&line) {
                if total_duration_us > 0 {
                    let pct = (us as f64 / total_duration_us as f64 * 100.0).min(100.0);
                    let _ = progress.send(pct).await;
                }
            }
        }
    });

    let result = tokio::select! {
        status = child.wait() => {
            let _ = line_reader.await;
            status.map_err(|e| anyhow!("ffmpeg processo falhou: {}", e))
        }
        _ = cancel_token.cancelled() => {
            let _ = child.kill().await;
            let _ = line_reader.await;
            return Ok(ConversionResult {
                success: false,
                output_path: opts.output_path.clone(),
                file_size_bytes: 0,
                duration_seconds: 0.0,
                error: Some("Conversão cancelada".to_string()),
            });
        }
    };

    match result {
        Ok(status) if status.success() => {
            let _ = progress_tx.send(100.0).await;
            let meta = tokio::fs::metadata(output_path).await;
            let file_size = meta.map(|m| m.len()).unwrap_or(0);

            let duration = probe(output_path)
                .await
                .map(|i| i.duration_seconds)
                .unwrap_or(0.0);

            Ok(ConversionResult {
                success: true,
                output_path: opts.output_path.clone(),
                file_size_bytes: file_size,
                duration_seconds: duration,
                error: None,
            })
        }
        Ok(status) => Ok(ConversionResult {
            success: false,
            output_path: opts.output_path.clone(),
            file_size_bytes: 0,
            duration_seconds: 0.0,
            error: Some(format!("ffmpeg saiu com código {}", status)),
        }),
        Err(e) => Ok(ConversionResult {
            success: false,
            output_path: opts.output_path.clone(),
            file_size_bytes: 0,
            duration_seconds: 0.0,
            error: Some(e.to_string()),
        }),
    }
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
        return Err(anyhow!("ffmpeg não disponível"));
    }

    let temp_dir = file.parent().unwrap_or(Path::new("."));
    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("mp4");
    let temp_output = temp_dir.join(format!(
        ".omniget_meta_{}.{}",
        uuid::Uuid::new_v4(),
        ext
    ));

    let is_audio_only = matches!(
        ext.to_lowercase().as_str(),
        "mp3" | "m4a" | "aac" | "ogg" | "opus" | "flac" | "wav" | "wma"
    );

    let thumbnail_path = if embed_thumbnail && is_audio_only {
        if let Some(ref url) = metadata.thumbnail_url {
            match download_thumbnail(http_client, url, temp_dir).await {
                Ok(p) => Some(p),
                Err(e) => {
                    tracing::warn!("Falha ao baixar thumbnail: {}", e);
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    let mut args: Vec<String> = vec!["-y".to_string(), "-i".to_string(), file.to_string_lossy().to_string()];

    if let Some(ref thumb) = thumbnail_path {
        args.extend(["-i".to_string(), thumb.to_string_lossy().to_string()]);
    }

    if let Some(ref thumb) = thumbnail_path {
        let _ = thumb;
        args.extend([
            "-map".to_string(), "0:a".to_string(),
            "-map".to_string(), "1:v".to_string(),
            "-c".to_string(), "copy".to_string(),
            "-disposition:v:0".to_string(), "attached_pic".to_string(),
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

    let output = tokio::process::Command::new("ffmpeg")
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| anyhow!("Falha ao executar ffmpeg: {}", e))?;

    if let Some(ref thumb) = thumbnail_path {
        let _ = tokio::fs::remove_file(thumb).await;
    }

    if !output.status.success() {
        let _ = tokio::fs::remove_file(&temp_output).await;
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("ffmpeg metadata falhou: {}", stderr));
    }

    tokio::fs::rename(&temp_output, file)
        .await
        .map_err(|e| anyhow!("Falha ao substituir arquivo: {}", e))?;

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
        .map_err(|e| anyhow!("Falha ao baixar thumbnail: {}", e))?;

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_string();

    let bytes = response
        .bytes()
        .await
        .map_err(|e| anyhow!("Falha ao ler thumbnail: {}", e))?;

    let ext = if content_type.contains("png") {
        "png"
    } else {
        "jpg"
    };

    let thumb_path = dest_dir.join(format!(".omniget_thumb_{}.{}", uuid::Uuid::new_v4(), ext));
    tokio::fs::write(&thumb_path, &bytes).await?;

    if ext == "png" {
        let jpg_path = dest_dir.join(format!(".omniget_thumb_{}.jpg", uuid::Uuid::new_v4()));
        let convert_result = tokio::process::Command::new("ffmpeg")
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

        let _ = tokio::fs::remove_file(&thumb_path).await;

        if let Ok(status) = convert_result {
            if status.success() {
                return Ok(jpg_path);
            }
        }
        let _ = tokio::fs::remove_file(&jpg_path).await;
        return Err(anyhow!("Falha ao converter thumbnail para JPEG"));
    }

    Ok(thumb_path)
}

fn parse_out_time_us(line: &str) -> Option<u64> {
    let line = line.trim();
    if let Some(val) = line.strip_prefix("out_time_us=") {
        return val.trim().parse::<u64>().ok();
    }
    if let Some(val) = line.strip_prefix("out_time_ms=") {
        return val.trim().parse::<u64>().ok().map(|ms| ms * 1000);
    }
    None
}
