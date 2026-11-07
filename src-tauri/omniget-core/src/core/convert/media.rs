use std::path::Path;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio_util::sync::CancellationToken;

use super::external::tail;
use super::formats::{self, Category};
use super::ConvertOptions;

fn even_scale(opts: &ConvertOptions) -> String {
    match (opts.max_width, opts.max_height) {
        (None, None) => "scale=trunc(iw/2)*2:trunc(ih/2)*2".to_string(),
        (w, h) => {
            let w = w.filter(|v| *v > 0).map(|v| format!("min({v}\\,iw)"));
            let h = h.filter(|v| *v > 0).map(|v| format!("min({v}\\,ih)"));
            format!(
                "scale={}:{}:force_original_aspect_ratio=decrease:force_divisible_by=2",
                w.unwrap_or_else(|| "iw".to_string()),
                h.unwrap_or_else(|| "ih".to_string())
            )
        }
    }
}

fn audio_args(to: &str) -> Vec<&'static str> {
    match to {
        "mp3" => vec!["-c:a", "libmp3lame", "-q:a", "2"],
        "m4a" | "aac" => vec!["-c:a", "aac", "-b:a", "192k"],
        "wav" => vec!["-c:a", "pcm_s16le"],
        "flac" => vec!["-c:a", "flac"],
        "ogg" => vec!["-c:a", "libvorbis", "-q:a", "5"],
        "opus" => vec!["-c:a", "libopus", "-b:a", "128k"],
        _ => vec![],
    }
}

pub fn build_args(from: &str, to: &str, opts: &ConvertOptions, copy: bool) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    let push = |a: &mut Vec<String>, items: &[&str]| a.extend(items.iter().map(|s| s.to_string()));
    let to_cat = formats::category_of(to);
    let from_cat = formats::category_of(from);
    match to_cat {
        Some(Category::Audio) => {
            push(&mut a, &["-vn", "-map", "0:a:0", "-map_metadata", "0"]);
            push(&mut a, &audio_args(to));
        }
        Some(Category::Image) if from_cat == Some(Category::Video) => {
            let w = opts.max_width.filter(|v| *v > 0).unwrap_or(640);
            a.push("-an".to_string());
            a.push("-vf".to_string());
            a.push(format!(
                "fps=12,scale='min({w},iw)':-1:flags=lanczos,split[s0][s1];[s0]palettegen[p];[s1][p]paletteuse"
            ));
            push(&mut a, &["-loop", "0"]);
        }
        _ => {
            if copy {
                push(&mut a, &["-map", "0", "-c", "copy"]);
            } else {
                push(&mut a, &["-map", "0:v:0", "-map", "0:a?"]);
                a.push("-vf".to_string());
                a.push(even_scale(opts));
                match to {
                    "webm" => push(
                        &mut a,
                        &[
                            "-c:v",
                            "libvpx-vp9",
                            "-crf",
                            "32",
                            "-b:v",
                            "0",
                            "-row-mt",
                            "1",
                            "-deadline",
                            "good",
                            "-cpu-used",
                            "4",
                            "-c:a",
                            "libopus",
                            "-b:a",
                            "128k",
                        ],
                    ),
                    "avi" => push(
                        &mut a,
                        &[
                            "-c:v",
                            "mpeg4",
                            "-q:v",
                            "3",
                            "-c:a",
                            "libmp3lame",
                            "-q:a",
                            "2",
                        ],
                    ),
                    _ => {
                        push(
                            &mut a,
                            &[
                                "-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-pix_fmt",
                                "yuv420p", "-c:a", "aac", "-b:a", "192k",
                            ],
                        );
                        if matches!(to, "mp4" | "mov") {
                            push(&mut a, &["-movflags", "+faststart"]);
                        }
                    }
                }
            }
        }
    }
    a
}

fn parse_time_us(line: &str) -> Option<u64> {
    let v = line
        .strip_prefix("out_time_us=")
        .or_else(|| line.strip_prefix("out_time_ms="))?;
    v.trim()
        .parse::<i64>()
        .ok()
        .filter(|v| *v >= 0)
        .map(|v| v as u64)
}

async fn run_ffmpeg(
    input: &Path,
    output: &Path,
    args: &[String],
    duration_us: u64,
    cancel: &CancellationToken,
    progress: &(dyn Fn(f64) + Send + Sync),
) -> Result<(), String> {
    let mut child = crate::core::process::command("ffmpeg")
        .arg("-y")
        .arg("-hide_banner")
        .arg("-nostdin")
        .arg("-v")
        .arg("error")
        .arg("-i")
        .arg(input)
        .args(args)
        .arg("-progress")
        .arg("pipe:1")
        .arg("-nostats")
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Could not start FFmpeg: {e}. Install FFmpeg in Settings."))?;
    let stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let err_reader = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(s) = stderr.as_mut() {
            let _ = s.read_to_end(&mut buf).await;
        }
        buf
    });
    let watch = async {
        if let Some(out) = stdout {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(us) = parse_time_us(&line) {
                    if duration_us > 0 {
                        progress((us as f64 / duration_us as f64).min(1.0));
                    }
                }
            }
        }
    };
    let status = tokio::select! {
        (s, _) = async { tokio::join!(child.wait(), watch) } => s.map_err(|e| format!("FFmpeg failed: {e}."))?,
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            return Err("Cancelled.".to_string());
        }
    };
    let err = err_reader.await.unwrap_or_default();
    if status.success() {
        return Ok(());
    }
    Err(format!(
        "FFmpeg could not convert this file: {}",
        tail(&String::from_utf8_lossy(&err))
    ))
}

pub async fn convert(
    input: &Path,
    from: &str,
    output: &Path,
    to: &str,
    opts: &ConvertOptions,
    cancel: &CancellationToken,
    progress: &(dyn Fn(f64) + Send + Sync),
) -> Result<(), String> {
    let probe = crate::core::ffmpeg::probe(input).await.ok();
    let duration_us = probe
        .as_ref()
        .map(|p| (p.duration_seconds * 1_000_000.0) as u64)
        .unwrap_or(0);
    if let Some(p) = &probe {
        let has = |kind: &str| p.streams.iter().any(|s| s.codec_type == kind);
        if formats::category_of(to) == Some(Category::Audio) && !has("audio") {
            return Err(format!(
                "This file has no audio track, so there is nothing to save as {}. Pick a video or image format instead.",
                to.to_uppercase()
            ));
        }
        if formats::category_of(from) == Some(Category::Video) && !has("video") && to == "gif" {
            return Err("This file has no video track, so it cannot become a GIF.".to_string());
        }
    }
    let can_copy = to == "mkv"
        && formats::category_of(from) == Some(Category::Video)
        && opts.max_width.is_none()
        && opts.max_height.is_none();
    if can_copy {
        let args = build_args(from, to, opts, true);
        if run_ffmpeg(input, output, &args, duration_us, cancel, progress)
            .await
            .is_ok()
        {
            return Ok(());
        }
        if cancel.is_cancelled() {
            return Err("Cancelled.".to_string());
        }
        let _ = std::fs::remove_file(output);
    }
    let args = build_args(from, to, opts, false);
    run_ffmpeg(input, output, &args, duration_us, cancel, progress).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_codecs_per_container() {
        let o = ConvertOptions::default();
        let mp3 = build_args("wav", "mp3", &o, false).join(" ");
        assert!(mp3.contains("libmp3lame -q:a 2"));
        assert!(mp3.contains("-vn"));
        let webm = build_args("mp4", "webm", &o, false).join(" ");
        assert!(webm.contains("libvpx-vp9") && webm.contains("libopus"));
        let mp4 = build_args("mkv", "mp4", &o, false).join(" ");
        assert!(mp4.contains("libx264") && mp4.contains("+faststart"));
        let gif = build_args("mp4", "gif", &o, false).join(" ");
        assert!(gif.contains("palettegen") && gif.contains("min(640,iw)"));
        assert!(build_args("mp4", "mkv", &o, true)
            .join(" ")
            .contains("-c copy"));
    }

    #[test]
    fn parses_progress_lines() {
        assert_eq!(parse_time_us("out_time_us=1500000"), Some(1_500_000));
        assert_eq!(parse_time_us("out_time_ms=-5"), None);
        assert_eq!(parse_time_us("frame=1"), None);
    }

    fn ffmpeg_present() -> bool {
        std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    async fn lavfi(args: &[&str], out: &Path) {
        let ok = crate::core::process::command("ffmpeg")
            .args(["-y", "-v", "error", "-f", "lavfi"])
            .args(args)
            .arg(out)
            .status()
            .await
            .unwrap()
            .success();
        assert!(ok);
    }

    #[tokio::test]
    async fn smoke_wav_to_mp3_and_mp4_to_gif() {
        if !ffmpeg_present() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let o = ConvertOptions::default();
        let cancel = CancellationToken::new();
        let wav = dir.path().join("tone.wav");
        lavfi(&["-i", "sine=frequency=440:duration=1"], &wav).await;
        let mp3 = dir.path().join("tone.mp3");
        convert(&wav, "wav", &mp3, "mp3", &o, &cancel, &|_| {})
            .await
            .unwrap();
        assert!(std::fs::metadata(&mp3).unwrap().len() > 0);
        let mp4 = dir.path().join("clip.mp4");
        lavfi(
            &[
                "-i",
                "testsrc=size=320x240:rate=25:duration=1",
                "-pix_fmt",
                "yuv420p",
            ],
            &mp4,
        )
        .await;
        let gif = dir.path().join("clip.gif");
        let seen = std::sync::Mutex::new(0.0f64);
        convert(&mp4, "mp4", &gif, "gif", &o, &cancel, &|p| {
            *seen.lock().unwrap() = p;
        })
        .await
        .unwrap();
        let bytes = std::fs::read(&gif).unwrap();
        assert!(bytes.starts_with(b"GIF8"));
    }
}
