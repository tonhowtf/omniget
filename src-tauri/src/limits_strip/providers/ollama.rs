//! Ollama, the runtime on this machine.
//!
//! No credential. One loopback request: `GET http://127.0.0.1:11434/api/ps`,
//! the models held in memory right now. Reply: `models[]` with `name`, `size`,
//! `size_vram`, `context_length`, `expires_at` (ISO) and
//! `details.{parameter_size, quantization_level}`. Nothing leaves the machine,
//! and a runtime that is not running is a quiet reading, not an error.

use crate::limits_strip::{iso_ms, json_or_error, LocalModel, ReadError, Reading, UsageProvider};

const ENDPOINT: &str = "http://127.0.0.1:11434/api/ps";

pub struct Ollama;

pub fn parse_ps(v: &serde_json::Value) -> Vec<LocalModel> {
    v.get("models")
        .and_then(|x| x.as_array())
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let name = m.get("name").or_else(|| m.get("model"))?.as_str()?;
            let text = |p: &str| m.pointer(p).and_then(|x| x.as_str()).map(str::to_string);
            Some(LocalModel {
                name: name.to_string(),
                size_bytes: m.get("size").and_then(|x| x.as_u64()),
                vram_bytes: m.get("size_vram").and_then(|x| x.as_u64()),
                context: m.get("context_length").and_then(|x| x.as_u64()),
                quant: text("/details/quantization_level"),
                params: text("/details/parameter_size"),
                expires_at: iso_ms(m.get("expires_at")),
            })
        })
        .collect()
}

#[async_trait::async_trait]
impl UsageProvider for Ollama {
    fn id(&self) -> &str {
        "ollama"
    }
    fn label(&self) -> &'static str {
        "Ollama"
    }
    fn local(&self) -> bool {
        true
    }
    fn beta(&self) -> bool {
        false
    }

    async fn detect(&self) -> bool {
        super::installed(".ollama", "ollama")
    }

    async fn read(&self) -> Result<Reading, ReadError> {
        let Ok(resp) = super::loopback().get(ENDPOINT).send().await else {
            return Ok(super::not_running());
        };
        let local_models = parse_ps(&json_or_error(resp).await?);
        Ok(Reading {
            note: local_models
                .is_empty()
                .then(|| "running, no model loaded".to_string()),
            local_models,
            ..Default::default()
        })
    }
}
