use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::sync::CancellationToken;

use super::errors::{self, code_message};
use super::Platform;

const READY_TIMEOUT: Duration = Duration::from_secs(12);
const FINALIZE_TIMEOUT: Duration = Duration::from_secs(20);

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

pub async fn run_job(
    args: &[String],
    cancel: &CancellationToken,
    on_time: &(dyn Fn(f64) + Send + Sync),
) -> Result<(), String> {
    let mut child = crate::core::process::command("ffmpeg")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| code_message(errors::NO_FFMPEG, &format!("Could not start FFmpeg: {e}")))?;
    let stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let err_reader = tokio::spawn(async move {
        let mut buf = Vec::new();
        if let Some(s) = stderr.as_mut() {
            let _ = s.read_to_end(&mut buf).await;
        }
        buf
    });
    let watch_out = async {
        if let Some(out) = stdout {
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(us) = parse_time_us(&line) {
                    on_time(us as f64 / 1_000_000.0);
                }
            }
        }
    };
    let status = tokio::select! {
        (s, _) = async { tokio::join!(child.wait(), watch_out) } => s.map_err(|e| code_message(errors::EXPORT_FAILED, &e.to_string()))?,
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            return Err(errors::CANCELLED.to_string());
        }
    };
    let err = err_reader.await.unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        Err(code_message(
            errors::EXPORT_FAILED,
            &errors::tail(&String::from_utf8_lossy(&err)),
        ))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordOutcome {
    pub path: String,
    pub ok: bool,
    pub duration: f64,
    pub size_bytes: u64,
    pub forced: bool,
    pub stopped_by_user: bool,
    pub error: Option<String>,
}

pub struct Recorder {
    pub path: PathBuf,
    stop_tx: mpsc::Sender<()>,
    done: watch::Receiver<Option<RecordOutcome>>,
}

impl Recorder {
    pub async fn stop(&self) -> RecordOutcome {
        let _ = self.stop_tx.send(()).await;
        self.wait().await
    }

    pub async fn wait(&self) -> RecordOutcome {
        let mut rx = self.done.clone();
        loop {
            if let Some(o) = rx.borrow().clone() {
                return o;
            }
            if rx.changed().await.is_err() {
                return rx.borrow().clone().unwrap_or(RecordOutcome {
                    path: self.path.to_string_lossy().to_string(),
                    ok: false,
                    duration: 0.0,
                    size_bytes: 0,
                    forced: false,
                    stopped_by_user: false,
                    error: Some(errors::RECORD_FAILED.to_string()),
                });
            }
        }
    }

    pub fn finished(&self) -> bool {
        self.done.borrow().is_some()
    }
}

fn file_size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

pub async fn start_recording(
    args: Vec<String>,
    output: PathBuf,
    platform: Platform,
    with_mic: bool,
) -> Result<Recorder, String> {
    let mut child = crate::core::process::command("ffmpeg")
        .args(&args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| code_message(errors::NO_FFMPEG, &format!("Could not start FFmpeg: {e}")))?;
    let mut stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let log = Arc::new(Mutex::new(String::new()));
    let media_us = Arc::new(AtomicU64::new(0));
    let (ready_tx, ready_rx) = oneshot::channel::<()>();
    let ready_tx = Arc::new(Mutex::new(Some(ready_tx)));
    let fire = {
        let ready_tx = ready_tx.clone();
        move || {
            if let Some(tx) = ready_tx.lock().ok().and_then(|mut g| g.take()) {
                let _ = tx.send(());
            }
        }
    };

    let out_task = {
        let media_us = media_us.clone();
        let fire = fire.clone();
        tokio::spawn(async move {
            if let Some(out) = stdout {
                let mut lines = BufReader::new(out).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(us) = parse_time_us(&line) {
                        media_us.store(us, Ordering::Relaxed);
                    }
                    if line.starts_with("progress=") {
                        fire();
                    }
                }
            }
        })
    };
    let err_task = {
        let log = log.clone();
        let fire = fire.clone();
        tokio::spawn(async move {
            if let Some(err) = stderr {
                let mut lines = BufReader::new(err).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if line.contains("Press [q]") {
                        fire();
                    }
                    if let Ok(mut l) = log.lock() {
                        if l.len() > 64 * 1024 {
                            let cut = l.len() - 32 * 1024;
                            let at = (cut..l.len()).find(|i| l.is_char_boundary(*i)).unwrap_or(0);
                            l.drain(..at);
                        }
                        l.push_str(&line);
                        l.push('\n');
                    }
                }
            }
        })
    };

    let (stop_tx, mut stop_rx) = mpsc::channel::<()>(2);
    let (done_tx, done_rx) = watch::channel::<Option<RecordOutcome>>(None);
    let out_path = output.clone();
    tokio::spawn(async move {
        let mut forced = false;
        let mut by_user = false;
        let status = tokio::select! {
            s = child.wait() => s.ok(),
            _ = stop_rx.recv() => {
                by_user = true;
                if let Some(mut si) = stdin.take() {
                    let _ = si.write_all(b"q").await;
                    let _ = si.flush().await;
                    drop(si);
                }
                match tokio::time::timeout(FINALIZE_TIMEOUT, child.wait()).await {
                    Ok(s) => s.ok(),
                    Err(_) => {
                        forced = true;
                        let _ = child.kill().await;
                        child.wait().await.ok()
                    }
                }
            }
        };
        drop(stdin);
        let _ = out_task.await;
        let _ = err_task.await;
        let text = log.lock().map(|l| l.clone()).unwrap_or_default();
        let size = file_size(&out_path);
        let success = status.map(|s| s.success()).unwrap_or(false);
        let ok = size > 0 && (success || by_user);
        let error = if ok {
            None
        } else if size == 0 && success {
            Some(code_message(errors::EMPTY_OUTPUT, &errors::tail(&text)))
        } else {
            let code = errors::classify_record_failure(&text, platform, with_mic);
            Some(code_message(code, &errors::tail(&text)))
        };
        if !ok && size == 0 {
            let _ = std::fs::remove_file(&out_path);
        }
        let outcome = RecordOutcome {
            path: out_path.to_string_lossy().to_string(),
            ok,
            duration: media_us.load(Ordering::Relaxed) as f64 / 1_000_000.0,
            size_bytes: size,
            forced,
            stopped_by_user: by_user,
            error,
        };
        let _ = done_tx.send(Some(outcome));
    });

    let recorder = Recorder {
        path: output,
        stop_tx,
        done: done_rx,
    };
    let mut done = recorder.done.clone();
    let early = async {
        loop {
            if done.borrow().is_some() {
                return;
            }
            if done.changed().await.is_err() {
                return;
            }
        }
    };
    tokio::select! {
        _ = ready_rx => {}
        _ = early => {}
        _ = tokio::time::sleep(READY_TIMEOUT) => {}
    }
    if recorder.finished() {
        let o = recorder.wait().await;
        return Err(o.error.unwrap_or_else(|| errors::RECORD_FAILED.to_string()));
    }
    Ok(recorder)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_progress_time() {
        assert_eq!(parse_time_us("out_time_us=1500000"), Some(1_500_000));
        assert_eq!(parse_time_us("out_time_ms=20"), Some(20));
        assert_eq!(parse_time_us("out_time_us=-5"), None);
        assert_eq!(parse_time_us("frame=10"), None);
    }
}
