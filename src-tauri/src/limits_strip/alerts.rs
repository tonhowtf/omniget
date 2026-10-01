use super::{LimitWindow, Reading};
use serde::Serialize;

const RESET_MIN_DROP: f32 = 0.10;
const RESET_SLACK_MS: i64 = 60_000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Alert {
    Threshold {
        window: String,
        label: String,
        percent: u8,
    },
    Reset {
        window: String,
        label: String,
    },
    Pace {
        window: String,
        label: String,
        runs_out_at: i64,
        in_minutes: i64,
    },
}

pub fn crossed(prev: f32, next: f32, thresholds: &[u8]) -> Option<u8> {
    thresholds
        .iter()
        .copied()
        .filter(|t| {
            let t = f32::from(*t) / 100.0;
            prev < t && next >= t
        })
        .max()
}

pub fn was_reset(prev: &LimitWindow, next: &LimitWindow, now_ms: i64) -> bool {
    let (Some(before), Some(after)) = (prev.used, next.used) else {
        return false;
    };
    if before - after < RESET_MIN_DROP {
        return false;
    }
    match (prev.resets_at, next.resets_at) {
        (Some(old), Some(new)) => old <= now_ms || new > old + RESET_SLACK_MS,
        (Some(old), None) => old <= now_ms,
        _ => false,
    }
}

pub fn between(
    prev: Option<&Reading>,
    next: &Reading,
    thresholds: &[u8],
    now_ms: i64,
) -> Vec<Alert> {
    let Some(prev) = prev else {
        return Vec::new();
    };
    let mut thresholds = thresholds.to_vec();
    if !thresholds.contains(&100) {
        thresholds.push(100);
    }
    let thresholds = thresholds.as_slice();
    let mut out = Vec::new();
    for w in &next.windows {
        let Some(old) = prev.windows.iter().find(|p| p.id == w.id) else {
            continue;
        };
        if was_reset(old, w, now_ms) {
            out.push(Alert::Reset {
                window: w.id.clone(),
                label: w.label.clone(),
            });
        } else if let (Some(a), Some(b)) = (old.used, w.used) {
            if let Some(percent) = crossed(a, b, thresholds) {
                out.push(Alert::Threshold {
                    window: w.id.clone(),
                    label: w.label.clone(),
                    percent,
                });
            }
        }
    }
    out
}

pub fn pace(next: &Reading, warned: &[(String, i64)], now_ms: i64) -> Vec<Alert> {
    next.windows
        .iter()
        .filter_map(|w| {
            let at = super::pace::at_risk(w, now_ms)?;
            let period = w.resets_at?;
            if warned.iter().any(|(id, r)| *id == w.id && *r == period) {
                return None;
            }
            Some(Alert::Pace {
                window: w.id.clone(),
                label: w.label.clone(),
                runs_out_at: at,
                in_minutes: ((at - now_ms).max(0) + 59_999) / 60_000,
            })
        })
        .collect()
}

pub fn wanted(
    alerts: Vec<Alert>,
    thresholds_on: bool,
    reset_on: bool,
    pace_on: bool,
) -> Vec<Alert> {
    alerts
        .into_iter()
        .filter(|a| match a {
            Alert::Threshold { .. } => thresholds_on,
            Alert::Reset { .. } => reset_on,
            Alert::Pace { .. } => pace_on,
        })
        .collect()
}

fn minutes(m: i64) -> String {
    match (m / 60, m % 60) {
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} h"),
        (h, m) => format!("{h} h {m} min"),
    }
}

pub fn notification(provider_label: &str, alert: &Alert) -> (String, String) {
    match alert {
        Alert::Threshold { label, percent, .. } if *percent >= 100 => (
            format!("{provider_label}: limit reached"),
            format!("{label} is used up."),
        ),
        Alert::Threshold { label, percent, .. } => (
            format!("{provider_label}: {percent}% used"),
            format!("{label} passed {percent}%."),
        ),
        Alert::Pace {
            label, in_minutes, ..
        } => (
            format!("{provider_label}: on pace to run out"),
            format!(
                "{label} runs out in about {} at this pace, before it resets.",
                minutes(*in_minutes)
            ),
        ),
        Alert::Reset { label, .. } => (
            format!("{provider_label}: limit reset"),
            format!("{label} is fresh again."),
        ),
    }
}

pub fn summary(provider_label: &str, alerts: &[Alert]) -> Option<(String, String)> {
    match alerts {
        [] => None,
        [one] => Some(notification(provider_label, one)),
        many => {
            let rank = |a: &Alert| match a {
                Alert::Threshold { percent, .. } if *percent >= 100 => 0,
                Alert::Pace { .. } => 1,
                Alert::Threshold { .. } => 2,
                Alert::Reset { .. } => 3,
            };
            let lead = many.iter().min_by_key(|a| rank(a))?;
            let (title, _) = notification(provider_label, lead);
            let body = many
                .iter()
                .map(|a| notification(provider_label, a).1)
                .collect::<Vec<_>>()
                .join(" ");
            Some((title, body))
        }
    }
}
