use serde::{Deserialize, Serialize};

use super::{ERR_CLI_AUTH, ERR_CLI_RATE};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RateWindow {
    pub used: f32,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuotaSource {
    Real,
    Estimated,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateSnapshot {
    pub status: RateStatus,
    pub window_5h: Option<RateWindow>,
    pub window_7d: Option<RateWindow>,
    pub source: QuotaSource,
}

impl RateSnapshot {
    pub fn worst(&self) -> Option<RateWindow> {
        match (self.window_5h, self.window_7d) {
            (Some(a), Some(b)) => Some(if a.used >= b.used { a } else { b }),
            (a, b) => a.or(b),
        }
    }

    pub fn quota_remaining(&self) -> Option<f32> {
        if self.status == RateStatus::Rejected {
            return Some(0.0);
        }
        self.worst().map(RateWindow::remaining)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CliSignal {
    Event(super::super::types::TurnEvent),
    Rate(RateSnapshot),
    Session(String),
    Ignored,
}

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

const RATE_MARKERS: &[&str] = &[
    "usage limit reached",
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
