//! ffprobe of a video file and the "can the webview play this?" decision.
//!
//! The decision is an informed first guess, never the last word: the player
//! reports a failure and `video_play_fallback` moves one layer down
//! (direct → remux → transcode_audio → transcode).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{Chapter, PlayMode, StreamInfo};

pub const VIDEO_EXTS: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "wmv", "flv", "ts", "m2ts", "mts", "mpg", "mpeg",
    "ogv", "3gp", "divx", "vob", "asf", "rmvb", "f4v",
];

pub const SUB_EXTS: &[&str] = &["srt", "vtt", "ass", "ssa"];

/// Subtitle codecs ffmpeg can turn into WebVTT. Bitmap ones (PGS, VobSub)
/// would need burning in, which is left to the external player.
pub const TEXT_SUB_CODECS: &[&str] = &["subrip", "srt", "ass", "ssa", "mov_text", "webvtt", "text"];

pub fn ext_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

pub fn is_video_path(path: &Path) -> bool {
    VIDEO_EXTS.contains(&ext_of(path).as_str())
}

pub fn video_mime(path: &str) -> &'static str {
    match ext_of(Path::new(path)).as_str() {
        "mp4" | "m4v" | "f4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "ogv" => "video/ogg",
        "avi" => "video/x-msvideo",
        "ts" | "m2ts" | "mts" => "video/mp2t",
        "mpg" | "mpeg" | "vob" => "video/mpeg",
        "wmv" | "asf" => "video/x-ms-wmv",
        "flv" => "video/x-flv",
        "3gp" => "video/3gpp",
        _ => "application/octet-stream",
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ProbeData {
    pub duration_ms: i64,
    /// ffprobe `format_name` ("mov,mp4,m4a,3gp,3g2,mj2", "matroska,webm", ...)
    pub format: String,
    pub bitrate: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub vcodec: String,
    pub acodec: String,
    pub v_profile: String,
    pub v_pix_fmt: String,
    pub streams: Vec<StreamInfo>,
    pub chapters: Vec<Chapter>,
}

impl ProbeData {
    pub fn audio_streams(&self) -> impl Iterator<Item = &StreamInfo> {
        self.streams.iter().filter(|s| s.kind == "audio")
    }
    pub fn text_subs(&self) -> impl Iterator<Item = &StreamInfo> {
        self.streams
            .iter()
            .filter(|s| s.kind == "subtitle" && TEXT_SUB_CODECS.contains(&s.codec.as_str()))
    }
}

fn num_str(v: &serde_json::Value) -> Option<f64> {
    v.as_str()
        .and_then(|s| s.parse::<f64>().ok())
        .or_else(|| v.as_f64())
}

fn tag(v: &serde_json::Value, name: &str) -> String {
    v.get("tags")
        .and_then(|t| {
            t.as_object().and_then(|o| {
                o.iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(name))
                    .and_then(|(_, v)| v.as_str())
            })
        })
        .unwrap_or("")
        .to_string()
}

/// Full ffprobe (format, streams, chapters). `ffmpeg::probe` in the core
/// leaves out chapters, languages and dispositions, which the player needs.
pub async fn probe(path: &Path) -> Result<ProbeData, String> {
    let out = omniget_core::core::process::command("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            "-show_chapters",
        ])
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("ffprobe unavailable: {e}"))?;
    if !out.status.success() {
        return Err("ffprobe could not read the file".into());
    }
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    Ok(parse_probe(&json))
}

pub fn parse_probe(json: &serde_json::Value) -> ProbeData {
    let mut d = ProbeData::default();
    let fmt = &json["format"];
    d.format = fmt["format_name"].as_str().unwrap_or("").to_string();
    d.duration_ms = num_str(&fmt["duration"])
        .map(|s| (s * 1000.0) as i64)
        .unwrap_or(0);
    d.bitrate = num_str(&fmt["bit_rate"]).map(|b| b as i64);
    let empty = Vec::new();
    for s in json["streams"].as_array().unwrap_or(&empty) {
        let kind = s["codec_type"].as_str().unwrap_or("");
        if !matches!(kind, "video" | "audio" | "subtitle") {
            continue;
        }
        let disp = &s["disposition"];
        let flag = |k: &str| disp[k].as_i64().unwrap_or(0) != 0;
        // cover art is a "video" stream too
        if kind == "video" && flag("attached_pic") {
            continue;
        }
        let codec = s["codec_name"].as_str().unwrap_or("").to_string();
        let info = StreamInfo {
            index: s["index"].as_i64().unwrap_or(0),
            kind: kind.to_string(),
            codec: codec.clone(),
            lang: tag(s, "language"),
            title: tag(s, "title"),
            default: flag("default"),
            forced: flag("forced"),
            channels: s["channels"].as_i64(),
            width: s["width"].as_i64(),
            height: s["height"].as_i64(),
        };
        if kind == "video" && d.vcodec.is_empty() {
            d.vcodec = codec;
            d.width = info.width;
            d.height = info.height;
            d.v_profile = s["profile"].as_str().unwrap_or("").to_string();
            d.v_pix_fmt = s["pix_fmt"].as_str().unwrap_or("").to_string();
            if d.duration_ms == 0 {
                d.duration_ms = num_str(&s["duration"])
                    .map(|v| (v * 1000.0) as i64)
                    .unwrap_or(0);
            }
        }
        d.streams.push(info);
    }
    // the default audio track decides the audio codec
    let audio: Vec<&StreamInfo> = d.audio_streams().collect();
    if let Some(a) = audio.iter().find(|a| a.default).or(audio.first()) {
        d.acodec = a.codec.clone();
    }
    for c in json["chapters"].as_array().unwrap_or(&empty) {
        let start = num_str(&c["start_time"]).unwrap_or(0.0);
        let end = num_str(&c["end_time"]).unwrap_or(start);
        d.chapters.push(Chapter {
            start_ms: (start * 1000.0) as i64,
            end_ms: (end * 1000.0) as i64,
            title: tag(c, "title"),
            kind: None,
        });
    }
    d
}

// ---------- the decision ----------

fn container_ok(ext: &str, format: &str) -> bool {
    match ext {
        "mp4" | "m4v" | "mov" => format.contains("mp4") || format.contains("mov"),
        "webm" => format.contains("webm") || format.contains("matroska"),
        _ => false,
    }
}

fn video_ok(codec: &str, profile: &str, pix_fmt: &str) -> bool {
    let ten_bit = pix_fmt.contains("10") || pix_fmt.contains("12");
    match codec {
        "" => true,
        // browsers decode 8-bit H.264 only (no High 10 / 4:2:2)
        "h264" => !ten_bit && !profile.contains("10") && !profile.contains("4:2:2"),
        "vp8" | "vp9" | "av1" => true,
        "hevc" => cfg!(target_os = "macos"),
        _ => false,
    }
}

pub fn audio_ok(codec: &str) -> bool {
    match codec {
        "" | "aac" | "mp3" | "opus" | "vorbis" | "flac" => true,
        // WebKit decodes Dolby Digital
        "ac3" | "eac3" => cfg!(target_os = "macos"),
        _ => false,
    }
}

fn codec_label(c: &str) -> String {
    match c {
        "h264" => "H.264".into(),
        "hevc" => "HEVC".into(),
        "mpeg4" => "MPEG-4 Part 2".into(),
        "mpeg2video" => "MPEG-2".into(),
        "vc1" => "VC-1".into(),
        "ac3" => "Dolby Digital".into(),
        "eac3" => "Dolby Digital Plus".into(),
        "truehd" => "Dolby TrueHD".into(),
        "dts" => "DTS".into(),
        "" => "unknown".into(),
        other => other.to_uppercase(),
    }
}

/// First guess at how to deliver the file, with the reason in plain English.
/// `audio_rel` is the chosen audio track (0 = first audio stream).
pub fn decide(path: &Path, d: &ProbeData, audio_rel: usize) -> (PlayMode, String) {
    let ext = ext_of(path);
    let acodec = d
        .audio_streams()
        .nth(audio_rel)
        .map(|s| s.codec.clone())
        .unwrap_or_else(|| d.acodec.clone());
    if !video_ok(&d.vcodec, &d.v_profile, &d.v_pix_fmt) {
        let why = if d.vcodec == "h264" {
            "10-bit H.264".to_string()
        } else {
            codec_label(&d.vcodec)
        };
        return (
            PlayMode::Transcode,
            format!("{why} video can't play here, converting to H.264"),
        );
    }
    if !audio_ok(&acodec) {
        return (
            PlayMode::TranscodeAudio,
            format!(
                "{} audio can't play here, converting the audio to AAC",
                codec_label(&acodec)
            ),
        );
    }
    if !container_ok(&ext, &d.format) {
        return (
            PlayMode::Remux,
            format!(
                "{} file repackaged to MP4 without re-encoding",
                if ext.is_empty() {
                    "This".to_string()
                } else {
                    ext.to_uppercase()
                }
            ),
        );
    }
    if audio_rel > 0 {
        return (
            PlayMode::Remux,
            "Repackaged to switch the audio track".to_string(),
        );
    }
    (
        PlayMode::Direct,
        format!(
            "Playing the file directly ({} + {})",
            codec_label(&d.vcodec),
            codec_label(&acodec)
        ),
    )
}

/// The next layer after `failed`.
pub fn next_mode(failed: PlayMode) -> Option<PlayMode> {
    match failed {
        PlayMode::Direct => Some(PlayMode::Remux),
        PlayMode::Remux => Some(PlayMode::TranscodeAudio),
        PlayMode::TranscodeAudio => Some(PlayMode::Transcode),
        _ => None,
    }
}

pub fn fallback_reason(failed: PlayMode, next: PlayMode) -> String {
    let what = match failed {
        PlayMode::Direct => "Direct play failed",
        PlayMode::Remux => "Repackaging failed",
        PlayMode::TranscodeAudio => "Audio conversion failed",
        _ => "Playback failed",
    };
    let then = match next {
        PlayMode::Remux => "repackaging to MP4",
        PlayMode::TranscodeAudio => "converting the audio to AAC",
        PlayMode::Transcode => "converting to H.264",
        _ => "trying again",
    };
    format!("{what}, {then}")
}

// ---------- external subtitles ----------

/// Subtitle files next to the video with the same base name: `movie.srt`,
/// `movie.pt-BR.srt`, `movie.en.forced.ass`. Returns `(path, lang, forced)`.
pub fn external_subs(video: &Path) -> Vec<(PathBuf, String, bool)> {
    let (Some(dir), Some(stem)) = (video.parent(), video.file_stem().and_then(|s| s.to_str()))
    else {
        return Vec::new();
    };
    let stem_l = stem.to_lowercase();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let p = e.path();
        if !SUB_EXTS.contains(&ext_of(&p).as_str()) {
            continue;
        }
        let Some(sstem) = p.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let sl = sstem.to_lowercase();
        if sl != stem_l && !sl.starts_with(&format!("{stem_l}.")) {
            continue;
        }
        let rest = &sstem[stem.len().min(sstem.len())..];
        let mut lang = String::new();
        let mut forced = false;
        for part in rest.split('.').filter(|s| !s.is_empty()) {
            let pl = part.to_ascii_lowercase();
            if pl == "forced" {
                forced = true;
            } else if matches!(pl.as_str(), "sdh" | "cc" | "hi" | "default") {
            } else if lang.is_empty() && part.len() <= 8 {
                lang = part.to_string();
            }
        }
        out.push((p, lang, forced));
    }
    out.sort_by(|a, b| {
        omniget_core::core::video::naming::natural_cmp(
            &a.0.to_string_lossy(),
            &b.0.to_string_lossy(),
        )
    });
    out
}

/// Language code to a short English label ("pt-BR" → "Portuguese (BR)").
pub fn lang_label(lang: &str) -> String {
    let (base, region) = match lang.split_once(['-', '_']) {
        Some((b, r)) => (b.to_ascii_lowercase(), Some(r.to_ascii_uppercase())),
        None => (lang.to_ascii_lowercase(), None),
    };
    let name = match base.as_str() {
        "en" | "eng" => "English",
        "pt" | "por" | "pob" => "Portuguese",
        "es" | "spa" => "Spanish",
        "fr" | "fre" | "fra" => "French",
        "de" | "ger" | "deu" => "German",
        "it" | "ita" => "Italian",
        "ja" | "jpn" => "Japanese",
        "zh" | "chi" | "zho" => "Chinese",
        "ko" | "kor" => "Korean",
        "ru" | "rus" => "Russian",
        "ar" | "ara" => "Arabic",
        "nl" | "dut" | "nld" => "Dutch",
        "pl" | "pol" => "Polish",
        "tr" | "tur" => "Turkish",
        "sv" | "swe" => "Swedish",
        "el" | "gre" | "ell" => "Greek",
        "fa" | "per" | "fas" => "Persian",
        "hi" | "hin" => "Hindi",
        "uk" | "ukr" => "Ukrainian",
        "und" | "" => return String::new(),
        _ => return lang.to_string(),
    };
    match region {
        Some(r) => format!("{name} ({r})"),
        None => name.to_string(),
    }
}
