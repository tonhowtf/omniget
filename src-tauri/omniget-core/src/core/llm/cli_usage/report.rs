//! The dashboard the Accounts tab draws: cost per day and per model, tool
//! heat map, sessions, and the 5 h / 7 d windows.
//!
//! The app's own ledger (`core/tools/usage.rs`) and the CLIs' histories are
//! two different books of the same spend, so the report merges them in
//! `by_source` (`cli:claude`, `cli:codex`, `app`) while keeping `by_day` and
//! `by_model` as the union.
//!
//! Costs are priced here, on read, with the LiteLLM table (`pricing.rs`) and
//! the very same `usage::cost_of` the app ledger uses, so a cached token
//! costs the same in both books.
//!
//! Budget: nothing in this module runs at boot. [`report`] is called from the
//! `llm_cli_usage_report` command, i.e. on a click or when the tab opens.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;

use super::parse::CliUsageEntry;
use super::scan::{ScanState, ScanStats};
use super::windows::{PlanLimits, UsageWindow};
use crate::core::tools::pricing::ModelPrice;
use crate::core::tools::usage::{Bucket, UsageEntry};

/// Weekday (0 = Monday) × hour of day.
pub type ToolHeat = [[u32; 24]; 7];

#[derive(Debug, Clone, Default, Serialize)]
pub struct SessionSummary {
    pub cli: &'static str,
    pub account: String,
    pub session_id: String,
    pub project: String,
    pub started_at: i64,
    pub ended_at: i64,
    pub messages: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_usd: f64,
    pub models: Vec<String>,
    pub tool_calls: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Totals {
    pub messages: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_usd: f64,
    /// Responses whose model is not in the price table.
    pub unknown_price: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CliUsageReport {
    pub generated_at: i64,
    pub by_day: Vec<Bucket>,
    pub by_model: Vec<Bucket>,
    /// `cli:claude`, `cli:codex` and `app`: where the spend was booked.
    pub by_source: Vec<Bucket>,
    pub by_tool_hour: ToolHeat,
    pub by_tool: Vec<(String, u32)>,
    pub sessions: Vec<SessionSummary>,
    pub windows: Vec<UsageWindow>,
    pub totals: Totals,
    pub stats: ScanStats,
    pub duplicates_skipped: u32,
    pub state_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReportOptions {
    /// How far back the buckets go.
    pub days: u32,
    /// Local offset in minutes, so the heat map matches the user's clock.
    pub tz_offset_minutes: i32,
    /// Fold the app's own ledger into `by_day`/`by_model`/`by_source`.
    pub include_app_ledger: bool,
    /// Count subagent turns (`isSidechain`) as their own messages.
    pub include_sidechains: bool,
    pub limits: BTreeMap<String, PlanLimits>,
    /// Estimate the 5 h and weekly windows from the history. They look back a
    /// week, so the report then reads a week of logs even for `days: 1`; a
    /// caller that only wants spend (the usage icon) turns this off and reads
    /// just the last `days`.
    pub estimate_windows: bool,
    /// Epoch ms; injected so the tests do not depend on the clock.
    pub now_ms: i64,
}

impl Default for ReportOptions {
    fn default() -> Self {
        Self {
            days: 30,
            tz_offset_minutes: 0,
            include_app_ledger: true,
            include_sidechains: true,
            limits: BTreeMap::new(),
            estimate_windows: true,
            now_ms: chrono::Utc::now().timestamp_millis(),
        }
    }
}

/// Price one CLI response with the same formula as the app ledger.
fn cost_of(entry: &CliUsageEntry, prices: &HashMap<String, ModelPrice>) -> Option<f64> {
    if let Some(c) = entry.cost_usd {
        return Some(c);
    }
    let price = prices.get(&entry.model)?;
    let shim = UsageEntry {
        input_tokens: entry.input_tokens,
        output_tokens: entry.output_tokens,
        cache_read_tokens: entry.cache_read_tokens,
        cache_write_tokens: entry.cache_write_tokens,
        ..Default::default()
    };
    crate::core::tools::usage::cost_of(price, &shim)
}

fn add(map: &mut BTreeMap<String, Bucket>, key: String, e: &CliUsageEntry, cost: Option<f64>) {
    let b = map.entry(key.clone()).or_insert_with(|| Bucket {
        key,
        ..Default::default()
    });
    b.calls += 1;
    b.input_tokens += e.input_tokens;
    b.output_tokens += e.output_tokens;
    b.cache_read_tokens += e.cache_read_tokens;
    b.cache_write_tokens += e.cache_write_tokens;
    match cost {
        Some(c) => b.cost_usd += c,
        None => b.unknown_price += 1,
    }
}

fn day_key(ts_ms: i64, tz_offset_minutes: i32) -> String {
    let shifted = ts_ms + tz_offset_minutes as i64 * 60_000;
    chrono::DateTime::from_timestamp_millis(shifted)
        .map(|t| t.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

/// `(weekday 0=Mon, hour)` in the user's own clock.
fn heat_slot(ts_ms: i64, tz_offset_minutes: i32) -> Option<(usize, usize)> {
    let shifted = ts_ms + tz_offset_minutes as i64 * 60_000;
    let t = chrono::DateTime::from_timestamp_millis(shifted)?;
    use chrono::{Datelike, Timelike};
    Some((
        t.weekday().num_days_from_monday() as usize,
        t.hour() as usize,
    ))
}

/// Pure build step: everything the UI shows, from entries already scanned.
pub fn build(
    entries: &[CliUsageEntry],
    windows: Vec<UsageWindow>,
    prices: &HashMap<String, ModelPrice>,
    app_ledger: &[UsageEntry],
    stats: ScanStats,
    opts: &ReportOptions,
) -> CliUsageReport {
    let since = opts.now_ms - opts.days.max(1) as i64 * 24 * 3_600_000;
    let mut by_day: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut by_model: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut by_source: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut by_tool: BTreeMap<String, u32> = BTreeMap::new();
    let mut heat: ToolHeat = [[0; 24]; 7];
    let mut sessions: BTreeMap<(String, String), SessionSummary> = BTreeMap::new();
    let mut totals = Totals::default();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut duplicates = 0u32;

    for e in entries {
        if e.ts_ms < since {
            continue;
        }
        // Tool calls are counted even on a line that carries no tokens (Codex
        // writes them apart), but a duplicated response is counted once.
        if let Some(key) = e.dedupe_key.as_deref() {
            if !seen.insert(key) {
                duplicates += 1;
                continue;
            }
        }
        if !e.tools.is_empty() {
            if let Some((d, h)) = heat_slot(e.ts_ms, opts.tz_offset_minutes) {
                for name in &e.tools {
                    heat[d][h] += 1;
                    *by_tool.entry(name.clone()).or_default() += 1;
                }
            }
        }
        if e.input_tokens + e.output_tokens == 0 {
            continue;
        }
        if e.sidechain && !opts.include_sidechains {
            continue;
        }
        let cost = cost_of(e, prices);
        add(
            &mut by_day,
            day_key(e.ts_ms, opts.tz_offset_minutes),
            e,
            cost,
        );
        add(&mut by_model, e.model.clone(), e, cost);
        add(&mut by_source, format!("cli:{}", e.cli.as_str()), e, cost);

        totals.messages += 1;
        totals.input_tokens += e.input_tokens;
        totals.output_tokens += e.output_tokens;
        totals.cache_read_tokens += e.cache_read_tokens;
        totals.cache_write_tokens += e.cache_write_tokens;
        match cost {
            Some(c) => totals.cost_usd += c,
            None => totals.unknown_price += 1,
        }

        let s = sessions
            .entry((e.account.clone(), e.session_id.clone()))
            .or_insert_with(|| SessionSummary {
                cli: e.cli.as_str(),
                account: e.account.clone(),
                session_id: e.session_id.clone(),
                project: e.project.clone(),
                started_at: e.ts_ms,
                ..Default::default()
            });
        s.started_at = s.started_at.min(e.ts_ms);
        s.ended_at = s.ended_at.max(e.ts_ms);
        s.messages += 1;
        s.input_tokens += e.input_tokens;
        s.output_tokens += e.output_tokens;
        s.cache_read_tokens += e.cache_read_tokens;
        s.cache_write_tokens += e.cache_write_tokens;
        s.cost_usd += cost.unwrap_or(0.0);
        s.tool_calls += e.tools.len() as u32;
        if !e.model.is_empty() && !s.models.iter().any(|m| m == &e.model) {
            s.models.push(e.model.clone());
        }
    }

    if opts.include_app_ledger {
        for a in app_ledger {
            let Some(ts) = chrono::DateTime::parse_from_rfc3339(&a.ts)
                .ok()
                .map(|t| t.timestamp_millis())
            else {
                continue;
            };
            if ts < since {
                continue;
            }
            let shim = CliUsageEntry {
                ts_ms: ts,
                model: a.model.clone(),
                input_tokens: a.input_tokens,
                output_tokens: a.output_tokens,
                cache_read_tokens: a.cache_read_tokens,
                cache_write_tokens: a.cache_write_tokens,
                cost_usd: a.cost_usd,
                ..Default::default()
            };
            let cost = cost_of(&shim, prices);
            add(
                &mut by_day,
                day_key(ts, opts.tz_offset_minutes),
                &shim,
                cost,
            );
            add(&mut by_model, a.model.clone(), &shim, cost);
            add(&mut by_source, "app".to_string(), &shim, cost);
        }
    }

    let mut by_model: Vec<Bucket> = by_model.into_values().collect();
    by_model.sort_by(|a, b| {
        b.cost_usd
            .partial_cmp(&a.cost_usd)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.calls.cmp(&a.calls))
    });
    let mut sessions: Vec<SessionSummary> = sessions.into_values().collect();
    sessions.sort_by_key(|s| std::cmp::Reverse(s.ended_at));
    let mut by_tool: Vec<(String, u32)> = by_tool.into_iter().collect();
    by_tool.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    CliUsageReport {
        generated_at: opts.now_ms,
        by_day: by_day.into_values().collect(),
        by_model,
        by_source: by_source.into_values().collect(),
        by_tool_hour: heat,
        by_tool,
        sessions,
        windows,
        totals,
        stats,
        duplicates_skipped: duplicates,
        state_path: super::state_path().map(|p| p.to_string_lossy().to_string()),
    }
}

/// Models worth asking the price table about.
fn models_of(entries: &[CliUsageEntry], app_ledger: &[UsageEntry]) -> Vec<String> {
    let mut set: HashSet<String> = HashSet::new();
    for e in entries {
        if !e.model.is_empty() {
            set.insert(e.model.clone());
        }
    }
    for a in app_ledger {
        if !a.model.is_empty() {
            set.insert(a.model.clone());
        }
    }
    set.into_iter().collect()
}

/// Scan, price and build. Called from the `llm_cli_usage_report` command, on
/// a click or when the Accounts tab opens — never at boot.
///
/// `accounts` maps an account label to its isolated config dir (the pointer,
/// never its contents).
pub async fn report(
    accounts: &[(String, std::path::PathBuf)],
    opts: &ReportOptions,
) -> CliUsageReport {
    let roots = super::scan::default_roots(accounts);
    let state_path = super::state_path();
    // The entries of past scans are not kept in memory, so a report reads the
    // whole history from byte 0. Resuming from the saved offsets (as this did
    // until 2026-09-26) dropped every file that had not grown since the last
    // scan: a week of use showed as the last two sessions. The offsets are
    // still saved for the incremental readers that share the state file.
    // Only the logs written inside the report's reach are read. The estimated
    // 5 h and weekly windows look back a week, so the reach is never shorter
    // than that, plus a day of slack for a session that spans midnight.
    let reach_days = if opts.estimate_windows {
        opts.days.max(8) as u64
    } else {
        opts.days.max(1) as u64 + 1
    };
    let modified_since = std::time::SystemTime::UNIX_EPOCH
        + std::time::Duration::from_millis(opts.now_ms.max(0) as u64)
        - std::time::Duration::from_secs(reach_days * 86_400);
    // Parsing is plain file IO and JSON: off the async runtime, so the app's
    // commands never queue behind it.
    let scanned = tokio::task::spawn_blocking(move || {
        let mut state = ScanState::default();
        let outcome = super::scan::scan_roots_since(&roots, &mut state, Some(modified_since));
        (state, outcome)
    })
    .await;
    let (state, outcome) = scanned.unwrap_or_default();
    if let Some(p) = &state_path {
        let _ = state.save(p);
    }
    let app_ledger = if opts.include_app_ledger {
        crate::core::tools::usage::read_all()
    } else {
        Vec::new()
    };
    let mut prices: HashMap<String, ModelPrice> = HashMap::new();
    for model in models_of(&outcome.entries, &app_ledger) {
        if let Some(p) = crate::core::tools::pricing::price_for(&model).await {
            prices.insert(model, p);
        }
    }
    let estimated = if opts.estimate_windows {
        super::windows::estimated_windows(&outcome.entries, opts.now_ms, &opts.limits)
    } else {
        Vec::new()
    };
    let mut real = super::windows::windows_from_samples(&outcome.limits, opts.now_ms);
    if let Some(text) = super::capacity_path().and_then(|p| std::fs::read_to_string(p).ok()) {
        real.extend(super::windows::real_windows(
            &super::windows::parse_capacity_cache(&text),
            opts.now_ms,
        ));
    }
    let windows = super::windows::merge_windows(real, estimated);
    build(
        &outcome.entries,
        windows,
        &prices,
        &app_ledger,
        outcome.stats,
        opts,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::llm::cli_usage::parse::CliKind;

    const NOON: i64 = 1_772_625_600_000; // 2026-03-04T12:00:00Z, a Wednesday

    fn price() -> HashMap<String, ModelPrice> {
        let mut m = HashMap::new();
        m.insert(
            "claude-opus-5".to_string(),
            ModelPrice {
                key: "claude-opus-5".into(),
                provider: "anthropic".into(),
                mode: "chat".into(),
                input_per_m: Some(3.0),
                output_per_m: Some(15.0),
                cache_read_per_m: Some(0.3),
                cache_write_per_m: Some(3.75),
                max_input_tokens: None,
                max_output_tokens: None,
                input_per_second: None,
                input_per_character: None,
                supports_vision: false,
                supports_tools: true,
                supports_reasoning: false,
                supports_caching: true,
                deprecation_date: None,
            },
        );
        m
    }

    fn entry(ts_ms: i64, model: &str, tools: &[&str]) -> CliUsageEntry {
        CliUsageEntry {
            cli: CliKind::Claude,
            account: "default".into(),
            session_id: "s1".into(),
            project: "proj".into(),
            ts_ms,
            model: model.into(),
            input_tokens: 1_000_000,
            output_tokens: 0,
            cache_read_tokens: 1_000_000,
            tools: tools.iter().map(|t| t.to_string()).collect(),
            ..Default::default()
        }
    }

    fn opts() -> ReportOptions {
        ReportOptions {
            now_ms: NOON,
            include_app_ledger: false,
            ..Default::default()
        }
    }

    #[test]
    fn buckets_and_cost_use_the_same_formula_as_the_app_ledger() {
        let e = vec![entry(NOON, "claude-opus-5", &["Bash"])];
        let r = build(&e, vec![], &price(), &[], ScanStats::default(), &opts());
        assert_eq!(r.by_day.len(), 1);
        assert_eq!(r.by_day[0].key, "2026-03-04");
        // One million tokens, all served from cache, at 0.30 USD/M.
        assert!(
            (r.totals.cost_usd - 0.3).abs() < 1e-9,
            "{}",
            r.totals.cost_usd
        );
        assert_eq!(r.by_model[0].key, "claude-opus-5");
        assert_eq!(r.by_source[0].key, "cli:claude");
        assert_eq!(r.totals.unknown_price, 0);
    }

    #[test]
    fn a_model_outside_the_price_table_is_counted_but_not_priced() {
        let e = vec![entry(NOON, "gpt-6-astra", &[])];
        let r = build(&e, vec![], &price(), &[], ScanStats::default(), &opts());
        assert_eq!(r.totals.unknown_price, 1);
        assert_eq!(r.totals.cost_usd, 0.0);
        assert_eq!(r.totals.messages, 1);
    }

    #[test]
    fn a_duplicated_response_is_counted_once() {
        let mut a = entry(NOON, "claude-opus-5", &[]);
        a.dedupe_key = Some("msg_1:req_1".into());
        let r = build(
            &[a.clone(), a.clone()],
            vec![],
            &price(),
            &[],
            ScanStats::default(),
            &opts(),
        );
        assert_eq!(r.totals.messages, 1);
        assert_eq!(r.duplicates_skipped, 1);
    }

    #[test]
    fn the_tool_heat_map_lands_on_the_local_hour() {
        let e = vec![entry(NOON, "claude-opus-5", &["Bash", "Read"])];
        let r = build(&e, vec![], &price(), &[], ScanStats::default(), &opts());
        // 2026-03-04 is a Wednesday: index 2, hour 12 in UTC.
        assert_eq!(r.by_tool_hour[2][12], 2);
        let shifted = ReportOptions {
            tz_offset_minutes: -180, // America/Recife
            ..opts()
        };
        let r = build(&e, vec![], &price(), &[], ScanStats::default(), &shifted);
        assert_eq!(r.by_tool_hour[2][9], 2);
        assert_eq!(r.by_tool[0], ("Bash".to_string(), 1));
    }

    #[test]
    fn sessions_summarise_their_own_turns() {
        let e = vec![
            entry(NOON, "claude-opus-5", &["Bash"]),
            entry(NOON + 60_000, "claude-sonnet-4-5", &[]),
        ];
        let r = build(&e, vec![], &price(), &[], ScanStats::default(), &opts());
        assert_eq!(r.sessions.len(), 1);
        let s = &r.sessions[0];
        assert_eq!(s.messages, 2);
        assert_eq!(s.models.len(), 2);
        assert_eq!(s.tool_calls, 1);
        assert_eq!(s.started_at, NOON);
        assert_eq!(s.ended_at, NOON + 60_000);
    }

    #[test]
    fn the_app_ledger_is_folded_in_as_its_own_source() {
        let app = vec![UsageEntry {
            ts: "2026-03-04T12:00:00Z".into(),
            model: "claude-opus-5".into(),
            input_tokens: 1_000_000,
            output_tokens: 0,
            cache_read_tokens: 1_000_000,
            ..Default::default()
        }];
        let o = ReportOptions {
            include_app_ledger: true,
            ..opts()
        };
        let r = build(
            &[entry(NOON, "claude-opus-5", &[])],
            vec![],
            &price(),
            &app,
            ScanStats::default(),
            &o,
        );
        let keys: Vec<&str> = r.by_source.iter().map(|b| b.key.as_str()).collect();
        assert_eq!(keys, vec!["app", "cli:claude"]);
        // The day bucket carries both books.
        assert_eq!(r.by_day[0].calls, 2);
        // `totals` stays the CLI book, so the two are never double counted.
        assert_eq!(r.totals.messages, 1);
    }

    #[test]
    fn entries_older_than_the_window_are_left_out() {
        let e = vec![
            entry(NOON - 40 * 24 * 3_600_000, "claude-opus-5", &["Bash"]),
            entry(NOON, "claude-opus-5", &[]),
        ];
        let r = build(&e, vec![], &price(), &[], ScanStats::default(), &opts());
        assert_eq!(r.totals.messages, 1);
        assert_eq!(r.by_tool_hour.iter().flatten().sum::<u32>(), 0);
    }

    #[test]
    fn the_report_serialises_for_the_ui() {
        let r = build(
            &[entry(NOON, "claude-opus-5", &["Bash"])],
            vec![],
            &price(),
            &[],
            ScanStats::default(),
            &opts(),
        );
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"by_tool_hour\""));
        assert!(json.contains("\"by_source\""));
    }
}
