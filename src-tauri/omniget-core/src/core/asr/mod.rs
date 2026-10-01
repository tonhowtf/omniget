pub mod audio;
pub mod mel;
pub mod models;
pub mod tokens;
pub mod whisper;

use std::ffi::{OsStr, OsString};
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
    pub lang: String,
    pub text: String,
    pub segments: Vec<Segment>,
    pub model: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub runtime_installed: bool,
    pub runtime_downloadable: bool,
    pub default_model: String,
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

pub fn is_ready(model: Option<&str>) -> bool {
    crate::core::onnxrt::is_installed()
        && models::pick(model)
            .map(models::is_downloaded)
            .unwrap_or(false)
}

pub async fn install(model: Option<&str>, progress: &ProgressFn) -> anyhow::Result<Status> {
    let spec = models::pick(model)?;
    models::install(spec, progress).await?;
    Ok(status())
}

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
