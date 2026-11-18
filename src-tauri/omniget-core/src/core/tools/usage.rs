use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageEntry {
    pub ts: String,
    pub task: String,
    pub provider: String,
    pub model: String,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub characters: u64,
    #[serde(default)]
    pub seconds: f64,
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub agent_id: String,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_token_ms: Option<u32>,
}

impl UsageEntry {
    pub fn now(task: &str, provider: &str, model: &str) -> Self {
        Self {
            ts: chrono::Utc::now().to_rfc3339(),
            task: task.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            ..Self::default()
        }
    }
}

fn path() -> Option<PathBuf> {
    super::tools_dir().map(|d| d.join("ai_usage.jsonl"))
}

pub fn record_entry(mut entry: UsageEntry) {
    if entry.ts.is_empty() {
        entry.ts = chrono::Utc::now().to_rfc3339();
    }
    let Some(p) = path() else { return };
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let Ok(line) = serde_json::to_string(&entry) else {
        return;
    };
    use std::io::Write;
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&p)
    {
        Ok(mut f) => {
            let _ = writeln!(f, "{}", line);
        }
        Err(e) => tracing::warn!("[usage] could not record: {}", e),
    }
}

pub fn read_all() -> Vec<UsageEntry> {
    let Some(p) = path() else { return vec![] };
    let Ok(text) = std::fs::read_to_string(&p) else {
        return vec![];
    };
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Bucket {
    pub key: String,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    pub cost_usd: f64,
    pub unknown_price: u64,
}

pub fn cost_of(price: &super::pricing::ModelPrice, e: &UsageEntry) -> Option<f64> {
    let in_per = price.input_per_m? / 1_000_000.0;
    let out_per = price.output_per_m.unwrap_or(0.0) / 1_000_000.0;
    let read_per = price.cache_read_per_m.map(|v| v / 1e6).unwrap_or(in_per);
    let write_per = price.cache_write_per_m.map(|v| v / 1e6).unwrap_or(in_per);
    let cached = e.cache_read_tokens.min(e.input_tokens);
    let written = e.cache_write_tokens.min(e.input_tokens - cached);
    let fresh = e.input_tokens - cached - written;
    Some(
        fresh as f64 * in_per
            + cached as f64 * read_per
            + written as f64 * write_per
            + e.output_tokens as f64 * out_per,
    )
}
