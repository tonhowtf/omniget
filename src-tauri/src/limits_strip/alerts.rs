//! Threshold crossings and "limit reset", as pure functions of two readings.
//! The engine turns what comes out into a notification and a
//! `limits://chime` event; nothing here touches a clock or the app.

use super::{LimitWindow, Reading};
use serde::Serialize;

/// A reset only counts when the window had something to give back.
const RESET_MIN_DROP: f32 = 0.10;
/// Slack when comparing two `resets_at` of the same window.
const RESET_SLACK_MS: i64 = 60_000;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Alert {
    /// `percent` is the threshold that was crossed, not the usage.
    Threshold {
        window: String,
        label: String,
        percent: u8,
    },
    Reset {
        window: String,
        label: String,
    },
    /// At the current pace the window runs out before it resets
    /// (`super::pace::at_risk`). Raised once per window period.
    Pace {
        window: String,
        label: String,
        runs_out_at: i64,
        /// Minutes from the reading to `runs_out_at`, for the notification.
        in_minutes: i64,
    },
}

/// The highest threshold that sits in `(prev, next]`.
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

/// The window emptied because its period rolled over, not because a rolling
/// average drifted down: usage fell by a real amount and either the old reset
/// time has passed or the provider now names a later one.
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

/// What changed between two readings of one provider. A first reading raises
/// nothing: opening the strip at 90% is not news, going from 79% to 81% is.
pub fn between(
    prev: Option<&Reading>,
    next: &Reading,
    thresholds: &[u8],
    now_ms: i64,
) -> Vec<Alert> {
    let Some(prev) = prev else {
        return Vec::new();
    };
    // Running out is always news, whatever thresholds the user picked.
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

/// Windows of `next` that run out before their reset at the current pace
/// and were not warned about yet. `warned` holds `(window id, resets_at)` of
/// the warnings already raised, so each period warns at most once.
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

/// Drops what the user asked not to hear about.
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

/// Title and body of the notification. English on purpose: it is raised by the
/// backend, which has no locale; the strip's own pulse carries no words.
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

/// One notification for everything a reading raised: several banners at once
/// are noise, and on macOS each one is a blocking call on a runtime thread.
pub fn summary(provider_label: &str, alerts: &[Alert]) -> Option<(String, String)> {
    match alerts {
        [] => None,
        [one] => Some(notification(provider_label, one)),
        many => {
            // The most urgent title leads: used up, then pace, then the rest.
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
