//! Audio conversion presets turned into ffmpeg arguments. Tags and cover art
//! are carried over; the caller runs ffmpeg and reports progress.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preset {
    Mp3V0,
    Mp3_320,
    Mp3_192,
    Aac256,
    Opus160,
    Opus96,
    Vorbis,
    Flac,
    Alac,
    Wav,
}

impl Preset {
    pub fn extension(self) -> &'static str {
        match self {
            Preset::Mp3V0 | Preset::Mp3_320 | Preset::Mp3_192 => "mp3",
            Preset::Aac256 | Preset::Alac => "m4a",
            Preset::Opus160 | Preset::Opus96 => "opus",
            Preset::Vorbis => "ogg",
            Preset::Flac => "flac",
            Preset::Wav => "wav",
        }
    }

    fn codec_args(self) -> Vec<&'static str> {
        match self {
            Preset::Mp3V0 => vec!["-c:a", "libmp3lame", "-q:a", "0"],
            Preset::Mp3_320 => vec!["-c:a", "libmp3lame", "-b:a", "320k"],
            Preset::Mp3_192 => vec!["-c:a", "libmp3lame", "-b:a", "192k"],
            Preset::Aac256 => vec!["-c:a", "aac", "-b:a", "256k"],
            Preset::Opus160 => vec!["-c:a", "libopus", "-b:a", "160k", "-vbr", "on"],
            Preset::Opus96 => vec!["-c:a", "libopus", "-b:a", "96k", "-vbr", "on"],
            Preset::Vorbis => vec!["-c:a", "libvorbis", "-q:a", "6"],
            Preset::Flac => vec!["-c:a", "flac", "-compression_level", "8"],
            Preset::Alac => vec!["-c:a", "alac"],
            Preset::Wav => vec!["-c:a", "pcm_s16le"],
        }
    }

    /// Containers that can carry an attached cover picture.
    fn keeps_cover(self) -> bool {
        matches!(
            self,
            Preset::Mp3V0
                | Preset::Mp3_320
                | Preset::Mp3_192
                | Preset::Aac256
                | Preset::Alac
                | Preset::Flac
        )
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConvertOptions {
    /// Resample (e.g. 44100, 48000). None keeps the source rate (Opus is
    /// always 48 kHz internally).
    #[serde(default)]
    pub sample_rate: Option<u32>,
    /// Apply this gain (dB) while converting, e.g. the ReplayGain value, to
    /// bake normalization into the file.
    #[serde(default)]
    pub gain_db: Option<f64>,
    /// Cut leading/trailing silence.
    #[serde(default)]
    pub trim_silence: bool,
    /// Downmix to mono.
    #[serde(default)]
    pub mono: bool,
    #[serde(default)]
    pub start_s: Option<f64>,
    #[serde(default)]
    pub end_s: Option<f64>,
    #[serde(default)]
    pub fade_in_s: Option<f64>,
    #[serde(default)]
    pub fade_out_s: Option<f64>,
}

fn filters(o: &ConvertOptions, duration_s: Option<f64>) -> Vec<String> {
    let mut f = Vec::new();
    if o.trim_silence {
        // leading silence, then reverse trick for the tail
        f.push("silenceremove=start_periods=1:start_threshold=-50dB".to_string());
        f.push("areverse".to_string());
        f.push("silenceremove=start_periods=1:start_threshold=-50dB".to_string());
        f.push("areverse".to_string());
    }
    if let Some(g) = o.gain_db.filter(|g| g.abs() > 0.001) {
        f.push(format!("volume={g:.2}dB"));
    }
    if let Some(fi) = o.fade_in_s.filter(|v| *v > 0.0) {
        f.push(format!("afade=t=in:st=0:d={fi:.3}"));
    }
    if let Some(fo) = o.fade_out_s.filter(|v| *v > 0.0) {
        let end = match (o.end_s, o.start_s, duration_s) {
            (Some(e), s, _) => e - s.unwrap_or(0.0),
            (None, s, Some(d)) => d - s.unwrap_or(0.0),
            _ => 0.0,
        };
        if end > fo {
            f.push(format!("afade=t=out:st={:.3}:d={fo:.3}", end - fo));
        }
    }
    f
}

/// Full ffmpeg argument list (without the program name).
pub fn ffmpeg_args(
    input: &str,
    output: &str,
    preset: Preset,
    o: &ConvertOptions,
    duration_s: Option<f64>,
) -> Vec<String> {
    let mut a: Vec<String> = vec!["-hide_banner".into(), "-nostdin".into(), "-y".into()];
    if let Some(s) = o.start_s.filter(|s| *s > 0.0) {
        a.extend(["-ss".into(), format!("{s:.3}")]);
    }
    if let Some(e) = o.end_s {
        a.extend(["-to".into(), format!("{e:.3}")]);
    }
    a.extend(["-i".into(), input.into()]);
    a.extend(["-map".into(), "0:a:0".into()]);
    if preset.keeps_cover() {
        a.extend(["-map".into(), "0:v?".into(), "-c:v".into(), "copy".into()]);
        a.extend(["-disposition:v".into(), "attached_pic".into()]);
    } else {
        a.push("-vn".into());
    }
    a.extend(["-map_metadata".into(), "0".into()]);
    if preset.extension() == "mp3" {
        a.extend(["-id3v2_version".into(), "3".into()]);
    }
    let f = filters(o, duration_s);
    if !f.is_empty() {
        a.extend(["-af".into(), f.join(",")]);
    }
    a.extend(preset.codec_args().into_iter().map(String::from));
    match (preset, o.sample_rate) {
        (Preset::Opus160 | Preset::Opus96, _) => {}
        (_, Some(r)) => a.extend(["-ar".into(), r.to_string()]),
        _ => {}
    }
    if o.mono {
        a.extend(["-ac".into(), "1".into()]);
    }
    a.extend(["-progress".into(), "pipe:1".into(), "-nostats".into()]);
    a.push(output.into());
    a
}

/// `out_time_us=123456` lines from `-progress pipe:1`.
pub fn parse_progress_line(line: &str) -> Option<f64> {
    let v = line
        .strip_prefix("out_time_us=")
        .or_else(|| line.strip_prefix("out_time_ms="))?;
    v.trim().parse::<f64>().ok().map(|us| us / 1_000_000.0)
}
