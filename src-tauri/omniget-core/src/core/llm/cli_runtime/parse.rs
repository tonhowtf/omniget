//! CLI-agnostic half of the stream parser: the normalised signal a line turns
//! into, the quota windows, and the text classifiers. Owned by f4-cli-runtime.
//!
//! Everything here is pure: a `&str` in, a value out, no clock, no filesystem,
//! no process. That is what makes the fixtures a real test.
//!
//! Tolerance is the contract (plan §3 Fase 4, "Risco"): an unknown event is
//! [`CliSignal::Ignored`] and a `debug!` line, never an error. The format moves
//! between CLI versions and a new field must not take the turn down.

use serde::{Deserialize, Serialize};

use super::{ERR_CLI_AUTH, ERR_CLI_RATE};

/// One window of a subscription plan, normalised to `0.0..=1.0` used.
///
/// The two official sources disagree on units (`estudos/74` §A.2): the
/// stream-json `rate_limit_event` reports `utilization` as `0..1`, the status
/// line reports `used_percentage` as `0..100`. [`RateWindow::from_utilization`]
/// and [`RateWindow::from_percentage`] are the two doors in.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RateWindow {
    /// Share of the window already used, 0.0..=1.0.
    pub used: f32,
    /// Unix epoch seconds when the window rolls over.
    pub resets_at: Option<i64>,
}

impl RateWindow {
    pub fn from_utilization(utilization: f64, resets_at: Option<i64>) -> Self {
        Self {
            used: clamp01(utilization as f32),
            resets_at,
        }
    }

    pub fn from_percentage(percentage: f64, resets_at: Option<i64>) -> Self {
        Self {
            used: clamp01((percentage / 100.0) as f32),
            resets_at,
        }
    }

    /// What the router asks for: the share still available.
    pub fn remaining(self) -> f32 {
        1.0 - self.used
    }
}

fn clamp01(v: f32) -> f32 {
    if v.is_nan() {
        0.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

/// `status` of a `rate_limit_event`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RateStatus {
    Allowed,
    AllowedWarning,
    Rejected,
    Unknown,
}

impl RateStatus {
    pub fn parse(s: &str) -> RateStatus {
        match s {
            "allowed" => RateStatus::Allowed,
            "allowed_warning" => RateStatus::AllowedWarning,
            "rejected" => RateStatus::Rejected,
            _ => RateStatus::Unknown,
        }
    }
}

/// Where a quota number came from. `Estimated` is the JSONL scan of
/// `cli_usage` (F4 sibling); the UI labels it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaSource {
    /// `rate_limit_event` on the stream, or the installed status line.
    Real,
    /// Derived from the local JSONL history.
    Estimated,
}

/// A quota reading for one account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateSnapshot {
    pub status: RateStatus,
    pub window_5h: Option<RateWindow>,
    pub window_7d: Option<RateWindow>,
    pub source: QuotaSource,
}

impl RateSnapshot {
    /// The window that decides: the most used of the two.
    pub fn worst(&self) -> Option<RateWindow> {
        match (self.window_5h, self.window_7d) {
            (Some(a), Some(b)) => Some(if a.used >= b.used { a } else { b }),
            (a, b) => a.or(b),
        }
    }

    /// `Capacity::quota_remaining` for the router.
    pub fn quota_remaining(&self) -> Option<f32> {
        if self.status == RateStatus::Rejected {
            return Some(0.0);
        }
        self.worst().map(RateWindow::remaining)
    }
}

/// What one line of CLI output means to us. A line can mean more than one
/// thing (a `result` line carries usage *and* the finish reason), so parsers
/// return a `Vec`.
#[derive(Debug, Clone, PartialEq)]
pub enum CliSignal {
    /// Pass this straight to the turn stream.
    Event(super::super::types::TurnEvent),
    /// The CLI reported its quota windows.
    Rate(RateSnapshot),
    /// The CLI's own session id, for `--resume` and for the logs.
    Session(String),
    /// Known and deliberately dropped, or unknown and tolerated.
    Ignored,
}

/// Auth failures. The first two are verbatim from the recorded 2.1.276
/// fixture; the rest are the `SDKAssistantMessageError` variants documented at
/// code.claude.com/docs/en/agent-sdk/typescript plus the Codex wording.
const AUTH_MARKERS: &[&str] = &[
    "not logged in",
    "please run /login",
    "authentication_failed",
    "oauth_org_not_allowed",
    "account_on_hold",
    "cloud_credential_error",
    "invalid api key",
    "oauth token has expired",
    "oauth token expired",
    "invalid bearer token",
    "credentials not found",
    "codex login",
    "not authenticated",
];

/// Quota exhaustion. The `hit your … limit` wordings are verbatim from
/// code.claude.com/docs/en/errors; `rate_limit` and `overloaded` are
/// `SDKAssistantMessageError` variants. `ERR_CLI_RATE` sends the turn to the
/// next candidate without losing it (plan §9.2).
const RATE_MARKERS: &[&str] = &[
    "usage limit reached",
    // Codex 0.158: "You’ve hit your usage limit. Visit …" (`usageLimitExceeded`).
    "hit your usage limit",
    "hit your session limit",
    "hit your weekly limit",
    "hit your opus limit",
    "hit your sonnet limit",
    "spend limit",
    "rate limit",
    "rate_limit",
    "too many requests",
    "exceeded your current quota",
    "request rejected (429)",
    "credits_required",
];

/// Maps a CLI error string to a stable `ERR_CLI_*` code. `None` when the text
/// carries no verdict of its own and the caller should keep its default.
pub fn classify_error_text(text: &str) -> Option<&'static str> {
    let lower = text.to_ascii_lowercase();
    if RATE_MARKERS.iter().any(|m| lower.contains(m)) {
        return Some(ERR_CLI_RATE);
    }
    if AUTH_MARKERS.iter().any(|m| lower.contains(m)) {
        return Some(ERR_CLI_AUTH);
    }
    None
}

/// Splits a chunk of stdout into complete lines, returning the leftover.
/// `session.rs` reads with `BufReader::lines`, but the fixtures go through
/// here so the tests exercise blank lines and a trailing partial line.
pub fn split_lines<'a>(buf: &'a str, out: &mut Vec<&'a str>) -> &'a str {
    let mut rest = buf;
    while let Some(idx) = rest.find('\n') {
        let (line, tail) = rest.split_at(idx);
        let line = line.trim_end_matches('\r');
        if !line.trim().is_empty() {
            out.push(line);
        }
        rest = &tail[1..];
    }
    rest
}
