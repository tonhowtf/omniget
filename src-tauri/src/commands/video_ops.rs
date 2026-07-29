use omniget_core::core::ffmpeg_ops;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::Emitter;

#[derive(Clone, Serialize)]
pub struct VideoOpProposal {
    pub args: Vec<String>,
    pub out_ext: String,
}

#[derive(Clone, Serialize)]
pub struct VideoOpResult {
    pub output_path: String,
}

fn ensure_input(input: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(input);
    if !p.is_file() {
        return Err("Input file not found".to_string());
    }
    Ok(p)
}

fn output_for(input: &Path, tag: &str, ext: &str) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string());
    let dir = input.parent().map(PathBuf::from).unwrap_or_default();
    dir.join(format!("{stem}.{tag}.{ext}"))
}

#[cfg(not(target_os = "android"))]
async fn run_ffmpeg(
    app: &tauri::AppHandle,
    input: &Path,
    args: &[String],
    output: &Path,
) -> Result<(), String> {
    let ffmpeg = omniget_core::core::dependencies::find_tool("ffmpeg")
        .await
        .ok_or_else(|| "ffmpeg unavailable".to_string())?;

    let _ = app.emit("video-op-progress", "running");

    let mut cmd = omniget_core::core::process::command(&ffmpeg);
    cmd.arg("-y")
        .arg("-i")
        .arg(input.to_string_lossy().to_string());
    for a in args {
        cmd.arg(a);
    }
    cmd.arg(output.to_string_lossy().to_string());

    let status = cmd
        .status()
        .await
        .map_err(|e| format!("ffmpeg failed to start: {}", e))?;

    if !status.success() {
        let _ = app.emit("video-op-progress", "error");
        return Err("ffmpeg processing failed".to_string());
    }
    let _ = app.emit("video-op-progress", "done");
    Ok(())
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn detect_shot_changes(
    input: String,
    threshold: Option<f64>,
) -> Result<Vec<f64>, String> {
    let input_path = ensure_input(&input)?;
    let ffmpeg = omniget_core::core::dependencies::find_tool("ffmpeg")
        .await
        .ok_or_else(|| "ffmpeg unavailable".to_string())?;
    let thr = threshold.unwrap_or(0.4).clamp(0.1, 0.9);
    let output = omniget_core::core::process::command(&ffmpeg)
        .arg("-i")
        .arg(input_path.to_string_lossy().to_string())
        .arg("-filter:v")
        .arg(format!("select='gt(scene,{})',showinfo", thr))
        .arg("-an")
        .arg("-f")
        .arg("null")
        .arg("-")
        .output()
        .await
        .map_err(|e| format!("ffmpeg failed: {}", e))?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    Ok(ffmpeg_ops::parse_scene_times(&stderr))
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn waveform_peaks(input: String, buckets: Option<usize>) -> Result<Vec<f32>, String> {
    let input_path = ensure_input(&input)?;
    let ffmpeg = omniget_core::core::dependencies::find_tool("ffmpeg")
        .await
        .ok_or_else(|| "ffmpeg unavailable".to_string())?;
    let n = buckets.unwrap_or(2000).clamp(100, 20000);
    let output = omniget_core::core::process::command(&ffmpeg)
        .arg("-i")
        .arg(input_path.to_string_lossy().to_string())
        .arg("-ac")
        .arg("1")
        .arg("-ar")
        .arg("2000")
        .arg("-f")
        .arg("s16le")
        .arg("-")
        .output()
        .await
        .map_err(|e| format!("ffmpeg failed: {}", e))?;
    if !output.status.success() {
        return Err("ffmpeg audio extraction failed".to_string());
    }
    Ok(ffmpeg_ops::pcm_s16le_peaks(&output.stdout, n))
}

/// Quanto silencio o Smart Speed removeria, sem escrever arquivo nenhum.
///
/// Preview antes de aplicar: o usuario ve o ganho de duracao e decide, em vez
/// de esperar uma conversao inteira para descobrir que nao valia a pena.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SilenceEstimate {
    pub total_secs: f64,
    pub silence_secs: f64,
    pub saved_percent: f64,
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn video_op_silence_estimate(input: String) -> Result<SilenceEstimate, String> {
    let input_path = ensure_input(&input)?;
    let ffmpeg = omniget_core::core::dependencies::find_tool("ffmpeg")
        .await
        .ok_or_else(|| "ffmpeg unavailable".to_string())?;

    let total_us = omniget_core::core::ffmpeg::get_duration_us(&input_path)
        .await
        .map_err(|e| format!("could not read duration: {}", e))?;
    let total_secs = total_us as f64 / 1_000_000.0;

    let mut cmd = omniget_core::core::process::command(&ffmpeg);
    cmd.arg("-i").arg(input_path.to_string_lossy().to_string());
    for arg in ffmpeg_ops::silence_probe_args() {
        cmd.arg(arg);
    }
    let output = cmd
        .arg("-")
        .output()
        .await
        .map_err(|e| format!("ffmpeg failed: {}", e))?;

    // silencedetect escreve no stderr mesmo em execucao bem sucedida.
    let stderr = String::from_utf8_lossy(&output.stderr);
    let silence_secs = ffmpeg_ops::parse_silence_total_secs(&stderr);
    let saved_percent = if total_secs > 0.0 {
        (silence_secs / total_secs * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };

    Ok(SilenceEstimate {
        total_secs,
        silence_secs,
        saved_percent,
    })
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn video_op_preset(
    app: tauri::AppHandle,
    input: String,
    action: String,
    start: Option<String>,
    end: Option<String>,
) -> Result<VideoOpResult, String> {
    let input_path = ensure_input(&input)?;
    let p = ffmpeg_ops::preset(&action, start.as_deref(), end.as_deref())?;
    // Defense in depth: presets are built safely but still re-validated.
    ffmpeg_ops::validate_transform_args(&p.args)?;
    let output = output_for(&input_path, &action, p.out_ext);
    run_ffmpeg(&app, &input_path, &p.args, &output).await?;
    Ok(VideoOpResult {
        output_path: output.to_string_lossy().to_string(),
    })
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn video_op_propose(instruction: String) -> Result<VideoOpProposal, String> {
    let cfg = omniget_core::core::ai::get();
    if !cfg.is_configured() {
        return Err("ai_not_configured".to_string());
    }
    let system = "You convert a user's video-editing request into a single-line ffmpeg command. \
Use exactly 'ffmpeg -i input <options> output.<ext>'. Choose a sensible output extension. \
Only use codec/filter/trim options. No shell operators, no extra text, output ONLY the command.";
    let raw = omniget_core::core::ai::chat(system, &instruction).await?;
    let args = ffmpeg_ops::sanitize_ai_command(&raw)?;
    // Best-effort extension guess from the model's output line.
    let out_ext = raw
        .rsplit('.')
        .next()
        .map(|e| e.trim().to_lowercase())
        .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()) && e.len() <= 4)
        .unwrap_or_else(|| "mp4".to_string());
    Ok(VideoOpProposal { args, out_ext })
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn video_op_run(
    app: tauri::AppHandle,
    input: String,
    args: Vec<String>,
    out_ext: String,
) -> Result<VideoOpResult, String> {
    let input_path = ensure_input(&input)?;
    // The args came back from the frontend after user review — never trust
    // them; re-run the allowlist before touching ffmpeg.
    ffmpeg_ops::validate_transform_args(&args)?;
    let ext: String = out_ext
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(4)
        .collect();
    let ext = if ext.is_empty() {
        "mp4".to_string()
    } else {
        ext
    };
    let output = output_for(&input_path, "ai", &ext);
    run_ffmpeg(&app, &input_path, &args, &output).await?;
    Ok(VideoOpResult {
        output_path: output.to_string_lossy().to_string(),
    })
}
