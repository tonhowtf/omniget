//! "Spent today" per account, from the same Claude Code / Codex history the
//! usage monitor reads (`core::llm::cli_usage::report`, as `omniget usage`
//! does). Reading the history is not free, so it runs only while the panel is
//! open and at most every [`FRESH_MS`]; the numbers wait in a cache.

use chrono::{Local, TimeZone};
use omniget_core::core::llm::cli_runtime::accounts::{AccountStore, CliAccount};
use omniget_core::core::llm::cli_usage::report::{report, ReportOptions, SessionSummary};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::AppHandle;

const FRESH_MS: i64 = 5 * 60_000;

static CACHE: Mutex<Option<(i64, HashMap<String, f64>)>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Ring id of a history session: `claude` for the terminal's own login,
/// `claude:<account>` for an account saved in the app.
pub fn ring_of(s: &SessionSummary) -> String {
    if s.account == "default" || s.account.is_empty() {
        s.cli.to_string()
    } else {
        format!("{}:{}", s.cli, s.account)
    }
}

/// USD per ring for the sessions that ended after local midnight.
pub fn today_by_ring(sessions: &[SessionSummary], midnight_ms: i64) -> HashMap<String, f64> {
    let mut out: HashMap<String, f64> = HashMap::new();
    for s in sessions.iter().filter(|s| s.ended_at >= midnight_ms) {
        *out.entry(ring_of(s)).or_default() += s.cost_usd;
    }
    out
}

pub fn cached() -> HashMap<String, f64> {
    CACHE
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|(_, m)| m.clone()))
        .unwrap_or_default()
}

fn local_midnight(now: i64) -> i64 {
    Local
        .timestamp_millis_opt(now)
        .single()
        .and_then(|n| n.date_naive().and_hms_opt(0, 0, 0))
        .and_then(|d| Local.from_local_datetime(&d).single())
        .map(|d| d.timestamp_millis())
        .unwrap_or(now)
}

/// Starts a read of the history when the cache is older than [`FRESH_MS`];
/// when it lands the panel and the icon are redrawn.
pub fn refresh_if_stale(app: &AppHandle) {
    let now = crate::limits_strip::now_ms();
    let fresh = CACHE
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|(at, _)| now - at < FRESH_MS))
        .unwrap_or(false);
    if fresh || RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        // Cleared on the way out even if the report panics: otherwise spend
        // would never refresh again until a restart.
        struct Running;
        impl Drop for Running {
            fn drop(&mut self) {
                RUNNING.store(false, Ordering::SeqCst);
            }
        }
        let running = Running;
        let accounts: Vec<CliAccount> = AccountStore::default_store()
            .map(|s| s.list().iter().filter(|a| !a.disabled).cloned().collect())
            .unwrap_or_default();
        let roots: Vec<(String, PathBuf)> = accounts
            .iter()
            .filter(|a| !a.config_dir.as_os_str().is_empty())
            .map(|a| (a.id.clone(), a.config_dir.clone()))
            .collect();
        let opts = ReportOptions {
            days: 1,
            tz_offset_minutes: Local::now().offset().local_minus_utc() / 60,
            include_app_ledger: false,
            // Spend only: no window estimate, so only today's logs are read.
            estimate_windows: false,
            ..Default::default()
        };
        let r = report(&roots, &opts).await;
        let map = today_by_ring(&r.sessions, local_midnight(opts.now_ms));
        if let Ok(mut g) = CACHE.lock() {
            *g = Some((crate::limits_strip::now_ms(), map));
        }
        drop(running);
        super::republish(&app);
    });
}
