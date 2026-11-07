use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::core::sessions::model::TokenUsage;
use crate::core::tools::pricing::{self, ModelPrice};

#[derive(Debug, Clone, Default)]
pub struct PriceBook {
    prices: HashMap<String, Option<ModelPrice>>,
}

type Prices = HashMap<String, Option<ModelPrice>>;
static MEMO: Mutex<Option<(Instant, Prices)>> = Mutex::new(None);
const MEMO_TTL: Duration = Duration::from_secs(3600);

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
