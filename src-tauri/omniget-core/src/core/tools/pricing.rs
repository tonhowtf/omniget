//! Tabela de preços de modelos (estudos 14, 15, 17): o JSON do LiteLLM é
//! baixado uma vez por dia para `<app_data>/tools/pricing/litellm.json`.
//! Preços internos ficam por token em dólares; a UI mostra por milhão.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::anyhow;
use serde::Serialize;

const LITELLM_URL: &str = "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
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

#[derive(Debug, Clone, Serialize)]
pub struct PricingInfo {
    pub models: usize,
    pub updated_at: Option<String>,
    pub path: Option<String>,
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
            // Sem rede: usa o que tiver em disco, mesmo velho.
            if let Ok(text) = tokio::fs::read_to_string(&path).await {
                if let Ok(v) = serde_json::from_str(&text) {
                    return Ok(v);
                }
            }
            match other {
                Ok(r) => Err(anyhow!("tabela de precos: HTTP {}", r.status())),
                Err(e) => Err(anyhow!("tabela de precos indisponivel: {}", e)),
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

pub async fn info(force_refresh: bool) -> anyhow::Result<PricingInfo> {
    let v = load(force_refresh).await?;
    let path = cache_path();
    let updated = path
        .as_ref()
        .and_then(|p| std::fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
    Ok(PricingInfo {
        models: v.as_object().map(|o| o.len().saturating_sub(1)).unwrap_or(0),
        updated_at: updated,
        path: path.map(|p| p.to_string_lossy().to_string()),
    })
}

/// Busca por substring nas chaves; todas as palavras precisam bater.
pub async fn search(query: &str, mode: &str, limit: usize) -> anyhow::Result<Vec<ModelPrice>> {
    let v = load(false).await?;
    let obj = v.as_object().ok_or_else(|| anyhow!("tabela de precos invalida"))?;
    let tokens: Vec<String> = query.to_lowercase().split_whitespace().map(|s| s.to_string()).collect();
    let mut out: Vec<ModelPrice> = obj
        .iter()
        .filter(|(k, _)| *k != "sample_spec")
        .filter(|(k, val)| {
            let hay = format!("{} {}", k.to_lowercase(), val["litellm_provider"].as_str().unwrap_or("").to_lowercase());
            tokens.iter().all(|t| hay.contains(t))
                && (mode.is_empty() || val["mode"].as_str() == Some(mode))
        })
        .map(|(k, val)| to_price(k, val))
        .collect();
    out.sort_by(|a, b| a.key.len().cmp(&b.key.len()).then(a.key.cmp(&b.key)));
    out.truncate(limit.max(1));
    Ok(out)
}

/// Normaliza como o LiteLLM: tenta a chave exata, sem prefixo de provedor,
/// com prefixo conhecido e sem sufixo de data.
pub async fn price_for(model: &str) -> Option<ModelPrice> {
    let v = load(false).await.ok()?;
    let obj = v.as_object()?;
    let m = model.trim();
    let mut candidates = vec![m.to_string()];
    if let Some((_, rest)) = m.split_once('/') {
        candidates.push(rest.to_string());
    }
    for p in ["openai/", "anthropic/", "gemini/", "openrouter/", "groq/", "deepseek/", "mistral/"] {
        candidates.push(format!("{}{}", p, m));
    }
    // "claude-sonnet-4-5-20250929" -> "claude-sonnet-4-5"
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

/// Custo em dólares de uma chamada, se o modelo estiver na tabela.
pub fn cost(price: &ModelPrice, input_tokens: u64, output_tokens: u64) -> Option<f64> {
    let i = price.input_per_m? / 1_000_000.0 * input_tokens as f64;
    let o = price.output_per_m.unwrap_or(0.0) / 1_000_000.0 * output_tokens as f64;
    Some(i + o)
}

pub type PriceMap = HashMap<String, ModelPrice>;
