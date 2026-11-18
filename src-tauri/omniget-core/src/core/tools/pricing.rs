use std::path::PathBuf;

use anyhow::anyhow;
use serde::Serialize;

const LITELLM_URL: &str =
    "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

#[derive(Debug, Clone, Serialize)]
pub struct ModelPrice {
    pub key: String,
    pub provider: String,
    pub mode: String,
    pub input_per_m: Option<f64>,
    pub output_per_m: Option<f64>,
    pub cache_read_per_m: Option<f64>,
    pub cache_write_per_m: Option<f64>,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub input_per_second: Option<f64>,
    pub input_per_character: Option<f64>,
    pub supports_vision: bool,
    pub supports_tools: bool,
    pub supports_reasoning: bool,
    pub supports_caching: bool,
    pub deprecation_date: Option<String>,
}

fn cache_path() -> Option<PathBuf> {
    super::tools_dir().map(|d| d.join("pricing").join("litellm.json"))
}

async fn load(force: bool) -> anyhow::Result<serde_json::Value> {
    let path = cache_path().ok_or_else(|| anyhow!("Could not determine data directory"))?;
    let fresh = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .map(|t| t.elapsed().map(|e| e < MAX_AGE).unwrap_or(false))
        .unwrap_or(false);
    if !force && fresh {
        if let Ok(text) = tokio::fs::read_to_string(&path).await {
            if let Ok(v) = serde_json::from_str(&text) {
                return Ok(v);
            }
        }
    }
    let client = super::client()?;
    match client.get(LITELLM_URL).send().await {
        Ok(resp) if resp.status().is_success() => {
            let text = resp.text().await?;
            let v: serde_json::Value = serde_json::from_str(&text)?;
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            tokio::fs::write(&path, &text).await?;
            Ok(v)
        }
        other => {
            if let Ok(text) = tokio::fs::read_to_string(&path).await {
                if let Ok(v) = serde_json::from_str(&text) {
                    return Ok(v);
                }
            }
            match other {
                Ok(r) => Err(anyhow!("price table: HTTP {}", r.status())),
                Err(e) => Err(anyhow!("price table unavailable: {}", e)),
            }
        }
    }
}

fn per_m(v: &serde_json::Value, key: &str) -> Option<f64> {
    v.get(key).and_then(|x| x.as_f64()).map(|x| x * 1_000_000.0)
}

fn to_price(key: &str, v: &serde_json::Value) -> ModelPrice {
    ModelPrice {
        key: key.to_string(),
        provider: v["litellm_provider"].as_str().unwrap_or("").to_string(),
        mode: v["mode"].as_str().unwrap_or("").to_string(),
        input_per_m: per_m(v, "input_cost_per_token"),
        output_per_m: per_m(v, "output_cost_per_token"),
        cache_read_per_m: per_m(v, "cache_read_input_token_cost"),
        cache_write_per_m: per_m(v, "cache_creation_input_token_cost"),
        max_input_tokens: v["max_input_tokens"].as_u64(),
        max_output_tokens: v["max_output_tokens"].as_u64(),
        input_per_second: v["input_cost_per_second"].as_f64(),
        input_per_character: v["input_cost_per_character"].as_f64(),
        supports_vision: v["supports_vision"].as_bool().unwrap_or(false),
        supports_tools: v["supports_function_calling"].as_bool().unwrap_or(false),
        supports_reasoning: v["supports_reasoning"].as_bool().unwrap_or(false),
        supports_caching: v["supports_prompt_caching"].as_bool().unwrap_or(false),
        deprecation_date: v["deprecation_date"].as_str().map(|s| s.to_string()),
    }
}

pub async fn price_for(model: &str) -> Option<ModelPrice> {
    let v = load(false).await.ok()?;
    let obj = v.as_object()?;
    let m = model.trim();
    let mut candidates = vec![m.to_string()];
    if let Some((_, rest)) = m.split_once('/') {
        candidates.push(rest.to_string());
    }
    for p in [
        "openai/",
        "anthropic/",
        "gemini/",
        "openrouter/",
        "groq/",
        "deepseek/",
        "mistral/",
    ] {
        candidates.push(format!("{}{}", p, m));
    }
    if m.len() > 9 {
        let (head, tail) = m.split_at(m.len() - 9);
        if tail.starts_with('-') && tail[1..].chars().all(|c| c.is_ascii_digit()) {
            candidates.push(head.to_string());
        }
    }
    for c in candidates {
        if let Some(val) = obj.get(&c) {
            return Some(to_price(&c, val));
        }
    }
    None
}

pub fn cost(price: &ModelPrice, input_tokens: u64, output_tokens: u64) -> Option<f64> {
    let i = price.input_per_m? / 1_000_000.0 * input_tokens as f64;
    let o = price.output_per_m.unwrap_or(0.0) / 1_000_000.0 * output_tokens as f64;
    Some(i + o)
}
