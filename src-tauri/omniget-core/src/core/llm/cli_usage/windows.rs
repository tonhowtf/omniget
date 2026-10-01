use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::parse::{CliUsageEntry, RateLimitSample};

pub const FIVE_HOURS_MS: i64 = 5 * 3_600 * 1_000;
pub const SEVEN_DAYS_MS: i64 = 7 * 24 * 3_600 * 1_000;
pub const CAPACITY_MAX_AGE_MS: i64 = 6 * 3_600 * 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    FiveHours,
    SevenDays,
}

impl WindowKind {
    pub fn span_ms(self) -> i64 {
        match self {
            WindowKind::FiveHours => FIVE_HOURS_MS,
            WindowKind::SevenDays => SEVEN_DAYS_MS,
        }
    }

    pub fn minutes(self) -> u32 {
        match self {
            WindowKind::FiveHours => 300,
            WindowKind::SevenDays => 10_080,
        }
    }

    pub fn from_minutes(minutes: u32) -> Option<Self> {
        match minutes {
            300 => Some(WindowKind::FiveHours),
            10_080 => Some(WindowKind::SevenDays),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowSource {
    Real,
    Estimated,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageWindow {
    pub account: String,
    pub window: WindowKind,
    pub used_tokens: u64,
    pub started_at: i64,
    pub resets_at: i64,
    pub used_fraction: Option<f32>,
    pub source: WindowSource,
}

impl UsageWindow {
    pub fn remaining(&self) -> Option<f32> {
        self.used_fraction.map(|f| (1.0 - f).clamp(0.0, 1.0))
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanLimits {
    pub five_hour_tokens: Option<u64>,
    pub seven_day_tokens: Option<u64>,
}

impl PlanLimits {
    pub fn new(five_hour_tokens: u64, seven_day_tokens: u64) -> Self {
        Self {
            five_hour_tokens: Some(five_hour_tokens),
            seven_day_tokens: Some(seven_day_tokens),
        }
    }

    pub fn ceiling(&self, window: WindowKind) -> Option<u64> {
        match window {
            WindowKind::FiveHours => self.five_hour_tokens,
            WindowKind::SevenDays => self.seven_day_tokens,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub started_at: i64,
    pub ends_at: i64,
    pub used_tokens: u64,
    pub entries: u32,
}

impl Block {
    pub fn contains(&self, ts_ms: i64) -> bool {
        ts_ms >= self.started_at && ts_ms < self.ends_at
    }
}

fn floor_to_hour(ts_ms: i64) -> i64 {
    let hour = 3_600_000;
    ts_ms.div_euclid(hour) * hour
}

fn billed(e: &CliUsageEntry) -> u64 {
    e.input_tokens + e.output_tokens
}

pub fn five_hour_blocks(entries: &[CliUsageEntry]) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut last_ts = 0i64;
    for e in entries {
        if billed(e) == 0 {
            continue;
        }
        let ts = e.ts_ms;
        let open = match blocks.last() {
            Some(b) => ts >= b.ends_at || ts - last_ts >= FIVE_HOURS_MS,
            None => true,
        };
        if open {
            let started = floor_to_hour(ts);
            blocks.push(Block {
                started_at: started,
                ends_at: started + FIVE_HOURS_MS,
                used_tokens: 0,
                entries: 0,
            });
        }
        let b = blocks.last_mut().expect("just pushed");
        b.used_tokens += billed(e);
        b.entries += 1;
        last_ts = ts;
    }
    blocks
}

pub fn estimated_windows(
    entries: &[CliUsageEntry],
    now_ms: i64,
    limits: &BTreeMap<String, PlanLimits>,
) -> Vec<UsageWindow> {
    let mut by_account: BTreeMap<String, Vec<CliUsageEntry>> = BTreeMap::new();
    for e in entries {
        by_account
            .entry(e.account.clone())
            .or_default()
            .push(e.clone());
    }
    let mut out = Vec::new();
    for (account, list) in by_account {
        let plan = limits.get(&account).copied().unwrap_or_default();
        let blocks = five_hour_blocks(&list);
        let active = blocks.iter().rev().find(|b| b.contains(now_ms));
        let (started, ends, used) = match active {
            Some(b) => (b.started_at, b.ends_at, b.used_tokens),
            None => {
                let started = floor_to_hour(now_ms);
                (started, started + FIVE_HOURS_MS, 0)
            }
        };
        out.push(window(
            &account,
            WindowKind::FiveHours,
            used,
            started,
            ends,
            plan,
        ));
        let since = now_ms - SEVEN_DAYS_MS;
        let used: u64 = list
            .iter()
            .filter(|e| e.ts_ms >= since && e.ts_ms <= now_ms)
            .map(billed)
            .sum();
        out.push(window(
            &account,
            WindowKind::SevenDays,
            used,
            since,
            now_ms + SEVEN_DAYS_MS,
            plan,
        ));
    }
    out
}

fn window(
    account: &str,
    kind: WindowKind,
    used_tokens: u64,
    started_at: i64,
    resets_at: i64,
    plan: PlanLimits,
) -> UsageWindow {
    let used_fraction = plan
        .ceiling(kind)
        .filter(|c| *c > 0)
        .map(|c| (used_tokens as f32 / c as f32).clamp(0.0, 1.0));
    UsageWindow {
        account: account.to_string(),
        window: kind,
        used_tokens,
        started_at,
        resets_at,
        used_fraction,
        source: WindowSource::Estimated,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CapacityWindow {
    #[serde(default, alias = "usedPercentage", alias = "used_percent")]
    pub used_percentage: f32,
    #[serde(default, alias = "resetsAt")]
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CapacityRecord {
    #[serde(default, alias = "account", alias = "accountId")]
    pub account_id: String,
    #[serde(default, alias = "fiveHour")]
    pub five_hour: Option<CapacityWindow>,
    #[serde(default, alias = "sevenDay")]
    pub seven_day: Option<CapacityWindow>,
    #[serde(default, alias = "observedAt")]
    pub observed_at: Option<i64>,
}

pub fn parse_capacity_cache(text: &str) -> Vec<CapacityRecord> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    match v {
        serde_json::Value::Array(items) => items
            .into_iter()
            .filter_map(|i| serde_json::from_value(i).ok())
            .collect(),
        serde_json::Value::Object(ref map) => {
            if map.contains_key("five_hour")
                || map.contains_key("fiveHour")
                || map.contains_key("account_id")
            {
                serde_json::from_value(v).ok().into_iter().collect()
            } else {
                map.iter()
                    .filter_map(|(k, val)| {
                        let mut rec: CapacityRecord = serde_json::from_value(val.clone()).ok()?;
                        if rec.account_id.is_empty() {
                            rec.account_id = k.clone();
                        }
                        Some(rec)
                    })
                    .collect()
            }
        }
        _ => Vec::new(),
    }
}

pub fn real_windows(records: &[CapacityRecord], now_ms: i64) -> Vec<UsageWindow> {
    let mut out = Vec::new();
    for rec in records {
        let observed_ms = rec.observed_at.map(|s| s * 1000).unwrap_or(0);
        if observed_ms <= 0 || now_ms - observed_ms > CAPACITY_MAX_AGE_MS {
            continue;
        }
        for (kind, w) in [
            (WindowKind::FiveHours, rec.five_hour.as_ref()),
            (WindowKind::SevenDays, rec.seven_day.as_ref()),
        ] {
            let Some(w) = w else { continue };
            let resets_ms = w.resets_at.map(|s| s * 1000).unwrap_or(0);
            if resets_ms > 0 && resets_ms <= now_ms {
                continue;
            }
            let resets_ms = if resets_ms > 0 {
                resets_ms
            } else {
                observed_ms + kind.span_ms()
            };
            out.push(UsageWindow {
                account: rec.account_id.clone(),
                window: kind,
                used_tokens: 0,
                started_at: resets_ms - kind.span_ms(),
                resets_at: resets_ms,
                used_fraction: Some((w.used_percentage / 100.0).clamp(0.0, 1.0)),
                source: WindowSource::Real,
            });
        }
    }
    out
}

pub fn windows_from_samples(
    samples: &[(String, RateLimitSample)],
    now_ms: i64,
) -> Vec<UsageWindow> {
    let mut latest: BTreeMap<(String, u32), &RateLimitSample> = BTreeMap::new();
    for (account, s) in samples {
        let key = (account.clone(), s.window_minutes);
        match latest.get(&key) {
            Some(prev) if prev.observed_at_ms >= s.observed_at_ms => {}
            _ => {
                latest.insert(key, s);
            }
        }
    }
    let mut out = Vec::new();
    for ((account, minutes), s) in latest {
        let Some(kind) = WindowKind::from_minutes(minutes) else {
            continue;
        };
        if now_ms - s.observed_at_ms > CAPACITY_MAX_AGE_MS {
            continue;
        }
        let resets_ms = s.resets_at.map(|v| v * 1000).unwrap_or(0);
        if resets_ms > 0 && resets_ms <= now_ms {
            continue;
        }
        let resets_ms = if resets_ms > 0 {
            resets_ms
        } else {
            s.observed_at_ms + kind.span_ms()
        };
        out.push(UsageWindow {
            account,
            window: kind,
            used_tokens: 0,
            started_at: resets_ms - kind.span_ms(),
            resets_at: resets_ms,
            used_fraction: Some((s.used_percent / 100.0).clamp(0.0, 1.0)),
            source: WindowSource::Real,
        });
    }
    out
}

pub fn merge_windows(real: Vec<UsageWindow>, estimated: Vec<UsageWindow>) -> Vec<UsageWindow> {
    let mut by_key: BTreeMap<(String, u32), UsageWindow> = BTreeMap::new();
    for w in estimated {
        by_key.insert((w.account.clone(), w.window.minutes()), w);
    }
    for mut w in real {
        let key = (w.account.clone(), w.window.minutes());
        if let Some(est) = by_key.get(&key) {
            w.used_tokens = est.used_tokens;
        }
        by_key.insert(key, w);
    }
    by_key.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::llm::cli_usage::parse::CliKind;

    fn entry(account: &str, ts_ms: i64, tokens: u64) -> CliUsageEntry {
        CliUsageEntry {
            cli: CliKind::Claude,
            account: account.to_string(),
            ts_ms,
            input_tokens: tokens,
            ..Default::default()
        }
    }

    const NOON: i64 = 1_772_625_600_000;

    #[test]
    fn a_block_closes_exactly_five_hours_after_it_opened() {
        let e = vec![
            entry("a", NOON + 60_000, 10),
            entry("a", NOON + FIVE_HOURS_MS - 1, 10),
            entry("a", NOON + FIVE_HOURS_MS, 10),
        ];
        let blocks = five_hour_blocks(&e);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].started_at, NOON);
        assert_eq!(blocks[0].ends_at, NOON + FIVE_HOURS_MS);
        assert_eq!(blocks[0].used_tokens, 20);
        assert_eq!(blocks[1].started_at, NOON + FIVE_HOURS_MS);
        assert_eq!(blocks[1].used_tokens, 10);
    }

    #[test]
    fn a_long_gap_opens_a_new_block_floored_to_the_hour() {
        let e = vec![
            entry("a", NOON + 90_000, 10),
            entry("a", NOON + 6 * 3_600_000 + 12_000, 10),
        ];
        let blocks = five_hour_blocks(&e);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1].started_at, NOON + 6 * 3_600_000);
    }

    #[test]
    fn the_five_hour_window_is_empty_once_it_turns_over() {
        let mut limits = BTreeMap::new();
        limits.insert("a".to_string(), PlanLimits::new(1_000, 10_000));
        let e = vec![entry("a", NOON, 400)];
        let w = estimated_windows(&e, NOON + 3_600_000, &limits);
        let five = w
            .iter()
            .find(|w| w.window == WindowKind::FiveHours)
            .unwrap();
        assert_eq!(five.used_tokens, 400);
        assert_eq!(five.used_fraction, Some(0.4));
        let w = estimated_windows(&e, NOON + FIVE_HOURS_MS + 1, &limits);
        let five = w
            .iter()
            .find(|w| w.window == WindowKind::FiveHours)
            .unwrap();
        assert_eq!(five.used_tokens, 0);
        assert_eq!(five.used_fraction, Some(0.0));
        assert!(five.started_at >= NOON + FIVE_HOURS_MS);
    }

    #[test]
    fn the_seven_day_window_only_counts_the_last_seven_days() {
        let e = vec![
            entry("a", NOON - SEVEN_DAYS_MS - 1, 999),
            entry("a", NOON - 3 * 24 * 3_600_000, 50),
            entry("a", NOON, 25),
        ];
        let w = estimated_windows(&e, NOON, &BTreeMap::new());
        let seven = w
            .iter()
            .find(|w| w.window == WindowKind::SevenDays)
            .unwrap();
        assert_eq!(seven.used_tokens, 75);
        assert_eq!(seven.used_fraction, None);
        assert_eq!(seven.source, WindowSource::Estimated);
    }

    #[test]
    fn the_capacity_cache_is_read_in_its_three_shapes() {
        let one = r#"{"account_id":"max1","five_hour":{"used_percentage":42.0,"resets_at":100},"seven_day":{"used_percentage":8.0},"observed_at":50}"#;
        assert_eq!(parse_capacity_cache(one).len(), 1);
        let many = format!("[{one}]");
        assert_eq!(parse_capacity_cache(&many)[0].account_id, "max1");
        let keyed = r#"{"max2":{"five_hour":{"used_percentage":10.0},"observed_at":50}}"#;
        let recs = parse_capacity_cache(keyed);
        assert_eq!(recs[0].account_id, "max2");
        assert!(parse_capacity_cache("not json").is_empty());
    }

    #[test]
    fn a_fresh_capacity_record_is_a_real_window() {
        let now = NOON;
        let recs = vec![CapacityRecord {
            account_id: "max1".into(),
            five_hour: Some(CapacityWindow {
                used_percentage: 42.0,
                resets_at: Some(now / 1000 + 3_600),
            }),
            seven_day: Some(CapacityWindow {
                used_percentage: 8.0,
                resets_at: None,
            }),
            observed_at: Some(now / 1000 - 60),
        }];
        let w = real_windows(&recs, now);
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].source, WindowSource::Real);
        assert!((w[0].used_fraction.unwrap() - 0.42).abs() < 1e-6);
        assert!((w[0].remaining().unwrap() - 0.58).abs() < 1e-6);
        assert_eq!(w[0].resets_at, now + 3_600_000);
    }

    #[test]
    fn a_stale_or_already_reset_capacity_record_is_dropped() {
        let now = NOON;
        let old = vec![CapacityRecord {
            account_id: "max1".into(),
            five_hour: Some(CapacityWindow {
                used_percentage: 99.0,
                resets_at: None,
            }),
            observed_at: Some(now / 1000 - 7 * 3_600),
            ..Default::default()
        }];
        assert!(real_windows(&old, now).is_empty(), "older than six hours");
        let reset = vec![CapacityRecord {
            account_id: "max1".into(),
            five_hour: Some(CapacityWindow {
                used_percentage: 99.0,
                resets_at: Some(now / 1000 - 10),
            }),
            observed_at: Some(now / 1000 - 60),
            ..Default::default()
        }];
        assert!(
            real_windows(&reset, now).is_empty(),
            "window already turned"
        );
    }

    #[test]
    fn real_wins_over_estimated_but_keeps_the_token_count() {
        let est = estimated_windows(&[entry("max1", NOON, 700)], NOON, &BTreeMap::new());
        let real = real_windows(
            &[CapacityRecord {
                account_id: "max1".into(),
                five_hour: Some(CapacityWindow {
                    used_percentage: 30.0,
                    resets_at: Some(NOON / 1000 + 600),
                }),
                observed_at: Some(NOON / 1000),
                ..Default::default()
            }],
            NOON,
        );
        let merged = merge_windows(real, est);
        assert_eq!(merged.len(), 2);
        let five = merged
            .iter()
            .find(|w| w.window == WindowKind::FiveHours)
            .unwrap();
        assert_eq!(five.source, WindowSource::Real);
        assert_eq!(five.used_tokens, 700, "the estimate's tokens survive");
        assert!((five.used_fraction.unwrap() - 0.30).abs() < 1e-6);
        let seven = merged
            .iter()
            .find(|w| w.window == WindowKind::SevenDays)
            .unwrap();
        assert_eq!(seven.source, WindowSource::Estimated);
    }

    #[test]
    fn codex_samples_become_real_windows_newest_first() {
        let samples = vec![
            (
                "work".to_string(),
                RateLimitSample {
                    window_minutes: 300,
                    used_percent: 5.0,
                    resets_at: Some(NOON / 1000 + 600),
                    observed_at_ms: NOON - 120_000,
                },
            ),
            (
                "work".to_string(),
                RateLimitSample {
                    window_minutes: 300,
                    used_percent: 12.5,
                    resets_at: Some(NOON / 1000 + 600),
                    observed_at_ms: NOON - 60_000,
                },
            ),
        ];
        let w = windows_from_samples(&samples, NOON);
        assert_eq!(w.len(), 1);
        assert!((w[0].used_fraction.unwrap() - 0.125).abs() < 1e-6);
        assert_eq!(w[0].source, WindowSource::Real);
    }
}
