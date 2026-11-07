use std::path::Path;

use super::errors::{code_message, BAD_OPTIONS};
use super::{Container, Platform, Quality, RecordOptions, Segment};

const EVEN: &str = "scale=trunc(iw/2)*2:trunc(ih/2)*2";

fn s(v: impl Into<String>) -> String {
    v.into()
}

pub fn secs(t: f64) -> String {
    format!("{:.3}", t.max(0.0))
}

fn path_arg(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

pub fn validate_record(opts: &RecordOptions) -> Result<(), String> {
    if opts.screen.trim().is_empty() {
        return Err(code_message(BAD_OPTIONS, "Pick a screen to record."));
    }
    if !(1..=120).contains(&opts.fps) {
        return Err(code_message(BAD_OPTIONS, "Frame rate must be 1 to 120."));
    }
    if let Some(r) = opts.region {
        if r.w < 16 || r.h < 16 {
            return Err(code_message(
                BAD_OPTIONS,
                "The region must be at least 16 × 16 pixels.",
            ));
        }
    }
    Ok(())
}

fn crf(q: Quality) -> &'static str {
    match q {
        Quality::High => "18",
        Quality::Balanced => "23",
        Quality::Small => "28",
    }
}

fn preset(fps: u32) -> &'static str {
    if fps >= 50 {
        "superfast"
    } else {
        "veryfast"
    }
}

pub fn build_record_args(
    opts: &RecordOptions,
    platform: Platform,
    output: &Path,
) -> Result<Vec<String>, String> {
    validate_record(opts)?;
    let fps = opts.fps.to_string();
    let cursor = if opts.cursor { "1" } else { "0" };
    let mic = opts.mic.as_deref().map(str::trim).filter(|m| !m.is_empty());
    let mut a = vec![
        s("-hide_banner"),
        s("-nostats"),
        s("-n"),
        s("-progress"),
        s("pipe:1"),
    ];
    let mut separate_audio = false;
    let mut filters: Vec<String> = Vec::new();
    match platform {
        Platform::Macos => {
            a.extend([
                s("-f"),
                s("avfoundation"),
                s("-capture_cursor"),
                s(cursor),
                s("-framerate"),
                fps.clone(),
                s("-thread_queue_size"),
                s("1024"),
                s("-i"),
            ]);
            a.push(match mic {
                Some(m) => format!("{}:{m}", opts.screen),
                None => opts.screen.clone(),
            });
            if let Some(r) = opts.region {
                filters.push(format!("crop={}:{}:{}:{}", r.w, r.h, r.x, r.y));
            }
        }
        Platform::Windows => {
            a.extend([
                s("-f"),
                s("gdigrab"),
                s("-framerate"),
                fps.clone(),
                s("-draw_mouse"),
                s(cursor),
                s("-thread_queue_size"),
                s("1024"),
            ]);
            if let Some(r) = opts.region {
                a.extend([
                    s("-offset_x"),
                    r.x.to_string(),
                    s("-offset_y"),
                    r.y.to_string(),
                    s("-video_size"),
                    format!("{}x{}", r.w, r.h),
                ]);
            }
            a.extend([s("-i"), opts.screen.clone()]);
            if let Some(m) = mic {
                a.extend([
                    s("-f"),
                    s("dshow"),
                    s("-thread_queue_size"),
                    s("1024"),
                    s("-i"),
                    format!("audio={m}"),
                ]);
                separate_audio = true;
            }
        }
        Platform::Linux | Platform::Other => {
            a.extend([
                s("-f"),
                s("x11grab"),
                s("-framerate"),
                fps.clone(),
                s("-draw_mouse"),
                s(cursor),
                s("-thread_queue_size"),
                s("1024"),
            ]);
            let input = match opts.region {
                Some(r) => {
                    a.extend([s("-video_size"), format!("{}x{}", r.w, r.h)]);
                    format!("{}+{},{}", opts.screen, r.x, r.y)
                }
                None => opts.screen.clone(),
            };
            a.extend([s("-i"), input]);
            if let Some(m) = mic {
                let (fmt, dev) = match m.split_once(':') {
                    Some(("pulse", d)) => ("pulse", d),
                    Some(("alsa", d)) => ("alsa", d),
                    _ => ("pulse", m),
                };
                a.extend([
                    s("-f"),
                    s(fmt),
                    s("-thread_queue_size"),
                    s("1024"),
                    s("-i"),
                    s(dev),
                ]);
                separate_audio = true;
            }
        }
    }
    if separate_audio {
        a.extend([s("-map"), s("0:v:0"), s("-map"), s("1:a:0")]);
    }
    filters.push(s(EVEN));
    a.extend([s("-vf"), filters.join(",")]);
    a.extend([
        s("-r"),
        fps,
        s("-c:v"),
        s("libx264"),
        s("-preset"),
        s(preset(opts.fps)),
        s("-crf"),
        s(crf(opts.quality)),
        s("-pix_fmt"),
        s("yuv420p"),
    ]);
    if mic.is_some() {
        a.extend([s("-c:a"), s("aac"), s("-b:a"), s("160k")]);
    }
    if matches!(opts.container, Container::Mp4 | Container::Mov) {
        a.extend([s("-movflags"), s("+faststart")]);
    }
    a.push(path_arg(output));
    Ok(a)
}

fn job_prefix() -> Vec<String> {
    vec![
        s("-hide_banner"),
        s("-nostdin"),
        s("-n"),
        s("-v"),
        s("error"),
        s("-progress"),
        s("pipe:1"),
        s("-nostats"),
    ]
}

fn faststart(output: &Path, a: &mut Vec<String>) {
    let ext = super::naming::ext_of(output);
    if matches!(ext.as_str(), "mp4" | "mov" | "m4v") {
        a.extend([s("-movflags"), s("+faststart")]);
    }
}

pub fn build_cut_args(
    input: &Path,
    seg: &Segment,
    output: &Path,
    precise: bool,
    strip_audio: bool,
) -> Vec<String> {
    let mut a = job_prefix();
    a.extend([
        s("-ss"),
        secs(seg.start),
        s("-i"),
        path_arg(input),
        s("-t"),
        secs(seg.duration()),
        s("-map"),
        s("0:v?"),
    ]);
    if !strip_audio {
        a.extend([s("-map"), s("0:a?")]);
    }
    if precise {
        let ext = super::naming::ext_of(output);
        if ext == "webm" {
            a.extend([
                s("-c:v"),
                s("libvpx-vp9"),
                s("-crf"),
                s("30"),
                s("-b:v"),
                s("0"),
                s("-row-mt"),
                s("1"),
            ]);
            if !strip_audio {
                a.extend([s("-c:a"), s("libopus"), s("-b:a"), s("128k")]);
            }
        } else {
            a.extend([
                s("-c:v"),
                s("libx264"),
                s("-preset"),
                s("veryfast"),
                s("-crf"),
                s("18"),
                s("-pix_fmt"),
                s("yuv420p"),
            ]);
            if !strip_audio {
                a.extend([s("-c:a"), s("aac"), s("-b:a"), s("192k")]);
            }
        }
    } else {
        a.extend([s("-c"), s("copy"), s("-avoid_negative_ts"), s("make_zero")]);
    }
    a.extend([s("-map_metadata"), s("0")]);
    faststart(output, &mut a);
    a.push(path_arg(output));
    a
}

pub fn concat_list(parts: &[impl AsRef<Path>]) -> String {
    parts
        .iter()
        .map(|p| {
            let text = p.as_ref().to_string_lossy().replace('\'', "'\\''");
            format!("file '{text}'\n")
        })
        .collect()
}

pub fn build_concat_args(list: &Path, output: &Path) -> Vec<String> {
    let mut a = job_prefix();
    a.extend([
        s("-f"),
        s("concat"),
        s("-safe"),
        s("0"),
        s("-i"),
        path_arg(list),
        s("-map"),
        s("0"),
        s("-c"),
        s("copy"),
    ]);
    faststart(output, &mut a);
    a.push(path_arg(output));
    a
}

pub fn build_snapshot_args(input: &Path, at: f64, output: &Path) -> Vec<String> {
    vec![
        s("-hide_banner"),
        s("-nostdin"),
        s("-n"),
        s("-v"),
        s("error"),
        s("-ss"),
        secs(at),
        s("-i"),
        path_arg(input),
        s("-frames:v"),
        s("1"),
        s("-update"),
        s("1"),
        path_arg(output),
    ]
}

pub fn clean_segments(segments: &[Segment], duration: Option<f64>) -> Vec<Segment> {
    segments
        .iter()
        .filter_map(|seg| {
            let mut start = seg.start.max(0.0);
            let mut end = seg.end;
            if let Some(d) = duration.filter(|d| *d > 0.0) {
                start = start.min(d);
                end = end.min(d);
            }
            (start.is_finite() && end.is_finite() && end - start >= 0.05)
                .then_some(Segment { start, end })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::Region;
    use super::*;

    fn opts() -> RecordOptions {
        RecordOptions {
            screen: "2".into(),
            region: None,
            fps: 30,
            mic: Some("1".into()),
            cursor: true,
            container: Container::Mp4,
            quality: Quality::Balanced,
            out_dir: None,
        }
    }

    fn joined(a: &[String]) -> String {
        a.join(" ")
    }

    #[test]
    fn mac_record_with_mic_and_region() {
        let mut o = opts();
        o.region = Some(Region {
            x: 10,
            y: 20,
            w: 1280,
            h: 720,
        });
        let a = build_record_args(&o, Platform::Macos, Path::new("/o/r.mp4")).unwrap();
        let j = joined(&a);
        assert!(j.contains("-f avfoundation -capture_cursor 1 -framerate 30"));
        assert!(j.contains("-i 2:1"));
        assert!(j.contains("-vf crop=1280:720:10:20,scale=trunc(iw/2)*2:trunc(ih/2)*2"));
        assert!(j.contains("-c:v libx264 -preset veryfast -crf 23 -pix_fmt yuv420p"));
        assert!(j.contains("-c:a aac"));
        assert!(j.contains("-movflags +faststart"));
        assert!(a.contains(&"-n".to_string()));
        assert!(!a.contains(&"-nostdin".to_string()));
        assert_eq!(a.last().unwrap(), "/o/r.mp4");
    }

    #[test]
    fn mac_record_without_mic_mkv_60fps() {
        let mut o = opts();
        o.mic = None;
        o.cursor = false;
        o.fps = 60;
        o.container = Container::Mkv;
        o.quality = Quality::High;
        let a = build_record_args(&o, Platform::Macos, Path::new("/o/r.mkv")).unwrap();
        let j = joined(&a);
        assert!(j.contains("-capture_cursor 0"));
        assert!(j.contains("-i 2 "));
        assert!(j.contains("-preset superfast -crf 18"));
        assert!(!j.contains("-c:a"));
        assert!(!j.contains("faststart"));
    }

    #[test]
    fn windows_record_uses_gdigrab_offsets_and_dshow() {
        let mut o = opts();
        o.screen = "desktop".into();
        o.mic = Some("Microphone (Realtek)".into());
        o.region = Some(Region {
            x: 0,
            y: 40,
            w: 800,
            h: 600,
        });
        let a = build_record_args(&o, Platform::Windows, Path::new("C:/o/r.mp4")).unwrap();
        let j = joined(&a);
        assert!(j.contains("-f gdigrab"));
        assert!(j.contains("-offset_x 0 -offset_y 40 -video_size 800x600 -i desktop"));
        assert!(a.contains(&"audio=Microphone (Realtek)".to_string()));
        assert!(j.contains("-map 0:v:0 -map 1:a:0"));
        assert!(!j.contains("crop="));
    }

    #[test]
    fn linux_record_uses_x11grab_and_pulse() {
        let mut o = opts();
        o.screen = ":0".into();
        o.mic = Some("pulse:alsa_input.usb".into());
        o.region = Some(Region {
            x: 5,
            y: 6,
            w: 640,
            h: 480,
        });
        let a = build_record_args(&o, Platform::Linux, Path::new("/o/r.mov")).unwrap();
        let j = joined(&a);
        assert!(j.contains("-video_size 640x480 -i :0+5,6"));
        assert!(j.contains("-f pulse -thread_queue_size 1024 -i alsa_input.usb"));
        assert!(j.contains("-movflags +faststart"));
    }

    #[test]
    fn rejects_bad_options() {
        let mut o = opts();
        o.screen = " ".into();
        assert!(build_record_args(&o, Platform::Macos, Path::new("/o")).is_err());
        let mut o = opts();
        o.fps = 0;
        assert!(validate_record(&o).is_err());
        let mut o = opts();
        o.region = Some(Region {
            x: 0,
            y: 0,
            w: 8,
            h: 100,
        });
        assert!(validate_record(&o)
            .unwrap_err()
            .starts_with("CAPTURE_BAD_OPTIONS"));
    }

    #[test]
    fn lossless_cut_seeks_before_input_and_copies() {
        let seg = Segment {
            start: 1.5,
            end: 4.0,
        };
        let a = build_cut_args(
            Path::new("/v/in.mp4"),
            &seg,
            Path::new("/v/out.mp4"),
            false,
            false,
        );
        let j = joined(&a);
        let ss = a.iter().position(|x| x == "-ss").unwrap();
        let i = a.iter().position(|x| x == "-i").unwrap();
        assert!(ss < i);
        assert!(j.contains("-ss 1.500 -i /v/in.mp4 -t 2.500"));
        assert!(j.contains("-map 0:v? -map 0:a? -c copy -avoid_negative_ts make_zero"));
        assert!(j.contains("-n"));
    }

    #[test]
    fn strip_audio_and_precise() {
        let seg = Segment {
            start: 0.0,
            end: 2.0,
        };
        let a = build_cut_args(
            Path::new("/v/in.mkv"),
            &seg,
            Path::new("/v/o.mkv"),
            true,
            true,
        );
        let j = joined(&a);
        assert!(!j.contains("0:a?"));
        assert!(j.contains("-c:v libx264"));
        assert!(!j.contains("-c:a"));
        assert!(!j.contains("-c copy"));
        let w = build_cut_args(
            Path::new("/v/in.webm"),
            &seg,
            Path::new("/v/o.webm"),
            true,
            false,
        );
        assert!(joined(&w).contains("-c:v libvpx-vp9"));
        assert!(joined(&w).contains("-c:a libopus"));
    }

    #[test]
    fn concat_and_snapshot() {
        let list = concat_list(&[Path::new("/t/a.mp4"), Path::new("/t/it's.mp4")]);
        assert_eq!(list, "file '/t/a.mp4'\nfile '/t/it'\\''s.mp4'\n");
        let c = build_concat_args(Path::new("/t/l.txt"), Path::new("/t/o.mp4"));
        assert!(joined(&c).contains("-f concat -safe 0 -i /t/l.txt -map 0 -c copy"));
        let snap = build_snapshot_args(Path::new("/v/a.mp4"), 12.25, Path::new("/v/a.png"));
        assert!(joined(&snap).contains("-ss 12.250 -i /v/a.mp4 -frames:v 1"));
    }

    #[test]
    fn clean_segments_drops_tiny_and_clamps() {
        let segs = [
            Segment {
                start: -1.0,
                end: 2.0,
            },
            Segment {
                start: 3.0,
                end: 3.01,
            },
            Segment {
                start: 8.0,
                end: 20.0,
            },
            Segment {
                start: 5.0,
                end: 4.0,
            },
        ];
        let got = clean_segments(&segs, Some(10.0));
        assert_eq!(
            got,
            vec![
                Segment {
                    start: 0.0,
                    end: 2.0
                },
                Segment {
                    start: 8.0,
                    end: 10.0
                }
            ]
        );
    }
}
