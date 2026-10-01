use super::LimitWindow;
use serde::Serialize;

pub const MIN_SHOWN: f64 = 0.03;
pub const MIN_ALERT: f64 = 0.15;

pub fn elapsed(resets_at: i64, span_ms: i64, now: i64) -> Option<f64> {
    if span_ms <= 0 {
        return None;
    }
    let until_reset = resets_at - now;
    if until_reset <= 0 || until_reset > span_ms {
        return None;
    }
    Some((span_ms - until_reset) as f64 / span_ms as f64)
}

pub fn runs_out_at(used: f64, resets_at: i64, span_ms: i64, now: i64) -> Option<i64> {
    let gone = elapsed(resets_at, span_ms, now)?;
    let actual = used.clamp(0.0, 1.0);
    if actual >= 1.0 {
        return Some(now);
    }
    if gone <= 0.0 || actual <= 0.0 {
        return None;
    }
    let elapsed_ms = gone * span_ms as f64;
    let left_ms = (1.0 - actual) / (actual / elapsed_ms);
    (left_ms < (resets_at - now) as f64).then(|| now + left_ms as i64)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Pace {
    pub expected: f64,
    pub delta: f64,
    pub runs_out_at: Option<i64>,
}

pub fn of(w: &LimitWindow, now: i64) -> Option<Pace> {
    let used = w.used? as f64;
    let (resets_at, span) = (w.resets_at?, w.span_ms?);
    let expected = elapsed(resets_at, span, now)?;
    if expected < MIN_SHOWN {
        return None;
    }
    Some(Pace {
        expected,
        delta: used - expected,
        runs_out_at: runs_out_at(used, resets_at, span, now),
    })
}

pub fn at_risk(w: &LimitWindow, now: i64) -> Option<i64> {
    let used = w.used? as f64;
    if used >= 1.0 {
        return None;
    }
    let (resets_at, span) = (w.resets_at?, w.span_ms?);
    if elapsed(resets_at, span, now)? < MIN_ALERT {
        return None;
    }
    runs_out_at(used, resets_at, span, now)
}
