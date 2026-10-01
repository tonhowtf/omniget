//! The limits strip: a small always-on-top strip pinned to a screen edge with
//! one ring per coding assistant, showing how much of each usage limit is used,
//! when it resets and whether the assistant is working, waiting or done.
//!
//! Layout of the module:
//!
//! | file            | what lives there                                            |
//! |-----------------|-------------------------------------------------------------|
//! | `prefs.rs`      | `limits-strip.json`: master switch, edge, provider switches  |
//! | `providers/`    | one small reader per assistant behind [`UsageProvider`]      |
//! | `engine.rs`     | the pollers, the shared snapshot, the `limits://state` event |
//! | `activity.rs`   | working / waiting / done per ring, from the bus and the logs |
//! | `alerts.rs`     | threshold crossings and "limit reset", pure and tested       |
//! | `placement.rs`  | edge geometry and snapping, pure and tested                  |
//! | `commands.rs`   | the window and the `limits_strip_*` commands                 |
//!
//! Privacy stance: everything is OFF until the user switches the strip on, and
//! every reader has its own switch; one that is off is never read. A reader
//! borrows the credential the tool itself already keeps on this machine, reads
//! it only, never refreshes or rewrites it, and talks only to that provider's
//! own endpoint. No browser cookie store is ever opened. Token values never
//! reach a log, an event, the UI or the prefs file: they travel inside
//! [`Secret`], whose `Debug` prints a placeholder. Nothing is polled while the
//! strip window is closed.

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

/// A credential borrowed from another tool. It can be sent, and nothing else:
/// no `Display`, no `Serialize`, and a `Debug` that never shows the value.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// `None` for an empty value: an emptied credential is a signed-out tool.
    pub fn new(raw: &str) -> Option<Secret> {
        let raw = raw.trim();
        (!raw.is_empty()).then(|| Secret(raw.to_string()))
    }

    /// The value, for the one place that puts it in a request header.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(***)")
    }
}

/// One limit window of one provider: "5h limit 38% used, resets in 2 h".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LimitWindow {
    pub id: String,
    /// English label; the UI translates the ids it knows and falls back here.
    pub label: String,
    /// 0..1 (may exceed 1 on an overdrawn quota). `None` when the provider
    /// publishes a count with no ceiling: the ring draws only its track.
    pub used: Option<f32>,
    pub used_abs: Option<f64>,
    pub limit_abs: Option<f64>,
    /// What the absolute numbers count: `requests`, `credits`, `tokens`, `usd`.
    pub unit: Option<String>,
    /// Epoch milliseconds.
    pub resets_at: Option<i64>,
    /// Length of the window when known.
    pub span_ms: Option<i64>,
    /// Windows that belong in a boxed sub-group of the card ("Spark", "Bonus").
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

/// A model a local runtime has in memory right now.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LocalModel {
    pub name: String,
    pub size_bytes: Option<u64>,
    pub vram_bytes: Option<u64>,
    pub context: Option<u64>,
    pub quant: Option<String>,
    pub params: Option<String>,
    /// Epoch ms at which the runtime unloads it.
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Reading {
    pub windows: Vec<LimitWindow>,
    pub plan: Option<String>,
    pub account: Option<String>,
    /// A short line for the card: "no limit published", "unlimited plan".
    pub note: Option<String>,
    pub local_models: Vec<LocalModel>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ReadError {
    /// The tool is installed but its login is gone or expired; the owner tool
    /// renews it, never us.
    NeedsAuth(String),
    /// Seconds the provider asked us to stay away.
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

/// Who a ring belongs to: the account name the user gave it in the app and
/// the e-mail the tool itself shows. Neither is a credential.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountTag {
    pub label: Option<String>,
    pub email: Option<String>,
}

/// One assistant. `detect` is cheap and offline (does the credential or the
/// state file exist on this OS?); `read` may go to the provider's own endpoint.
#[async_trait::async_trait]
pub trait UsageProvider: Send + Sync {
    /// Stable ring id and prefs key. One per reader, except the CLIs with
    /// several logins: `claude` / `codex` for the default profile and
    /// `claude:<account-id>` / `codex:<account-id>` for the app's accounts.
    fn id(&self) -> &str;
    fn label(&self) -> &'static str;
    /// Which login this ring reads, when the tool can hold several.
    fn account(&self) -> Option<&AccountTag> {
        None
    }
    /// The label with the account name, for notifications.
    fn display_label(&self) -> String {
        match self
            .account()
            .and_then(|a| a.label.as_deref().or(a.email.as_deref()))
        {
            Some(who) => format!("{} · {who}", self.label()),
            None => self.label().to_string(),
        }
    }
    /// A runtime on this machine (loopback only), not a metered account.
    fn local(&self) -> bool {
        false
    }
    /// `true` until the endpoint was checked against the real tool: the UI
    /// labels the reader "beta".
    fn beta(&self) -> bool {
        true
    }
    /// The engine never polls faster than [`engine::MIN_INTERVAL`], whatever
    /// a reader says here.
    fn poll_interval(&self) -> Duration {
        engine::MIN_INTERVAL
    }
    async fn detect(&self) -> bool;
    async fn read(&self) -> Result<Reading, ReadError>;
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// RFC 3339 with or without fractional seconds, to epoch ms.
pub fn iso_ms(v: Option<&serde_json::Value>) -> Option<i64> {
    let s = v?.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

/// A number that may arrive as a JSON number or as a decimal string.
pub fn num(v: Option<&serde_json::Value>) -> Option<f64> {
    let v = v?;
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
}

/// Epoch that may be seconds or milliseconds.
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

/// What a non-2xx status means. Built from the status and `Retry-After`
/// only: the body of an error reply is never quoted, so nothing a provider
/// echoes back can end up in a message.
pub fn status_error(status: u16, retry_after: Option<u64>) -> ReadError {
    match status {
        401 | 403 => ReadError::NeedsAuth(format!("HTTP {status}")),
        429 => ReadError::RateLimited(retry_after.unwrap_or(0).max(60)),
        _ => ReadError::Other(format!("HTTP {status}")),
    }
}

/// Turns a reqwest response into JSON or the matching [`ReadError`].
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
    // The URL can carry nothing secret here, but strip it anyway.
    ReadError::Other(e.without_url().to_string())
}

pub fn home() -> Option<std::path::PathBuf> {
    dirs::home_dir()
}

pub fn read_json(path: &std::path::Path) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}
