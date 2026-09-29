//! The two multilingual models memory uses, both on the XLM-RoBERTa
//! vocabulary (one `tokenizer.json`, read by [`super::unigram`]):
//! - `multilingual-e5-small`: 384-d vectors for about 100 languages, so a
//!   question in Portuguese finds a fact saved in English and back;
//! - `mmarco-mMiniLMv2-L12-H384-v1`: a cross-encoder that reads the query and
//!   one memory together and scores how well it answers (the reranker
//!   Hindsight runs, but multilingual: theirs is the English ms-marco one).
//!
//! Same rules as [`super::minilm`]: pinned revisions with sha256 and size,
//! nothing downloaded without an explicit call, sessions created on first use
//! and dropped after [`IDLE_TTL`].

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::minilm::{fetch_asset, has_asset, model_dir, pool_and_normalize, Asset};
use super::unigram::{pad, Unigram};
use super::{EmbedError, ERR_EMBED_INFER, ERR_EMBED_MODEL, ERR_EMBED_SHAPE, ERR_EMBED_VOCAB};
use crate::core::tools::ProgressFn;

pub const DIM: usize = 384;
pub const IDLE_TTL: Duration = Duration::from_secs(600);
const BATCH: usize = 32;
/// Facts are short; this keeps a batch fast.
const MAX_LEN: usize = 256;

/// XLM-R tokenizer (the one of the e5 revision; the reranker's has the same
/// vocabulary and normalizer rules).
pub const TOKENIZER: Asset = Asset {
    file: "xlmr-tokenizer.json",
    url: "https://huggingface.co/Xenova/multilingual-e5-small/resolve/761b726dd34fb83930e26aab4e9ac3899aa1fa78/tokenizer.json",
    sha256: "0b44a9d7b51c3c62626640cda0e2c2f70fdacdc25bbbd68038369d14ebdf4c39",
    bytes: 17_082_730,
};

/// `intfloat/multilingual-e5-small`, int8 (MIT).
pub const E5: Asset = Asset {
    file: "multilingual-e5-small.onnx",
    url: "https://huggingface.co/Xenova/multilingual-e5-small/resolve/761b726dd34fb83930e26aab4e9ac3899aa1fa78/onnx/model_quantized.onnx",
    sha256: "f80102d3f2a1229f387d3c81909990d8945513e347b0eab049f7de3c6f98c193",
    bytes: 118_308_185,
};

/// `cross-encoder/mmarco-mMiniLMv2-L12-H384-v1`, int8 for this CPU
/// (Apache-2.0).
#[cfg(target_arch = "aarch64")]
pub const RERANKER: Asset = Asset {
    file: "mmarco-mminilm-l12.onnx",
    url: "https://huggingface.co/cross-encoder/mmarco-mMiniLMv2-L12-H384-v1/resolve/1427fd652930e4ba29e8149678df786c240d8825/onnx/model_qint8_arm64.onnx",
    sha256: "1825907d6c1a9001ff78124780bbde20a614a8c3df3b63409cf3c72c6fe5c8b4",
    bytes: 118_620_017,
};
#[cfg(not(target_arch = "aarch64"))]
pub const RERANKER: Asset = Asset {
    file: "mmarco-mminilm-l12.onnx",
    url: "https://huggingface.co/cross-encoder/mmarco-mMiniLMv2-L12-H384-v1/resolve/1427fd652930e4ba29e8149678df786c240d8825/onnx/model_quint8_avx2.onnx",
    sha256: "6c2513767fb63d008a4377bef7a7a3555433d9436342bb53e35a3a72ffc52d4b",
    bytes: 118_620_016,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    E5,
    Reranker,
}

impl Model {
    fn asset(self) -> Asset {
        match self {
            Model::E5 => E5,
            Model::Reranker => RERANKER,
        }
    }
}

/// Files on disk and the ONNX runtime resolvable.
pub fn is_ready(m: Model) -> bool {
    has_asset(&TOKENIZER) && has_asset(&m.asset()) && crate::core::onnxrt::is_installed()
}

pub async fn ensure(m: Model, progress: &ProgressFn) -> Result<PathBuf, EmbedError> {
    fetch_asset(&TOKENIZER, progress).await?;
    fetch_asset(&m.asset(), progress).await
}

// ── Sessions ─────────────────────────────────────────────────────────────

struct Loaded {
    session: ort::session::Session,
    inputs: Vec<String>,
    last_used: Instant,
}

fn tokenizer() -> Result<&'static Unigram, EmbedError> {
    static TOK: OnceLock<Unigram> = OnceLock::new();
    if let Some(t) = TOK.get() {
        return Ok(t);
    }
    let dir = model_dir().ok_or_else(|| EmbedError::new(ERR_EMBED_MODEL, "no app data dir"))?;
    let text = std::fs::read_to_string(dir.join(TOKENIZER.file))
        .map_err(|e| EmbedError::new(ERR_EMBED_VOCAB, format!("reading the tokenizer: {e}")))?;
    let t = Unigram::from_json(&text).map_err(|e| EmbedError::new(ERR_EMBED_VOCAB, e.0))?;
    Ok(TOK.get_or_init(|| t))
}

fn cell(m: Model) -> &'static Mutex<Option<Loaded>> {
    static E: OnceLock<Mutex<Option<Loaded>>> = OnceLock::new();
    static R: OnceLock<Mutex<Option<Loaded>>> = OnceLock::new();
    match m {
        Model::E5 => E.get_or_init(|| Mutex::new(None)),
        Model::Reranker => R.get_or_init(|| Mutex::new(None)),
    }
}

fn with_session<T>(
    m: Model,
    f: impl FnOnce(&mut Loaded) -> Result<T, EmbedError>,
) -> Result<T, EmbedError> {
    let mut guard = match cell(m).lock() {
        Ok(g) => g,
        Err(p) => {
            let mut g = p.into_inner();
            *g = None;
            g
        }
    };
    if guard
        .as_ref()
        .map(|l| l.last_used.elapsed() >= IDLE_TTL)
        .unwrap_or(false)
    {
        *guard = None;
    }
    if guard.is_none() {
        start_reaper();
        let dir = model_dir().ok_or_else(|| EmbedError::new(ERR_EMBED_MODEL, "no app data dir"))?;
        let path = dir.join(m.asset().file);
        if !path.is_file() {
            return Err(EmbedError::new(
                ERR_EMBED_MODEL,
                "the model is not downloaded",
            ));
        }
        let session = crate::core::tools::onnx::session_from_file(&path)
            .map_err(|e| EmbedError::new(super::ERR_EMBED_RUNTIME, e.to_string()))?;
        let inputs = session
            .inputs()
            .iter()
            .map(|i| i.name().to_string())
            .collect();
        *guard = Some(Loaded {
            session,
            inputs,
            last_used: Instant::now(),
        });
    }
    let l = guard.as_mut().expect("just loaded");
    let out = f(l);
    l.last_used = Instant::now();
    out
}

/// The two sessions hold ≈ 250 MB together; a sleeping thread drops each one
/// after [`IDLE_TTL`] without work, so memory that is not being searched
/// costs nothing resident.
fn start_reaper() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        let _ = std::thread::Builder::new()
            .name("xlmr-reaper".into())
            .spawn(|| loop {
                std::thread::sleep(Duration::from_secs(60));
                for m in [Model::E5, Model::Reranker] {
                    if let Ok(mut g) = cell(m).try_lock() {
                        if g.as_ref()
                            .is_some_and(|l| l.last_used.elapsed() >= IDLE_TTL)
                        {
                            *g = None;
                        }
                    }
                }
            });
    });
}

/// Drops both sessions (settings changed, quitting).
pub fn release_now() {
    for m in [Model::E5, Model::Reranker] {
        if let Ok(mut g) = cell(m).lock() {
            *g = None;
        }
    }
}

type Outputs = (Vec<i64>, Vec<f32>);

fn run(l: &mut Loaded, batch: &[super::tokenizer::Encoding]) -> Result<Outputs, EmbedError> {
    let rows = batch.len();
    let width = batch.first().map(|e| e.ids.len()).unwrap_or(0);
    let shape = vec![rows as i64, width as i64];
    let flat = |f: &dyn Fn(&super::tokenizer::Encoding) -> &Vec<u32>| -> Vec<i64> {
        batch
            .iter()
            .flat_map(|e| f(e).iter().map(|v| *v as i64))
            .collect()
    };
    let ids = flat(&|e| &e.ids);
    let mask = flat(&|e| &e.attention_mask);
    let types = flat(&|e| &e.type_ids);
    let mut inputs: Vec<(
        std::borrow::Cow<'static, str>,
        ort::session::SessionInputValue<'_>,
    )> = Vec::new();
    for name in l.inputs.clone() {
        let data = match name.as_str() {
            "input_ids" => ids.clone(),
            "attention_mask" => mask.clone(),
            "token_type_ids" => types.clone(),
            other => {
                return Err(EmbedError::new(
                    ERR_EMBED_SHAPE,
                    format!("unknown model input `{other}`"),
                ))
            }
        };
        let t = ort::value::Tensor::from_array((shape.clone(), data))
            .map_err(|e| EmbedError::new(ERR_EMBED_INFER, format!("tensor: {e}")))?;
        inputs.push((std::borrow::Cow::Owned(name), t.into()));
    }
    let outputs = l
        .session
        .run(inputs)
        .map_err(|e| EmbedError::new(ERR_EMBED_INFER, format!("inference failed: {e}")))?;
    let (s, raw) = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|e| EmbedError::new(ERR_EMBED_INFER, format!("reading the output: {e}")))?;
    Ok((s.to_vec(), raw.to_vec()))
}

/// e5 vectors. `query` picks the prefix e5 was trained with: `query: ` for
/// what is being searched, `passage: ` for what is stored. **Blocking.**
pub fn embed(texts: &[&str], query: bool) -> Result<Vec<Vec<f32>>, EmbedError> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let tok = tokenizer()?;
    let prefix = if query { "query: " } else { "passage: " };
    let mut out = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(BATCH) {
        let enc = pad(chunk
            .iter()
            .map(|t| tok.encode(&format!("{prefix}{t}"), MAX_LEN))
            .collect());
        let (shape, raw) = with_session(Model::E5, |l| run(l, &enc))?;
        if shape.len() != 3 || shape[2] as usize != DIM {
            return Err(EmbedError::new(
                ERR_EMBED_SHAPE,
                format!("expected [batch, tokens, {DIM}], got {shape:?}"),
            ));
        }
        let tokens = shape[1].max(0) as usize;
        let stride = tokens * DIM;
        for (i, e) in enc.iter().enumerate() {
            let slice = raw
                .get(i * stride..(i + 1) * stride)
                .ok_or_else(|| EmbedError::new(ERR_EMBED_SHAPE, "output shorter than the batch"))?;
            out.push(pool_and_normalize(slice, &e.attention_mask, tokens, DIM).to_vec());
        }
    }
    Ok(out)
}

/// Relevance of each document to `query`, 0..1 (sigmoid of the logit).
/// **Blocking.**
pub fn rerank(query: &str, docs: &[&str]) -> Result<Vec<f32>, EmbedError> {
    if docs.is_empty() {
        return Ok(Vec::new());
    }
    let tok = tokenizer()?;
    let mut out = Vec::with_capacity(docs.len());
    for chunk in docs.chunks(BATCH) {
        let enc = pad(chunk
            .iter()
            .map(|d| tok.encode_pair(query, d, 320))
            .collect());
        let (shape, raw) = with_session(Model::Reranker, |l| run(l, &enc))?;
        let per = shape.last().copied().unwrap_or(1).max(1) as usize;
        for i in 0..enc.len() {
            let logit = raw.get(i * per).copied().unwrap_or(0.0);
            out.push(1.0 / (1.0 + (-logit).exp()));
        }
    }
    Ok(out)
}
