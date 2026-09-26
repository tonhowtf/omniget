//! Per-agent spend ceiling, cut **before** the turn is sent. Owned by
//! f2-llm-coordinator.
//!
//! State lives in `<app_data>/llm/budget.json` (`OMNIGET_DATA_DIR` overrides the
//! root, like `core::secrets` and the profile store), keyed by agent id inside a
//! UTC day window. Everything the check touches is in memory, so the check is a
//! map lookup and two comparisons; the file is only rewritten when a turn is
//! recorded.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::agent::Budget;
use super::error::{LlmError, ERR_LLM_BUDGET};

/// One agent's spend inside the current UTC day.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentSpend {
    #[serde(default)]
    pub usd: f64,
    #[serde(default)]
    pub turns: u32,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    /// Turns whose cost the provider did not report. Never booked as zero
    /// dollars silently: the UI shows "cost unknown" for them.
    #[serde(default)]
    pub unknown_cost_turns: u32,
}

impl AgentSpend {
    pub fn tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// The whole file: one UTC day and the per-agent totals inside it, plus the
/// lifetime pools (a mission's cap is for the mission, not for a day).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct BudgetState {
    /// `YYYY-MM-DD` in UTC. A different day wipes `agents`.
    #[serde(default)]
    pub day: String,
    #[serde(default)]
    pub agents: HashMap<String, AgentSpend>,
    /// Pools that never roll over ([`is_lifetime_pool`]): a mission's spend
    /// counts against its cap for as long as the mission lives.
    #[serde(default)]
    pub lifetime: HashMap<String, AgentSpend>,
}

/// Pools whose ledger is kept for their whole life instead of per UTC day.
pub const LIFETIME_POOL_PREFIX: &str = "mission:";

pub fn is_lifetime_pool(pool: &str) -> bool {
    pool.starts_with(LIFETIME_POOL_PREFIX)
}

/// Under a USD cap with no token/turn limit (`strict_unknown`), at most this
/// many turns of unknown cost are admitted over the pool's life: an unknown
/// price is never treated as free forever.
pub const STRICT_UNKNOWN_COST_TURNS_MAX: u32 = 40;

impl BudgetState {
    /// Drop the day totals when the UTC day turned over. Lifetime pools stay.
    fn roll(&mut self, day: &str) {
        if self.day != day {
            self.day = day.to_string();
            self.agents.clear();
        }
    }

    fn spend(&self, pool: &str) -> AgentSpend {
        let map = if is_lifetime_pool(pool) {
            &self.lifetime
        } else {
            &self.agents
        };
        map.get(pool).cloned().unwrap_or_default()
    }

    fn spend_mut(&mut self, pool: &str) -> &mut AgentSpend {
        let map = if is_lifetime_pool(pool) {
            &mut self.lifetime
        } else {
            &mut self.agents
        };
        map.entry(pool.to_string()).or_default()
    }
}

pub fn utc_day(now: DateTime<Utc>) -> String {
    now.format("%Y-%m-%d").to_string()
}

/// Default location: `<app_data>/llm/budget.json`.
pub fn default_path() -> Option<PathBuf> {
    crate::core::paths::app_data_dir().map(|d| d.join("llm").join("budget.json"))
}

/// What a turn is expected to cost before it is sent. `usd: None` means the
/// price is unknown (CLI subscription, local model): it stays unknown and
/// the pool's token/turn limits decide instead.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Estimate {
    pub usd: Option<f64>,
    pub tokens: u64,
}

/// Admission limits of one pool (an agent's day, a group room). `None` =
/// no limit of that kind.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PoolLimits {
    pub usd: Option<f64>,
    pub tokens: Option<u64>,
    pub turns: Option<u32>,
    /// Under a USD cap with no token/turn limit, admit only one turn of
    /// unknown cost at a time (a group pool wants this; an agent's own day
    /// keeps its turns concurrent and only stops once the cap is reached).
    #[serde(default)]
    pub strict_unknown: bool,
}

/// One reservation in flight.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InFlight {
    pub id: String,
    pub pool: String,
    pub estimate: Estimate,
    pub created_ms: i64,
}

/// What a pool holds right now, for the UI and the tests.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct PoolView {
    pub spent: AgentSpend,
    pub in_flight: usize,
    pub reserved_usd: f64,
    /// Reservations whose cost is unknown (never counted as zero).
    pub reserved_unknown_cost: usize,
    pub reserved_tokens: u64,
}

#[derive(Debug)]
pub struct BudgetStore {
    path: Option<PathBuf>,
    state: Mutex<BudgetState>,
    inflight: Mutex<HashMap<String, InFlight>>,
}

impl Default for BudgetStore {
    fn default() -> Self {
        Self::memory()
    }
}

impl BudgetStore {
    /// Load from `<app_data>/llm/budget.json`, or start empty if it is missing
    /// or corrupt (a broken ledger must never block the app).
    pub fn open() -> Self {
        match default_path() {
            Some(p) => Self::at(p),
            None => Self::memory(),
        }
    }

    pub fn at(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let state = std::fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<BudgetState>(&t).ok())
            .unwrap_or_default();
        Self {
            path: Some(path),
            state: Mutex::new(state),
            inflight: Mutex::new(HashMap::new()),
        }
    }

    /// No file at all: tests and the `--no-persist` path.
    pub fn memory() -> Self {
        Self {
            path: None,
            state: Mutex::new(BudgetState::default()),
            inflight: Mutex::new(HashMap::new()),
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn snapshot(&self) -> BudgetState {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn spent_today(&self, agent_id: &str) -> AgentSpend {
        self.spent_today_at(agent_id, Utc::now())
    }

    pub fn spent_today_at(&self, agent_id: &str, now: DateTime<Utc>) -> AgentSpend {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.roll(&utc_day(now));
        state.spend(agent_id)
    }

    /// Dollars left today, `None` when the agent has no daily ceiling.
    pub fn remaining_usd(&self, agent_id: &str, budget: &Budget) -> Option<f64> {
        let cap = budget.usd_per_day?;
        Some((cap - self.spent_today(agent_id).usd).max(0.0))
    }

    /// The gate: called before every turn. Returns `ERR_LLM_BUDGET` when the
    /// daily ceiling is already reached or when the turn's own token estimate
    /// is over `tokens_per_turn`.
    pub fn check(
        &self,
        agent_id: &str,
        budget: &Budget,
        estimated_tokens: u32,
    ) -> Result<(), LlmError> {
        self.check_at(agent_id, budget, estimated_tokens, Utc::now())
    }

    pub fn check_at(
        &self,
        agent_id: &str,
        budget: &Budget,
        estimated_tokens: u32,
        now: DateTime<Utc>,
    ) -> Result<(), LlmError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.roll(&utc_day(now));
        if let Some(cap) = budget.usd_per_day {
            let spent = state.agents.get(agent_id).map(|s| s.usd).unwrap_or(0.0);
            if spent >= cap {
                return Err(LlmError::new(
                    ERR_LLM_BUDGET,
                    format!("daily budget reached: {spent:.4} of {cap:.4} USD"),
                ));
            }
        }
        if let Some(max) = budget.tokens_per_turn {
            if estimated_tokens > max {
                return Err(LlmError::new(
                    ERR_LLM_BUDGET,
                    format!("turn needs {estimated_tokens} tokens, the ceiling is {max}"),
                ));
            }
        }
        Ok(())
    }

    /// Book a finished turn and persist. Cost is what the provider reported, or
    /// what the pricing table computed upstream; `None` books zero.
    pub fn record(&self, agent_id: &str, usd: Option<f64>, input_tokens: u32, output_tokens: u32) {
        self.record_at(agent_id, usd, input_tokens, output_tokens, Utc::now());
    }

    pub fn record_at(
        &self,
        agent_id: &str,
        usd: Option<f64>,
        input_tokens: u32,
        output_tokens: u32,
        now: DateTime<Utc>,
    ) {
        {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.roll(&utc_day(now));
            let entry = state.spend_mut(agent_id);
            match usd {
                Some(v) => entry.usd += v,
                None => entry.unknown_cost_turns += 1,
            }
            entry.turns += 1;
            entry.input_tokens += input_tokens as u64;
            entry.output_tokens += output_tokens as u64;
        }
        self.save();
    }

    // ── In-flight reservations (spec 02 "Orçamento", B04) ─────────────

    /// Reserves room for one turn in `pool` before it is dispatched. The
    /// check and the reservation happen under one lock, so two children near
    /// the limit cannot both pass. Admission counts what was spent today plus
    /// everything still in flight:
    /// - `usd`: known spend + known reservations + this estimate must stay
    ///   within the cap. An estimate of unknown cost is not zero: under a USD
    ///   cap with no token/turn limit, only one unknown-cost turn may be in
    ///   flight at a time, and only while the known spend is under the cap;
    /// - `tokens`: tokens spent + reserved + estimated ≤ limit;
    /// - `turns`: turns booked + in flight + 1 ≤ limit.
    ///
    /// Returns the reservation id; [`Self::settle`] or [`Self::release`] it.
    pub fn reserve(
        &self,
        pool: &str,
        limits: &PoolLimits,
        estimate: Estimate,
    ) -> Result<String, LlmError> {
        self.reserve_at(pool, limits, estimate, Utc::now())
    }

    pub fn reserve_at(
        &self,
        pool: &str,
        limits: &PoolLimits,
        estimate: Estimate,
        now: DateTime<Utc>,
    ) -> Result<String, LlmError> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.roll(&utc_day(now));
        let spent = state.spend(pool);
        let mut inflight = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
        let mine: Vec<&InFlight> = inflight.values().filter(|r| r.pool == pool).collect();
        let reserved_usd: f64 = mine.iter().filter_map(|r| r.estimate.usd).sum();
        let unknown = mine.iter().filter(|r| r.estimate.usd.is_none()).count();
        let reserved_tokens: u64 = mine.iter().map(|r| r.estimate.tokens).sum();
        let deny = |why: String| Err(LlmError::new(ERR_LLM_BUDGET, why));
        if let Some(cap) = limits.usd {
            let known = spent.usd + reserved_usd;
            match estimate.usd {
                Some(est) if known + est > cap + 1e-12 => {
                    return deny(format!(
                        "budget reserved: {known:.4} USD spent or in flight + {est:.4} would pass {cap:.4}"
                    ))
                }
                Some(_) => {}
                None => {
                    if known >= cap {
                        return deny(format!("daily budget reached: {known:.4} of {cap:.4} USD"));
                    }
                    let alternative = limits.tokens.is_some() || limits.turns.is_some();
                    if limits.strict_unknown && !alternative && unknown > 0 {
                        return deny(format!(
                            "a turn of unknown cost is already in flight under the {cap:.4} USD cap"
                        ));
                    }
                    if limits.strict_unknown
                        && !alternative
                        && spent.unknown_cost_turns as usize + unknown
                            >= STRICT_UNKNOWN_COST_TURNS_MAX as usize
                    {
                        return deny(format!(
                            "{} turns of unknown cost already booked under the {cap:.4} USD cap; set a token or turn limit to go on",
                            spent.unknown_cost_turns
                        ));
                    }
                }
            }
        }
        if let Some(max) = limits.tokens {
            let total = spent.tokens() + reserved_tokens + estimate.tokens;
            if total > max {
                return deny(format!(
                    "token budget: {} used or in flight + {} would pass {max}",
                    spent.tokens() + reserved_tokens,
                    estimate.tokens
                ));
            }
        }
        if let Some(max) = limits.turns {
            if spent.turns as usize + mine.len() + 1 > max as usize {
                return deny(format!("turn budget: {max} turns used or in flight"));
            }
        }
        let id = uuid::Uuid::new_v4().to_string();
        inflight.insert(
            id.clone(),
            InFlight {
                id: id.clone(),
                pool: pool.to_string(),
                estimate,
                created_ms: now.timestamp_millis(),
            },
        );
        Ok(id)
    }

    /// Replaces the reservation by what the turn really used. Unknown cost
    /// is booked as unknown (a failed or retried turn still counts).
    pub fn settle(
        &self,
        reservation: &str,
        usd: Option<f64>,
        input_tokens: u32,
        output_tokens: u32,
    ) {
        let held = self
            .inflight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(reservation);
        if let Some(r) = held {
            self.record(&r.pool, usd, input_tokens, output_tokens);
        }
    }

    /// Raises a lifetime pool to at least `floor` (the mission's own durable
    /// spend), so a lost or reset ledger file never gives a mission its cap
    /// back. Day pools are left alone.
    pub fn seed_lifetime(&self, pool: &str, floor: &AgentSpend) {
        if !is_lifetime_pool(pool) {
            return;
        }
        let changed = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let e = state.spend_mut(pool);
            let before = e.clone();
            e.usd = e.usd.max(floor.usd);
            e.turns = e.turns.max(floor.turns);
            e.unknown_cost_turns = e.unknown_cost_turns.max(floor.unknown_cost_turns);
            if e.tokens() < floor.tokens() {
                e.input_tokens += floor.tokens() - e.tokens();
            }
            *e != before
        };
        if changed {
            self.save();
        }
    }

    /// Whether `pool` has reservations in flight (a refusal while one is in
    /// flight may pass once it settles; one without is final).
    pub fn has_in_flight(&self, pool: &str) -> bool {
        self.inflight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .any(|r| r.pool == pool)
    }

    /// Drops a reservation without booking anything (cancelled before the
    /// provider saw it). Returns whether it was held.
    pub fn release(&self, reservation: &str) -> bool {
        self.inflight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(reservation)
            .is_some()
    }

    pub fn in_flight(&self) -> Vec<InFlight> {
        self.inflight
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect()
    }

    pub fn pool(&self, pool: &str) -> PoolView {
        let spent = self.spent_today(pool);
        let inflight = self.inflight.lock().unwrap_or_else(|e| e.into_inner());
        let mine: Vec<&InFlight> = inflight.values().filter(|r| r.pool == pool).collect();
        PoolView {
            spent,
            in_flight: mine.len(),
            reserved_usd: mine.iter().filter_map(|r| r.estimate.usd).sum(),
            reserved_unknown_cost: mine.iter().filter(|r| r.estimate.usd.is_none()).count(),
            reserved_tokens: mine.iter().map(|r| r.estimate.tokens).sum(),
        }
    }

    /// Atomic write: temp file next to the target, then rename.
    pub fn save(&self) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        let state = self.snapshot();
        let Ok(text) = serde_json::to_string_pretty(&state) else {
            return;
        };
        if let Some(dir) = path.parent() {
            if std::fs::create_dir_all(dir).is_err() {
                return;
            }
        }
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn day(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 12, 0, 0).unwrap()
    }

    fn budget(usd: Option<f64>, tokens: Option<u32>) -> Budget {
        Budget {
            usd_per_day: usd,
            tokens_per_turn: tokens,
            max_tool_calls_per_turn: 4,
        }
    }

    #[test]
    fn an_empty_budget_never_cuts() {
        let store = BudgetStore::memory();
        assert!(store.check("a", &Budget::default(), 1_000_000).is_ok());
    }

    #[test]
    fn cuts_when_the_daily_ceiling_is_reached() {
        let store = BudgetStore::memory();
        let b = budget(Some(1.0), None);
        let now = day(2026, 9, 18);
        store.record_at("a", Some(0.60), 100, 100, now);
        assert!(store.check_at("a", &b, 0, now).is_ok());
        store.record_at("a", Some(0.50), 100, 100, now);
        let err = store.check_at("a", &b, 0, now).unwrap_err();
        assert_eq!(err.code, ERR_LLM_BUDGET);
        assert!(err.message.contains("1.0000"));
    }

    #[test]
    fn cuts_when_the_turn_is_bigger_than_the_token_ceiling() {
        let store = BudgetStore::memory();
        let b = budget(None, Some(8_000));
        assert!(store.check("a", &b, 7_999).is_ok());
        let err = store.check("a", &b, 8_001).unwrap_err();
        assert_eq!(err.code, ERR_LLM_BUDGET);
    }

    #[test]
    fn the_ceiling_is_per_agent() {
        let store = BudgetStore::memory();
        let b = budget(Some(0.10), None);
        let now = day(2026, 9, 18);
        store.record_at("a", Some(0.50), 0, 0, now);
        assert!(store.check_at("a", &b, 0, now).is_err());
        assert!(store.check_at("b", &b, 0, now).is_ok());
    }

    #[test]
    fn the_utc_day_turning_over_wipes_the_ledger() {
        let store = BudgetStore::memory();
        let b = budget(Some(1.0), None);
        store.record_at("a", Some(2.0), 0, 0, day(2026, 9, 18));
        assert!(store.check_at("a", &b, 0, day(2026, 9, 18)).is_err());
        assert!(store.check_at("a", &b, 0, day(2026, 9, 19)).is_ok());
        assert_eq!(store.spent_today_at("a", day(2026, 9, 19)).usd, 0.0);
    }

    #[test]
    fn record_accumulates_tokens_and_turns() {
        let store = BudgetStore::memory();
        let now = day(2026, 9, 18);
        store.record_at("a", Some(0.01), 100, 50, now);
        store.record_at("a", Some(0.02), 10, 5, now);
        let spend = store.spent_today_at("a", now);
        assert_eq!(spend.turns, 2);
        assert_eq!(spend.input_tokens, 110);
        assert_eq!(spend.output_tokens, 55);
        assert_eq!(spend.tokens(), 165);
        assert!((spend.usd - 0.03).abs() < 1e-9);
    }

    #[test]
    fn remaining_usd_never_goes_negative() {
        let store = BudgetStore::memory();
        store.record("a", Some(5.0), 0, 0);
        assert_eq!(
            store.remaining_usd("a", &budget(Some(1.0), None)),
            Some(0.0)
        );
        assert_eq!(store.remaining_usd("a", &budget(None, None)), None);
    }

    #[test]
    fn persists_and_reloads_from_disk() {
        let dir = std::env::temp_dir().join(format!("omniget-budget-{}", uuid::Uuid::new_v4()));
        let path = dir.join("llm").join("budget.json");
        let store = BudgetStore::at(&path);
        let now = Utc::now();
        store.record_at("a", Some(0.25), 10, 20, now);
        assert!(path.exists(), "record writes the file");
        let again = BudgetStore::at(&path);
        assert_eq!(again.spent_today_at("a", now).usd, 0.25);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_corrupt_file_starts_empty_instead_of_blocking() {
        let dir = std::env::temp_dir().join(format!("omniget-budget-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("budget.json");
        std::fs::write(&path, "{ not json").unwrap();
        let store = BudgetStore::at(&path);
        assert!(store.check("a", &budget(Some(1.0), None), 10).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// B04: two children near the limit; the reservation keeps the second
    /// out, whatever order they finish in, and cancelling frees the room.
    #[test]
    fn b04_reservations_keep_two_children_under_the_limit() {
        let store = BudgetStore::memory();
        // `settle` books at the real clock: a fixed day made this fail after 2026-09-24.
        let now = Utc::now();
        let usd = PoolLimits {
            usd: Some(1.0),
            ..Default::default()
        };
        store.record_at("room", Some(0.30), 0, 0, now);
        let a = store
            .reserve_at(
                "room",
                &usd,
                Estimate {
                    usd: Some(0.4),
                    tokens: 0,
                },
                now,
            )
            .unwrap();
        // 0.30 spent + 0.40 in flight + 0.40 > 1.0: refused.
        let err = store
            .reserve_at(
                "room",
                &usd,
                Estimate {
                    usd: Some(0.4),
                    tokens: 0,
                },
                now,
            )
            .unwrap_err();
        assert_eq!(err.code, ERR_LLM_BUDGET);
        assert_eq!(store.pool("room").in_flight, 1);
        // Cancel releases the room; the second child now fits.
        assert!(store.release(&a));
        assert!(!store.release(&a), "a reservation is released once");
        let b = store
            .reserve_at(
                "room",
                &usd,
                Estimate {
                    usd: Some(0.4),
                    tokens: 0,
                },
                now,
            )
            .unwrap();
        store.settle(&b, Some(0.35), 10, 10);
        assert!((store.spent_today_at("room", now).usd - 0.65).abs() < 1e-9);

        // Tokens: the same with a token pool.
        let tok = PoolLimits {
            tokens: Some(1_000),
            ..Default::default()
        };
        let x = store
            .reserve_at(
                "t",
                &tok,
                Estimate {
                    usd: None,
                    tokens: 600,
                },
                now,
            )
            .unwrap();
        assert!(store
            .reserve_at(
                "t",
                &tok,
                Estimate {
                    usd: None,
                    tokens: 600
                },
                now
            )
            .is_err());
        store.settle(&x, None, 300, 100);
        // Settled at 400 real tokens: 400 + 600 fits exactly.
        assert!(store
            .reserve_at(
                "t",
                &tok,
                Estimate {
                    usd: None,
                    tokens: 600
                },
                now
            )
            .is_ok());
    }

    #[test]
    fn unknown_cost_is_never_booked_as_free() {
        let store = BudgetStore::memory();
        // `settle` books at the real clock: a fixed day made this fail after 2026-09-24.
        let now = Utc::now();
        // An agent's own day: unknown-cost turns stay concurrent below the cap.
        let lax = PoolLimits {
            usd: Some(1.0),
            ..Default::default()
        };
        let x = store
            .reserve_at(
                "agent",
                &lax,
                Estimate {
                    usd: None,
                    tokens: 1,
                },
                now,
            )
            .unwrap();
        let y = store
            .reserve_at(
                "agent",
                &lax,
                Estimate {
                    usd: None,
                    tokens: 1,
                },
                now,
            )
            .unwrap();
        store.release(&x);
        store.release(&y);
        let usd = PoolLimits {
            usd: Some(1.0),
            strict_unknown: true,
            ..Default::default()
        };
        let a = store
            .reserve_at(
                "cli",
                &usd,
                Estimate {
                    usd: None,
                    tokens: 100,
                },
                now,
            )
            .unwrap();
        // A second unknown-cost turn under a USD cap with no alternative limit
        // is refused while the first is in flight.
        assert!(store
            .reserve_at(
                "cli",
                &usd,
                Estimate {
                    usd: None,
                    tokens: 100
                },
                now
            )
            .is_err());
        assert_eq!(store.pool("cli").reserved_unknown_cost, 1);
        store.settle(&a, None, 50, 50);
        let spent = store.spent_today_at("cli", now);
        assert_eq!(spent.usd, 0.0);
        assert_eq!(spent.unknown_cost_turns, 1);
        // With a turn limit as the alternative, unknown-cost turns run side by
        // side up to it.
        let turns = PoolLimits {
            usd: Some(1.0),
            turns: Some(3),
            ..Default::default()
        };
        let r1 = store
            .reserve_at("cli", &turns, Estimate::default(), now)
            .unwrap();
        let _r2 = store
            .reserve_at("cli", &turns, Estimate::default(), now)
            .unwrap();
        assert!(store
            .reserve_at("cli", &turns, Estimate::default(), now)
            .is_err());
        store.release(&r1);
    }

    /// F5: a mission pool is a lifetime ledger; the UTC day turning over
    /// gives nothing back. Agent day pools still roll.
    #[test]
    fn f5_mission_pools_do_not_reset_at_utc_midnight() {
        let store = BudgetStore::memory();
        let limits = PoolLimits {
            tokens: Some(1_000),
            strict_unknown: true,
            ..Default::default()
        };
        let d1 = Utc.with_ymd_and_hms(2026, 9, 25, 23, 50, 0).unwrap();
        let d2 = Utc.with_ymd_and_hms(2026, 9, 26, 0, 10, 0).unwrap();
        let r = store
            .reserve_at(
                "mission:m1",
                &limits,
                Estimate {
                    usd: None,
                    tokens: 900,
                },
                d1,
            )
            .unwrap();
        store.release(&r);
        store.record_at("mission:m1", None, 900, 0, d1);
        assert!(store
            .reserve_at(
                "mission:m1",
                &limits,
                Estimate {
                    usd: None,
                    tokens: 500
                },
                d1
            )
            .is_err());
        assert!(
            store
                .reserve_at(
                    "mission:m1",
                    &limits,
                    Estimate {
                        usd: None,
                        tokens: 500
                    },
                    d2
                )
                .is_err(),
            "midnight gives nothing back"
        );
        assert_eq!(store.spent_today_at("mission:m1", d2).tokens(), 900);
        // Agent day pools keep rolling.
        store.record_at("agent", Some(2.0), 0, 0, d1);
        assert_eq!(store.spent_today_at("agent", d2).usd, 0.0);
        // Reservations stay concurrent-safe on the lifetime pool.
        let a = store
            .reserve_at(
                "mission:m1",
                &limits,
                Estimate {
                    usd: None,
                    tokens: 60,
                },
                d2,
            )
            .unwrap();
        assert!(store
            .reserve_at(
                "mission:m1",
                &limits,
                Estimate {
                    usd: None,
                    tokens: 60
                },
                d2
            )
            .is_err());
        store.release(&a);
    }

    #[test]
    fn f5_lifetime_pools_persist_and_take_a_durable_floor() {
        let dir = std::env::temp_dir().join(format!("omniget-budget-{}", uuid::Uuid::new_v4()));
        let path = dir.join("budget.json");
        let store = BudgetStore::at(&path);
        store.record_at("mission:m", Some(0.1), 10, 10, day(2026, 9, 1));
        let again = BudgetStore::at(&path);
        assert_eq!(
            again.spent_today_at("mission:m", day(2026, 9, 30)).tokens(),
            20
        );
        // A lost ledger is re-seeded from the mission's own durable spend.
        let fresh = BudgetStore::memory();
        fresh.seed_lifetime(
            "mission:m",
            &AgentSpend {
                usd: 0.3,
                input_tokens: 500,
                turns: 2,
                ..Default::default()
            },
        );
        let v = fresh.pool("mission:m");
        assert_eq!(v.spent.tokens(), 500);
        assert!((v.spent.usd - 0.3).abs() < 1e-9);
        fresh.seed_lifetime(
            "agent",
            &AgentSpend {
                usd: 9.0,
                ..Default::default()
            },
        );
        assert_eq!(
            fresh.pool("agent").spent.usd,
            0.0,
            "day pools are never seeded"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// F6: under a USD-only strict cap, unknown-cost turns are bounded.
    #[test]
    fn f6_unknown_cost_turns_are_bounded_under_a_usd_only_cap() {
        let store = BudgetStore::memory();
        let l = PoolLimits {
            usd: Some(0.5),
            strict_unknown: true,
            ..Default::default()
        };
        for _ in 0..STRICT_UNKNOWN_COST_TURNS_MAX {
            let r = store
                .reserve(
                    "mission:u",
                    &l,
                    Estimate {
                        usd: None,
                        tokens: 5_000,
                    },
                )
                .unwrap();
            store.settle(&r, None, 4_000, 1_000);
        }
        let err = store
            .reserve(
                "mission:u",
                &l,
                Estimate {
                    usd: None,
                    tokens: 5_000,
                },
            )
            .unwrap_err();
        assert!(err.message.contains("unknown cost"), "{}", err.message);
        assert!(
            !store.has_in_flight("mission:u"),
            "a final refusal: nothing in flight to wait for"
        );
    }

    #[test]
    fn the_check_costs_well_under_a_hundred_microseconds() {
        let store = BudgetStore::memory();
        let b = budget(Some(10.0), Some(100_000));
        store.record("a", Some(0.1), 10, 10);
        let start = std::time::Instant::now();
        let n = 10_000;
        for _ in 0..n {
            store.check("a", &b, 1_000).unwrap();
        }
        let per = start.elapsed() / n;
        assert!(
            per < std::time::Duration::from_micros(100),
            "budget check took {per:?} per call"
        );
    }
}
