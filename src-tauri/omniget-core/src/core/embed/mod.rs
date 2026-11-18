pub mod minilm;
pub mod tokenizer;
pub mod unigram;
pub mod xlmr;

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

pub use minilm::{is_ready, Dim, DIM};
pub use tokenizer::Tokenizer;

pub const ERR_EMBED_MODEL: &str = "ERR_EMBED_MODEL";
pub const ERR_EMBED_RUNTIME: &str = "ERR_EMBED_RUNTIME";
pub const ERR_EMBED_VOCAB: &str = "ERR_EMBED_VOCAB";
pub const ERR_EMBED_INFER: &str = "ERR_EMBED_INFER";
pub const ERR_EMBED_SHAPE: &str = "ERR_EMBED_SHAPE";

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

pub fn embed_batch(texts: &[&str]) -> Result<Vec<Dim>, EmbedError> {
    minilm::embed_batch(texts)
}

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
