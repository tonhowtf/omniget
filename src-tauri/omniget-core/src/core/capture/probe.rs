use std::path::Path;
use std::process::Stdio;

use serde::{Deserialize, Serialize};

use super::errors::{code_message, tail, NOT_FOUND, NO_FFMPEG, PROBE_FAILED};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoStream {
    pub codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioStream {
    pub index: u32,
    pub codec: String,
    pub channels: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeResult {
    pub path: String,
    pub name: String,
    pub size_bytes: u64,
    pub duration: f64,
    pub format_name: String,
    pub video: Option<VideoStream>,
    pub audio: Vec<AudioStream>,
    pub keyframes: Vec<f64>,
}

fn rate(v: Option<&serde_json::Value>) -> Option<f64> {
    let text = v?.as_str()?;
    let (n, d) = text.split_once('/').unwrap_or((text, "1"));
    let n: f64 = n.parse().ok()?;
    let d: f64 = d.parse().ok()?;
    (d > 0.0 && n > 0.0).then(|| n / d)
}

fn num(v: Option<&serde_json::Value>) -> Option<f64> {
    match v? {
        serde_json::Value::String(s) => s.parse().ok(),
        serde_json::Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

pub struct ProbeMeta {
    pub duration: f64,
    pub start_time: f64,
    pub format_name: String,
    pub video: Option<VideoStream>,
    pub audio: Vec<AudioStream>,
}

pub fn parse_probe_json(json: &str) -> Result<ProbeMeta, String> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| code_message(PROBE_FAILED, &e.to_string()))?;
    let format = v.get("format");
    let start_time = num(format.and_then(|f| f.get("start_time")))
        .filter(|s| s.is_finite())
        .unwrap_or(0.0);
    let mut duration = num(format.and_then(|f| f.get("duration"))).unwrap_or(0.0);
    let format_name = format
        .and_then(|f| f.get("format_name"))
        .and_then(|f| f.as_str())
        .unwrap_or("")
        .to_string();
    let mut video = None;
    let mut audio = Vec::new();
    for st in v
        .get("streams")
        .and_then(|s| s.as_array())
        .into_iter()
        .flatten()
    {
        let kind = st.get("codec_type").and_then(|c| c.as_str()).unwrap_or("");
        let codec = st
            .get("codec_name")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        let attached = st
            .get("disposition")
            .and_then(|d| d.get("attached_pic"))
            .and_then(|a| a.as_i64())
            == Some(1);
        match kind {
            "video" if video.is_none() && !attached => {
                if duration <= 0.0 {
                    duration = num(st.get("duration")).unwrap_or(0.0);
                }
                video = Some(VideoStream {
                    codec,
                    width: st.get("width").and_then(|w| w.as_u64()).unwrap_or(0) as u32,
                    height: st.get("height").and_then(|h| h.as_u64()).unwrap_or(0) as u32,
                    fps: rate(st.get("avg_frame_rate")).or_else(|| rate(st.get("r_frame_rate"))),
                });
            }
            "audio" => audio.push(AudioStream {
                index: st.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as u32,
                codec,
                channels: st
                    .get("channels")
                    .and_then(|c| c.as_u64())
                    .map(|c| c as u32),
            }),
            _ => {}
        }
    }
    Ok(ProbeMeta {
        duration: duration.max(0.0),
        start_time,
        format_name,
        video,
        audio,
    })
}

pub fn shift_keyframes(keyframes: Vec<f64>, start_time: f64) -> Vec<f64> {
    let mut out: Vec<f64> = keyframes
        .into_iter()
        .map(|t| (t - start_time).max(0.0))
        .collect();
    out.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    out
}

pub fn parse_keyframes(csv: &str) -> Vec<f64> {
    let mut out: Vec<f64> = csv
        .lines()
        .filter_map(|line| {
            let mut cols = line.trim().split(',');
            let pts = cols.next()?.trim();
            let flags = cols.next().unwrap_or("").trim();
            if !flags.contains('K') {
                return None;
            }
            pts.parse::<f64>().ok().filter(|t| t.is_finite())
        })
        .collect();
    out.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    out.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    out
}

async fn ffprobe(args: &[&str], path: &Path) -> Result<String, String> {
    let out = crate::core::process::command("ffprobe")
        .args(args)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| code_message(NO_FFMPEG, &format!("Could not start ffprobe: {e}")))?;
    if !out.status.success() {
        return Err(code_message(
            PROBE_FAILED,
            &tail(&String::from_utf8_lossy(&out.stderr)),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

pub async fn probe(path: &Path) -> Result<ProbeResult, String> {
    let meta =
        std::fs::metadata(path).map_err(|_| code_message(NOT_FOUND, &path.to_string_lossy()))?;
    if !meta.is_file() {
        return Err(code_message(NOT_FOUND, &path.to_string_lossy()));
    }
    let json = ffprobe(
        &[
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ],
        path,
    )
    .await?;
    let ProbeMeta {
        duration,
        start_time,
        format_name,
        video,
        audio,
    } = parse_probe_json(&json)?;
    if video.is_none() {
        return Err(code_message(PROBE_FAILED, "This file has no video track."));
    }
    let keyframes = match ffprobe(
        &[
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=pts_time,flags",
            "-of",
            "csv=print_section=0",
        ],
        path,
    )
    .await
    {
        Ok(csv) => shift_keyframes(parse_keyframes(&csv), start_time),
        Err(_) => Vec::new(),
    };
    Ok(ProbeResult {
        path: path.to_string_lossy().to_string(),
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        size_bytes: meta.len(),
        duration,
        format_name,
        video,
        audio,
        keyframes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const JSON: &str = r#"{
      "streams": [
        {"index": 0, "codec_name": "mjpeg", "codec_type": "video", "width": 300, "height": 300, "disposition": {"attached_pic": 1}},
        {"index": 1, "codec_name": "h264", "codec_type": "video", "width": 1920, "height": 1080, "avg_frame_rate": "30000/1001", "r_frame_rate": "30/1", "duration": "12.5", "disposition": {"attached_pic": 0}},
        {"index": 2, "codec_name": "aac", "codec_type": "audio", "channels": 2},
        {"index": 3, "codec_name": "mov_text", "codec_type": "subtitle"}
      ],
      "format": {"format_name": "mov,mp4,m4a,3gp,3g2,mj2", "duration": "12.512000"}
    }"#;

    #[test]
    fn parses_streams_and_duration() {
        let m = parse_probe_json(JSON).unwrap();
        assert!((m.duration - 12.512).abs() < 1e-9);
        assert_eq!(m.start_time, 0.0);
        assert!(m.format_name.starts_with("mov,mp4"));
        let a = m.audio;
        let v = m.video.unwrap();
        assert_eq!(v.codec, "h264");
        assert_eq!((v.width, v.height), (1920, 1080));
        assert!((v.fps.unwrap() - 29.97).abs() < 0.01);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].index, 2);
        assert_eq!(a[0].channels, Some(2));
    }

    #[test]
    fn duration_falls_back_to_stream() {
        let j = r#"{"streams":[{"codec_type":"video","codec_name":"vp9","width":2,"height":2,"duration":"3.0"}],"format":{"format_name":"matroska,webm"}}"#;
        let m = parse_probe_json(j).unwrap();
        assert_eq!(m.duration, 3.0);
        assert!(m.video.is_some());
        assert!(m.audio.is_empty());
        assert!(parse_probe_json("nope").is_err());
    }

    #[test]
    fn keyframes_from_packet_flags() {
        let csv = "0.000000,K__\n0.033367,___\n2.002000,K_\nN/A,K__\n4.004000,K__\n2.002000,K__\n";
        assert_eq!(parse_keyframes(csv), vec![0.0, 2.002, 4.004]);
        assert!(parse_keyframes("").is_empty());
    }

    #[test]
    fn keyframes_shift_by_container_start_time() {
        let j = r#"{"streams":[{"codec_type":"video","codec_name":"h264","width":2,"height":2}],"format":{"format_name":"mpegts","duration":"6.0","start_time":"1.443444"}}"#;
        let m = parse_probe_json(j).unwrap();
        assert!((m.start_time - 1.443444).abs() < 1e-9);
        let csv = "1.466667,K_\n1.500000,__\n3.466667,K_\n5.466667,K_\n";
        let k = shift_keyframes(parse_keyframes(csv), m.start_time);
        let want = [0.023223, 2.023223, 4.023223];
        assert_eq!(k.len(), want.len());
        for (got, exp) in k.iter().zip(want) {
            assert!((got - exp).abs() < 1e-6);
        }
        assert_eq!(shift_keyframes(vec![0.0, 1.0, 3.0], 1.0), vec![0.0, 2.0]);
        assert_eq!(shift_keyframes(vec![-0.021, 2.0], 0.0), vec![0.0, 2.0]);
        assert_eq!(shift_keyframes(vec![-0.25, 1.75], -0.25), vec![0.0, 2.0]);
    }
}
