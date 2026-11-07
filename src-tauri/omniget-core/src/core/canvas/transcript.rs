use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::core::asr::{Segment, Transcript};

pub const PART_MINUTES: i64 = 15;
const BLOCK_MS: i64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    M4a,
    Mp3,
    Wav,
}

impl AudioFormat {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "m4a" | "aac" => Some(Self::M4a),
            "mp3" => Some(Self::Mp3),
            "wav" => Some(Self::Wav),
            _ => None,
        }
    }

    pub fn ext(self) -> &'static str {
        match self {
            Self::M4a => "m4a",
            Self::Mp3 => "mp3",
            Self::Wav => "wav",
        }
    }

    fn codec_args(self) -> &'static [&'static str] {
        match self {
            Self::M4a => &["-c:a", "aac", "-b:a", "96k"],
            Self::Mp3 => &["-c:a", "libmp3lame", "-q:a", "4"],
            Self::Wav => &["-c:a", "pcm_s16le", "-ar", "16000", "-ac", "1"],
        }
    }
}

pub fn audio_target(src: &Path, dir: &Path, format: AudioFormat) -> PathBuf {
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "audio".into());
    let mut out = dir.join(format!("{stem}.{}", format.ext()));
    let mut n = 2;
    while out.exists() || out == src {
        out = dir.join(format!("{stem}-{n}.{}", format.ext()));
        n += 1;
    }
    out
}

pub async fn extract_audio(src: &Path, dst: &Path, format: AudioFormat) -> Result<PathBuf, String> {
    let ffmpeg = crate::core::dependencies::find_tool("ffmpeg")
        .await
        .ok_or("FFMPEG_MISSING: install FFmpeg in Settings → Dependencies")?;
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("AUDIO_FAILED: {e}"))?;
    }
    let out = crate::core::process::command(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(src)
        .args(["-vn", "-map", "0:a:0"])
        .args(format.codec_args())
        .arg(dst)
        .output()
        .await
        .map_err(|e| format!("AUDIO_FAILED: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let line = err
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("no audio track");
        return Err(format!("AUDIO_FAILED: {line}"));
    }
    Ok(dst.to_path_buf())
}

pub async fn duration_ms(src: &Path) -> Option<u64> {
    let ffprobe = crate::core::dependencies::find_tool("ffprobe").await?;
    let out = crate::core::process::command(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(src)
        .output()
        .await
        .ok()?;
    let secs: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    (secs.is_finite() && secs > 0.0).then_some((secs * 1000.0) as u64)
}

pub fn clock(ms: i64) -> String {
    let s = ms.max(0) / 1000;
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

fn srt_clock(ms: i64) -> String {
    let ms = ms.max(0);
    format!("{},{:03}", clock(ms), ms % 1000)
}

pub struct Block {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

pub fn blocks(segments: &[Segment], span_ms: i64) -> Vec<Block> {
    let mut out: Vec<Block> = Vec::new();
    for s in segments.iter().filter(|s| !s.text.trim().is_empty()) {
        match out.last_mut() {
            Some(b) if s.start_ms - b.start_ms < span_ms => {
                b.text.push(' ');
                b.text.push_str(s.text.trim());
                b.end_ms = s.end_ms;
            }
            _ => out.push(Block {
                start_ms: s.start_ms,
                end_ms: s.end_ms,
                text: s.text.trim().to_string(),
            }),
        }
    }
    out
}

pub struct Meta<'a> {
    pub title: &'a str,
    pub source: &'a Path,
    pub duration_ms: Option<u64>,
    pub engine: &'a str,
}

fn header(t: &Transcript, m: &Meta) -> String {
    let words = t.text.split_whitespace().count();
    format!(
        "# {}\n\n- Source: `{}`\n- Duration: {}\n- Language: {}\n- Transcript: {} ({})\n- Words: {words} (about {} tokens)\n",
        m.title,
        m.source.display(),
        m.duration_ms.map(|d| clock(d as i64)).unwrap_or_else(|| "?".into()),
        t.lang,
        m.engine,
        t.model,
        words * 4 / 3,
    )
}

pub fn transcript_markdown(t: &Transcript, m: &Meta) -> String {
    let mut s = header(t, m);
    s.push_str("\nEach paragraph starts with the time it was said, `[hh:mm:ss]`.\n\n");
    for b in blocks(&t.segments, BLOCK_MS) {
        s.push_str(&format!("[{}] {}\n\n", clock(b.start_ms), b.text));
    }
    s
}

pub fn srt(t: &Transcript) -> String {
    let mut s = String::new();
    for (i, seg) in t
        .segments
        .iter()
        .filter(|s| !s.text.trim().is_empty())
        .enumerate()
    {
        s.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            srt_clock(seg.start_ms),
            srt_clock(seg.end_ms),
            seg.text.trim()
        ));
    }
    s
}

pub fn parts(t: &Transcript) -> Vec<(i64, i64, String)> {
    let span = PART_MINUTES * 60_000;
    let mut out: Vec<(i64, i64, String)> = Vec::new();
    for b in blocks(&t.segments, BLOCK_MS) {
        let idx = (b.start_ms / span) as usize;
        while out.len() <= idx {
            let n = out.len() as i64;
            out.push((n * span, (n + 1) * span, String::new()));
        }
        out[idx]
            .2
            .push_str(&format!("[{}] {}\n\n", clock(b.start_ms), b.text));
    }
    out.retain(|p| !p.2.trim().is_empty());
    out
}

pub struct Written {
    pub dir: PathBuf,
    pub transcript: PathBuf,
    pub context: PathBuf,
    pub parts: Vec<PathBuf>,
}

pub fn context_placeholder(m: &Meta) -> String {
    format!(
        "# Context: {}\n\n_The context pack is being prepared. The full transcript is in `transcricao.md`._\n",
        m.title
    )
}

pub fn write_all(dir: &Path, t: &Transcript, m: &Meta) -> Result<Written, String> {
    let io = |e: std::io::Error| format!("TRANSCRIPT_WRITE: {e}");
    std::fs::create_dir_all(dir.join("partes")).map_err(io)?;
    let transcript = dir.join("transcricao.md");
    std::fs::write(&transcript, transcript_markdown(t, m)).map_err(io)?;
    std::fs::write(dir.join("transcricao.srt"), srt(t)).map_err(io)?;
    std::fs::write(
        dir.join("segmentos.json"),
        serde_json::to_string(&t.segments).map_err(|e| format!("TRANSCRIPT_WRITE: {e}"))?,
    )
    .map_err(io)?;
    let mut written = Vec::new();
    let all = parts(t);
    for (i, (from, to, body)) in all.iter().enumerate() {
        let p = dir.join("partes").join(format!("parte-{:02}.md", i + 1));
        std::fs::write(
            &p,
            format!(
                "# Part {} of {}: {} to {}\n\n{body}",
                i + 1,
                all.len(),
                clock(*from),
                clock(*to)
            ),
        )
        .map_err(io)?;
        written.push(p);
    }
    let context = dir.join("contexto.md");
    if !context.exists() {
        std::fs::write(&context, context_placeholder(m)).map_err(io)?;
    }
    Ok(Written {
        dir: dir.to_path_buf(),
        transcript,
        context,
        parts: written,
    })
}

pub fn summarizer_prompt(dir: &Path, parts: usize, title: &str) -> String {
    format!(
        "You turn a long recording into a context pack for another LLM (Claude or any model), not into minutes for people. Write in the recording's language, section titles included.\n\
         Folder: {dir}. The full transcript is transcricao.md; it is split into {parts} parts in partes/parte-NN.md (about {PART_MINUTES} minutes each, every paragraph starts with [hh:mm:ss]).\n\
         Method (map, then reduce):\n\
         1. For each part, read only that part and write partes/notas-NN.md: what was discussed, decisions, action items (owner, due date if said), open questions, numbers and facts, names and terms, and the 2-3 most important literal quotes, every item with its [hh:mm:ss].\n\
         2. Then read only the notes and write contexto.md (replace it) with these sections:\n\
         - \"How to use this context\": source ({title}), duration, language, and that any [hh:mm:ss] can be looked up in transcricao.md for the exact words.\n\
         - \"Summary\": 5-8 lines a model needs before anything else.\n\
         - \"Timeline\": 6-15 chapters, each with its time range, a title and 2-4 dense bullets.\n\
         - \"Decisions\", \"Actions\" (who, what, when, [hh:mm:ss]), \"Open questions\", \"Risks\".\n\
         - \"Facts and numbers\", \"People and terms\" (a short glossary), \"Quotes\" (literal, with [hh:mm:ss]).\n\
         Rules: never invent what is not in the transcript; mark anything uncertain (speaker unclear, garbled audio) as such; keep contexto.md under about 2,500 words so it fits any context window; prefer facts over adjectives.\n\
         When contexto.md is written, run `omniget canvas say \"Context ready: contexto.md\"` if that command exists, and stop.",
        dir = dir.display()
    )
}

pub fn slug(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let s = s
        .split('-')
        .filter(|p| !p.is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() {
        "gravacao".into()
    } else {
        s
    }
}

pub fn is_media(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "mp4"
            | "mov"
            | "mkv"
            | "webm"
            | "avi"
            | "m4v"
            | "mpg"
            | "mpeg"
            | "wmv"
            | "flv"
            | "ts"
            | "mp3"
            | "m4a"
            | "aac"
            | "wav"
            | "flac"
            | "ogg"
            | "opus"
            | "wma"
            | "aiff"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: i64, text: &str) -> Segment {
        Segment {
            start_ms: start,
            end_ms: start + 4000,
            text: text.into(),
        }
    }

    #[test]
    fn segments_group_into_minute_blocks_and_quarter_hour_parts() {
        let t = Transcript {
            lang: "pt".into(),
            text: "a b c".into(),
            segments: vec![
                seg(0, "olá"),
                seg(30_000, "tudo bem"),
                seg(61_000, "seguindo"),
                seg(16 * 60_000, "parte dois"),
            ],
            model: "m".into(),
        };
        let b = blocks(&t.segments, BLOCK_MS);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].text, "olá tudo bem");
        let p = parts(&t);
        assert_eq!(p.len(), 2);
        assert!(p[1].2.starts_with("[00:16:00] parte dois"));
        assert!(srt(&t).starts_with("1\n00:00:00,000 --> 00:00:04,000\nolá"));
    }

    #[test]
    fn formats_and_names() {
        assert_eq!(AudioFormat::parse("MP3"), Some(AudioFormat::Mp3));
        assert_eq!(AudioFormat::parse("ogg"), None);
        assert_eq!(clock(3_725_000), "01:02:05");
        assert_eq!(
            slug("Reunião de Planejamento.mp4"),
            "reunião-de-planejamento-mp4"
        );
        assert!(is_media(Path::new("/x/call.MOV")));
        assert!(!is_media(Path::new("/x/notes.md")));
    }
}
