pub mod fonts;
pub mod graph;
pub mod job;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Aspect {
    #[default]
    #[serde(rename = "keep")]
    Keep,
    #[serde(rename = "9:16")]
    Vertical,
    #[serde(rename = "1:1")]
    Square,
    #[serde(rename = "4:5")]
    Portrait,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum FrameMode {
    #[default]
    Crop,
    Fit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Fast,
    #[default]
    Balanced,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reframe {
    #[serde(default)]
    pub aspect: Aspect,
    #[serde(default)]
    pub mode: FrameMode,
    #[serde(default = "half")]
    pub position: f64,
}

fn half() -> f64 {
    0.5
}

impl Default for Reframe {
    fn default() -> Self {
        Self {
            aspect: Aspect::Keep,
            mode: FrameMode::Crop,
            position: half(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intro {
    pub seconds: f64,
    #[serde(default)]
    pub title_lines: Vec<String>,
    #[serde(default)]
    pub subtitle_lines: Vec<String>,
    pub background: String,
    pub color: String,
    #[serde(default)]
    pub fade: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderRequest {
    pub input: String,
    #[serde(default)]
    pub out_dir: Option<String>,
    #[serde(default)]
    pub ass: Option<String>,
    #[serde(default)]
    pub reframe: Reframe,
    #[serde(default)]
    pub intro: Option<Intro>,
    #[serde(default)]
    pub fade_out: bool,
    #[serde(default)]
    pub quality: Quality,
}

#[derive(Debug, Clone, Serialize)]
pub struct RenderOutcome {
    pub output: String,
    pub size_bytes: u64,
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Probe {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_video: bool,
    pub has_audio: bool,
    pub video_codec: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Tools {
    pub ffmpeg: bool,
    pub subtitles: bool,
    pub drawtext: bool,
}

pub async fn tools() -> Tools {
    let out = crate::core::process::command("ffmpeg")
        .args(["-hide_banner", "-filters"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout);
            parse_filters(&text)
        }
        _ => Tools::default(),
    }
}

pub fn parse_filters(text: &str) -> Tools {
    let has = |name: &str| {
        text.lines()
            .any(|l| l.split_whitespace().nth(1) == Some(name))
    };
    Tools {
        ffmpeg: true,
        subtitles: has("subtitles"),
        drawtext: has("drawtext"),
    }
}

pub fn parse_rotation(json: &str) -> i64 {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return 0;
    };
    let Some(stream) = v
        .get("streams")
        .and_then(|s| s.as_array())
        .and_then(|a| a.first())
    else {
        return 0;
    };
    let from_side = stream
        .get("side_data_list")
        .and_then(|l| l.as_array())
        .and_then(|l| {
            l.iter()
                .find_map(|d| d.get("rotation").and_then(|r| r.as_f64()))
        });
    let from_tag = stream
        .get("tags")
        .and_then(|t| t.get("rotate"))
        .and_then(|r| r.as_str())
        .and_then(|s| s.trim().parse::<f64>().ok());
    from_side.or(from_tag).unwrap_or(0.0).round() as i64
}

async fn rotation(path: &Path) -> i64 {
    let out = crate::core::process::command("ffprobe")
        .args([
            "-v",
            "quiet",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream_tags=rotate:stream_side_data=rotation",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => parse_rotation(&String::from_utf8_lossy(&o.stdout)),
        _ => 0,
    }
}

pub async fn probe(path: &Path) -> Result<Probe, String> {
    let meta = std::fs::metadata(path).map_err(|_| {
        "The file could not be opened. It may have been moved or deleted.".to_string()
    })?;
    if !meta.is_file() {
        return Err("This is a folder, not a video. Pick a video file.".to_string());
    }
    if meta.len() == 0 {
        return Err("The file is empty (0 bytes). Pick a video that has content.".to_string());
    }
    let info = crate::core::ffmpeg::probe(path).await.map_err(|e| {
        let text = e.to_string();
        if text.contains("Failed to run ffprobe") {
            "FFmpeg is not installed, so Studio cannot read videos. Install FFmpeg in Settings > Dependencies.".to_string()
        } else {
            "FFmpeg could not read this file. It may be damaged or not a video.".to_string()
        }
    })?;
    let video = info
        .streams
        .iter()
        .find(|s| s.codec_type == "video" && s.width.unwrap_or(0) > 0);
    let has_audio = info.streams.iter().any(|s| s.codec_type == "audio");
    let Some(video) = video else {
        return Err(
            "This file has no picture. Studio works with videos; for audio only, use the Converter."
                .to_string(),
        );
    };
    let (mut w, mut h) = (video.width.unwrap_or(0), video.height.unwrap_or(0));
    if rotation(path).await.rem_euclid(180) == 90 {
        std::mem::swap(&mut w, &mut h);
    }
    let duration = if info.duration_seconds > 0.0 {
        info.duration_seconds
    } else {
        video.duration_seconds.unwrap_or(0.0)
    };
    Ok(Probe {
        path: path.to_string_lossy().into_owned(),
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size_bytes: meta.len(),
        duration_ms: (duration * 1000.0).round().max(0.0) as u64,
        width: w,
        height: h,
        fps: video.fps.unwrap_or(30.0),
        has_video: true,
        has_audio,
        video_codec: Some(video.codec_name.clone()).filter(|c| !c.is_empty()),
    })
}

pub fn unique_output(input: &Path, out_dir: Option<&Path>) -> PathBuf {
    let dir = out_dir
        .map(Path::to_path_buf)
        .or_else(|| input.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let stem = input
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "video".to_string());
    let mut n = 1u32;
    loop {
        let name = if n == 1 {
            format!("{stem} (studio).mp4")
        } else {
            format!("{stem} (studio {n}).mp4")
        };
        let candidate = dir.join(name);
        if !candidate.exists() && candidate != input {
            return candidate;
        }
        n += 1;
    }
}

pub fn is_subtitle_path(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "srt" | "vtt" | "ass"))
        .unwrap_or(false)
}

pub fn partial_path(input: &Path, out_dir: Option<&Path>) -> PathBuf {
    let target = unique_output(input, out_dir);
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "video.mp4".to_string());
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    target.with_file_name(format!(".{name}.{}-{stamp}.part", std::process::id()))
}

pub fn publish(partial: &Path, input: &Path, out_dir: Option<&Path>) -> Result<PathBuf, String> {
    for _ in 0..64 {
        let target = unique_output(input, out_dir);
        match std::fs::hard_link(partial, &target) {
            Ok(()) => {
                let _ = std::fs::remove_file(partial);
                return Ok(target);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists || target.exists() => continue,
            Err(_) => {
                return std::fs::rename(partial, &target)
                    .map(|_| target)
                    .map_err(|e| format!("Studio could not name the finished video: {e}."));
            }
        }
    }
    Err("Studio could not find a free name for the finished video.".to_string())
}

pub const FONT_REGULAR: &[u8] =
    include_bytes!("../../../../../static/vendor/pdfjs/standard_fonts/LiberationSans-Regular.ttf");
pub const FONT_BOLD: &[u8] =
    include_bytes!("../../../../../static/vendor/pdfjs/standard_fonts/LiberationSans-Bold.ttf");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_filter_list() {
        let text = " T.C drawtext          V->V       Draw text\n .. subtitles         V->V       Render text subtitles\n";
        let t = parse_filters(text);
        assert!(t.drawtext && t.subtitles && t.ffmpeg);
        let t = parse_filters(" TS gblur V->V blur\n");
        assert!(!t.drawtext && !t.subtitles);
    }

    #[test]
    fn parses_rotation() {
        assert_eq!(
            parse_rotation(r#"{"streams":[{"side_data_list":[{"rotation":-90}]}]}"#),
            -90
        );
        assert_eq!(
            parse_rotation(r#"{"streams":[{"tags":{"rotate":"270"}}]}"#),
            270
        );
        assert_eq!(parse_rotation(r#"{"streams":[]}"#), 0);
        assert_eq!(parse_rotation("nope"), 0);
        assert_eq!((-90i64).rem_euclid(180), 90);
    }

    #[test]
    fn output_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("clip.mov");
        std::fs::write(&input, b"x").unwrap();
        let first = unique_output(&input, None);
        assert_eq!(first, dir.path().join("clip (studio).mp4"));
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(
            unique_output(&input, None),
            dir.path().join("clip (studio 2).mp4")
        );
        let other = tempfile::tempdir().unwrap();
        assert_eq!(
            unique_output(&input, Some(other.path())),
            other.path().join("clip (studio).mp4")
        );
    }

    #[test]
    fn only_subtitle_files_pass() {
        assert!(is_subtitle_path("/a/b.srt"));
        assert!(is_subtitle_path("C:\\x\\Clip.VTT"));
        assert!(is_subtitle_path("/a/b.ass"));
        assert!(!is_subtitle_path("/Users/me/.ssh/id_rsa"));
        assert!(!is_subtitle_path("/Users/me/.zshrc"));
        assert!(!is_subtitle_path("/a/b.srt.sh"));
        assert!(!is_subtitle_path("/a/srt"));
    }

    #[test]
    fn publish_never_replaces_a_file_that_appeared() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("clip.mov");
        std::fs::write(&input, b"x").unwrap();
        let partial = partial_path(&input, None);
        assert!(partial
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".clip (studio).mp4."));
        std::fs::write(&partial, b"new").unwrap();
        let other = dir.path().join("clip (studio).mp4");
        std::fs::write(&other, b"someone else").unwrap();
        let out = publish(&partial, &input, None).unwrap();
        assert_eq!(out, dir.path().join("clip (studio 2).mp4"));
        assert_eq!(std::fs::read(&other).unwrap(), b"someone else");
        assert_eq!(std::fs::read(&out).unwrap(), b"new");
        assert!(!partial.exists());
    }

    #[test]
    fn serde_names_match_frontend() {
        let r: RenderRequest = serde_json::from_str(
            r#"{"input":"/a.mp4","reframe":{"aspect":"9:16","mode":"fit","position":0.2},"quality":"high","fade_out":true}"#,
        )
        .unwrap();
        assert_eq!(r.reframe.aspect, Aspect::Vertical);
        assert_eq!(r.reframe.mode, FrameMode::Fit);
        assert_eq!(r.quality, Quality::High);
        assert!(r.intro.is_none() && r.fade_out);
        let r: RenderRequest = serde_json::from_str(r#"{"input":"/a.mp4"}"#).unwrap();
        assert_eq!(r.reframe.aspect, Aspect::Keep);
        assert_eq!(r.quality, Quality::Balanced);
    }
}
