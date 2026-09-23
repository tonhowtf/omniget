//! Preço de tokens de sessão com a tabela do LiteLLM (`core::tools::pricing`).
//!
//! `PriceBook` é montado de forma assíncrona (a tabela pode precisar de
//! download) e depois usado de forma síncrona nas agregações. Cada modelo é
//! resolvido uma vez por processo e lembrado por uma hora.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::core::sessions::model::TokenUsage;
use crate::core::tools::pricing::{self, ModelPrice};

#[derive(Debug, Clone, Default)]
pub struct PriceBook {
    prices: HashMap<String, Option<ModelPrice>>,
}

static MEMO: Mutex<Option<(Instant, HashMap<String, Option<ModelPrice>>)>> = Mutex::new(None);
const MEMO_TTL: Duration = Duration::from_secs(3600);

/// Nome do modelo como a tabela conhece: sem sufixo de contexto (`[1m]`),
/// sem data e sem prefixo de provedor quando não acha com ele.
pub fn normalize_model(m: &str) -> String {
    let mut s = m.trim().to_string();
    if let Some(i) = s.find('[') {
        s.truncate(i);
    }
    s.trim().to_string()
}

fn candidates(model: &str) -> Vec<String> {
    let base = normalize_model(model);
    let mut v = vec![base.clone()];
    // `anthropic/claude-x`, `openai/gpt-x` já são tentados pelo pricing; aqui
    // tenta também sem sufixos de variante que as ferramentas acrescentam.
    for suffix in ["-thinking", "-high", "-low", "-medium", "-fast", ":free"] {
        if let Some(stripped) = base.strip_suffix(suffix) {
            v.push(stripped.to_string());
        }
    }
    if let Some((_, rest)) = base.rsplit_once('/') {
        v.push(rest.to_string());
    }
    v
}

impl PriceBook {
    /// Resolve todos os modelos pedidos (memo de uma hora).
    pub async fn load(models: impl IntoIterator<Item = String>) -> PriceBook {
        let wanted: Vec<String> = models
            .into_iter()
            .filter(|m| !m.is_empty() && m != "<synthetic>")
            .collect();
        let mut memo = {
            let g = MEMO.lock().ok();
            match g.as_ref().and_then(|g| g.as_ref()) {
                Some((at, map)) if at.elapsed() < MEMO_TTL => map.clone(),
                _ => HashMap::new(),
            }
        };
        let mut book = PriceBook::default();
        for m in wanted {
            if book.prices.contains_key(&m) {
                continue;
            }
            let price = match memo.get(&m) {
                Some(p) => p.clone(),
                None => {
                    let mut found = None;
                    for c in candidates(&m) {
                        if let Some(p) = pricing::price_for(&c).await {
                            found = Some(p);
                            break;
                        }
                    }
                    memo.insert(m.clone(), found.clone());
                    found
                }
            };
            book.prices.insert(m, price);
        }
        if let Ok(mut g) = MEMO.lock() {
            let keep_at = g
                .as_ref()
                .filter(|(at, _)| at.elapsed() < MEMO_TTL)
                .map(|(at, _)| *at);
            *g = Some((keep_at.unwrap_or_else(Instant::now), memo));
        }
        book
    }

    pub fn empty() -> PriceBook {
        PriceBook::default()
    }

    pub fn price(&self, model: &str) -> Option<&ModelPrice> {
        self.prices.get(model).and_then(|p| p.as_ref())
    }

    /// Custo de um uso: entrada fresca pelo preço de entrada, cache lido e
    /// escrito pelos preços de cache (ou de entrada, se a tabela não tiver),
    /// saída + raciocínio pelo preço de saída.
    pub fn cost(&self, model: &str, u: &TokenUsage) -> Option<f64> {
        let p = self.price(model)?;
        let inp = p.input_per_m? / 1e6;
        let out = p.output_per_m.unwrap_or(0.0) / 1e6;
        let read = p.cache_read_per_m.map(|v| v / 1e6).unwrap_or(inp);
        let write = p.cache_write_per_m.map(|v| v / 1e6).unwrap_or(inp);
        Some(
            u.input as f64 * inp
                + u.cache_read as f64 * read
                + u.cache_write as f64 * write
                + (u.output + u.reasoning) as f64 * out,
        )
    }
}

/// Custo consolidado de um conjunto de usos.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CostSummary {
    pub cost_usd: f64,
    /// Parte que veio gravada pela própria ferramenta.
    pub recorded_usd: f64,
    /// Parte calculada pela tabela de preço.
    pub computed_usd: f64,
    /// Tokens de modelos sem preço conhecido (não entram no custo).
    pub unpriced_tokens: u64,
    pub unpriced_models: Vec<String>,
    /// `recorded` | `computed` | `mixed` | `unknown`.
    pub source: String,
    /// Custo que o CLI gravou para o processo inteiro, subagentes incluídos
    /// (Claude `cost-state` de uma sessão com filhos). Não entra em
    /// `cost_usd`, que é só desta sessão.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree_recorded_usd: Option<f64>,
    /// Soma calculada pela tabela para esta sessão + subagentes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree_computed_usd: Option<f64>,
}

impl CostSummary {
    pub fn add_recorded(&mut self, c: f64) {
        self.recorded_usd += c;
        self.cost_usd += c;
    }
    pub fn add_usage(&mut self, book: &PriceBook, model: &str, u: &TokenUsage) {
        if u.total() == 0 {
            return;
        }
        match book.cost(model, u) {
            Some(c) => {
                self.computed_usd += c;
                self.cost_usd += c;
            }
            None => {
                self.unpriced_tokens += u.total();
                let m = if model.is_empty() {
                    "?".to_string()
                } else {
                    model.to_string()
                };
                if !self.unpriced_models.contains(&m) {
                    self.unpriced_models.push(m);
                }
            }
        }
    }
    pub fn finish(&mut self) {
        self.source = match (self.recorded_usd > 0.0, self.computed_usd > 0.0) {
            (true, true) => "mixed",
            (true, false) => "recorded",
            (false, true) => "computed",
            (false, false) => "unknown",
        }
        .into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_suffix_is_dropped() {
        assert_eq!(normalize_model("claude-opus-5[1m]"), "claude-opus-5");
        assert!(candidates("anthropic/claude-opus-5").contains(&"claude-opus-5".to_string()));
    }

    #[test]
    fn cache_tokens_use_cache_prices() {
        let mut book = PriceBook::default();
        book.prices.insert(
            "m".into(),
            Some(ModelPrice {
                key: "m".into(),
                provider: "x".into(),
                mode: "chat".into(),
                input_per_m: Some(3.0),
                output_per_m: Some(15.0),
                cache_read_per_m: Some(0.3),
                cache_write_per_m: Some(3.75),
                max_input_tokens: None,
                max_output_tokens: None,
                input_per_second: None,
                input_per_character: None,
                supports_vision: false,
                supports_tools: false,
                supports_reasoning: false,
                supports_caching: true,
                deprecation_date: None,
            }),
        );
        let u = TokenUsage {
            input: 1_000_000,
            output: 1_000_000,
            cache_read: 1_000_000,
            cache_write: 1_000_000,
            reasoning: 0,
        };
        let c = book.cost("m", &u).unwrap();
        assert!((c - (3.0 + 15.0 + 0.3 + 3.75)).abs() < 1e-9);
    }
}
