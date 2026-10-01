//! Usage per turn and rate limits per instance (plan §3.8, T8).
//!
//! - Cost: a driver that reports tokens but no money gets its turn priced
//!   from the LiteLLM table (`core::tools::pricing`), cache reads/writes
//!   included.
//! - Limits: `account.rate-limits.updated` of the drivers (live) merged with
//!   the windows the limits strip probes, by window id; live wins.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::llm::drivers::{
    RateLimitWindow, RateLimitsPayload, RuntimeEvent, RuntimeEventKind, TokenUsage,
};
use crate::core::tools::pricing::{self, ModelPrice};

/// Money of one turn at list price. `None` without an input price or with
/// no tokens at all. `input_tokens` excludes cache reads/writes (the
/// `TokenUsage` convention); reasoning is inside `output_tokens`.
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

/// Price a turn by its model (the usage's own, else `fallback_model`). The
/// table is cached for a day; a cold cache costs one download, bounded here.
pub async fn price_usage(u: &TokenUsage, fallback_model: Option<&str>) -> Option<f64> {
    let model = u
        .model
        .as_deref()
        .or(fallback_model)
        .map(str::trim)
        .filter(|m| !m.is_empty())?;
    // `provider:model` refs of the native driver.
    let model = model.split_once(':').map(|(_, m)| m).unwrap_or(model);
    let price = tokio::time::timeout(Duration::from_secs(8), pricing::price_for(model))
        .await
        .ok()
        .flatten()?;
    cost_of(&price, u)
}

/// One limit window as the Central shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitWindowView {
    pub id: String,
    pub label: String,
    /// 0–100.
    pub used_percent: Option<f64>,
    pub window_minutes: Option<u64>,
    /// ISO-8601.
    pub resets_at: Option<String>,
    /// `live` (a driver said so this session) or `probe` (limits strip).
    pub source: String,
}

impl LimitWindowView {
    pub fn live(w: &RateLimitWindow) -> Self {
        Self {
            id: w.id.clone(),
            label: if w.label.is_empty() {
                w.id.clone()
            } else {
                w.label.clone()
            },
            used_percent: w.used_percent,
            window_minutes: w.window_minutes,
            resets_at: w.resets_at.clone(),
            source: "live".into(),
        }
    }
}

/// Probe windows first (their order), each replaced by the live window of
/// the same id; live windows the probe does not know go at the end.
pub fn merge_windows(
    probe: Vec<LimitWindowView>,
    live: &[RateLimitWindow],
) -> Vec<LimitWindowView> {
    let mut out: Vec<LimitWindowView> = probe
        .into_iter()
        .map(|p| match live.iter().find(|l| l.id == p.id) {
            Some(l) => {
                let mut v = LimitWindowView::live(l);
                if v.label == v.id && !p.label.is_empty() {
                    v.label = p.label.clone();
                }
                v
            }
            None => p,
        })
        .collect();
    for l in live {
        if !out.iter().any(|o| o.id == l.id) {
            out.push(LimitWindowView::live(l));
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveLimits {
    pub instance_id: String,
    pub driver: String,
    pub windows: Vec<RateLimitWindow>,
    pub credits: Option<Value>,
    pub updated_at: String,
}

/// The last `account.rate-limits.updated` of each instance, in memory (a
/// restart forgets it; the probes of the strip cover the gap).
#[derive(Default)]
pub struct LimitsBook {
    live: Mutex<HashMap<String, LiveLimits>>,
}

impl LimitsBook {
    /// Keeps the event if it is a rate-limits update. An empty window list
    /// means the driver has none to report: its bars go.
    pub fn note(&self, ev: &RuntimeEvent) -> bool {
        let RuntimeEventKind::RateLimitsUpdated(RateLimitsPayload { windows, credits }) = &ev.kind
        else {
            return false;
        };
        let mut g = self.live.lock().unwrap_or_else(|e| e.into_inner());
        if windows.is_empty() && credits.is_none() {
            g.remove(&ev.instance_id);
        } else {
            g.insert(
                ev.instance_id.clone(),
                LiveLimits {
                    instance_id: ev.instance_id.clone(),
                    driver: ev.driver.clone(),
                    windows: windows.clone(),
                    credits: credits.clone(),
                    updated_at: ev.created_at.clone(),
                },
            );
        }
        true
    }

    pub fn all(&self) -> Vec<LiveLimits> {
        let mut v: Vec<LiveLimits> = self
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect();
        v.sort_by(|a, b| a.instance_id.cmp(&b.instance_id));
        v
    }
}
