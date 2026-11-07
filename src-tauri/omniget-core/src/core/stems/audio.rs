use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};

use super::separate::{PcmSink, PcmSource, CHANNELS, SAMPLE_RATE};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StemFormat {
    #[default]
    Flac,
    Mp3,
    Wav,
}

impl StemFormat {
    pub fn extension(self) -> &'static str {
        match self {
            StemFormat::Flac => "flac",
            StemFormat::Mp3 => "mp3",
            StemFormat::Wav => "wav",
        }
    }

    fn codec_args(self) -> &'static [&'static str] {
        match self {
            StemFormat::Flac => &["-c:a", "flac", "-sample_fmt", "s16"],
            StemFormat::Mp3 => &["-c:a", "libmp3lame", "-b:a", "320k"],
            StemFormat::Wav => &["-c:a", "pcm_s16le"],
        }
    }

    fn muxer(self) -> &'static str {
        match self {
            StemFormat::Flac => "flac",
            StemFormat::Mp3 => "mp3",
            StemFormat::Wav => "wav",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct StemTags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: String,
    pub year: Option<u32>,
}

fn tail_reader(mut r: impl Read + Send + 'static) -> Arc<Mutex<String>> {
    let tail = Arc::new(Mutex::new(String::new()));
    let t = tail.clone();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(&mut r);
        let mut buf = Vec::new();
        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            let line = String::from_utf8_lossy(&buf);
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(mut g) = t.lock() {
                *g = line.chars().take(400).collect();
            }
        }
    });
    tail
}

fn last_line(tail: &Arc<Mutex<String>>) -> String {
    tail.lock().map(|g| g.clone()).unwrap_or_default()
}

pub fn decode_args(input: &Path) -> Vec<String> {
    let mut a: Vec<String> = ["-hide_banner", "-nostdin", "-loglevel", "error", "-i"]
        .map(String::from)
        .to_vec();
    a.push(input.to_string_lossy().to_string());
    a.extend(
        [
            "-map",
            "0:a:0",
            "-vn",
            "-ac",
            &CHANNELS.to_string(),
            "-ar",
            &SAMPLE_RATE.to_string(),
            "-f",
            "f32le",
            "-acodec",
            "pcm_f32le",
            "pipe:1",
        ]
        .map(String::from),
    );
    a
}

pub fn encode_args(output: &Path, format: StemFormat, tags: &StemTags) -> Vec<String> {
    let mut a: Vec<String> = [
        "-hide_banner",
        "-nostdin",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "f32le",
        "-ar",
        &SAMPLE_RATE.to_string(),
        "-ac",
        &CHANNELS.to_string(),
        "-i",
        "pipe:0",
    ]
    .map(String::from)
    .to_vec();
    a.extend(format.codec_args().iter().map(|s| s.to_string()));
    let year = tags.year.map(|y| y.to_string()).unwrap_or_default();
    for (k, v) in [
        ("title", tags.title.as_str()),
        ("artist", tags.artist.as_str()),
        ("album", tags.album.as_str()),
        ("album_artist", tags.album_artist.as_str()),
        ("genre", tags.genre.as_str()),
        ("date", year.as_str()),
    ] {
        if !v.trim().is_empty() {
            a.push("-metadata".into());
            a.push(format!("{k}={}", v.trim()));
        }
    }
    a.push("-f".into());
    a.push(format.muxer().into());
    a.push(output.to_string_lossy().to_string());
    a
}

pub struct FfmpegSource {
    child: Child,
    stdout: ChildStdout,
    carry: Vec<u8>,
    tail: Arc<Mutex<String>>,
    got: bool,
}

impl FfmpegSource {
    pub fn open(input: &Path) -> anyhow::Result<Self> {
        let mut child = crate::core::process::std_command("ffmpeg")
            .args(decode_args(input))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("ffmpeg is not available")?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("ffmpeg without stdout"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("ffmpeg without stderr"))?;
        Ok(Self {
            child,
            stdout,
            carry: Vec::new(),
            tail: tail_reader(stderr),
            got: false,
        })
    }
}

impl PcmSource for FfmpegSource {
    fn read(&mut self, out: &mut [f32]) -> anyhow::Result<usize> {
        let mut bytes = vec![0u8; out.len() * 4];
        loop {
            let have = self.carry.len();
            bytes[..have].copy_from_slice(&self.carry);
            let n = self
                .stdout
                .read(&mut bytes[have..])
                .context("reading ffmpeg")?;
            let total = have + n;
            if n == 0 {
                if !self.got {
                    let ok = self.child.wait().map(|s| s.success()).unwrap_or(false);
                    let why = last_line(&self.tail);
                    return Err(match (ok, why.is_empty()) {
                        (true, _) => anyhow!("this file has no audio"),
                        (false, true) => anyhow!("ffmpeg could not read this file"),
                        (false, false) => anyhow!("ffmpeg could not read this file: {why}"),
                    });
                }
                let ok = self.child.wait().map(|s| s.success()).unwrap_or(false);
                if !ok {
                    let why = last_line(&self.tail);
                    return Err(if why.is_empty() {
                        anyhow!("ffmpeg stopped before the end of this file")
                    } else {
                        anyhow!("ffmpeg stopped before the end of this file: {why}")
                    });
                }
                return Ok(0);
            }
            let whole = total / 4 * 4;
            self.carry = bytes[whole..total].to_vec();
            if whole == 0 {
                continue;
            }
            for (o, b) in out.iter_mut().zip(bytes[..whole].chunks_exact(4)) {
                *o = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            }
            self.got = true;
            return Ok(whole / 4);
        }
    }
}

impl Drop for FfmpegSource {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct FfmpegSink {
    child: Child,
    stdin: Option<ChildStdin>,
    tail: Arc<Mutex<String>>,
    bytes: Vec<u8>,
}

impl FfmpegSink {
    pub fn create(output: &Path, format: StemFormat, tags: &StemTags) -> anyhow::Result<Self> {
        let mut child = crate::core::process::std_command("ffmpeg")
            .args(encode_args(output, format, tags))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("ffmpeg is not available")?;
        let stdin = child.stdin.take();
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("ffmpeg without stderr"))?;
        Ok(Self {
            child,
            stdin,
            tail: tail_reader(stderr),
            bytes: Vec::new(),
        })
    }

    pub fn finish(mut self) -> anyhow::Result<()> {
        drop(self.stdin.take());
        let status = self.child.wait().context("waiting for ffmpeg")?;
        if status.success() {
            return Ok(());
        }
        let why = last_line(&self.tail);
        Err(if why.is_empty() {
            anyhow!("ffmpeg could not write the file")
        } else {
            anyhow!("ffmpeg could not write the file: {why}")
        })
    }
}

impl PcmSink for FfmpegSink {
    fn write(&mut self, interleaved: &[f32]) -> anyhow::Result<()> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("the encoder is closed"))?;
        self.bytes.clear();
        self.bytes.reserve(interleaved.len() * 4);
        for v in interleaved {
            self.bytes
                .extend_from_slice(&v.clamp(-1.0, 1.0).to_le_bytes());
        }
        if let Err(e) = stdin.write_all(&self.bytes) {
            let why = last_line(&self.tail);
            return Err(if why.is_empty() {
                anyhow!("the encoder stopped: {e}")
            } else {
                anyhow!("the encoder stopped: {why}")
            });
        }
        Ok(())
    }
}

impl Drop for FfmpegSink {
    fn drop(&mut self) {
        if self.stdin.is_some() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_args_skip_empty_tags() {
        let tags = StemTags {
            title: "Song (Instrumental)".into(),
            artist: "Band".into(),
            year: Some(1999),
            ..Default::default()
        };
        let a = encode_args(Path::new("/tmp/x.flac"), StemFormat::Flac, &tags);
        assert!(a.contains(&"title=Song (Instrumental)".to_string()));
        assert!(a.contains(&"date=1999".to_string()));
        assert!(!a.iter().any(|s| s.starts_with("album=")));
        assert_eq!(a.last().map(String::as_str), Some("/tmp/x.flac"));
    }

    #[test]
    fn decode_args_force_stereo_44k() {
        let a = decode_args(Path::new("in.mkv"));
        let at = |k: &str| a.iter().position(|s| s == k).map(|i| a[i + 1].clone());
        assert_eq!(at("-ac").as_deref(), Some("2"));
        assert_eq!(at("-ar").as_deref(), Some("44100"));
        assert_eq!(at("-map").as_deref(), Some("0:a:0"));
    }
}
