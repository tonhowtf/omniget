use std::path::Path;

use super::{Aspect, FrameMode, Intro, Quality};

pub const FADE_SECS: f64 = 0.6;
pub const INTRO_FADE_SECS: f64 = 0.5;
const MAX_BASE: u32 = 2160;

pub fn even(v: f64) -> u32 {
    let n = v.max(2.0).floor() as u32;
    (n - n % 2).max(2)
}

pub fn aspect_ratio(aspect: Aspect) -> Option<(u32, u32)> {
    match aspect {
        Aspect::Keep => None,
        Aspect::Vertical => Some((9, 16)),
        Aspect::Square => Some((1, 1)),
        Aspect::Portrait => Some((4, 5)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crop {
    pub w: u32,
    pub h: u32,
    pub x: u32,
    pub y: u32,
}

pub fn crop_rect(src_w: u32, src_h: u32, aw: u32, ah: u32, position: f64) -> Crop {
    let (sw, sh) = (src_w as f64, src_h as f64);
    let target = aw as f64 / ah as f64;
    let (cw, ch) = if sw / sh > target {
        (even(sh * target), even(sh))
    } else {
        (even(sw), even(sw / target))
    };
    let cw = cw.min(src_w.max(2));
    let ch = ch.min(src_h.max(2));
    let pos = if position.is_finite() {
        position.clamp(0.0, 1.0)
    } else {
        0.5
    };
    let x = ((src_w - cw) as f64 * pos).round() as u32;
    let y = (src_h - ch) / 2;
    Crop { w: cw, h: ch, x, y }
}

pub fn output_size(src_w: u32, src_h: u32, aspect: Aspect, mode: FrameMode) -> (u32, u32) {
    match aspect_ratio(aspect) {
        None => (even(src_w as f64), even(src_h as f64)),
        Some((aw, ah)) => match mode {
            FrameMode::Crop => {
                let c = crop_rect(src_w, src_h, aw, ah, 0.5);
                (c.w, c.h)
            }
            FrameMode::Fit => {
                let base = src_w.min(src_h).min(MAX_BASE) as f64;
                let (w, h) = if aw <= ah {
                    (base, base * ah as f64 / aw as f64)
                } else {
                    (base * aw as f64 / ah as f64, base)
                };
                (even(w), even(h))
            }
        },
    }
}

pub fn escape_filter_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let raw = if cfg!(windows) {
        raw.replace('\\', "/")
    } else {
        raw.into_owned()
    };
    escape_filter_value(&raw)
}

pub fn escape_filter_value(value: &str) -> String {
    let mut first = String::with_capacity(value.len() + 8);
    for c in value.chars() {
        if matches!(c, '\\' | '\'' | ':') {
            first.push('\\');
        }
        first.push(c);
    }
    let mut out = String::with_capacity(first.len() + 8);
    for c in first.chars() {
        if matches!(c, '\\' | '\'' | '[' | ']' | ',' | ';') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub fn valid_hex(color: &str) -> Option<String> {
    let c = color.trim().trim_start_matches('#');
    (c.len() == 6 && c.chars().all(|ch| ch.is_ascii_hexdigit()))
        .then(|| format!("0x{}", c.to_ascii_uppercase()))
}

pub fn quality_args(q: Quality) -> [&'static str; 4] {
    match q {
        Quality::Fast => ["-preset", "veryfast", "-crf", "26"],
        Quality::Balanced => ["-preset", "medium", "-crf", "21"],
        Quality::High => ["-preset", "slow", "-crf", "18"],
    }
}

#[derive(Debug, Clone)]
pub struct Source {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub duration_secs: f64,
    pub has_audio: bool,
}

#[derive(Debug, Clone)]
pub struct IntroFiles<'a> {
    pub intro: &'a Intro,
    pub font: &'a Path,
    pub title_files: Vec<&'a Path>,
    pub subtitle_files: Vec<&'a Path>,
}

#[derive(Debug, Clone)]
pub struct GraphSpec<'a> {
    pub source: Source,
    pub aspect: Aspect,
    pub mode: FrameMode,
    pub position: f64,
    pub ass: Option<&'a Path>,
    pub fonts_dir: Option<&'a Path>,
    pub intro: Option<IntroFiles<'a>>,
    pub fade_out: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Graph {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub lavfi_inputs: Vec<String>,
    pub filter: String,
    pub video_label: String,
    pub audio_label: Option<String>,
    pub total_secs: f64,
}

pub fn intro_seconds(intro: &Intro) -> f64 {
    if intro.seconds.is_finite() {
        intro.seconds.clamp(1.0, 15.0)
    } else {
        3.0
    }
}

pub fn round_fps(fps: f64) -> u32 {
    if fps.is_finite() && fps >= 1.0 {
        (fps.round() as u32).clamp(1, 120)
    } else {
        30
    }
}

pub struct IntroLayout {
    pub title_size: u32,
    pub subtitle_size: u32,
    pub title_ys: Vec<u32>,
    pub subtitle_ys: Vec<u32>,
}

pub fn intro_layout(width: u32, height: u32, titles: usize, subtitles: usize) -> IntroLayout {
    let short = width.min(height) as f64;
    let title_size = (short * 0.085).round().max(8.0);
    let subtitle_size = (short * 0.045).round().max(6.0);
    let title_lh = title_size * 1.2;
    let subtitle_lh = subtitle_size * 1.3;
    let gap = if titles > 0 && subtitles > 0 {
        subtitle_size * 0.8
    } else {
        0.0
    };
    let block = titles as f64 * title_lh + gap + subtitles as f64 * subtitle_lh;
    let top = ((height as f64 - block) / 2.0).max(0.0);
    let title_ys = (0..titles)
        .map(|i| (top + i as f64 * title_lh + (title_lh - title_size) / 2.0).round() as u32)
        .collect();
    let sub_top = top + titles as f64 * title_lh + gap;
    let subtitle_ys = (0..subtitles)
        .map(|i| {
            (sub_top + i as f64 * subtitle_lh + (subtitle_lh - subtitle_size) / 2.0).round() as u32
        })
        .collect();
    IntroLayout {
        title_size: title_size as u32,
        subtitle_size: subtitle_size as u32,
        title_ys,
        subtitle_ys,
    }
}

fn drawtext(font: &Path, file: &Path, color: &str, size: u32, y: u32) -> String {
    format!(
        "drawtext=fontfile={}:textfile={}:expansion=none:fontcolor={}:fontsize={}:x=(w-text_w)/2:y={}",
        escape_filter_path(font),
        escape_filter_path(file),
        color,
        size,
        y
    )
}

pub fn build(spec: &GraphSpec) -> Result<Graph, String> {
    let src = &spec.source;
    if src.width < 2 || src.height < 2 {
        return Err("The video has no picture size, so it cannot be reframed.".to_string());
    }
    let (ow, oh) = output_size(src.width, src.height, spec.aspect, spec.mode);
    let fps = round_fps(src.fps);
    let duration = src.duration_secs.max(0.0);
    let mut chains: Vec<String> = Vec::new();
    let mut main: Vec<String> = Vec::new();

    match (aspect_ratio(spec.aspect), spec.mode) {
        (None, _) => main.push(format!("scale={ow}:{oh}")),
        (Some((aw, ah)), FrameMode::Crop) => {
            let c = crop_rect(src.width, src.height, aw, ah, spec.position);
            main.push(format!("crop={}:{}:{}:{}", c.w, c.h, c.x, c.y));
            main.push(format!("scale={ow}:{oh}"));
        }
        (Some(_), FrameMode::Fit) => {
            let radius = (ow.min(oh) / 40).clamp(4, 60);
            chains.push(format!(
                "[0:v]split=2[sbg][sfg];[sbg]scale={ow}:{oh}:force_original_aspect_ratio=increase,crop={ow}:{oh},boxblur={radius}:2[sbb];[sfg]scale={ow}:{oh}:force_original_aspect_ratio=decrease,setsar=1[sff];[sbb][sff]overlay=(W-w)/2:(H-h)/2[sfit]"
            ));
        }
    }

    if let Some(ass) = spec.ass {
        let mut f = format!("subtitles=filename={}", escape_filter_path(ass));
        if let Some(dir) = spec.fonts_dir {
            f.push_str(&format!(":fontsdir={}", escape_filter_path(dir)));
        }
        main.push(f);
    }
    if spec.intro.is_some() {
        main.push(format!("fps={fps}"));
    }
    main.push("setsar=1".to_string());
    main.push("format=yuv420p".to_string());
    let fade = spec.fade_out && duration > FADE_SECS * 2.0;
    let fade_start = (duration - FADE_SECS).max(0.0);
    if fade {
        main.push(format!("fade=t=out:st={fade_start:.3}:d={FADE_SECS}"));
    }
    let head = if chains.is_empty() { "[0:v]" } else { "[sfit]" };
    chains.push(format!("{head}{}[vm]", main.join(",")));

    if src.has_audio {
        let mut a =
            "[0:a:0]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo".to_string();
        if fade {
            a.push_str(&format!(",afade=t=out:st={fade_start:.3}:d={FADE_SECS}"));
        }
        a.push_str("[am]");
        chains.push(a);
    }

    let mut lavfi_inputs = Vec::new();
    let mut total = duration;
    let (video_label, audio_label) = match &spec.intro {
        None => (
            "[vm]".to_string(),
            src.has_audio.then(|| "[am]".to_string()),
        ),
        Some(files) => {
            let secs = intro_seconds(files.intro);
            total += secs;
            let bg = valid_hex(&files.intro.background).ok_or_else(|| {
                "The intro background color is not a valid hex color.".to_string()
            })?;
            let fg = valid_hex(&files.intro.color)
                .ok_or_else(|| "The intro text color is not a valid hex color.".to_string())?;
            lavfi_inputs.push(format!("color=c={bg}:s={ow}x{oh}:r={fps}:d={secs:.3}"));
            if src.has_audio {
                lavfi_inputs.push(format!(
                    "anullsrc=r=48000:cl=stereo,atrim=duration={secs:.3}"
                ));
            }
            let layout = intro_layout(ow, oh, files.title_files.len(), files.subtitle_files.len());
            let mut card: Vec<String> = Vec::new();
            for (file, y) in files.title_files.iter().zip(&layout.title_ys) {
                card.push(drawtext(files.font, file, &fg, layout.title_size, *y));
            }
            for (file, y) in files.subtitle_files.iter().zip(&layout.subtitle_ys) {
                card.push(drawtext(files.font, file, &fg, layout.subtitle_size, *y));
            }
            if files.intro.fade {
                let out = (secs - INTRO_FADE_SECS).max(0.0);
                card.push(format!("fade=t=in:st=0:d={INTRO_FADE_SECS}"));
                card.push(format!("fade=t=out:st={out:.3}:d={INTRO_FADE_SECS}"));
            }
            card.push("setsar=1".to_string());
            card.push("format=yuv420p".to_string());
            chains.push(format!("[1:v]{}[vi]", card.join(",")));
            if src.has_audio {
                chains.push("[2:a]aformat=sample_fmts=fltp:channel_layouts=stereo[ai]".to_string());
                chains.push("[vi][ai][vm][am]concat=n=2:v=1:a=1[vo][ao]".to_string());
                ("[vo]".to_string(), Some("[ao]".to_string()))
            } else {
                chains.push("[vi][vm]concat=n=2:v=1:a=0[vo]".to_string());
                ("[vo]".to_string(), None)
            }
        }
    };

    Ok(Graph {
        width: ow,
        height: oh,
        fps,
        lavfi_inputs,
        filter: chains.join(";"),
        video_label,
        audio_label,
        total_secs: total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn src(w: u32, h: u32, audio: bool) -> Source {
        Source {
            width: w,
            height: h,
            fps: 29.97,
            duration_secs: 10.0,
            has_audio: audio,
        }
    }

    fn spec(aspect: Aspect, mode: FrameMode) -> GraphSpec<'static> {
        GraphSpec {
            source: src(1920, 1080, true),
            aspect,
            mode,
            position: 0.5,
            ass: None,
            fonts_dir: None,
            intro: None,
            fade_out: false,
        }
    }

    #[test]
    fn even_rounds_down_to_even() {
        assert_eq!(even(607.5), 606);
        assert_eq!(even(1080.0), 1080);
        assert_eq!(even(1.0), 2);
        assert_eq!(even(-4.0), 2);
    }

    #[test]
    fn crop_landscape_to_vertical() {
        let c = crop_rect(1920, 1080, 9, 16, 0.5);
        assert_eq!((c.w, c.h, c.y), (606, 1080, 0));
        assert_eq!(c.x, 657);
        assert_eq!(crop_rect(1920, 1080, 9, 16, 0.0).x, 0);
        assert_eq!(crop_rect(1920, 1080, 9, 16, 1.0).x, 1314);
        assert_eq!(crop_rect(1920, 1080, 9, 16, f64::NAN).x, 657);
        assert_eq!(crop_rect(1920, 1080, 9, 16, 7.0).x, 1314);
    }

    #[test]
    fn crop_vertical_to_square_is_centered() {
        let c = crop_rect(1080, 1920, 1, 1, 0.9);
        assert_eq!((c.w, c.h, c.x, c.y), (1080, 1080, 0, 420));
    }

    #[test]
    fn output_sizes() {
        assert_eq!(
            output_size(1920, 1080, Aspect::Keep, FrameMode::Crop),
            (1920, 1080)
        );
        assert_eq!(
            output_size(1921, 1081, Aspect::Keep, FrameMode::Fit),
            (1920, 1080)
        );
        assert_eq!(
            output_size(1920, 1080, Aspect::Vertical, FrameMode::Fit),
            (1080, 1920)
        );
        assert_eq!(
            output_size(1920, 1080, Aspect::Portrait, FrameMode::Fit),
            (1080, 1350)
        );
        assert_eq!(
            output_size(1920, 1080, Aspect::Square, FrameMode::Crop),
            (1080, 1080)
        );
        assert_eq!(
            output_size(3840, 4000, Aspect::Square, FrameMode::Fit),
            (2160, 2160)
        );
    }

    #[test]
    fn escapes_paths_for_filtergraphs() {
        assert_eq!(escape_filter_value("/tmp/a.ass"), "/tmp/a.ass");
        assert_eq!(
            escape_filter_value("/x/we'ird, [dir]; x:y/s.ass"),
            r"/x/we\\\'ird\, \[dir\]\; x\\:y/s.ass"
        );
        assert_eq!(
            escape_filter_value("C:/Users/A B/sub.ass"),
            r"C\\:/Users/A B/sub.ass"
        );
        assert_eq!(escape_filter_value(r"a\b"), r"a\\\\b");
    }

    #[test]
    fn validates_hex_colors() {
        assert_eq!(valid_hex("#ff9f0a").as_deref(), Some("0xFF9F0A"));
        assert_eq!(valid_hex("123ABC").as_deref(), Some("0x123ABC"));
        assert_eq!(valid_hex("#fff"), None);
        assert_eq!(valid_hex("red:0"), None);
    }

    #[test]
    fn keep_graph_is_simple() {
        let g = build(&spec(Aspect::Keep, FrameMode::Crop)).unwrap();
        assert_eq!(
            g.filter,
            "[0:v]scale=1920:1080,setsar=1,format=yuv420p[vm];[0:a:0]aresample=48000,aformat=sample_fmts=fltp:channel_layouts=stereo[am]"
        );
        assert_eq!(g.video_label, "[vm]");
        assert_eq!(g.audio_label.as_deref(), Some("[am]"));
        assert!(g.lavfi_inputs.is_empty());
        assert_eq!(g.fps, 30);
    }

    #[test]
    fn crop_graph_with_captions_and_fade() {
        let ass = PathBuf::from("/tmp/job/captions.ass");
        let fonts = PathBuf::from("/tmp/job/fonts");
        let mut s = spec(Aspect::Vertical, FrameMode::Crop);
        s.ass = Some(&ass);
        s.fonts_dir = Some(&fonts);
        s.fade_out = true;
        s.position = 0.0;
        let g = build(&s).unwrap();
        assert_eq!((g.width, g.height), (606, 1080));
        assert!(g.filter.starts_with("[0:v]crop=606:1080:0:0,scale=606:1080,subtitles=filename=/tmp/job/captions.ass:fontsdir=/tmp/job/fonts,setsar=1,format=yuv420p,fade=t=out:st=9.400:d=0.6[vm]"));
        assert!(g.filter.contains("afade=t=out:st=9.400:d=0.6[am]"));
    }

    #[test]
    fn fit_graph_blurs_background() {
        let g = build(&spec(Aspect::Vertical, FrameMode::Fit)).unwrap();
        assert!(g.filter.starts_with("[0:v]split=2[sbg][sfg];"));
        assert!(g.filter.contains("boxblur=27:2"));
        assert!(g.filter.contains("[sfit]setsar=1,format=yuv420p[vm]"));
        assert_eq!((g.width, g.height), (1080, 1920));
    }

    #[test]
    fn intro_concats_with_silence() {
        let intro = Intro {
            seconds: 2.0,
            title_lines: vec!["Hi".into()],
            subtitle_lines: vec!["there".into()],
            background: "#101010".into(),
            color: "#ffffff".into(),
            fade: true,
        };
        let font = PathBuf::from("/f/bold.ttf");
        let t0 = PathBuf::from("/f/t0.txt");
        let s0 = PathBuf::from("/f/s0.txt");
        let mut s = spec(Aspect::Keep, FrameMode::Crop);
        s.intro = Some(IntroFiles {
            intro: &intro,
            font: &font,
            title_files: vec![&t0],
            subtitle_files: vec![&s0],
        });
        let g = build(&s).unwrap();
        assert_eq!(
            g.lavfi_inputs[0],
            "color=c=0x101010:s=1920x1080:r=30:d=2.000"
        );
        assert_eq!(
            g.lavfi_inputs[1],
            "anullsrc=r=48000:cl=stereo,atrim=duration=2.000"
        );
        assert!(g.filter.contains("fps=30,setsar=1"));
        assert!(g.filter.contains("drawtext=fontfile=/f/bold.ttf:textfile=/f/t0.txt:expansion=none:fontcolor=0xFFFFFF:fontsize=92"));
        assert!(g
            .filter
            .contains("fade=t=in:st=0:d=0.5,fade=t=out:st=1.500:d=0.5"));
        assert!(g
            .filter
            .ends_with("[vi][ai][vm][am]concat=n=2:v=1:a=1[vo][ao]"));
        assert_eq!(g.total_secs, 12.0);
        assert_eq!(g.audio_label.as_deref(), Some("[ao]"));
    }

    #[test]
    fn intro_without_audio_skips_audio_concat() {
        let intro = Intro {
            seconds: 99.0,
            title_lines: vec!["Hi".into()],
            subtitle_lines: vec![],
            background: "#000000".into(),
            color: "#ffffff".into(),
            fade: false,
        };
        let font = PathBuf::from("/f/bold.ttf");
        let t0 = PathBuf::from("/f/t0.txt");
        let mut s = spec(Aspect::Keep, FrameMode::Crop);
        s.source.has_audio = false;
        s.intro = Some(IntroFiles {
            intro: &intro,
            font: &font,
            title_files: vec![&t0],
            subtitle_files: vec![],
        });
        let g = build(&s).unwrap();
        assert_eq!(g.lavfi_inputs.len(), 1);
        assert!(g.filter.ends_with("[vi][vm]concat=n=2:v=1:a=0[vo]"));
        assert_eq!(g.audio_label, None);
        assert_eq!(g.total_secs, 25.0);
    }

    #[test]
    fn rejects_bad_intro_color() {
        let intro = Intro {
            seconds: 3.0,
            title_lines: vec![],
            subtitle_lines: vec![],
            background: "red".into(),
            color: "#ffffff".into(),
            fade: false,
        };
        let font = PathBuf::from("/f/bold.ttf");
        let mut s = spec(Aspect::Keep, FrameMode::Crop);
        s.intro = Some(IntroFiles {
            intro: &intro,
            font: &font,
            title_files: vec![],
            subtitle_files: vec![],
        });
        assert!(build(&s).is_err());
    }

    #[test]
    fn intro_layout_centers_block() {
        let l = intro_layout(1080, 1920, 2, 1);
        assert_eq!(l.title_size, 92);
        assert_eq!(l.subtitle_size, 49);
        assert_eq!(l.title_ys.len(), 2);
        assert!(l.title_ys[0] < l.title_ys[1]);
        assert!(l.subtitle_ys[0] > l.title_ys[1]);
        let block_mid = (l.title_ys[0] + l.subtitle_ys[0] + 49) / 2;
        assert!((block_mid as i64 - 960).abs() < 60);
    }
}
