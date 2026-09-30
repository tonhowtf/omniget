//! Downloads › Tools › Even out volume: EBU R128 loudness normalisation for
//! files already on disk, several at a time.
//!
//! Idea from YTSage, which can run `loudnorm` on audio as it downloads. Here
//! it works on any downloaded audio or video, with ffmpeg's two-pass
//! `loudnorm`: the first pass measures, the second applies a linear gain with
//! the measured values, so the sound is not pumped the way a one-pass
//! normaliser does. The original is kept; the result is written next to it.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Loudness targets offered in the tool, in LUFS.
const TARGETS: &[f64] = &[-14.0, -16.0, -23.0];
const TRUE_PEAK: f64 = -1.5;
const LRA: f64 = 11.0;

#[derive(Debug, Clone, Deserialize, PartialEq)]
struct Measured {
    input_i: String,
    input_tp: String,
    input_lra: String,
    input_thresh: String,
    target_offset: String,
    #[serde(default)]
    output_i: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NormalizeResult {
    pub input: String,
    pub output: Option<String>,
    /// Integrated loudness before, in LUFS.
    pub before_lufs: Option<f64>,
    /// Integrated loudness of the file written, in LUFS.
    pub after_lufs: Option<f64>,
    pub error: Option<String>,
}

/// The JSON block `loudnorm=print_format=json` writes at the end of stderr.
fn parse_measurement(stderr: &str) -> Option<Measured> {
    let start = stderr.rfind("[Parsed_loudnorm")?;
    let tail = &stderr[start..];
    let open = tail.find('{')?;
    let close = tail[open..].find('}')? + open;
    serde_json::from_str(&tail[open..=close]).ok()
}

fn loudnorm(target: f64) -> String {
    format!("loudnorm=I={target}:TP={TRUE_PEAK}:LRA={LRA}")
}

fn second_pass_filter(target: f64, m: &Measured) -> String {
    format!(
        "{}:measured_I={}:measured_TP={}:measured_LRA={}:measured_thresh={}:offset={}:linear=true:print_format=json",
        loudnorm(target),
        m.input_i,
        m.input_tp,
        m.input_lra,
        m.input_thresh,
        m.target_offset
    )
}

/// Encoder for the audio of the output, picked from the container so the
/// file keeps its format. Video streams are copied untouched.
fn audio_codec_args(ext: &str) -> Option<Vec<&'static str>> {
    Some(match ext {
        "mp3" => vec!["-c:a", "libmp3lame", "-q:a", "2"],
        "m4a" | "aac" | "mp4" | "m4v" | "mov" | "mkv" => vec!["-c:a", "aac", "-b:a", "192k"],
        "opus" | "webm" => vec!["-c:a", "libopus", "-b:a", "160k"],
        "ogg" | "oga" => vec!["-c:a", "libvorbis", "-q:a", "6"],
        "flac" => vec!["-c:a", "flac"],
        "wav" => vec!["-c:a", "pcm_s16le"],
        _ => return None,
    })
}

fn output_path(input: &Path, target: f64) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string());
    let ext = input
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir = input.parent().map(PathBuf::from).unwrap_or_default();
    let tag = format!("{}LUFS", target.abs() as i64);
    let mut out = dir.join(format!("{stem} ({tag}).{ext}"));
    let mut n = 2;
    while out.exists() {
        out = dir.join(format!("{stem} ({tag} {n}).{ext}"));
        n += 1;
    }
    out
}

async fn normalize_one(
    ffmpeg: &Path,
    input: &Path,
    target: f64,
) -> Result<NormalizeResult, String> {
    let ext = input
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let codec = audio_codec_args(&ext).ok_or_else(|| format!("Unsupported file type: .{ext}"))?;

    // Pass 1: measure.
    let probe = omniget_core::core::process::command(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(input)
        .args(["-map", "0:a:0", "-af"])
        .arg(format!("{}:print_format=json", loudnorm(target)))
        .args(["-f", "null", "-"])
        .output()
        .await
        .map_err(|e| format!("ffmpeg failed to start: {e}"))?;
    let measured = parse_measurement(&String::from_utf8_lossy(&probe.stderr))
        .ok_or_else(|| "This file has no audio that can be measured".to_string())?;

    // Pass 2: apply, copying video and other streams.
    let output = output_path(input, target);
    let run = omniget_core::core::process::command(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-y", "-i"])
        .arg(input)
        .args(["-map", "0", "-c", "copy", "-af"])
        .arg(second_pass_filter(target, &measured))
        .args(&codec)
        .arg(&output)
        .output()
        .await
        .map_err(|e| format!("ffmpeg failed to start: {e}"))?;
    if !run.status.success() {
        let _ = std::fs::remove_file(&output);
        return Err("ffmpeg could not write the file".to_string());
    }
    let after = parse_measurement(&String::from_utf8_lossy(&run.stderr));
    Ok(NormalizeResult {
        input: input.to_string_lossy().to_string(),
        output: Some(output.to_string_lossy().to_string()),
        before_lufs: measured.input_i.parse().ok(),
        // The second pass prints its own measurement; `output_i` is the level written.
        after_lufs: after.and_then(|m| m.output_i).and_then(|v| v.parse().ok()),
        error: None,
    })
}

#[tauri::command]
pub async fn normalize_loudness(
    inputs: Vec<String>,
    target_lufs: f64,
) -> Result<Vec<NormalizeResult>, String> {
    if !TARGETS
        .iter()
        .any(|t| (t - target_lufs).abs() < f64::EPSILON)
    {
        return Err("Unsupported loudness target".to_string());
    }
    let ffmpeg = omniget_core::core::dependencies::find_tool("ffmpeg")
        .await
        .ok_or_else(|| "ffmpeg unavailable".to_string())?;
    let mut results = Vec::with_capacity(inputs.len());
    for input in inputs {
        let path = PathBuf::from(&input);
        let result = if !path.is_file() {
            Err("File not found".to_string())
        } else {
            normalize_one(&ffmpeg, &path, target_lufs).await
        };
        results.push(result.unwrap_or_else(|error| NormalizeResult {
            input,
            output: None,
            before_lufs: None,
            after_lufs: None,
            error: Some(error),
        }));
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STDERR: &str = r#"size=N/A time=00:00:10.00 bitrate=N/A speed= 500x
[Parsed_loudnorm_0 @ 0x600000c9c000]
{
	"input_i" : "-27.61",
	"input_tp" : "-9.34",
	"input_lra" : "5.60",
	"input_thresh" : "-38.02",
	"output_i" : "-16.20",
	"output_tp" : "-1.50",
	"output_lra" : "4.40",
	"output_thresh" : "-26.55",
	"normalization_type" : "dynamic",
	"target_offset" : "0.20"
}
"#;

    #[test]
    fn reads_the_loudnorm_json() {
        let m = parse_measurement(STDERR).unwrap();
        assert_eq!(m.input_i, "-27.61");
        assert_eq!(m.target_offset, "0.20");
        assert_eq!(m.output_i.as_deref(), Some("-16.20"));
    }

    #[test]
    fn no_json_means_no_audio() {
        assert!(parse_measurement("Output file does not contain any stream").is_none());
    }

    #[test]
    fn second_pass_carries_the_measurement() {
        let f = second_pass_filter(-16.0, &parse_measurement(STDERR).unwrap());
        assert!(f.starts_with("loudnorm=I=-16:TP=-1.5:LRA=11:measured_I=-27.61:"));
        assert!(f.contains("offset=0.20:linear=true"));
    }

    #[test]
    fn keeps_the_container() {
        assert_eq!(audio_codec_args("mp3").unwrap()[1], "libmp3lame");
        assert_eq!(audio_codec_args("webm").unwrap()[1], "libopus");
        assert!(audio_codec_args("txt").is_none());
    }

    #[test]
    fn output_sits_next_to_the_input() {
        let out = output_path(Path::new("/tmp/nope/song.mp3"), -14.0);
        assert_eq!(out, PathBuf::from("/tmp/nope/song (14LUFS).mp3"));
    }
}
