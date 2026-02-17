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
