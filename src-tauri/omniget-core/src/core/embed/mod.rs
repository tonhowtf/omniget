//! Local embeddings (MiniLM-L6-v2 over `ort`, own WordPiece). Plan §2.1 line 17.
//!
//! Default source is the ONNX model we manage ourselves (`minilm`): offline,
//! no external process, 384 dimensions. When the user has Ollama installed and
//! picks it in Settings, `ollama_embed` answers instead — with its own
//! dimension count, so whoever stores vectors must store which source produced
//! them (see `EmbedSource::dim`).
//!
//! Everything here is blocking except `ensure_model` and the Ollama path: call
//! `embed_batch` from inside `tokio::task::spawn_blocking`.

pub mod minilm;
pub mod ollama_embed;
pub mod tokenizer;
pub mod unigram;
pub mod xlmr;

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

pub use minilm::{
    dim as onnx_dim, ensure_model, is_ready, release_if_idle, release_now, session_age, Dim, DIM,
};
pub use tokenizer::Tokenizer;

pub const ERR_EMBED_MODEL: &str = "ERR_EMBED_MODEL";
pub const ERR_EMBED_RUNTIME: &str = "ERR_EMBED_RUNTIME";
pub const ERR_EMBED_DOWNLOAD: &str = "ERR_EMBED_DOWNLOAD";
pub const ERR_EMBED_VOCAB: &str = "ERR_EMBED_VOCAB";
pub const ERR_EMBED_INFER: &str = "ERR_EMBED_INFER";
pub const ERR_EMBED_SHAPE: &str = "ERR_EMBED_SHAPE";
pub const ERR_EMBED_OLLAMA: &str = "ERR_EMBED_OLLAMA";

/// Same shape as `LlmError`: a stable code the UI can map plus a message for
/// the log. No `retryable` here — an embedding never rate-limits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedError {
    pub code: Cow<'static, str>,
    pub message: String,
}

impl EmbedError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code: Cow::Borrowed(code),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for EmbedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for EmbedError {}

/// Where the vectors come from. Stored next to every vector, because two
/// sources produce different dimensions and are not comparable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EmbedSource {
    /// MiniLM-L6-v2 in ONNX, managed by us. The default.
    #[default]
    Onnx,
    /// `nomic-embed-text` on a local Ollama, when the user has one.
    Ollama,
}

impl EmbedSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            EmbedSource::Onnx => "onnx",
            EmbedSource::Ollama => "ollama",
        }
    }

    /// Dimensions of each source. Fixed per model, so an index that mixes
    /// sources is a bug the caller can catch before storing.
    pub fn dim(&self) -> usize {
        match self {
            EmbedSource::Onnx => DIM,
            EmbedSource::Ollama => ollama_embed::DIM,
        }
    }

    /// Parses the Settings value; anything unknown falls back to the default.
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "ollama" => EmbedSource::Ollama,
            _ => EmbedSource::Onnx,
        }
    }
}

/// The contract of §4: 384-d normalized vectors, in batches, blocking.
pub fn embed_batch(texts: &[&str]) -> Result<Vec<Dim>, EmbedError> {
    minilm::embed_batch(texts)
}

/// Same thing through whichever source the user picked. Async because Ollama
/// is a local HTTP service; the ONNX path just hops to `spawn_blocking`.
pub async fn embed_batch_with(
    source: EmbedSource,
    texts: &[&str],
) -> Result<Vec<Vec<f32>>, EmbedError> {
    match source {
        EmbedSource::Onnx => {
            let owned: Vec<String> = texts.iter().map(|t| (*t).to_string()).collect();
            tokio::task::spawn_blocking(move || {
                let refs: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
                minilm::embed_batch(&refs).map(|v| v.into_iter().map(|e| e.to_vec()).collect())
            })
            .await
            .map_err(|e| EmbedError::new(ERR_EMBED_INFER, format!("batch task died: {e}")))?
        }
        EmbedSource::Ollama => ollama_embed::embed_batch(None, None, texts).await,
    }
}

/// Cosine similarity. Vectors out of this module are already L2-normalized, so
/// this is a dot product — but we divide anyway, because callers also feed it
/// stored vectors that may come from elsewhere. Returns 0.0 for a zero vector
/// or a length mismatch instead of `NaN`.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut na = 0.0f32;
    let mut nb = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na <= 0.0 || nb <= 0.0 {
        return 0.0;
    }
    (dot / (na.sqrt() * nb.sqrt())).clamp(-1.0, 1.0)
}

/// Indices of the `k` closest vectors to `query`, best first. Small and
/// linear on purpose: agent memory is thousands of entries, not millions.
pub fn top_k(query: &[f32], corpus: &[Vec<f32>], k: usize) -> Vec<(usize, f32)> {
    let mut scored: Vec<(usize, f32)> = corpus
        .iter()
        .enumerate()
        .map(|(i, v)| (i, cosine(query, v)))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.truncate(k);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosseno_de_vetores_iguais_e_um_e_de_opostos_e_menos_um() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![-1.0, -2.0, -3.0];
        assert!((cosine(&a, &a) - 1.0).abs() < 1e-6);
        assert!((cosine(&a, &b) + 1.0).abs() < 1e-6);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
    }

    #[test]
    fn cosseno_nao_devolve_nan_em_caso_degenerado() {
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), 0.0);
        assert_eq!(cosine(&[], &[]), 0.0);
    }

    #[test]
    fn cosseno_fica_no_intervalo_mesmo_com_erro_de_ponto_flutuante() {
        let a: Vec<f32> = (0..384).map(|i| (i as f32) * 1e-3).collect();
        let c = cosine(&a, &a);
        assert!((-1.0..=1.0).contains(&c));
    }

    #[test]
    fn top_k_ordena_do_mais_parecido_para_o_menos() {
        let q = vec![1.0, 0.0];
        let corpus = vec![vec![0.0, 1.0], vec![1.0, 0.0], vec![0.7, 0.7]];
        let got = top_k(&q, &corpus, 2);
        assert_eq!(got.iter().map(|(i, _)| *i).collect::<Vec<_>>(), vec![1, 2]);
        assert!(got[0].1 > got[1].1);
    }

    #[test]
    fn top_k_com_corpus_menor_que_k_devolve_o_que_tem() {
        assert_eq!(top_k(&[1.0], &[vec![1.0]], 10).len(), 1);
        assert!(top_k(&[1.0], &[], 3).is_empty());
    }

    #[test]
    fn a_fonte_padrao_e_onnx_e_cada_fonte_tem_sua_dimensao() {
        assert_eq!(EmbedSource::default(), EmbedSource::Onnx);
        assert_eq!(EmbedSource::parse("OLLAMA"), EmbedSource::Ollama);
        assert_eq!(EmbedSource::parse("qualquer coisa"), EmbedSource::Onnx);
        assert_eq!(EmbedSource::Onnx.dim(), 384);
        assert_eq!(EmbedSource::Ollama.dim(), 768);
        assert_ne!(EmbedSource::Onnx.dim(), EmbedSource::Ollama.dim());
    }

    #[test]
    fn a_fonte_atravessa_o_json_com_o_mesmo_nome_da_settings() {
        let s = serde_json::to_string(&EmbedSource::Ollama).expect("json");
        assert_eq!(s, "\"ollama\"");
        assert_eq!(EmbedSource::Onnx.as_str(), "onnx");
    }

    #[test]
    fn o_erro_carrega_codigo_estavel() {
        let e = EmbedError::new(ERR_EMBED_MODEL, "sem modelo");
        assert_eq!(e.code, "ERR_EMBED_MODEL");
        assert_eq!(e.to_string(), "ERR_EMBED_MODEL: sem modelo");
        let round: EmbedError = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(round, e);
    }
}
