//! Downloads › Tools › Join videos: put several files in order and save them
//! as one. When every file has the same codecs, size and sample rate, the
//! parts are copied end to end with no re-encoding; otherwise they can be
//! re-encoded to the size of the first file.
//!
//! The idea comes from Videomass (jeanslack/Videomass, GPL-3.0), whose
//! "Concatenate" panel checks the files with ffprobe before a lossless join.
//! Reimplemented here; no code was copied.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::Value;

const MAX_FILES: usize = 100;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct StreamInfo {
    /// `video:h264,audio:aac` in stream order.
    pub codecs: Vec<String>,
    pub width: Option<u64>,
    pub height: Option<u64>,
    pub sample_rate: Option<String>,
    pub has_audio: bool,
    pub duration_secs: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JoinCheck {
    /// True when the files can be joined without re-encoding.
    pub lossless: bool,
    /// Why they cannot, in plain words, naming the first file that differs.
    pub reason: Option<String>,
    /// True when a re-encoded join is possible (all files have audio, or none).
    pub can_reencode: bool,
    pub total_secs: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct JoinResult {
    pub output_path: String,
}

/// The parts of `ffprobe -show_streams -show_format -of json` a join needs.
pub fn stream_info(probe: &Value) -> StreamInfo {
    let mut info = StreamInfo::default();
    for s in probe["streams"].as_array().into_iter().flatten() {
        let kind = s["codec_type"].as_str().unwrap_or("");
        if kind != "video" && kind != "audio" {
            continue;
        }
        // A cover picture inside an audio file is not a video track.
        if kind == "video" && s["disposition"]["attached_pic"].as_i64() == Some(1) {
            continue;
        }
        info.codecs.push(format!(
            "{kind}:{}",
            s["codec_name"].as_str().unwrap_or("?")
        ));
        if kind == "video" && info.width.is_none() {
            info.width = s["width"].as_u64();
            info.height = s["height"].as_u64();
        }
        if kind == "audio" {
            info.has_audio = true;
            if info.sample_rate.is_none() {
                info.sample_rate = s["sample_rate"].as_str().map(String::from);
            }
        }
    }
    info.duration_secs = probe["format"]["duration"]
        .as_str()
        .and_then(|d| d.parse().ok());
    info
}

/// Compare every file with the first, like a lossless join needs.
pub fn check(names: &[String], infos: &[StreamInfo]) -> JoinCheck {
    let total_secs = infos.iter().filter_map(|i| i.duration_secs).sum();
    let all_audio = infos.iter().all(|i| i.has_audio);
    let no_audio = infos.iter().all(|i| !i.has_audio);
    let any_video = infos.iter().any(|i| i.width.is_some());
    let can_reencode = infos.len() >= 2 && any_video && (all_audio || no_audio);
    let mut reason = None;
    if let Some(first) = infos.first() {
        for (name, info) in names.iter().zip(infos).skip(1) {
            let why = if info.codecs != first.codecs {
                Some("different codecs or tracks")
            } else if (info.width, info.height) != (first.width, first.height) {
                Some("a different picture size")
            } else if info.sample_rate != first.sample_rate {
                Some("a different audio sample rate")
            } else {
                None
            };
            if let Some(why) = why {
                reason = Some(format!("{name} has {why} than the first file"));
                break;
            }
        }
    }
    JoinCheck {
        lossless: infos.len() >= 2 && reason.is_none(),
        reason,
        can_reencode,
        total_secs,
    }
}

/// The concat demuxer's list: `file '<path>'`, with `'` escaped.
pub fn concat_list(inputs: &[PathBuf]) -> String {
    inputs
        .iter()
        .map(|p| format!("file '{}'\n", p.to_string_lossy().replace('\'', "'\\''")))
        .collect()
}

/// `filter_complex` that scales every part to `w`×`h` (letterboxed) and joins them.
pub fn reencode_filter(n: usize, w: u64, h: u64, audio: bool) -> String {
    let mut f = String::new();
    for i in 0..n {
        f.push_str(&format!(
            "[{i}:v:0]scale={w}:{h}:force_original_aspect_ratio=decrease,pad={w}:{h}:-1:-1,setsar=1,fps=30,format=yuv420p[v{i}];"
        ));
        if audio {
            f.push_str(&format!("[{i}:a:0]aresample=48000[a{i}];"));
        }
    }
    for i in 0..n {
        f.push_str(&format!("[v{i}]"));
        if audio {
            f.push_str(&format!("[a{i}]"));
        }
    }
    f.push_str(&format!(
        "concat=n={n}:v=1:a={}[v]{}",
        u8::from(audio),
        if audio { "[a]" } else { "" }
    ));
    f
}

/// `<first stem> joined.<ext>` next to the first file, never over an existing one.
pub fn output_path(first: &Path, ext: &str) -> PathBuf {
    let stem = first
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "video".to_string());
    let dir = first.parent().map(PathBuf::from).unwrap_or_default();
    let mut candidate = dir.join(format!("{stem} joined.{ext}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem} joined ({n}).{ext}"));
        n += 1;
    }
    candidate
}

fn inputs(paths: &[String]) -> Result<Vec<PathBuf>, String> {
    if paths.len() < 2 {
        return Err("Pick at least two files".to_string());
    }
    if paths.len() > MAX_FILES {
        return Err(format!("Pick at most {MAX_FILES} files"));
    }
    paths
        .iter()
        .map(|p| {
            let p = PathBuf::from(p);
            if p.is_file() {
                Ok(p)
            } else {
                Err(format!("File not found: {}", p.display()))
            }
        })
        .collect()
}

fn name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

#[cfg(not(target_os = "android"))]
async fn probe_all(files: &[PathBuf]) -> Result<Vec<StreamInfo>, String> {
    let ffprobe = omniget_core::core::dependencies::find_tool("ffprobe")
        .await
        .ok_or_else(|| "ffprobe unavailable".to_string())?;
    let mut out = Vec::with_capacity(files.len());
    for f in files {
        let res = omniget_core::core::process::command(&ffprobe)
            .args([
                "-v",
                "error",
                "-show_streams",
                "-show_format",
                "-of",
                "json",
            ])
            .arg(f)
            .output()
            .await
            .map_err(|e| format!("ffprobe failed to start: {e}"))?;
        let json: Value = serde_json::from_slice(&res.stdout)
            .map_err(|_| format!("{} is not a media file ffprobe can read", name(f)))?;
        out.push(stream_info(&json));
    }
    Ok(out)
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn join_check(paths: Vec<String>) -> Result<JoinCheck, String> {
    let files = inputs(&paths)?;
    let infos = probe_all(&files).await?;
    let names: Vec<String> = files.iter().map(|f| name(f)).collect();
    Ok(check(&names, &infos))
}

#[cfg(not(target_os = "android"))]
#[tauri::command]
pub async fn join_videos(paths: Vec<String>, reencode: bool) -> Result<JoinResult, String> {
    let files = inputs(&paths)?;
    let infos = probe_all(&files).await?;
    let names: Vec<String> = files.iter().map(|f| name(f)).collect();
    let verdict = check(&names, &infos);
    let ffmpeg = omniget_core::core::dependencies::find_tool("ffmpeg")
        .await
        .ok_or_else(|| "ffmpeg unavailable".to_string())?;
    let mut cmd = omniget_core::core::process::command(&ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
    let list_file;
    let output = if !reencode {
        if !verdict.lossless {
            return Err(verdict
                .reason
                .unwrap_or_else(|| "These files cannot be joined without re-encoding".into()));
        }
        let ext = files[0]
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| "mp4".to_string());
        let output = output_path(&files[0], &ext);
        list_file = std::env::temp_dir().join(format!(
            "omniget-join-{}.txt",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ));
        std::fs::write(&list_file, concat_list(&files))
            .map_err(|e| format!("Could not write the file list: {e}"))?;
        cmd.args(["-f", "concat", "-safe", "0", "-i"])
            .arg(&list_file)
            .args(["-map", "0", "-c", "copy", "-map_metadata", "0"]);
        output
    } else {
        if !verdict.can_reencode {
            return Err(
                "Some files have sound and others do not; join those groups separately".to_string(),
            );
        }
        list_file = PathBuf::new();
        let first = infos
            .iter()
            .find(|i| i.width.is_some())
            .cloned()
            .unwrap_or_default();
        // Even numbers: H.264 with yuv420p refuses odd sizes.
        let w = first.width.unwrap_or(1280) / 2 * 2;
        let h = first.height.unwrap_or(720) / 2 * 2;
        let audio = infos.iter().all(|i| i.has_audio);
        for f in &files {
            cmd.arg("-i").arg(f);
        }
        cmd.args([
            "-filter_complex",
            &reencode_filter(files.len(), w, h, audio),
        ])
        .args(["-map", "[v]"]);
        if audio {
            cmd.args(["-map", "[a]", "-c:a", "aac", "-b:a", "192k"]);
        }
        cmd.args([
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "20",
            "-movflags",
            "+faststart",
        ]);
        output_path(&files[0], "mp4")
    };
    cmd.arg(&output);
    let status = cmd
        .status()
        .await
        .map_err(|e| format!("ffmpeg failed to start: {e}"));
    if !list_file.as_os_str().is_empty() {
        let _ = std::fs::remove_file(&list_file);
    }
    if !status?.success() {
        let _ = std::fs::remove_file(&output);
        return Err("ffmpeg processing failed".to_string());
    }
    Ok(JoinResult {
        output_path: output.to_string_lossy().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn video(codec: &str, w: u64, h: u64, rate: Option<&str>) -> StreamInfo {
        let mut streams =
            vec![json!({"codec_type": "video", "codec_name": codec, "width": w, "height": h})];
        if let Some(r) = rate {
            streams.push(json!({"codec_type": "audio", "codec_name": "aac", "sample_rate": r}));
        }
        stream_info(&json!({"streams": streams, "format": {"duration": "10.5"}}))
    }

    #[test]
    fn reads_streams_and_skips_cover_art() {
        let info = stream_info(&json!({
            "streams": [
                {"codec_type": "audio", "codec_name": "mp3", "sample_rate": "44100"},
                {"codec_type": "video", "codec_name": "mjpeg", "width": 500, "height": 500, "disposition": {"attached_pic": 1}}
            ],
            "format": {"duration": "3.0"}
        }));
        assert_eq!(info.codecs, vec!["audio:mp3"]);
        assert_eq!(info.width, None);
        assert!(info.has_audio);
        assert_eq!(info.duration_secs, Some(3.0));
    }

    #[test]
    fn same_files_join_losslessly() {
        let names = vec!["a.mp4".to_string(), "b.mp4".to_string()];
        let c = check(
            &names,
            &[
                video("h264", 1920, 1080, Some("48000")),
                video("h264", 1920, 1080, Some("48000")),
            ],
        );
        assert!(c.lossless);
        assert!(c.reason.is_none());
        assert_eq!(c.total_secs, 21.0);
    }

    #[test]
    fn a_different_size_needs_reencoding_and_names_the_file() {
        let names = vec!["a.mp4".to_string(), "b.mp4".to_string()];
        let c = check(
            &names,
            &[
                video("h264", 1920, 1080, Some("48000")),
                video("h264", 1280, 720, Some("48000")),
            ],
        );
        assert!(!c.lossless);
        assert!(c
            .reason
            .unwrap()
            .starts_with("b.mp4 has a different picture size"));
        assert!(c.can_reencode);
    }

    #[test]
    fn mixed_audio_cannot_be_reencoded_together() {
        let names = vec!["a.mp4".to_string(), "b.mp4".to_string()];
        let c = check(
            &names,
            &[
                video("h264", 640, 360, Some("48000")),
                video("h264", 640, 360, None),
            ],
        );
        assert!(!c.lossless);
        assert!(!c.can_reencode);
    }

    #[test]
    fn list_escapes_quotes() {
        let list = concat_list(&[PathBuf::from("/v/it's.mp4"), PathBuf::from("/v/b.mp4")]);
        assert_eq!(list, "file '/v/it'\\''s.mp4'\nfile '/v/b.mp4'\n");
    }

    #[test]
    fn filter_joins_every_part() {
        let f = reencode_filter(2, 1280, 720, true);
        assert!(f.contains("[0:v:0]scale=1280:720"));
        assert!(f.ends_with("[v0][a0][v1][a1]concat=n=2:v=1:a=1[v][a]"));
        let f = reencode_filter(3, 640, 360, false);
        assert!(f.ends_with("[v0][v1][v2]concat=n=3:v=1:a=0[v]"));
    }
}
