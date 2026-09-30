//! Downloads › Tools › Trim with preview: open a video, see a strip of frames,
//! drag the start and the end, look at the exact frame under each handle, and
//! save only that part.
//!
//! The idea comes from Media Downloader for macOS (pixel-point), which opens a
//! trim panel with a frame strip after every download and cuts on the exact
//! frame by re-encoding. That project has no license, so nothing was copied:
//! this is written from scratch on the ffmpeg the app already uses. The fast
//! cut (no re-encode, starts on a keyframe) is the one the video tools of each
//! download already offer; this panel adds the preview and the exact cut.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde::Serialize;

/// Frames in the strip. Enough to find a scene, few enough to load fast.
const STRIP_FRAMES: usize = 12;
/// Height of a strip frame; the big preview frame is taller.
const STRIP_HEIGHT: u32 = 90;
const PREVIEW_HEIGHT: u32 = 360;
/// Shortest clip the exact cut accepts, in seconds.
const MIN_CLIP_SECS: f64 = 0.25;

#[derive(Debug, Clone, Serialize)]
pub struct TrimStrip {
    pub duration_secs: f64,
    /// `data:image/jpeg;base64,...`, evenly spaced over the video.
    pub frames: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrimResult {
    pub output_path: String,
}

fn ensure_input(input: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(input);
    if !p.is_file() {
        return Err("Input file not found".to_string());
    }
    Ok(p)
}

/// `Duration: 00:01:02.50,` in ffmpeg's banner, in seconds.
pub fn parse_duration(stderr: &str) -> Option<f64> {
    let rest = stderr.split("Duration: ").nth(1)?;
    let stamp = rest.split(',').next()?.trim();
    let mut secs = 0.0;
    for part in stamp.split(':') {
        secs = secs * 60.0 + part.parse::<f64>().ok()?;
    }
    (secs > 0.0).then_some(secs)
}

/// Seconds as `HH:MM:SS.mmm` for ffmpeg and for file names.
pub fn stamp(secs: f64) -> String {
    let ms = (secs.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        ms / 3_600_000,
        (ms / 60_000) % 60,
        (ms / 1000) % 60,
        ms % 1000
    )
}

/// `<stem> trim 00-01-02.500-00-01-10.000.<ext>` next to the source, never over
/// an existing file.
pub fn output_path(input: &Path, start: f64, end: f64, ext: &str) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "video".to_string());
    let dir = input.parent().map(PathBuf::from).unwrap_or_default();
    let range = format!("{}-{}", stamp(start), stamp(end)).replace(':', "-");
    let mut candidate = dir.join(format!("{stem} trim {range}.{ext}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem} trim {range} ({n}).{ext}"));
        n += 1;
    }
    candidate
}

/// The ffmpeg arguments for a cut, after `-y`.
///
/// Exact: `-ss` after `-i` decodes from the start and cuts on the frame asked
/// for, then re-encodes to H.264/AAC. Fast: `-ss` before `-i` jumps to the
/// keyframe at or before the start and copies the streams, with no quality loss.
pub fn cut_args(input: &Path, start: f64, end: f64, exact: bool, output: &Path) -> Vec<String> {
    let input = input.to_string_lossy().to_string();
    let mut a: Vec<String> = Vec::new();
    if exact {
        a.extend(["-i".into(), input, "-ss".into(), stamp(start)]);
        a.extend(["-t".into(), stamp(end - start)]);
        a.extend(
            [
                "-map",
                "0:v:0",
                "-map",
                "0:a?",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                "-movflags",
                "+faststart",
            ]
            .map(String::from),
        );
    } else {
        a.extend(["-ss".into(), stamp(start), "-i".into(), input]);
        a.extend(["-t".into(), stamp(end - start)]);
        a.extend(["-map", "0", "-c", "copy", "-avoid_negative_ts", "make_zero"].map(String::from));
    }
    a.push(output.to_string_lossy().to_string());
    a
}

#[cfg(not(target_os = "android"))]
async fn ffmpeg() -> Result<PathBuf, String> {
    omniget_core::core::dependencies::find_tool("ffmpeg")
        .await
        .ok_or_else(|| "ffmpeg unavailable".to_string())
}

#[cfg(not(target_os = "android"))]
async fn duration(ffmpeg: &Path, input: &Path) -> Result<f64, String> {
    let out = omniget_core::core::process::command(ffmpeg)
        .arg("-hide_banner")
        .arg("-i")
        .arg(input)
        .output()
        .await
        .map_err(|e| format!("ffmpeg failed to start: {e}"))?;
    parse_duration(&String::from_utf8_lossy(&out.stderr))
        .ok_or_else(|| "Could not read the length of this file".to_string())
}

#[cfg(not(target_os = "android"))]
async fn frame(ffmpeg: &Path, input: &Path, at: f64, height: u32) -> Result<String, String> {
    let out = omniget_core::core::process::command(ffmpeg)
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &stamp(at),
            "-i",
        ])
        .arg(input)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &format!("scale=-2:{height}"),
            "-q:v",
            "5",
            "-f",
            "image2pipe",
            "-vcodec",
            "mjpeg",
            "-",
        ])
        .output()
        .await
        .map_err(|e| format!("ffmpeg failed to start: {e}"))?;
    if !out.status.success() || out.stdout.is_empty() {
        return Err("Could not read a frame of this file".to_string());
    }
    Ok(format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&out.stdout)
    ))
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn trim_strip(input: String) -> Result<TrimStrip, String> {
    let input = ensure_input(&input)?;
    let ffmpeg = ffmpeg().await?;
    let duration_secs = duration(&ffmpeg, &input).await?;
    let step = duration_secs / STRIP_FRAMES as f64;
    let mut frames = Vec::with_capacity(STRIP_FRAMES);
    for i in 0..STRIP_FRAMES {
        // The middle of each slice: the very first frame is often black.
        frames.push(frame(&ffmpeg, &input, step * (i as f64 + 0.5), STRIP_HEIGHT).await?);
    }
    Ok(TrimStrip {
        duration_secs,
        frames,
    })
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn trim_frame(input: String, at: f64) -> Result<String, String> {
    let input = ensure_input(&input)?;
    frame(&ffmpeg().await?, &input, at, PREVIEW_HEIGHT).await
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn trim_export(
    input: String,
    start: f64,
    end: f64,
    exact: bool,
) -> Result<TrimResult, String> {
    let input = ensure_input(&input)?;
    if !(start.is_finite() && end.is_finite()) || start < 0.0 || end - start < MIN_CLIP_SECS {
        return Err("Pick a part of at least a quarter of a second".to_string());
    }
    let ext = if exact {
        "mp4".to_string()
    } else {
        input
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| "mp4".to_string())
    };
    let output = output_path(&input, start, end, &ext);
    let status = omniget_core::core::process::command(ffmpeg().await?)
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(cut_args(&input, start, end, exact, &output))
        .status()
        .await
        .map_err(|e| format!("ffmpeg failed to start: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&output);
        return Err("ffmpeg processing failed".to_string());
    }
    Ok(TrimResult {
        output_path: output.to_string_lossy().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_duration_from_the_banner() {
        let banner =
            "Input #0, mov,mp4\n  Duration: 00:12:14.08, start: 0.000000, bitrate: 4064 kb/s";
        assert_eq!(parse_duration(banner), Some(734.08));
        assert_eq!(parse_duration("  Duration: N/A, bitrate: N/A"), None);
        assert_eq!(parse_duration("nothing here"), None);
    }

    #[test]
    fn stamps_seconds() {
        assert_eq!(stamp(0.0), "00:00:00.000");
        assert_eq!(stamp(62.5), "00:01:02.500");
        assert_eq!(stamp(3725.0415), "01:02:05.042");
    }

    #[test]
    fn exact_cut_seeks_after_the_input_and_reencodes() {
        let a = cut_args(
            Path::new("/v/in.webm"),
            1.0,
            3.5,
            true,
            Path::new("/v/out.mp4"),
        );
        let i = a.iter().position(|x| x == "-i").unwrap();
        let ss = a.iter().position(|x| x == "-ss").unwrap();
        assert!(ss > i);
        assert!(a.contains(&"libx264".to_string()));
        assert_eq!(
            a[a.iter().position(|x| x == "-t").unwrap() + 1],
            "00:00:02.500"
        );
    }

    #[test]
    fn fast_cut_seeks_before_the_input_and_copies() {
        let a = cut_args(
            Path::new("/v/in.mp4"),
            10.0,
            20.0,
            false,
            Path::new("/v/out.mp4"),
        );
        let i = a.iter().position(|x| x == "-i").unwrap();
        let ss = a.iter().position(|x| x == "-ss").unwrap();
        assert!(ss < i);
        assert!(a.windows(2).any(|w| w[0] == "-c" && w[1] == "copy"));
    }

    #[test]
    fn names_the_clip_next_to_the_source() {
        let p = output_path(Path::new("/nowhere/My video.mp4"), 1.0, 2.0, "mp4");
        assert_eq!(
            p,
            PathBuf::from("/nowhere/My video trim 00-00-01.000-00-00-02.000.mp4")
        );
    }
}
