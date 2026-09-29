//! MiniLM-L6-v2 in ONNX: pinned catalog, on-demand download, batched
//! inference on the `ort` runtime that is already linked (`load-dynamic`).
//!
//! Same shape as `core/tools/onnx.rs` and `core/whisper.rs`: every file is an
//! asset with a URL pinned to an immutable Hugging Face revision, a sha256 and
//! a byte count, checked on the freshly downloaded file before it becomes
//! official. Nothing is downloaded without an explicit call.
//!
//! The session is created on first use and dropped after `IDLE_TTL` without
//! work — there is no background thread, the check happens when someone calls
//! `embed_batch` or `release_if_idle`. With the world asleep, nothing here
//! runs and nothing here is resident.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::{
    tokenizer::Tokenizer, EmbedError, ERR_EMBED_DOWNLOAD, ERR_EMBED_INFER, ERR_EMBED_MODEL,
    ERR_EMBED_RUNTIME, ERR_EMBED_SHAPE, ERR_EMBED_VOCAB,
};
use crate::core::tools::ProgressFn;

/// Output width of `all-MiniLM-L6-v2`.
pub const DIM: usize = 384;

/// One vector. Fixed size so a mismatch is a compile error, not a runtime one.
pub type Dim = [f32; DIM];

/// How many texts go into one `run()`. Bigger batches stop paying off once the
/// matmul saturates the cores and cost RSS, which is the tighter budget here.
pub const BATCH: usize = 64;

/// The session is dropped after this long without a batch.
pub const IDLE_TTL: Duration = Duration::from_secs(600);

/// Directory override for tests and for anyone who already has the files.
/// Must contain `MODEL.file` and `VOCAB.file` with the pinned sha256.
pub const DIR_ENV: &str = "OMNIGET_EMBED_MODEL_DIR";

/// A managed file of the model.
#[derive(Debug, Clone, Copy)]
pub struct Asset {
    pub file: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub bytes: u64,
}

/// Immutable revision of `Xenova/all-MiniLM-L6-v2` (commit of 2026-09-18);
/// `resolve/<sha>` never moves, unlike `resolve/main`. The URLs below spell it
/// out (a `const` cannot be interpolated into a literal); the test checks both
/// assets carry this exact revision.
#[cfg(test)]
const HF_REV: &str = "751bff37182d3f1213fa05d7196b954e230abad9";

/// Int8-quantized graph: 23 MB instead of the 90 MB of the fp32 export, which
/// is what plan §2.1 line 17 budgets. The regression fixtures were recorded
/// with this exact file, so swapping it means re-recording them.
pub const MODEL: Asset = Asset {
    file: "minilm-l6-v2.onnx",
    url: "https://huggingface.co/Xenova/all-MiniLM-L6-v2/resolve/751bff37182d3f1213fa05d7196b954e230abad9/onnx/model_quantized.onnx",
    sha256: "afdb6f1a0e45b715d0bb9b11772f032c399babd23bfc31fed1c170afc848bdb1",
    bytes: 22_972_370,
};

/// The BERT-uncased vocabulary of the same revision. Downloaded instead of
/// compiled in, so the 226 KB do not ride in every binary.
pub const VOCAB: Asset = Asset {
    file: "minilm-l6-v2.vocab.txt",
    url: "https://huggingface.co/Xenova/all-MiniLM-L6-v2/resolve/751bff37182d3f1213fa05d7196b954e230abad9/vocab.txt",
    sha256: "07eced375cec144d27c900241f3e339478dec958f92fddbc551f295c992038a3",
    bytes: 231_508,
};

pub const ASSETS: &[Asset] = &[MODEL, VOCAB];

pub const LICENSE: &str = "Apache-2.0";
pub const SOURCE: &str = "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2";

pub fn dim() -> usize {
    DIM
}

/// Pure part of the directory choice, so it is testable without touching the
/// process environment (which is shared by every test thread).
fn resolve_dir(env_value: Option<&str>, fallback: Option<PathBuf>) -> Option<PathBuf> {
    match env_value.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Some(PathBuf::from(s)),
        None => fallback,
    }
}

/// Where the two files live. `OMNIGET_EMBED_MODEL_DIR` wins so a test (or a
/// user with the files already on disk) never downloads.
pub fn model_dir() -> Option<PathBuf> {
    let env = std::env::var(DIR_ENV).ok();
    resolve_dir(env.as_deref(), crate::core::tools::onnx::models_dir())
}

pub fn model_path() -> Option<PathBuf> {
    model_dir().map(|d| d.join(MODEL.file))
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

/// Both files on disk with the right size. Cheap: no hashing, no session.
pub fn is_downloaded() -> bool {
    ASSETS.iter().all(has_asset)
}

/// Downloaded **and** the `ort` runtime resolvable, i.e. a batch would run
/// without touching the network.
pub fn is_ready() -> bool {
    is_downloaded() && crate::core::onnxrt::is_installed()
}

pub fn sha256_of(path: &Path) -> Result<String, EmbedError> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)
        .map_err(|e| EmbedError::new(ERR_EMBED_DOWNLOAD, format!("opening {path:?}: {e}")))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| EmbedError::new(ERR_EMBED_DOWNLOAD, format!("reading {path:?}: {e}")))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub(crate) async fn fetch_asset(a: &Asset, progress: &ProgressFn) -> Result<PathBuf, EmbedError> {
    let dest = asset_path(a).ok_or_else(|| {
        EmbedError::new(ERR_EMBED_MODEL, "no app data directory to store the model")
    })?;
    if has_asset(a) {
        return Ok(dest);
    }
    let dir = dest
        .parent()
        .ok_or_else(|| EmbedError::new(ERR_EMBED_MODEL, "model path without a parent"))?
        .to_path_buf();
    std::fs::create_dir_all(&dir)
        .map_err(|e| EmbedError::new(ERR_EMBED_DOWNLOAD, format!("creating {dir:?}: {e}")))?;

    let tmp = dir.join(format!(".{}.download", a.file));
    let id = format!("embed-model:{}", a.file);
    let client = crate::core::tools::client()
        .map_err(|e| EmbedError::new(ERR_EMBED_DOWNLOAD, e.to_string()))?;
    crate::core::tools::download_to(&client, a.url, &tmp, progress, &id)
        .await
        .map_err(|e| EmbedError::new(ERR_EMBED_DOWNLOAD, format!("{}: {e}", a.url)))?;

    let expected = a.sha256;
    let tmp2 = tmp.clone();
    let dest2 = dest.clone();
    let p = progress.clone();
    let id2 = id.clone();
    tokio::task::spawn_blocking(move || -> Result<(), EmbedError> {
        crate::core::tools::report(&p, &id2, "verify", 0, None, Some("checking sha256".into()));
        let got = sha256_of(&tmp2)?;
        if got != expected {
            let _ = std::fs::remove_file(&tmp2);
            return Err(EmbedError::new(
                ERR_EMBED_DOWNLOAD,
                format!("checksum mismatch: expected {expected}, got {got}"),
            ));
        }
        if dest2.exists() {
            let _ = std::fs::remove_file(&dest2);
        }
        std::fs::rename(&tmp2, &dest2)
            .map_err(|e| EmbedError::new(ERR_EMBED_DOWNLOAD, format!("moving to {dest2:?}: {e}")))
    })
    .await
    .map_err(|e| EmbedError::new(ERR_EMBED_DOWNLOAD, format!("verify task died: {e}")))??;

    crate::core::tools::report(progress, &id, "done", a.bytes, Some(a.bytes), None);
    Ok(dest)
}

/// Guarantees model + vocabulary on disk and returns the path of the `.onnx`.
/// Only touches the network when a file is missing or the wrong size.
pub async fn ensure_model(progress: &ProgressFn) -> Result<PathBuf, EmbedError> {
    fetch_asset(&VOCAB, progress).await?;
    fetch_asset(&MODEL, progress).await
}

/// Deletes the downloaded files. Drops the live session first, otherwise
/// Windows refuses to unlink a mapped file.
pub fn remove_model() -> Result<(), EmbedError> {
    release_now();
    for a in ASSETS {
        if let Some(p) = asset_path(a) {
            if p.is_file() {
                std::fs::remove_file(&p).map_err(|e| {
                    EmbedError::new(ERR_EMBED_MODEL, format!("deleting {p:?}: {e}"))
                })?;
            }
        }
    }
    Ok(())
}

// ── Session, created on demand and released when idle ──────────────────

struct Loaded {
    session: ort::session::Session,
    tokenizer: Tokenizer,
    /// Input names the graph actually declares (`token_type_ids` is optional
    /// in some exports), in the order the graph lists them.
    input_names: Vec<String>,
    last_used: Instant,
}

fn cell() -> &'static Mutex<Option<Loaded>> {
    static CELL: OnceLock<Mutex<Option<Loaded>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(None))
}

/// Poisoned mutex means a previous batch panicked inside `ort`; we take the
/// guard anyway and rebuild the session from scratch.
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

/// Drops the session when it has been idle for `IDLE_TTL`. Returns true when
/// something was actually released. Cheap enough to call from a timer tick.
pub fn release_if_idle() -> bool {
    let mut guard = lock();
    let stale = guard
        .as_ref()
        .map(|l| l.last_used.elapsed() >= IDLE_TTL)
        .unwrap_or(false);
    if stale {
        *guard = None;
    }
    stale
}

/// Drops the session right now (settings changed, model removed, quitting).
pub fn release_now() -> bool {
    lock().take().is_some()
}

/// How long the live session has been idle, or `None` when there is none.
/// Lets a caller assert "nothing resident" without reaching into the mutex.
pub fn session_age() -> Option<Duration> {
    lock().as_ref().map(|l| l.last_used.elapsed())
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

    // `session_from_file` calls `onnxrt::init()`, whose error already tells
    // the user how to install the runtime; we only re-tag the code.
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

/// Mean pooling over the non-padded positions, then L2 normalization — the
/// recipe `sentence-transformers/all-MiniLM-L6-v2` is trained with. Pure, so
/// it is tested without the model.
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
    // Named inputs, in the order the graph declares them, so an export without
    // `token_type_ids` still runs.
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

/// 384-d normalized vectors for every text, in chunks of `BATCH`.
///
/// **Blocking**: loads the session on first call and runs the graph on this
/// thread. Call it inside `tokio::task::spawn_blocking`.
pub fn embed_batch(texts: &[&str]) -> Result<Vec<Dim>, EmbedError> {
    if texts.is_empty() {
        return Ok(vec![]);
    }
    let mut guard = lock();
    // An idle session is stale by definition: drop before reusing.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::embed::cosine;

    #[test]
    fn os_assets_tem_sha256_tamanho_e_url_da_revisao_fixada() {
        assert_eq!(ASSETS.len(), 2);
        for a in ASSETS {
            assert_eq!(a.sha256.len(), 64, "{} tem sha256 torto", a.file);
            assert!(a.sha256.chars().all(|c| c.is_ascii_hexdigit()));
            assert!(a.bytes > 100_000, "{} parece pequeno demais", a.file);
            assert!(
                a.url.contains(HF_REV),
                "{} não aponta para a revisão fixada",
                a.file
            );
            assert!(a.url.starts_with("https://huggingface.co/"));
        }
        // O maior arquivo cabe no orçamento de 23 MB do plano §2.1 linha 17.
        let maior = ASSETS.iter().map(|a| a.bytes).max().unwrap_or(0);
        assert!(maior < 24 * 1024 * 1024, "download de {maior} bytes");
        assert_ne!(MODEL.sha256, VOCAB.sha256);
        assert_ne!(MODEL.file, VOCAB.file);
    }

    #[test]
    fn o_diretorio_do_modelo_obedece_a_variavel_de_ambiente() {
        // Sem mexer na env do processo (é compartilhada pelas threads de
        // teste): a decisão mora numa função pura.
        let marca = PathBuf::from("/tmp/omniget-embed-teste");
        let padrao = Some(PathBuf::from("/tmp/padrao"));
        assert_eq!(
            resolve_dir(Some("/tmp/omniget-embed-teste"), padrao.clone()),
            Some(marca)
        );
        assert_eq!(resolve_dir(Some("  "), padrao.clone()), padrao);
        assert_eq!(resolve_dir(None, padrao.clone()), padrao);
        assert_eq!(resolve_dir(None, None), None);
    }

    #[test]
    fn modelo_ausente_da_erro_com_codigo_estavel() {
        let vazio = std::env::temp_dir().join("omniget-embed-vazio");
        let _ = std::fs::create_dir_all(&vazio);
        let _ = std::fs::remove_file(vazio.join(MODEL.file));
        let e = load_from(&vazio).err().expect("sem modelo, tem que falhar");
        assert_eq!(e.code, ERR_EMBED_MODEL);
        assert!(e.message.contains(MODEL.file));
    }

    #[test]
    fn vocab_ausente_da_erro_de_vocab() {
        let dir = std::env::temp_dir().join("omniget-embed-sem-vocab");
        let _ = std::fs::create_dir_all(&dir);
        // Um `.onnx` de mentira basta: o vocabulário é lido antes da sessão.
        std::fs::write(dir.join(MODEL.file), b"not a model").expect("escreve");
        let _ = std::fs::remove_file(dir.join(VOCAB.file));
        let e = load_from(&dir).err().expect("sem vocab, tem que falhar");
        assert_eq!(e.code, ERR_EMBED_VOCAB);
    }

    #[test]
    fn lote_vazio_nao_carrega_sessao() {
        assert!(embed_batch(&[]).expect("vazio").is_empty());
        assert_eq!(session_age(), None, "nada pode ficar residente");
    }

    #[test]
    fn a_media_ignora_o_padding_e_o_vetor_sai_normalizado() {
        // 2 tokens reais + 1 de padding com valores absurdos.
        let mut hidden = vec![0.0f32; 3 * DIM];
        hidden[0] = 3.0;
        hidden[DIM] = 1.0;
        hidden[2 * DIM] = 1000.0;
        let v = pool_and_normalize(&hidden, &[1, 1, 0], 3, DIM);
        assert!((v[0] - 1.0).abs() < 1e-6, "só a dimensão 0 tem sinal");
        let norma: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norma - 1.0).abs() < 1e-5, "norma {norma}");
    }

    #[test]
    fn media_de_mascara_toda_zero_devolve_vetor_nulo() {
        let hidden = vec![5.0f32; 2 * DIM];
        let v = pool_and_normalize(&hidden, &[0, 0], 2, DIM);
        assert!(v.iter().all(|x| *x == 0.0));
        assert_eq!(cosine(&v, &v), 0.0);
    }

    #[test]
    fn a_media_e_mesmo_media_e_nao_soma() {
        let mut a = vec![0.0f32; 2 * DIM];
        a[0] = 2.0;
        a[DIM] = 4.0;
        let mut b = vec![0.0f32; 4 * DIM];
        b[0] = 2.0;
        b[DIM] = 4.0;
        b[2 * DIM] = 2.0;
        b[3 * DIM] = 4.0;
        let va = pool_and_normalize(&a, &[1, 1], 2, DIM);
        let vb = pool_and_normalize(&b, &[1, 1, 1, 1], 4, DIM);
        assert!((cosine(&va, &vb) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn o_lote_e_de_64_e_a_sessao_expira_em_dez_minutos() {
        assert_eq!(BATCH, 64);
        assert_eq!(IDLE_TTL, Duration::from_secs(600));
        assert_eq!(DIM, 384);
        assert!(!release_now(), "nenhum teste puro pode deixar sessão viva");
        assert!(!release_if_idle());
    }

    // ── Testes com o modelo real ───────────────────────────────────────
    //
    // Precisam do ONNX Runtime e dos dois arquivos do modelo. Rode com:
    //   OMNIGET_EMBED_MODEL_DIR=<pasta com os dois arquivos> \
    //   ORT_DYLIB_PATH=<libonnxruntime> \
    //   cargo test -p omniget-core --features desktop embed:: -- --ignored --nocapture
    // Sem a pasta, `ensure_model` baixa 23 MB da revisão fixada.

    const FIXTURES: &str = include_str!("../../../tests/embed_fixtures/minilm_vectors.json");

    #[derive(serde::Deserialize)]
    struct Fixture {
        model_sha256: String,
        dim: usize,
        cases: Vec<Case>,
    }

    #[derive(serde::Deserialize)]
    struct Case {
        text: String,
        vector: Vec<f32>,
    }

    #[test]
    fn as_fixtures_descrevem_o_modelo_fixado() {
        let f: Fixture = serde_json::from_str(FIXTURES).expect("fixtures");
        assert_eq!(f.model_sha256, MODEL.sha256, "fixtures de outro modelo");
        assert_eq!(f.dim, DIM);
        assert_eq!(f.cases.len(), 10);
        for c in &f.cases {
            assert_eq!(c.vector.len(), DIM, "vetor torto em {:?}", c.text);
            let n: f32 = c.vector.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!((n - 1.0).abs() < 1e-3, "fixture não normalizada: {n}");
        }
    }

    /// Os 10 textos de referência das fixtures (2 em inglês, 2 em português,
    /// CJK, acento, pontuação, vazio, número e um texto longo truncado).
    const TEXTOS_DE_REFERENCIA: [&str; 10] = [
        "how do I download a youtube video",
        "the quick brown fox jumps over the lazy dog",
        "baixar vídeo do youtube com legenda",
        "o OmniGet organiza downloads, ferramentas e agentes",
        "下载视频并保存到本地",
        "Café, açúcar e pão de queijo — três acentos",
        "!!! ??? ... ;;;",
        "",
        "2026-09-18 12:34:56 +0000",
        "an agent remembers what the user asked for, ranks the memories by \
         cosine similarity and feeds the top ones back into the next turn, \
         which is the whole point of having a local embedding model at all",
    ];

    /// Regrava `tests/embed_fixtures/minilm_vectors.json` com o modelo real.
    /// Só escreve quando `OMNIGET_EMBED_FIXTURE_OUT` aponta um arquivo:
    ///   OMNIGET_EMBED_FIXTURE_OUT=.../minilm_vectors.json \
    ///   OMNIGET_EMBED_MODEL_DIR=... ORT_DYLIB_PATH=... \
    ///   cargo test -p omniget-core --features desktop embed:: -- --ignored
    #[test]
    #[ignore = "modelo real: regrava as fixtures de regressão"]
    fn grava_as_fixtures() {
        let Ok(out) = std::env::var("OMNIGET_EMBED_FIXTURE_OUT") else {
            println!("OMNIGET_EMBED_FIXTURE_OUT não definido: nada a gravar");
            return;
        };
        let vs = embed_batch(&TEXTOS_DE_REFERENCIA).expect("embedding");
        let cases: Vec<serde_json::Value> = TEXTOS_DE_REFERENCIA
            .iter()
            .zip(&vs)
            .map(|(t, v)| serde_json::json!({ "text": t, "vector": v.to_vec() }))
            .collect();
        let doc = serde_json::json!({
            "model": MODEL.file,
            "model_sha256": MODEL.sha256,
            "model_url": MODEL.url,
            "dim": DIM,
            "pooling": "mean over attention mask, then L2",
            "cases": cases,
        });
        std::fs::write(&out, serde_json::to_string_pretty(&doc).expect("json")).expect("grava");
        println!("fixtures gravadas em {out}");
        release_now();
    }

    #[test]
    #[ignore = "modelo real: precisa do ONNX Runtime e dos 23 MB do MiniLM"]
    fn regressao_numerica_contra_as_fixtures() {
        let f: Fixture = serde_json::from_str(FIXTURES).expect("fixtures");
        let textos: Vec<&str> = f.cases.iter().map(|c| c.text.as_str()).collect();
        let got = embed_batch(&textos).expect("embedding");
        assert_eq!(got.len(), f.cases.len());
        for (c, v) in f.cases.iter().zip(&got) {
            let pior = c
                .vector
                .iter()
                .zip(v.iter())
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(pior < 1e-3, "{:?} desviou {pior}", c.text);
            assert!(cosine(&c.vector, v) > 0.999);
        }
        release_now();
    }

    #[test]
    #[ignore = "modelo real: semântica (frases parecidas ficam perto)"]
    fn frases_parecidas_ficam_mais_perto_que_frases_diferentes() {
        let v = embed_batch(&[
            "how do I download a youtube video",
            "baixar vídeo do youtube",
            "the cat sleeps on the sofa",
        ])
        .expect("embedding");
        let perto = cosine(&v[0], &v[1]);
        let longe = cosine(&v[0], &v[2]);
        assert!(perto > longe, "perto={perto} longe={longe}");
        release_now();
    }

    #[test]
    #[ignore = "modelo real: 1 000 embeddings de 40 tokens em lote de 64 (I-10)"]
    fn mil_embeddings_dentro_do_orcamento() {
        let frase = "the agent remembered that the user asked for a download of a long \
                     video from the tools tab and the batch was queued right away today";
        let textos: Vec<String> = (0..1000).map(|i| format!("{frase} #{i}")).collect();
        let refs: Vec<&str> = textos.iter().map(|s| s.as_str()).collect();
        // Aquece a sessão: o que se mede é o lote, não o carregamento.
        embed_batch(&refs[..1]).expect("warmup");
        let t0 = Instant::now();
        let got = embed_batch(&refs).expect("lote");
        let dt = t0.elapsed();
        assert_eq!(got.len(), 1000);
        let por_texto = dt.as_secs_f64() * 1000.0 / 1000.0;
        println!(
            "1000 embeddings em {:?} ({:.2} ms por texto, lote de {})",
            dt, por_texto, BATCH
        );
        // O alvo do plano (I-10) é 3 s num runner de 2 threads e só vale numa
        // máquina ociosa; aqui o teto é um sinal de vida, não o orçamento —
        // quem cobra os 3 s é o verificador, com a máquina parada.
        assert!(dt < Duration::from_secs(60), "lento demais: {dt:?}");
        release_now();
    }
}
