//! Alternative embedding source: `nomic-embed-text` on a local Ollama.
//!
//! Only used when the user picks it in Settings (`EmbedSource::Ollama`); the
//! default stays the ONNX model we manage. Nothing here starts, installs or
//! pings Ollama on its own — the host comes from `tools/ollama.rs`, and a
//! request only happens when somebody asks for vectors.
//!
//! Two endpoints, because both exist in the wild: `/api/embed` (batched, newer)
//! and `/api/embeddings` (one prompt per call, older). We try the batched one
//! and fall back on 404/405.

use serde_json::json;

use super::{EmbedError, ERR_EMBED_OLLAMA};

/// `nomic-embed-text` is 768-d, which is why a stored vector must carry the
/// source that produced it.
pub const DIM: usize = 768;

pub const DEFAULT_MODEL: &str = "nomic-embed-text";

/// Same 64 as the ONNX path: one HTTP request per chunk.
pub const BATCH: usize = 64;

fn base(host: Option<&str>) -> String {
    let raw = host
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(crate::core::tools::ollama::DEFAULT_HOST);
    raw.trim_end_matches('/').to_string()
}

/// Reads `{"embeddings": [[..], ..]}` (`/api/embed`).
pub fn parse_batch(body: &serde_json::Value, expected: usize) -> Result<Vec<Vec<f32>>, EmbedError> {
    let arr = body
        .get("embeddings")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            EmbedError::new(
                ERR_EMBED_OLLAMA,
                format!("answer without `embeddings`: {}", short(body)),
            )
        })?;
    if arr.len() != expected {
        return Err(EmbedError::new(
            ERR_EMBED_OLLAMA,
            format!("asked for {expected} vectors, got {}", arr.len()),
        ));
    }
    arr.iter().map(numbers).collect()
}

/// Reads `{"embedding": [..]}` (`/api/embeddings`).
pub fn parse_single(body: &serde_json::Value) -> Result<Vec<f32>, EmbedError> {
    let v = body.get("embedding").ok_or_else(|| {
        EmbedError::new(
            ERR_EMBED_OLLAMA,
            format!("answer without `embedding`: {}", short(body)),
        )
    })?;
    numbers(v)
}

fn numbers(v: &serde_json::Value) -> Result<Vec<f32>, EmbedError> {
    let arr = v
        .as_array()
        .ok_or_else(|| EmbedError::new(ERR_EMBED_OLLAMA, "a vector came back as a non-array"))?;
    let mut out = Vec::with_capacity(arr.len());
    for n in arr {
        out.push(n.as_f64().ok_or_else(|| {
            EmbedError::new(ERR_EMBED_OLLAMA, "a vector has a non-numeric element")
        })? as f32);
    }
    if out.is_empty() {
        return Err(EmbedError::new(
            ERR_EMBED_OLLAMA,
            "an empty vector came back",
        ));
    }
    Ok(out)
}

fn short(v: &serde_json::Value) -> String {
    let s = v.to_string();
    if s.len() > 200 {
        format!("{}…", &s[..200])
    } else {
        s
    }
}

/// L2-normalizes in place, so an Ollama vector is comparable with `cosine` the
/// same way the ONNX ones are. A zero vector stays zero.
pub fn normalize(v: &mut [f32]) {
    let norm = v
        .iter()
        .map(|x| (*x as f64) * (*x as f64))
        .sum::<f64>()
        .sqrt();
    if norm <= 0.0 {
        return;
    }
    for x in v.iter_mut() {
        *x = (*x as f64 / norm) as f32;
    }
}

/// Embeddings from a local Ollama. `host`/`model` default to
/// `http://127.0.0.1:11434` and `nomic-embed-text`.
pub async fn embed_batch(
    host: Option<&str>,
    model: Option<&str>,
    texts: &[&str],
) -> Result<Vec<Vec<f32>>, EmbedError> {
    if texts.is_empty() {
        return Ok(vec![]);
    }
    let b = base(host);
    let model = model
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_MODEL);
    let client = crate::core::tools::client()
        .map_err(|e| EmbedError::new(ERR_EMBED_OLLAMA, e.to_string()))?;

    let mut out: Vec<Vec<f32>> = Vec::with_capacity(texts.len());
    let mut batched_supported = true;
    for chunk in texts.chunks(BATCH) {
        if batched_supported {
            let resp = client
                .post(format!("{b}/api/embed"))
                .json(&json!({ "model": model, "input": chunk }))
                .send()
                .await
                .map_err(|e| {
                    EmbedError::new(ERR_EMBED_OLLAMA, format!("{b} did not answer: {e}"))
                })?;
            let status = resp.status();
            if status.is_success() {
                let body: serde_json::Value = resp.json().await.map_err(|e| {
                    EmbedError::new(ERR_EMBED_OLLAMA, format!("unreadable answer: {e}"))
                })?;
                let mut vs = parse_batch(&body, chunk.len())?;
                for v in &mut vs {
                    normalize(v);
                }
                out.extend(vs);
                continue;
            }
            if status.as_u16() != 404 && status.as_u16() != 405 {
                return Err(http_error(&b, status, resp.text().await.ok()));
            }
            // Old Ollama: no `/api/embed`. One call per text from here on.
            batched_supported = false;
        }
        for text in chunk {
            let resp = client
                .post(format!("{b}/api/embeddings"))
                .json(&json!({ "model": model, "prompt": text }))
                .send()
                .await
                .map_err(|e| {
                    EmbedError::new(ERR_EMBED_OLLAMA, format!("{b} did not answer: {e}"))
                })?;
            let status = resp.status();
            if !status.is_success() {
                return Err(http_error(&b, status, resp.text().await.ok()));
            }
            let body: serde_json::Value = resp.json().await.map_err(|e| {
                EmbedError::new(ERR_EMBED_OLLAMA, format!("unreadable answer: {e}"))
            })?;
            let mut v = parse_single(&body)?;
            normalize(&mut v);
            out.push(v);
        }
    }
    Ok(out)
}

fn http_error(host: &str, status: reqwest::StatusCode, body: Option<String>) -> EmbedError {
    let hint = if status.as_u16() == 404 {
        format!(" — is the `{DEFAULT_MODEL}` model pulled?")
    } else {
        String::new()
    };
    let tail = body
        .map(|b| b.chars().take(200).collect::<String>())
        .unwrap_or_default();
    EmbedError::new(
        ERR_EMBED_OLLAMA,
        format!("{host} answered HTTP {status}{hint} {tail}"),
    )
}
