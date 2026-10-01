use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use omniget_core::core::ffmpeg;
use omniget_core::core::hwaccel;
use omniget_core::core::process;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReencodeCodec {
    Av1,
    Hevc,
    H264,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReencodeRequest {
    pub input_path: String,
    pub output_path: Option<String>,
    pub codec: ReencodeCodec,
    pub cq: Option<u32>,
    pub replace_original: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReencodeResult {
    pub output_path: String,
    pub encoder_used: String,
    pub used_hwaccel: bool,
    pub original_size_bytes: u64,
    pub new_size_bytes: u64,
    pub savings_pct: f64,
    pub replaced_original: bool,
}

#[tauri::command]
pub async fn reencode_video(req: ReencodeRequest) -> Result<ReencodeResult, String> {
    if !ffmpeg::is_ffmpeg_available().await {
        return Err("ffmpeg not found".to_string());
    }

    let input = PathBuf::from(&req.input_path);
    if !input.is_file() {
        return Err(format!("source not found: {}", req.input_path));
    }
    let original_size = std::fs::metadata(&input).map(|m| m.len()).unwrap_or(0);

    let output = req.output_path.clone().unwrap_or_else(|| {
        let stem = input.file_stem().and_then(|s| s.to_str()).unwrap_or("out");
        let parent = input
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let ext = match req.codec {
            ReencodeCodec::Av1 | ReencodeCodec::Hevc => "mp4",
            ReencodeCodec::H264 => "mp4",
        };
        parent
            .join(format!("{}.reencoded.{}", stem, ext))
            .to_string_lossy()
            .into_owned()
    });
    let output_path = PathBuf::from(&output);
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {}", e))?;
    }

    let hw = hwaccel::detect_hwaccel().await;

    let cq = req.cq.unwrap_or(match req.codec {
        ReencodeCodec::Av1 => 32,
        ReencodeCodec::Hevc => 28,
        ReencodeCodec::H264 => 22,
    });

    let mut failures: Vec<String> = Vec::new();
    let mut used: Option<(String, bool)> = None;
    for (encoder, is_hw) in encoder_ladder(&req.codec, &hw) {
        if is_hw && !encoder_works(&encoder, cq).await {
            failures.push(format!("{}: not usable on this machine", encoder));
            continue;
        }
        let _ = std::fs::remove_file(&output_path);
        match run_reencode(&input, &output_path, &encoder, is_hw, cq).await {
            Ok(()) => {
                used = Some((encoder, is_hw));
                break;
            }
            Err(e) => failures.push(format!("{}: {}", encoder, e)),
        }
    }
    let Some((encoder, used_hw)) = used else {
        let _ = std::fs::remove_file(&output_path);
        return Err(format!("ffmpeg reencode failed. {}", failures.join(" | ")));
    };
    if !failures.is_empty() {
        tracing::warn!(
            "[reencode] fell back to {}: {}",
            encoder,
            failures.join(" | ")
        );
    }
    if !output_path.is_file() {
        return Err("output file missing after ffmpeg".to_string());
    }

    let new_size = std::fs::metadata(&output_path)
        .map(|m| m.len())
        .unwrap_or(0);
    let savings_pct = if original_size > 0 {
        100.0 * (original_size as f64 - new_size as f64) / original_size as f64
    } else {
        0.0
    };

    let replace = req.replace_original.unwrap_or(false);
    let mut replaced = false;
    if replace && new_size > 0 && new_size < original_size {
        let backup = backup_path(&input);
        std::fs::rename(&input, &backup)
            .map_err(|e| format!("rename original→backup failed: {}", e))?;
        if let Err(e) = std::fs::rename(&output_path, &input) {
            let _ = std::fs::rename(&backup, &input);
            return Err(format!("rename new→original failed: {}", e));
        }
        let _ = std::fs::remove_file(&backup);
        replaced = true;
    }

    let final_path = if replaced {
        input.clone()
    } else {
        output_path.clone()
    };

    Ok(ReencodeResult {
        output_path: final_path.to_string_lossy().into_owned(),
        encoder_used: encoder,
        used_hwaccel: used_hw,
        original_size_bytes: original_size,
        new_size_bytes: new_size,
        savings_pct,
        replaced_original: replaced,
    })
}

fn encoder_ladder(codec: &ReencodeCodec, hw: &hwaccel::HwAccelInfo) -> Vec<(String, bool)> {
    let (hardware, software): (&[&str], &str) = match codec {
        ReencodeCodec::Av1 => (&["av1_nvenc", "av1_qsv", "av1_amf"], "libsvtav1"),
        ReencodeCodec::Hevc => (
            &["hevc_nvenc", "hevc_qsv", "hevc_amf", "hevc_videotoolbox"],
            "libx265",
        ),
        ReencodeCodec::H264 => (
            &["h264_nvenc", "h264_qsv", "h264_amf", "h264_videotoolbox"],
            "libx264",
        ),
    };
    hardware
        .iter()
        .filter(|enc| hw.encoders.iter().any(|e| e == *enc))
        .map(|enc| (enc.to_string(), true))
        .chain(std::iter::once((software.to_string(), false)))
        .collect()
}

fn quality_args(encoder: &str, cq: u32) -> Vec<String> {
    let q = cq.to_string();
    let args: Vec<&str> = if encoder.ends_with("_nvenc") {
        vec!["-rc", "vbr", "-cq", &q]
    } else if encoder.ends_with("_qsv") {
        vec!["-global_quality", &q]
    } else if encoder.ends_with("_amf") {
        vec!["-rc", "cqp", "-qp_i", &q, "-qp_p", &q]
    } else if encoder.ends_with("_videotoolbox") {
        return vec!["-q:v".to_string(), videotoolbox_quality(cq).to_string()];
    } else {
        vec!["-crf", &q]
    };
    args.into_iter().map(str::to_string).collect()
}

fn videotoolbox_quality(cq: u32) -> u32 {
    100u32.saturating_sub(cq.saturating_mul(2)).clamp(1, 100)
}

static ENCODER_CHECKS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, bool>>,
> = std::sync::OnceLock::new();

async fn encoder_works(encoder: &str, cq: u32) -> bool {
    let checks = ENCODER_CHECKS.get_or_init(Default::default);
    if let Some(ok) = checks.lock().ok().and_then(|m| m.get(encoder).copied()) {
        return ok;
    }
    let mut args: Vec<String> = [
        "-hide_banner",
        "-loglevel",
        "error",
        "-f",
        "lavfi",
        "-i",
        "color=c=black:s=256x144:r=30:d=0.2",
        "-frames:v",
        "5",
        "-c:v",
        encoder,
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    args.extend(quality_args(encoder, cq));
    args.extend(["-f", "null", "-"].map(str::to_string));
    let run = process::command("ffmpeg")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let ok = matches!(
        tokio::time::timeout(std::time::Duration::from_secs(10), run).await,
        Ok(Ok(status)) if status.success()
    );
    if let Ok(mut m) = checks.lock() {
        m.insert(encoder.to_string(), ok);
    }
    ok
}

async fn run_reencode(
    input: &Path,
    output: &Path,
    encoder: &str,
    is_hw: bool,
    cq: u32,
) -> Result<(), String> {
    let mut args: Vec<String> = vec!["-y".into(), "-hide_banner".into()];
    if is_hw {
        args.extend(["-hwaccel", "auto"].map(str::to_string));
    }
    args.extend(["-i".to_string(), input.to_string_lossy().into_owned()]);
    args.extend(["-c:v".to_string(), encoder.to_string()]);
    args.extend(quality_args(encoder, cq));
    if encoder.starts_with("hevc") || encoder == "libx265" {
        args.extend(["-tag:v", "hvc1"].map(str::to_string));
    }
    args.extend(["-c:a", "copy", "-movflags", "+faststart"].map(str::to_string));
    args.push(output.to_string_lossy().into_owned());

    let out = process::command("ffmpeg")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("spawn ffmpeg failed: {}", e))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let last = stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    Err(format!("exit {}: {}", out.status, last))
}

fn backup_path(p: &Path) -> PathBuf {
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
    let parent = p.parent().unwrap_or_else(|| Path::new("."));
    if ext.is_empty() {
        parent.join(format!("{}.backup", stem))
    } else {
        parent.join(format!("{}.backup.{}", stem, ext))
    }
}
