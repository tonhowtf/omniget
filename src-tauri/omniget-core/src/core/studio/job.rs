use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio_util::sync::CancellationToken;

use super::graph::{self, GraphSpec, IntroFiles, Source};
use super::{RenderOutcome, RenderRequest};

pub const CANCELLED: &str = "Cancelled.";

pub fn parse_time_us(line: &str) -> Option<u64> {
    let v = line
        .strip_prefix("out_time_us=")
        .or_else(|| line.strip_prefix("out_time_ms="))?;
    v.trim()
        .parse::<i64>()
        .ok()
        .filter(|v| *v >= 0)
        .map(|v| v as u64)
}

fn tail(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let start = lines.len().saturating_sub(3);
    let joined = lines[start..].join(" ");
    if joined.is_empty() {
        "no details were reported".to_string()
    } else {
        joined.chars().take(400).collect()
    }
}

struct JobDir(PathBuf);

impl Drop for JobDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn job_dir() -> Result<JobDir, String> {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = crate::core::tools::temp_dir().join(format!("studio-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(dir.join("fonts"))
        .map_err(|e| format!("Studio could not create its work folder: {e}."))?;
    Ok(JobDir(dir))
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(path, bytes)
        .map_err(|e| format!("Studio could not write {}: {e}.", path.display()))
}

fn clean_line(line: &str) -> String {
    line.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}

pub async fn render(
    req: &RenderRequest,
    cancel: &CancellationToken,
    progress: &(dyn Fn(f64, &'static str) + Send + Sync),
) -> Result<RenderOutcome, String> {
    progress(0.0, "preparing");
    let input = PathBuf::from(&req.input);
    let probe = super::probe(&input).await?;
    let ass = req
        .ass
        .as_deref()
        .map(str::trim)
        .filter(|a| a.contains("Dialogue:"));
    let intro = req.intro.as_ref().map(|i| super::Intro {
        title_lines: i
            .title_lines
            .iter()
            .map(|l| clean_line(l))
            .filter(|l| !l.is_empty())
            .take(4)
            .collect(),
        subtitle_lines: i
            .subtitle_lines
            .iter()
            .map(|l| clean_line(l))
            .filter(|l| !l.is_empty())
            .take(3)
            .collect(),
        ..i.clone()
    });
    if ass.is_some() || intro.is_some() {
        let tools = super::tools().await;
        if !tools.ffmpeg {
            return Err("FFmpeg is not installed. Install it in Settings > Dependencies.".into());
        }
        if ass.is_some() && !tools.subtitles {
            return Err("This FFmpeg build cannot draw captions (it lacks libass). Install the FFmpeg that OmniGet manages in Settings > Dependencies, or turn off burned-in captions.".into());
        }
        if intro.is_some() && !tools.drawtext {
            return Err("This FFmpeg build cannot draw text (it lacks the drawtext filter). Install the FFmpeg that OmniGet manages in Settings > Dependencies, or turn off the intro card.".into());
        }
    }

    let dir = job_dir()?;
    let fonts = dir.0.join("fonts");
    let regular = fonts.join("LiberationSans-Regular.ttf");
    let bold = fonts.join("LiberationSans-Bold.ttf");
    write(&regular, super::FONT_REGULAR)?;
    write(&bold, super::FONT_BOLD)?;

    let ass_path = dir.0.join("captions.ass");
    if let Some(text) = ass {
        write(&ass_path, text.as_bytes())?;
    }
    let intro_font = match &intro {
        Some(i) => {
            let text = i
                .title_lines
                .iter()
                .chain(i.subtitle_lines.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join("\n");
            tokio::task::spawn_blocking(move || super::fonts::fallback_for(&text, super::FONT_BOLD))
                .await
                .ok()
                .flatten()
        }
        None => None,
    };
    let mut title_paths = Vec::new();
    let mut subtitle_paths = Vec::new();
    if let Some(i) = &intro {
        for (n, line) in i.title_lines.iter().enumerate() {
            let p = dir.0.join(format!("title-{n}.txt"));
            write(&p, line.as_bytes())?;
            title_paths.push(p);
        }
        for (n, line) in i.subtitle_lines.iter().enumerate() {
            let p = dir.0.join(format!("subtitle-{n}.txt"));
            write(&p, line.as_bytes())?;
            subtitle_paths.push(p);
        }
    }

    let spec = GraphSpec {
        source: Source {
            width: probe.width,
            height: probe.height,
            fps: probe.fps,
            duration_secs: probe.duration_ms as f64 / 1000.0,
            has_audio: probe.has_audio,
        },
        aspect: req.reframe.aspect,
        mode: req.reframe.mode,
        position: req.reframe.position,
        ass: ass.map(|_| ass_path.as_path()),
        fonts_dir: ass.map(|_| fonts.as_path()),
        intro: intro.as_ref().map(|i| IntroFiles {
            intro: i,
            font: intro_font.as_deref().unwrap_or(bold.as_path()),
            title_files: title_paths.iter().map(PathBuf::as_path).collect(),
            subtitle_files: subtitle_paths.iter().map(PathBuf::as_path).collect(),
        }),
        fade_out: req.fade_out,
    };
    let g = graph::build(&spec)?;

    let out_dir = req
        .out_dir
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(PathBuf::from);
    if let Some(d) = &out_dir {
        if !d.is_dir() {
            return Err("The output folder no longer exists. Choose another folder.".into());
        }
    }
    let partial = super::partial_path(&input, out_dir.as_deref());

    let mut cmd = crate::core::process::command("ffmpeg");
    cmd.args(["-hide_banner", "-nostdin", "-v", "error", "-n", "-i"])
        .arg(&input);
    for lavfi in &g.lavfi_inputs {
        cmd.args(["-f", "lavfi", "-i"]).arg(lavfi);
    }
    cmd.arg("-filter_complex")
        .arg(&g.filter)
        .args(["-map", &g.video_label]);
    if let Some(a) = &g.audio_label {
        cmd.args(["-map", a]);
    }
    cmd.args(["-c:v", "libx264"])
        .args(graph::quality_args(req.quality))
        .args(["-pix_fmt", "yuv420p"]);
    if g.audio_label.is_some() {
        cmd.args(["-c:a", "aac", "-b:a", "192k"]);
    }
    cmd.args(["-movflags", "+faststart", "-progress", "pipe:1", "-nostats"])
        .args(["-f", "mp4"])
        .arg(&partial)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    if cancel.is_cancelled() {
        return Err(CANCELLED.into());
    }
    let mut child = cmd.spawn().map_err(|e| {
        format!("Could not start FFmpeg: {e}. Install FFmpeg in Settings > Dependencies.")
    })?;
    progress(0.0, "rendering");
    let stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let err_reader = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(s) = stderr.as_mut() {
            let _ = s.read_to_end(&mut buf).await;
        }
        buf
    });
    let total_us = (g.total_secs * 1_000_000.0) as u64;
    let watch = async {
        if let Some(out) = stdout {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(us) = parse_time_us(&line) {
                    if total_us > 0 {
                        progress((us as f64 / total_us as f64).min(0.99), "rendering");
                    }
                }
            }
        }
    };
    let status = tokio::select! {
        (s, _) = async { tokio::join!(child.wait(), watch) } => s.map_err(|e| format!("FFmpeg failed: {e}."))?,
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            let _ = std::fs::remove_file(&partial);
            return Err(CANCELLED.into());
        }
    };
    let err = err_reader.await.unwrap_or_default();
    drop(dir);
    if !status.success() {
        let _ = std::fs::remove_file(&partial);
        return Err(format!(
            "FFmpeg stopped before the video was finished: {}",
            tail(&String::from_utf8_lossy(&err))
        ));
    }
    let output = match super::publish(&partial, &input, out_dir.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            let _ = std::fs::remove_file(&partial);
            return Err(e);
        }
    };
    progress(1.0, "done");
    let size_bytes = std::fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    Ok(RenderOutcome {
        output: output.to_string_lossy().into_owned(),
        size_bytes,
        duration_ms: (g.total_secs * 1000.0).round() as u64,
        width: g.width,
        height: g.height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::studio::{Aspect, FrameMode, Intro, Quality, Reframe};

    #[test]
    fn parses_progress_lines() {
        assert_eq!(parse_time_us("out_time_us=1500000"), Some(1_500_000));
        assert_eq!(parse_time_us("out_time_ms=-1"), None);
        assert_eq!(parse_time_us("frame=3"), None);
    }

    #[test]
    fn strips_control_characters() {
        assert_eq!(clean_line("  Hi\u{0}\nthere "), "Hithere");
    }

    fn ffmpeg_with(filter: &str) -> bool {
        crate::core::process::std_command("ffmpeg")
            .args(["-hide_banner", "-filters"])
            .output()
            .map(|o| {
                super::super::parse_filters(&String::from_utf8_lossy(&o.stdout)).ffmpeg
                    && String::from_utf8_lossy(&o.stdout).contains(filter)
            })
            .unwrap_or(false)
    }

    #[test]
    fn smoke_render_vertical_fit_with_intro() {
        let env = crate::core::test_env::exclusive();
        let data = tempfile::tempdir().unwrap();
        env.set_data_dir(data.path());
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(smoke_render_vertical_fit_with_intro_body());
    }

    async fn smoke_render_vertical_fit_with_intro_body() {
        if !ffmpeg_with(" drawtext ") || !ffmpeg_with(" subtitles ") {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("it's a [clip], ok.mp4");
        let ok = crate::core::process::std_command("ffmpeg")
            .args([
                "-y",
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x180:rate=25:duration=1",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1",
                "-shortest",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&input)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        let ass = "[Script Info]\nScriptType: v4.00+\nPlayResX: 180\nPlayResY: 320\n\n[V4+ Styles]\nFormat: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\nStyle: Default,Liberation Sans,20,&H00FFFFFF,&H00FFFFFF,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,1,2,0,2,10,10,10,1\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:00.00,0:00:01.00,Default,,0,0,0,,{\\k50}Hi {\\k50}there\n";
        let req = RenderRequest {
            input: input.to_string_lossy().into_owned(),
            out_dir: None,
            ass: Some(ass.to_string()),
            reframe: Reframe {
                aspect: Aspect::Vertical,
                mode: FrameMode::Fit,
                position: 0.5,
            },
            intro: Some(Intro {
                seconds: 1.0,
                title_lines: vec!["Title: 100% 'ok'".into()],
                subtitle_lines: vec!["sub, line".into()],
                background: "#202020".into(),
                color: "#ffffff".into(),
                fade: true,
            }),
            fade_out: true,
            quality: Quality::Fast,
        };
        let cancel = CancellationToken::new();
        let out = render(&req, &cancel, &|_, _| {}).await.unwrap();
        assert!(out.output.ends_with("(studio).mp4"));
        assert_eq!((out.width, out.height), (180, 320));
        assert!(out.size_bytes > 0);
        let again = render(&req, &cancel, &|_, _| {}).await.unwrap();
        assert!(again.output.ends_with("(studio 2).mp4"));
        let leftovers = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
            .count();
        assert_eq!(leftovers, 0);
        let mut cjk = req.clone();
        if let Some(i) = cjk.intro.as_mut() {
            i.title_lines = vec!["日本語のタイトル".into()];
        }
        let third = render(&cjk, &cancel, &|_, _| {}).await.unwrap();
        assert!(third.output.ends_with("(studio 3).mp4"));
    }
}
