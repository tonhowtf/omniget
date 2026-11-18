use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::{
    tokenizer::Tokenizer, EmbedError, ERR_EMBED_INFER, ERR_EMBED_MODEL, ERR_EMBED_RUNTIME,
    ERR_EMBED_SHAPE, ERR_EMBED_VOCAB,
};

pub const DIM: usize = 384;

pub type Dim = [f32; DIM];

pub const BATCH: usize = 64;

pub const IDLE_TTL: Duration = Duration::from_secs(600);

pub const DIR_ENV: &str = "OMNIGET_EMBED_MODEL_DIR";

#[derive(Debug, Clone, Copy)]
pub struct Asset {
    pub file: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

pub const MODEL: Asset = Asset {
    file: "minilm-l6-v2.onnx",
    url: "https://huggingface.co/Xenova/all-MiniLM-L6-v2/resolve/751bff37182d3f1213fa05d7196b954e230abad9/onnx/model_quantized.onnx",
    sha256: "afdb6f1a0e45b715d0bb9b11772f032c399babd23bfc31fed1c170afc848bdb1",
    bytes: 22_972_370,
};

pub const VOCAB: Asset = Asset {
    file: "minilm-l6-v2.vocab.txt",
    url: "https://huggingface.co/Xenova/all-MiniLM-L6-v2/resolve/751bff37182d3f1213fa05d7196b954e230abad9/vocab.txt",
    sha256: "07eced375cec144d27c900241f3e339478dec958f92fddbc551f295c992038a3",
    bytes: 231_508,
};

pub const ASSETS: &[Asset] = &[MODEL, VOCAB];

fn resolve_dir(env_value: Option<&str>, fallback: Option<PathBuf>) -> Option<PathBuf> {
    match env_value.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Some(PathBuf::from(s)),
        None => fallback,
    }
}

pub fn model_dir() -> Option<PathBuf> {
    let env = std::env::var(DIR_ENV).ok();
    resolve_dir(env.as_deref(), crate::core::tools::onnx::models_dir())
}

pub(crate) fn asset_path(a: &Asset) -> Option<PathBuf> {
    model_dir().map(|d| d.join(a.file))
}

pub(crate) fn has_asset(a: &Asset) -> bool {
    asset_path(a)
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| m.is_file() && m.len() == a.bytes)
        .unwrap_or(false)
}

pub fn is_downloaded() -> bool {
    ASSETS.iter().all(has_asset)
}

pub fn is_ready() -> bool {
    is_downloaded() && crate::core::onnxrt::is_installed()
}

struct Loaded {
    session: ort::session::Session,
    tokenizer: Tokenizer,
    input_names: Vec<String>,
    last_used: Instant,
}

fn cell() -> &'static Mutex<Option<Loaded>> {
    static CELL: OnceLock<Mutex<Option<Loaded>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

fn lock() -> std::sync::MutexGuard<'static, Option<Loaded>> {
    match cell().lock() {
        Ok(g) => g,
        Err(poisoned) => {
            let mut g = poisoned.into_inner();
            *g = None;
            g
        }
    }
}

fn load() -> Result<Loaded, EmbedError> {
    let dir =
        model_dir().ok_or_else(|| EmbedError::new(ERR_EMBED_MODEL, "no app data directory"))?;
    load_from(&dir)
}

fn load_from(dir: &Path) -> Result<Loaded, EmbedError> {
    let model = dir.join(MODEL.file);
    if !model.is_file() {
        return Err(EmbedError::new(
            ERR_EMBED_MODEL,
            format!(
                "the embedding model has not been downloaded yet (expected at {})",
                model.display()
            ),
        ));
    }
    let vocab_file = dir.join(VOCAB.file);
    let vocab_text = std::fs::read_to_string(&vocab_file).map_err(|e| {
        EmbedError::new(
            ERR_EMBED_VOCAB,
            format!("reading {}: {e}", vocab_file.display()),
        )
    })?;
    let tokenizer = Tokenizer::from_vocab_text(&vocab_text)
        .map_err(|e| EmbedError::new(ERR_EMBED_VOCAB, e.to_string()))?;

    let session = crate::core::tools::onnx::session_from_file(&model)
        .map_err(|e| EmbedError::new(ERR_EMBED_RUNTIME, e.to_string()))?;
    let input_names: Vec<String> = session
        .inputs()
        .iter()
        .map(|i| i.name().to_string())
        .collect();
    Ok(Loaded {
        session,
        tokenizer,
        input_names,
        last_used: Instant::now(),
    })
}

pub fn pool_and_normalize(hidden: &[f32], mask: &[u32], tokens: usize, dim: usize) -> Dim {
    let mut acc = [0.0f64; DIM];
    let mut kept = 0.0f64;
    for t in 0..tokens {
        if mask.get(t).copied().unwrap_or(0) == 0 {
            continue;
        }
        kept += 1.0;
        let base = t * dim;
        for (d, slot) in acc.iter_mut().enumerate().take(dim.min(DIM)) {
            *slot += hidden.get(base + d).copied().unwrap_or(0.0) as f64;
        }
    }
    let mut out = [0.0f32; DIM];
    if kept == 0.0 {
        return out;
    }
    let mut norm = 0.0f64;
    for slot in acc.iter_mut() {
        *slot /= kept;
        norm += *slot * *slot;
    }
    let norm = norm.sqrt();
    if norm <= 0.0 {
        return out;
    }
    for (slot, mean) in out.iter_mut().zip(acc.iter()) {
        *slot = (mean / norm) as f32;
    }
    out
}

fn run_chunk(l: &mut Loaded, texts: &[&str]) -> Result<Vec<Dim>, EmbedError> {
    let encodings = l.tokenizer.encode_batch(texts);
    let rows = encodings.len();
    let width = encodings.first().map(|e| e.len()).unwrap_or(0);
    if rows == 0 || width == 0 {
        return Ok(vec![[0.0; DIM]; rows]);
    }
    let shape = vec![rows as i64, width as i64];
    let mut ids: Vec<i64> = Vec::with_capacity(rows * width);
    let mut mask: Vec<i64> = Vec::with_capacity(rows * width);
    let mut types: Vec<i64> = Vec::with_capacity(rows * width);
    for e in &encodings {
        ids.extend(e.ids.iter().map(|v| *v as i64));
        mask.extend(e.attention_mask.iter().map(|v| *v as i64));
        types.extend(e.type_ids.iter().map(|v| *v as i64));
    }

    let tensor = |data: Vec<i64>| {
        ort::value::Tensor::from_array((shape.clone(), data))
            .map_err(|e| EmbedError::new(ERR_EMBED_INFER, format!("building the tensor: {e}")))
    };
    let mut inputs: Vec<(
        std::borrow::Cow<'static, str>,
        ort::session::SessionInputValue<'_>,
    )> = Vec::with_capacity(3);
    for name in l.input_names.clone() {
        let data = match name.as_str() {
            "input_ids" => ids.clone(),
            "attention_mask" => mask.clone(),
            "token_type_ids" => types.clone(),
            other => {
                return Err(EmbedError::new(
                    ERR_EMBED_SHAPE,
                    format!("the model asks for an unknown input `{other}`"),
                ))
            }
        };
        inputs.push((std::borrow::Cow::Owned(name), tensor(data)?.into()));
    }

    let outputs = l
        .session
        .run(inputs)
        .map_err(|e| EmbedError::new(ERR_EMBED_INFER, format!("inference failed: {e}")))?;
    let (out_shape, raw) = outputs[0]
        .try_extract_tensor::<f32>()
        .map_err(|e| EmbedError::new(ERR_EMBED_INFER, format!("reading the output: {e}")))?;
    if out_shape.len() != 3 || out_shape[2] as usize != DIM {
        return Err(EmbedError::new(
            ERR_EMBED_SHAPE,
            format!("expected [batch, tokens, {DIM}], got {:?}", &out_shape[..]),
        ));
    }
    let tokens = out_shape[1].max(0) as usize;
    let stride = tokens * DIM;
    let mut out = Vec::with_capacity(rows);
    for (i, e) in encodings.iter().enumerate() {
        let slice = raw
            .get(i * stride..(i + 1) * stride)
            .ok_or_else(|| EmbedError::new(ERR_EMBED_SHAPE, "output shorter than the batch"))?;
        out.push(pool_and_normalize(slice, &e.attention_mask, tokens, DIM));
    }
    Ok(out)
}

pub fn embed_batch(texts: &[&str]) -> Result<Vec<Dim>, EmbedError> {
    if texts.is_empty() {
        return Ok(vec![]);
    }
    let mut guard = lock();
    if guard
        .as_ref()
        .map(|l| l.last_used.elapsed() >= IDLE_TTL)
        .unwrap_or(false)
    {
        *guard = None;
    }
    if guard.is_none() {
        *guard = Some(load()?);
    }
    let loaded = guard.as_mut().expect("just loaded");
    let mut out = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(BATCH) {
        out.extend(run_chunk(loaded, chunk)?);
    }
    loaded.last_used = Instant::now();
    Ok(out)
}
