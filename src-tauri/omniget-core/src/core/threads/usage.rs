use std::time::Duration;

use crate::core::llm::drivers::TokenUsage;
use crate::core::tools::pricing::{self, ModelPrice};

pub fn cost_of(price: &ModelPrice, u: &TokenUsage) -> Option<f64> {
    let tokens = u.input_tokens + u.cached_input_tokens + u.cache_write_tokens + u.output_tokens;
    if tokens == 0 {
        return None;
    }
    let input = price.input_per_m?;
    let output = price.output_per_m.unwrap_or(0.0);
    let read = price.cache_read_per_m.unwrap_or(input);
    let write = price.cache_write_per_m.unwrap_or(input);
    Some(
        (u.input_tokens as f64 * input
            + u.cached_input_tokens as f64 * read
            + u.cache_write_tokens as f64 * write
            + u.output_tokens as f64 * output)
            / 1_000_000.0,
    )
}

pub async fn price_usage(u: &TokenUsage, fallback_model: Option<&str>) -> Option<f64> {
    let model = u
        .model
        .as_deref()
        .or(fallback_model)
        .map(str::trim)
        .filter(|m| !m.is_empty())?;
    let model = model.split_once(':').map(|(_, m)| m).unwrap_or(model);
    let price = tokio::time::timeout(Duration::from_secs(8), pricing::price_for(model))
        .await
        .ok()
        .flatten()?;
    cost_of(&price, u)
}
