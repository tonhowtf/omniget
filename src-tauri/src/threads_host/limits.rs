//! `threads_limits()`: the live rate limits the drivers reported
//! (`account.rate-limits.updated`) merged, by window id, with the windows the
//! limits strip last probed for the same CLI. Reads the strip's snapshot
//! only (no probe is started from here).

use omniget_core::core::threads::usage::{merge_windows, LimitWindowView};
use serde::Serialize;
use serde_json::Value;
use tauri::AppHandle;

use super::ThreadsHost;
use crate::limits_strip::{engine, LimitWindow};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceLimitsView {
    pub instance_id: String,
    pub driver: String,
    pub label: String,
    pub windows: Vec<LimitWindowView>,
    pub credits: Option<Value>,
    /// When a driver last reported limits in this session.
    pub live_at: Option<String>,
    /// Strip status of the matching provider (`ok`, `needs_auth`, …).
    pub probe_status: Option<String>,
    /// Epoch ms of the strip's last read.
    pub probe_at: Option<i64>,
}

fn probe_view(w: &LimitWindow) -> LimitWindowView {
    LimitWindowView {
        id: w.id.clone(),
        label: w.label.clone(),
        used_percent: w.used.map(|u| (u as f64 * 100.0 * 10.0).round() / 10.0),
        window_minutes: w.span_ms.map(|ms| (ms / 60_000).max(0) as u64),
        resets_at: w
            .resets_at
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|t| t.to_rfc3339()),
        source: "probe".into(),
    }
}

pub fn view(host: &ThreadsHost, app: &AppHandle) -> Vec<InstanceLimitsView> {
    let strip = engine::snapshot(app);
    let live = host.limits.all();
    let mut out = Vec::new();
    for inst in host.instances() {
        let instance = inst.instance;
        let ring = strip.rings.iter().find(|r| r.id == instance.driver);
        let entry = live.iter().find(|l| l.instance_id == instance.id);
        if ring.is_none() && entry.is_none() {
            continue;
        }
        let probe: Vec<LimitWindowView> = ring
            .and_then(|r| r.reading.as_ref())
            .map(|r| r.windows.iter().map(probe_view).collect())
            .unwrap_or_default();
        let windows = match entry {
            Some(l) => merge_windows(probe, &l.windows),
            None => probe,
        };
        out.push(InstanceLimitsView {
            instance_id: instance.id.clone(),
            driver: instance.driver.clone(),
            label: instance.label.clone(),
            windows,
            credits: entry.and_then(|l| l.credits.clone()),
            live_at: entry.map(|l| l.updated_at.clone()),
            probe_status: ring.map(|r| r.status.to_string()),
            probe_at: ring.and_then(|r| r.read_at),
        });
    }
    // Live limits of an instance the list does not show (a removed account).
    for l in live {
        if !out.iter().any(|o| o.instance_id == l.instance_id) {
            out.push(InstanceLimitsView {
                instance_id: l.instance_id.clone(),
                driver: l.driver.clone(),
                label: l.instance_id.clone(),
                windows: merge_windows(Vec::new(), &l.windows),
                credits: l.credits.clone(),
                live_at: Some(l.updated_at.clone()),
                probe_status: None,
                probe_at: None,
            });
        }
    }
    out
}
