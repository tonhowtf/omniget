//! Local speech recognition: Whisper in ONNX on the app's managed ONNX
//! Runtime, nothing sent anywhere. Written from scratch (log-mel, byte-level
//! BPE decoding, greedy search with Whisper's timestamp rules) so it needs no
//! Python and no extra native library beyond the runtime the app already
//! downloads on demand.
//!
//! - `status()` / `install(model)` manage the runtime and the model files;
//! - `transcribe_file(path, lang, model, progress)` returns timed segments.
//!
//! Used by /telegram and reusable by /video.

pub mod audio;
pub mod mel;
pub mod models;
pub mod tokens;
pub mod whisper;

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

use crate::core::tools::ProgressFn;

pub use models::DEFAULT_MODEL;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Segment {
    pub start_ms: i64,
    pub end_ms: i64,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transcript {
    /// ISO code Whisper detected (or was told), e.g. `pt`.
    pub lang: String,
    pub text: String,
    pub segments: Vec<Segment>,
    pub model: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub runtime_installed: bool,
    /// `false` when there is no official ONNX Runtime build for this system.
    pub runtime_downloadable: bool,
    pub default_model: String,
    /// The default model and the runtime are both on disk.
    pub ready: bool,
    pub models: Vec<models::ModelStatus>,
}

pub fn status() -> Status {
    let runtime = crate::core::onnxrt::is_installed();
    Status {
        runtime_installed: runtime,
        runtime_downloadable: crate::core::onnxrt::auto_variant_id().is_some(),
        default_model: DEFAULT_MODEL.to_string(),
        ready: is_ready(None),
        models: models::list(),
    }
}

/// Runtime and model on disk: a transcription would not touch the network.
pub fn is_ready(model: Option<&str>) -> bool {
    crate::core::onnxrt::is_installed()
        && models::pick(model)
            .map(models::is_downloaded)
            .unwrap_or(false)
}

/// Downloads the runtime and the model (default when `None`) if missing.
pub async fn install(model: Option<&str>, progress: &ProgressFn) -> anyhow::Result<Status> {
    let spec = models::pick(model)?;
    models::install(spec, progress).await?;
    Ok(status())
}

/// Idle time after which the loaded model is dropped.
const IDLE_TTL: Duration = Duration::from_secs(300);

struct Slot {
    model: whisper::Model,
    last_used: Instant,
}

fn cell() -> &'static Mutex<Option<Slot>> {
    static CELL: OnceLock<Mutex<Option<Slot>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

fn lock() -> std::sync::MutexGuard<'static, Option<Slot>> {
    match cell().lock() {
        Ok(g) => g,
        Err(poisoned) => {
            let mut g = poisoned.into_inner();
            *g = None;
            g
        }
    }
}

/// Drops the loaded model now. Returns whether one was loaded.
pub fn release() -> bool {
    lock().take().is_some()
}

fn schedule_release() {
    std::thread::spawn(|| {
        std::thread::sleep(IDLE_TTL + Duration::from_secs(1));
        let mut g = lock();
        if g.as_ref()
            .map(|s| s.last_used.elapsed() >= IDLE_TTL)
            .unwrap_or(false)
        {
            *g = None;
        }
    });
}

/// Transcribes a local file. Downloads the runtime and the model first when
/// they are missing. `progress` goes from 0.0 to 1.0.
pub async fn transcribe_file(
    path: &Path,
    lang: Option<&str>,
    model: Option<&str>,
    progress: impl Fn(f32) + Send + Sync + 'static,
) -> anyhow::Result<Transcript> {
    let duration_ms = crate::core::ffmpeg::probe(path)
        .await
        .ok()
        .map(|p| (p.duration_seconds * 1000.0) as u64)
        .filter(|d| *d > 0);
    transcribe_input(path.as_os_str(), duration_ms, lang, model, progress).await
}

/// Transcribes anything ffmpeg opens (a file or an http URL). With
/// `duration_ms` the progress is a real fraction; without it, it only
/// reports 1.0 at the end.
pub async fn transcribe_input(
    input: &OsStr,
    duration_ms: Option<u64>,
    lang: Option<&str>,
    model: Option<&str>,
    progress: impl Fn(f32) + Send + Sync + 'static,
) -> anyhow::Result<Transcript> {
    let spec = models::pick(model)?;
    if !is_ready(Some(spec.id)) {
        models::install(spec, &crate::core::tools::noop_progress()).await?;
    }
    let input: OsString = input.to_os_string();
    let lang = lang.map(str::to_string);
    let progress = Arc::new(progress);
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Transcript> {
        let mut guard = lock();
        let reuse = guard
            .as_ref()
            .map(|s| s.model.id == spec.id)
            .unwrap_or(false);
        if !reuse {
            *guard = None;
            *guard = Some(Slot {
                model: whisper::Model::load(spec)?,
                last_used: Instant::now(),
            });
        }
        let slot = guard.as_mut().ok_or_else(|| anyhow!("model slot empty"))?;
        let mut pcm = audio::PcmStream::open(&input)?;
        let total = duration_ms.map(|ms| ms as usize * mel::SAMPLE_RATE / 1000);
        let p = progress.clone();
        let report = move |done: usize| {
            if let Some(t) = total.filter(|t| *t > 0) {
                p((done as f32 / t as f32).min(0.99));
            }
        };
        let res = slot.model.transcribe(&mut pcm, lang.as_deref(), &report);
        slot.last_used = Instant::now();
        let (code, segments, got_audio) = match res {
            Ok(v) => v,
            Err(e) => {
                let _ = pcm.finish(true);
                return Err(e);
            }
        };
        pcm.finish(got_audio)?;
        // Whisper fills music and silence with stock phrases it saw in
        // subtitles; a segment made only of one of them is not speech.
        let segments: Vec<_> = segments
            .into_iter()
            .filter(|s| !is_stock_hallucination(&s.text))
            .collect();
        let text = segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<String>()
            .trim()
            .to_string();
        let segments = segments
            .into_iter()
            .map(|s| Segment {
                text: s.text.trim().to_string(),
                ..s
            })
            .collect();
        progress(1.0);
        Ok(Transcript {
            lang: code,
            text,
            segments,
            model: spec.id.to_string(),
        })
    })
    .await
    .map_err(|e| anyhow!("transcription task died: {e}"))?;
    schedule_release();
    result
}

#[cfg(test)]
mod tests {
    /// End to end with the real model: `say` makes speech, ffmpeg turns it
    /// into a WAV, Whisper reads it back. Downloads the runtime and
    /// whisper-base into `OMNIGET_DATA_DIR` (point it at a scratch folder).
    ///
    /// `OMNIGET_DATA_DIR=/tmp/asr cargo test -p omniget-core --lib asr::tests -- --ignored --nocapture`
    #[test]
    #[ignore = "network + macOS `say`: downloads the ONNX Runtime and whisper-base"]
    fn transcribes_real_speech() {
        let dir = std::env::temp_dir().join("omniget-asr-test");
        std::fs::create_dir_all(&dir).unwrap();
        let aiff = dir.join("x.aiff");
        let wav = dir.join("x.wav");
        let custom = std::env::var("ASR_TEST_TEXT").ok();
        let text = custom
            .clone()
            .unwrap_or_else(|| "hello world, this is a test".to_string());
        let mut say = std::process::Command::new("say");
        if let Ok(v) = std::env::var("ASR_TEST_VOICE") {
            say.args(["-v", &v]);
        }
        let ok = say
            .arg("-o")
            .arg(&aiff)
            .arg(&text)
            .status()
            .unwrap()
            .success();
        assert!(ok, "say failed");
        let ok = std::process::Command::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-i"])
            .arg(&aiff)
            .arg(&wav)
            .status()
            .unwrap()
            .success();
        assert!(ok, "ffmpeg failed");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let model = std::env::var("ASR_TEST_MODEL").ok();
        let lang = std::env::var("ASR_TEST_LANG").ok();
        let t0 = std::time::Instant::now();
        let t = rt
            .block_on(super::transcribe_file(
                &wav,
                lang.as_deref(),
                model.as_deref(),
                |p| eprintln!("progress {:.2}", p),
            ))
            .expect("transcription");
        eprintln!("took {:?}", t0.elapsed());
        eprintln!("{}", serde_json::to_string_pretty(&t).unwrap());
        assert!(!t.segments.is_empty());
        if custom.is_none() {
            let got = t.text.to_lowercase();
            assert!(got.contains("hello"), "got: {got}");
            assert!(got.contains("test"), "got: {got}");
            assert_eq!(t.lang, lang.unwrap_or_else(|| "en".into()));
        }
    }
}

/// Phrases Whisper is known to produce on music or silence.
fn is_stock_hallucination(text: &str) -> bool {
    let t: String = text
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || !c.is_ascii())
        .filter(|c| !matches!(c, '。' | '，' | '！' | '？' | '、'))
        .collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        return true;
    }
    const STOCK: &[&str] = &[
        "you",
        "you you",
        "music",
        "musica",
        "música",
        "音乐",
        "thank you",
        "thanks for watching",
        "thank you for watching",
        "please subscribe",
        "bye",
        "obrigado",
        "obrigada",
        "legendas pela comunidade amaraorg",
        "字幕由amaraorg社区提供",
        "请不吝点赞 订阅 转发 打赏支持明镜与点点栏目",
        "请不吝点赞订阅转发打赏支持明镜与点点栏目",
        "谢谢观看",
        "продолжение следует",
        "субтитры сделал dimatorzok",
    ];
    STOCK.contains(&t.as_str()) || t.split(' ').all(|w| w == "you")
}

#[cfg(test)]
mod stock_tests {
    use super::is_stock_hallucination;

    #[test]
    fn filters_stock_phrases_only() {
        assert!(is_stock_hallucination(" you"));
        assert!(is_stock_hallucination("Thank you."));
        assert!(is_stock_hallucination("字幕由Amara.org社区提供"));
        assert!(!is_stock_hallucination(
            "Thank you for the explanation about ownership."
        ));
        assert!(!is_stock_hallucination("Olá, pessoal"));
    }
}
