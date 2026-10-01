pub mod activity;
pub mod alerts;
pub mod commands;
pub mod engine;
pub mod pace;
pub mod placement;
pub mod prefs;
pub mod providers;

use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const EVENT_STATE: &str = "limits://state";
pub const EVENT_PREFS: &str = "limits://prefs";
pub const EVENT_CHIME: &str = "limits://chime";

#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(raw: &str) -> Option<Secret> {
        let raw = raw.trim();
        (!raw.is_empty()).then(|| Secret(raw.to_string()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LimitWindow {
    pub id: String,
    pub label: String,
    pub used: Option<f32>,
    pub used_abs: Option<f64>,
    pub limit_abs: Option<f64>,
    pub unit: Option<String>,
    pub resets_at: Option<i64>,
    pub span_ms: Option<i64>,
    pub group: Option<String>,
}

impl LimitWindow {
    pub fn percent(id: &str, label: &str, pct_0_100: f64) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            used: Some((pct_0_100 / 100.0).max(0.0) as f32),
            ..Default::default()
        }
    }

    pub fn ratio(id: &str, label: &str, used: f64, limit: f64, unit: &str) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            used: (limit > 0.0).then(|| (used / limit).max(0.0) as f32),
            used_abs: Some(used),
            limit_abs: (limit > 0.0).then_some(limit),
            unit: Some(unit.into()),
            ..Default::default()
        }
    }

    pub fn resets(mut self, at_ms: Option<i64>) -> Self {
        self.resets_at = at_ms;
        self
    }

    pub fn span(mut self, ms: i64) -> Self {
        self.span_ms = Some(ms);
        self
    }

    pub fn grouped(mut self, group: &str) -> Self {
        self.group = Some(group.into());
        self
    }
}

pub const HOUR_MS: i64 = 3_600_000;
pub const DAY_MS: i64 = 24 * HOUR_MS;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LocalModel {
    pub name: String,
    pub size_bytes: Option<u64>,
    pub vram_bytes: Option<u64>,
    pub context: Option<u64>,
    pub quant: Option<String>,
    pub params: Option<String>,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub windows: Vec<LimitWindow>,
    pub plan: Option<String>,
    pub account: Option<String>,
    pub note: Option<String>,
    pub local_models: Vec<LocalModel>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReadError {
    NeedsAuth(String),
    RateLimited(u64),
    Other(String),
}

impl ReadError {
    pub fn code(&self) -> &'static str {
        match self {
            ReadError::NeedsAuth(_) => "needs_auth",
            ReadError::RateLimited(_) => "rate_limited",
            ReadError::Other(_) => "error",
        }
    }

    pub fn message(&self) -> String {
        match self {
            ReadError::NeedsAuth(m) | ReadError::Other(m) => m.clone(),
            ReadError::RateLimited(s) => format!("rate limited, retrying in {s}s"),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountTag {
    pub label: Option<String>,
    pub email: Option<String>,
}

#[async_trait::async_trait]
pub trait UsageProvider: Send + Sync {
    fn id(&self) -> &str;
    fn label(&self) -> &'static str;
    fn account(&self) -> Option<&AccountTag> {
        None
    }
    fn display_label(&self) -> String {
        match self
            .account()
            .and_then(|a| a.label.as_deref().or(a.email.as_deref()))
        {
            Some(who) => format!("{} · {who}", self.label()),
            None => self.label().to_string(),
        }
    }
    fn local(&self) -> bool {
        false
    }
    fn beta(&self) -> bool {
        true
    }
    fn poll_interval(&self) -> Duration {
        engine::MIN_INTERVAL
    }
    async fn detect(&self) -> bool;
    async fn read(&self) -> Result<Reading, ReadError>;
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn iso_ms(v: Option<&serde_json::Value>) -> Option<i64> {
    let s = v?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

pub fn num(v: Option<&serde_json::Value>) -> Option<f64> {
    let v = v?;
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
}

pub fn epoch_ms(n: f64) -> i64 {
    if n > 1e10 {
        n as i64
    } else {
        (n * 1000.0) as i64
    }
}

pub fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .user_agent(concat!("OmniGet/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

pub fn status_error(status: u16, retry_after: Option<u64>) -> ReadError {
    match status {
        401 | 403 => ReadError::NeedsAuth(format!("HTTP {status}")),
        429 => ReadError::RateLimited(retry_after.unwrap_or(0).max(60)),
        _ => ReadError::Other(format!("HTTP {status}")),
    }
}

pub async fn json_or_error(resp: reqwest::Response) -> Result<serde_json::Value, ReadError> {
    let status = resp.status();
    if status.is_success() {
        return resp
            .json::<serde_json::Value>()
            .await
            .map_err(|e| ReadError::Other(format!("parse: {}", e.without_url())));
    }
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    Err(status_error(status.as_u16(), retry_after))
}

pub fn net_err(e: reqwest::Error) -> ReadError {
    ReadError::Other(e.without_url().to_string())
}

pub fn home() -> Option<std::path::PathBuf> {
    dirs::home_dir()
}

pub fn read_json(path: &std::path::Path) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}
