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

pub mod policy;
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
