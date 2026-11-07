use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use tokio::io::AsyncReadExt;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::devices::Platform;

pub const SAMPLE_RATE: u32 = 16_000;
const LEVEL_WINDOW: usize = (SAMPLE_RATE as usize) / 20;
const STDERR_CAP: usize = 8 * 1024;

pub fn input_spec(platform: Platform, device: Option<&str>, linux_pulse: bool) -> (String, String) {
    let device = device.map(str::trim).filter(|d| !d.is_empty());
    match platform {
        Platform::Mac => (
            "avfoundation".into(),
            format!(":{}", device.unwrap_or("default")),
        ),
        Platform::Windows => ("dshow".into(), format!("audio={}", device.unwrap_or(""))),
        Platform::Linux if linux_pulse => ("pulse".into(), device.unwrap_or("default").into()),
        Platform::Linux => ("alsa".into(), device.unwrap_or("default").into()),
    }
}

pub fn capture_args(
    platform: Platform,
    device: Option<&str>,
    max_seconds: u32,
    linux_pulse: bool,
) -> Vec<String> {
    let (format, input) = input_spec(platform, device, linux_pulse);
    let mut args: Vec<String> = ["-hide_banner", "-nostdin", "-loglevel", "error"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if platform == Platform::Windows {
        args.extend(["-audio_buffer_size".into(), "50".into()]);
    }
    args.extend([
        "-f".into(),
        format,
        "-i".into(),
        input,
        "-t".into(),
        max_seconds.max(1).to_string(),
        "-vn".into(),
        "-ac".into(),
        "1".into(),
        "-ar".into(),
        SAMPLE_RATE.to_string(),
        "-acodec".into(),
        "pcm_s16le".into(),
        "-flush_packets".into(),
        "1".into(),
        "-f".into(),
        "s16le".into(),
        "pipe:1".into(),
    ]);
    args
}

pub fn wav_bytes(samples: &[i16], sample_rate: u32) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

pub fn write_wav(dir: &Path, name: &str, samples: &[i16]) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(name);
    std::fs::write(&path, wav_bytes(samples, SAMPLE_RATE))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

pub fn rms_dbfs(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return -120.0;
    }
    let sum: f64 = samples.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    let rms = (sum / samples.len() as f64).sqrt() / 32768.0;
    if rms <= 1e-6 {
        -120.0
    } else {
        (20.0 * rms.log10()) as f32
    }
}

pub fn level_from_dbfs(db: f32) -> f32 {
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

pub fn duration_ms(samples: usize) -> u64 {
    samples as u64 * 1000 / SAMPLE_RATE as u64
}

#[derive(Debug, Default)]
pub struct Recording {
    pub samples: Vec<i16>,
    pub stderr: String,
    pub ended_by_itself: bool,
}

pub struct Capture {
    stop: CancellationToken,
    tail: Arc<std::sync::Mutex<Duration>>,
    handle: JoinHandle<anyhow::Result<Recording>>,
}

impl Capture {
    pub fn spawn(
        program: &Path,
        args: &[String],
        max: Duration,
        on_level: impl Fn(f32) + Send + Sync + 'static,
        on_ended: impl FnOnce() + Send + 'static,
    ) -> anyhow::Result<Self> {
        let mut child = crate::core::process::command(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("ffmpeg could not start")?;
        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("ffmpeg without stdout"))?;
        let mut stderr = child.stderr.take();
        let stop = CancellationToken::new();
        let tail = Arc::new(std::sync::Mutex::new(Duration::ZERO));
        let stop2 = stop.clone();
        let tail2 = tail.clone();
        let handle = tokio::spawn(async move {
            let err_task = tokio::spawn(async move {
                let mut text = Vec::new();
                if let Some(e) = stderr.as_mut() {
                    let mut buf = [0u8; 1024];
                    while let Ok(n) = e.read(&mut buf).await {
                        if n == 0 {
                            break;
                        }
                        if text.len() < STDERR_CAP {
                            text.extend_from_slice(&buf[..n]);
                        }
                    }
                }
                String::from_utf8_lossy(&text).trim().to_string()
            });
            let hard_deadline = tokio::time::Instant::now() + max + Duration::from_secs(3);
            let mut samples: Vec<i16> = Vec::new();
            let mut carry: Option<u8> = None;
            let mut buf = vec![0u8; 4096];
            let mut window: Vec<i16> = Vec::with_capacity(LEVEL_WINDOW);
            let mut stopping_at: Option<tokio::time::Instant> = None;
            let mut ended_by_itself = false;
            loop {
                let deadline = stopping_at.unwrap_or(hard_deadline).min(hard_deadline);
                let read = tokio::select! {
                    r = stdout.read(&mut buf) => Some(r),
                    _ = stop2.cancelled(), if stopping_at.is_none() => {
                        let t = tail2.lock().map(|t| *t).unwrap_or_default();
                        stopping_at = Some(tokio::time::Instant::now() + t);
                        continue;
                    }
                    _ = tokio::time::sleep_until(deadline) => None,
                };
                let n = match read {
                    None => break,
                    Some(Ok(0)) => {
                        ended_by_itself = stopping_at.is_none();
                        break;
                    }
                    Some(Ok(n)) => n,
                    Some(Err(e)) => {
                        tracing::warn!("[dictation] reading ffmpeg: {e}");
                        break;
                    }
                };
                let mut bytes = &buf[..n];
                if let Some(lo) = carry.take() {
                    if let Some((&hi, rest)) = bytes.split_first() {
                        let s = i16::from_le_bytes([lo, hi]);
                        samples.push(s);
                        window.push(s);
                        bytes = rest;
                    }
                }
                let mut chunks = bytes.chunks_exact(2);
                for c in &mut chunks {
                    let s = i16::from_le_bytes([c[0], c[1]]);
                    samples.push(s);
                    window.push(s);
                }
                if let [last] = chunks.remainder() {
                    carry = Some(*last);
                }
                if window.len() >= LEVEL_WINDOW {
                    on_level(level_from_dbfs(rms_dbfs(&window)));
                    window.clear();
                }
            }
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
            let stderr = tokio::time::timeout(Duration::from_secs(1), err_task)
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default();
            if ended_by_itself {
                on_ended();
            }
            Ok(Recording {
                samples,
                stderr,
                ended_by_itself,
            })
        });
        Ok(Self { stop, tail, handle })
    }

    pub async fn stop(self, tail: Duration) -> anyhow::Result<Recording> {
        if let Ok(mut t) = self.tail.lock() {
            *t = tail;
        }
        self.stop.cancel();
        self.handle
            .await
            .map_err(|e| anyhow!("capture task died: {e}"))?
    }

    pub fn is_finished(&self) -> bool {
        self.handle.is_finished()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_args_use_avfoundation_and_default_device() {
        let a = capture_args(Platform::Mac, None, 120, false);
        let joined = a.join(" ");
        assert!(joined.contains("-f avfoundation -i :default -t 120"));
        assert!(
            joined.ends_with("-ac 1 -ar 16000 -acodec pcm_s16le -flush_packets 1 -f s16le pipe:1")
        );
        let named = capture_args(Platform::Mac, Some("MacBook Air Microphone"), 30, false);
        assert!(named.contains(&":MacBook Air Microphone".to_string()));
    }

    #[test]
    fn windows_args_use_dshow() {
        let a = capture_args(Platform::Windows, Some("Microphone (Realtek)"), 60, false);
        assert!(a.contains(&"dshow".to_string()));
        assert!(a.contains(&"audio=Microphone (Realtek)".to_string()));
        assert!(a.contains(&"-audio_buffer_size".to_string()));
    }

    #[test]
    fn linux_args_pick_pulse_or_alsa() {
        let p = capture_args(Platform::Linux, None, 60, true);
        assert!(p.join(" ").contains("-f pulse -i default"));
        let a = capture_args(Platform::Linux, Some("  "), 60, false);
        assert!(a.join(" ").contains("-f alsa -i default"));
    }

    #[test]
    fn max_seconds_never_zero() {
        let a = capture_args(Platform::Mac, None, 0, false);
        let i = a.iter().position(|s| s == "-t").unwrap();
        assert_eq!(a[i + 1], "1");
    }

    #[test]
    fn wav_header_is_correct() {
        let w = wav_bytes(&[0, 1, -1], 16_000);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(w[4..8].try_into().unwrap()), 36 + 6);
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), 16_000);
        assert_eq!(u32::from_le_bytes(w[28..32].try_into().unwrap()), 32_000);
        assert_eq!(&w[36..40], b"data");
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()), 6);
        assert_eq!(w.len(), 50);
        assert_eq!(&w[46..48], &1i16.to_le_bytes());
    }

    #[test]
    fn levels() {
        assert_eq!(rms_dbfs(&[]), -120.0);
        assert_eq!(rms_dbfs(&[0; 100]), -120.0);
        let loud = rms_dbfs(&[i16::MAX; 100]);
        assert!(loud > -0.1 && loud <= 0.0);
        assert_eq!(level_from_dbfs(-120.0), 0.0);
        assert_eq!(level_from_dbfs(0.0), 1.0);
        assert!((level_from_dbfs(-30.0) - 0.5).abs() < 1e-6);
        assert_eq!(duration_ms(16_000), 1000);
    }

    fn lavfi(seconds: u32) -> Vec<String> {
        [
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-re",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=16000",
            "-t",
        ]
        .iter()
        .map(|s| s.to_string())
        .chain([
            seconds.to_string(),
            "-ac".into(),
            "1".into(),
            "-ar".into(),
            "16000".into(),
            "-acodec".into(),
            "pcm_s16le".into(),
            "-flush_packets".into(),
            "1".into(),
            "-f".into(),
            "s16le".into(),
            "pipe:1".into(),
        ])
        .collect()
    }

    fn ffmpeg() -> Option<PathBuf> {
        let ok = std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        ok.then(|| PathBuf::from("ffmpeg"))
    }

    #[tokio::test]
    async fn capture_stops_on_request_and_reports_levels() {
        let Some(bin) = ffmpeg() else {
            return;
        };
        let levels = Arc::new(std::sync::Mutex::new(Vec::<f32>::new()));
        let l2 = levels.clone();
        let cap = Capture::spawn(
            &bin,
            &lavfi(30),
            Duration::from_secs(30),
            move |l| l2.lock().unwrap().push(l),
            || {},
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while levels.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        let rec = cap.stop(Duration::from_millis(100)).await.unwrap();
        assert!(!rec.ended_by_itself);
        assert!(rec.samples.len() > 4_000, "got {}", rec.samples.len());
        assert!(rms_dbfs(&rec.samples) > -40.0);
        assert!(!levels.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn capture_reports_when_ffmpeg_ends_by_itself() {
        let Some(bin) = ffmpeg() else {
            return;
        };
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let cap = Capture::spawn(
            &bin,
            &lavfi(1),
            Duration::from_secs(1),
            |_| {},
            move || {
                let _ = tx.send(());
            },
        )
        .unwrap();
        tokio::time::timeout(Duration::from_secs(10), rx)
            .await
            .unwrap()
            .unwrap();
        assert!(cap.is_finished());
        let rec = cap.stop(Duration::ZERO).await.unwrap();
        assert!(rec.ended_by_itself);
        assert!(rec.samples.len() >= 15_000);
    }
}
