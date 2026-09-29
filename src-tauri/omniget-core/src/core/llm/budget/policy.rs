//! Budget policies with a scope and a consequence (roadmap 3.3, S18,
//! decision D5; paperclip P3/P4 `80#P3`/`80#P4`, omnigent P6 `85#P6`).
//!
//! The day ledger of [`super::BudgetStore`] cuts a turn before it is sent,
//! but it knows one scope (an agent, or a mission's lifetime pool) and one
//! metric (dollars the provider reported). This module adds the ceilings a
//! person writes down — "this loop may spend US$ 0.05", "at most 5 % of this
//! account's 5 h window", "2 M weighted tokens a day for everything" — and
//! what happens when one is crossed:
//!
//! * **hard** (`hard_stop`): the scope is paused, the work running in it is
//!   cancelled by whoever owns it (the app listens for
//!   [`BusEvent::BudgetCrossed`](crate::core::omni::bus::BusEvent)), and an
//!   **incident** opens with two answers: raise and resume, or keep paused;
//! * **soft**: the ceiling is a checkpoint. The next turn asks "continue?"
//!   through the durable approvals (2.2), once per window, and the answer is
//!   remembered;
//! * `warn_pct`: a notice and an audit line; nothing stops.
//!
//! The check runs before every turn (a paused scope starts nothing) and the
//! crossing is evaluated when the turn settles, so a scope passes its ceiling
//! by at most the one turn that crossed it (PROB C1).
//!
//! Metrics (D5):
//! * `usd`: what the provider reported, else the price table's equivalent.
//!   For a subscription account those dollars are an estimate, and a ceiling
//!   a person set on them still bites (`85#P6`, PROB C2). A model the table
//!   does not know asks once instead of counting as free;
//! * `weighted_tokens`: `billable_input` + output, the weights the mission
//!   caps already use (cache read 0.1x, cache write 1.25x);
//! * `quota_pct`: the share of an account's 5 h / 7 d window, read only from
//!   what the CLI reported on its own stream (`rate_limit_event`, the status
//!   line). Never the OAuth usage endpoint, never a credential
//!   (`cli_usage/windows.rs`). Without a reading the policy cannot bite, and
//!   says so instead of inventing a percentage.
//!
//! Nothing here acts without a policy: no policy, no price lookup, no
//! question, no refusal (B0 unchanged). A turn's ledger line is still
//! written, because that is how a loop shows what it cost (PROB C5).
//!
//! State: `<llm dir>/budget.db` — the policies, one ledger row per scope a
//! turn touched (so two policies never add the same usage twice, and one
//! policy counts it once), the incidents, the answers to checkpoints and the
//! quota baselines.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use chrono::{Datelike, TimeZone, Utc};
use futures::future::BoxFuture;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};

use crate::core::llm::audit::{self, Actor, Entity};
use crate::core::llm::error::{LlmError, ERR_LLM_BUDGET};

pub const DB_FILE: &str = "budget.db";
/// Ledger rows older than this are dropped when the file opens.
const LEDGER_KEEP_MS: i64 = 120 * 24 * 3_600 * 1_000;
const HOUR_MS: i64 = 3_600 * 1_000;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ── the vocabulary ──────────────────────────────────────────────────────

/// Who a ceiling is for. Written `global`, `agent:<id>`, `job:<id>`,
/// `loop:<id>`, `mission:<id>`, `account:<id>` everywhere (API, CLI, file).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    Global,
    Agent(String),
    Job(String),
    Loop(String),
    Mission(String),
    /// A CLI account (its id) or a native provider (`provider:<id>`).
    Account(String),
}

impl Scope {
    pub fn key(&self) -> String {
        match self {
            Scope::Global => "global".into(),
            Scope::Agent(id) => format!("agent:{id}"),
            Scope::Job(id) => format!("job:{id}"),
            Scope::Loop(id) => format!("loop:{id}"),
            Scope::Mission(id) => format!("mission:{id}"),
            Scope::Account(id) => format!("account:{id}"),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Scope::Global => "global",
            Scope::Agent(_) => "agent",
            Scope::Job(_) => "job",
            Scope::Loop(_) => "loop",
            Scope::Mission(_) => "mission",
            Scope::Account(_) => "account",
        }
    }

    /// The id after the colon (`""` for global).
    pub fn id(&self) -> &str {
        match self {
            Scope::Global => "",
            Scope::Agent(id)
            | Scope::Job(id)
            | Scope::Loop(id)
            | Scope::Mission(id)
            | Scope::Account(id) => id,
        }
    }

    pub fn parse(raw: &str) -> Result<Scope, String> {
        let raw = raw.trim();
        if raw == "global" {
            return Ok(Scope::Global);
        }
        let (kind, id) = raw
            .split_once(':')
            .ok_or_else(|| format!("{ERR_LLM_BUDGET}: scope `{raw}` is not global or kind:id"))?;
        if id.trim().is_empty() || id.len() > 200 {
            return Err(format!("{ERR_LLM_BUDGET}: scope `{raw}` has no id"));
        }
        let id = id.trim().to_string();
        Ok(match kind {
            "agent" => Scope::Agent(id),
            "job" => Scope::Job(id),
            "loop" => Scope::Loop(id),
            "mission" => Scope::Mission(id),
            "account" => Scope::Account(id),
            other => {
                return Err(format!(
                    "{ERR_LLM_BUDGET}: unknown scope kind `{other}` (global, agent, job, loop, mission, account)"
                ))
            }
        })
    }

    /// The scopes whose life is the work itself: their `run` window starts
    /// with them, not with the policy.
    fn is_work(&self) -> bool {
        matches!(self, Scope::Job(_) | Scope::Loop(_) | Scope::Mission(_))
    }
}

impl Serialize for Scope {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.key())
    }
}

impl<'de> Deserialize<'de> for Scope {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Scope::parse(&raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    Usd,
    WeightedTokens,
    QuotaPct,
}

impl Metric {
    pub fn as_str(self) -> &'static str {
        match self {
            Metric::Usd => "usd",
            Metric::WeightedTokens => "weighted_tokens",
            Metric::QuotaPct => "quota_pct",
        }
    }

    pub fn parse(raw: &str) -> Result<Metric, String> {
        match raw.trim() {
            "usd" => Ok(Metric::Usd),
            "weighted_tokens" | "tokens" => Ok(Metric::WeightedTokens),
            "quota_pct" | "quota" => Ok(Metric::QuotaPct),
            other => Err(format!(
                "{ERR_LLM_BUDGET}: unknown metric `{other}` (usd, weighted_tokens, quota_pct)"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Window {
    #[serde(rename = "turn")]
    Turn,
    #[serde(rename = "day")]
    Day,
    #[serde(rename = "week")]
    Week,
    #[serde(rename = "5h")]
    FiveHours,
    #[serde(rename = "7d")]
    SevenDays,
    #[serde(rename = "run")]
    Run,
}

impl Window {
    pub fn as_str(self) -> &'static str {
        match self {
            Window::Turn => "turn",
            Window::Day => "day",
            Window::Week => "week",
            Window::FiveHours => "5h",
            Window::SevenDays => "7d",
            Window::Run => "run",
        }
    }

    pub fn parse(raw: &str) -> Result<Window, String> {
        match raw.trim() {
            "turn" => Ok(Window::Turn),
            "day" => Ok(Window::Day),
            "week" => Ok(Window::Week),
            "5h" => Ok(Window::FiveHours),
            "7d" => Ok(Window::SevenDays),
            "run" => Ok(Window::Run),
            other => Err(format!(
                "{ERR_LLM_BUDGET}: unknown window `{other}` (turn, day, week, 5h, 7d, run)"
            )),
        }
    }

    /// Where the ledger sum of a `usd`/`weighted_tokens` policy starts: UTC
    /// day and ISO week, the rolling 5 h / 7 d, or the scope's whole life.
    fn start_ms(self, now: i64, policy: &BudgetPolicy) -> i64 {
        let today = Utc
            .timestamp_millis_opt(now)
            .single()
            .unwrap_or_else(Utc::now)
            .date_naive();
        let midnight = |d: chrono::NaiveDate| {
            d.and_hms_opt(0, 0, 0)
                .map(|t| t.and_utc().timestamp_millis())
                .unwrap_or(0)
        };
        match self {
            Window::Turn => now,
            Window::Day => midnight(today),
            Window::Week => midnight(
                today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64),
            ),
            Window::FiveHours => now - 5 * HOUR_MS,
            Window::SevenDays => now - 7 * 24 * HOUR_MS,
            Window::Run if policy.scope.is_work() => 0,
            Window::Run => policy.created_ms,
        }
    }

    /// The window a checkpoint answer holds for: one answer per day, week,
    /// 5 h / 7 d slice, or for the whole run.
    fn instance(self, now: i64) -> String {
        let day = Utc
            .timestamp_millis_opt(now)
            .single()
            .unwrap_or_else(Utc::now);
        match self {
            Window::Turn => format!("t{now}"),
            Window::Day => day.format("%Y-%m-%d").to_string(),
            Window::Week => {
                let w = day.iso_week();
                format!("{}-W{:02}", w.year(), w.week())
            }
            Window::FiveHours => format!("5h{}", now / (5 * HOUR_MS)),
            Window::SevenDays => format!("7d{}", now / (7 * 24 * HOUR_MS)),
            Window::Run => "run".into(),
        }
    }
}

fn yes() -> bool {
    true
}

/// One ceiling (D5): `{scope, metric, window, amount, warn_pct, hard_stop}`.
/// One policy per (scope, metric, window): saving another replaces it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetPolicy {
    #[serde(default)]
    pub id: String,
    pub scope: Scope,
    pub metric: Metric,
    pub window: Window,
    /// US$, weighted tokens, or percentage points of the quota window.
    pub amount: f64,
    /// Notice (and audit line) when this share of `amount` is passed.
    #[serde(default)]
    pub warn_pct: Option<f64>,
    /// `true` = pause and open an incident; `false` = ask "continue?" once
    /// per window.
    #[serde(default = "yes")]
    pub hard_stop: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub created_ms: i64,
    #[serde(default)]
    pub updated_ms: i64,
}

impl BudgetPolicy {
    pub fn validate(&self) -> Result<(), String> {
        if !self.amount.is_finite() || self.amount <= 0.0 {
            return Err(format!("{ERR_LLM_BUDGET}: the amount must be above zero"));
        }
        if let Some(w) = self.warn_pct {
            if !(w > 0.0 && w < 100.0) {
                return Err(format!(
                    "{ERR_LLM_BUDGET}: warn_pct is a share of the amount, between 0 and 100"
                ));
            }
        }
        if self.metric == Metric::QuotaPct {
            if !matches!(self.window, Window::FiveHours | Window::SevenDays) {
                return Err(format!(
                    "{ERR_LLM_BUDGET}: quota_pct reads an account's 5h or 7d window"
                ));
            }
            if self.amount > 100.0 {
                return Err(format!(
                    "{ERR_LLM_BUDGET}: quota_pct is at most 100 percentage points"
                ));
            }
        }
        Ok(())
    }

    fn unit(&self) -> &'static str {
        match self.metric {
            Metric::Usd => "USD",
            Metric::WeightedTokens => "weighted tokens",
            Metric::QuotaPct => "% of the window",
        }
    }

    fn describe(&self, observed: f64) -> String {
        let fmt = |v: f64| match self.metric {
            Metric::Usd => format!("{v:.4}"),
            Metric::WeightedTokens => format!("{}", v.round() as i64),
            Metric::QuotaPct => format!("{v:.1}"),
        };
        format!(
            "{} {} of {} {} ({})",
            self.scope.key(),
            fmt(observed),
            fmt(self.amount),
            self.unit(),
            self.window.as_str()
        )
    }
}

/// A hard ceiling that was crossed (or a checkpoint someone refused): the
/// scope stays paused while it is `open` (needs someone) or `paused` (kept
/// paused on purpose); `resolved` lets the work go on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Incident {
    pub id: String,
    pub policy_id: String,
    pub scope: Scope,
    pub metric: Metric,
    pub window: Window,
    pub observed: f64,
    pub limit: f64,
    /// `open | paused | resolved`
    pub state: String,
    /// `hard` (the ceiling) or `checkpoint` (a "continue?" answered no).
    pub kind: String,
    /// The agent whose turn crossed it.
    #[serde(default)]
    pub agent: Option<String>,
    pub opened_ms: i64,
    #[serde(default)]
    pub resolved_ms: Option<i64>,
    /// `raised:<amount>`, `kept_paused`, `window_reset`, `policy_removed`.
    #[serde(default)]
    pub resolution: Option<String>,
}

impl Incident {
    pub fn blocks(&self) -> bool {
        self.state == "open" || self.state == "paused"
    }
}

/// Why a turn may not start.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Block {
    pub policy_id: String,
    pub scope: Scope,
    pub metric: Metric,
    pub window: Window,
    pub observed: f64,
    pub limit: f64,
    pub incident: Option<String>,
    pub message: String,
}

impl Block {
    pub fn error(&self) -> LlmError {
        LlmError::new(ERR_LLM_BUDGET, self.message.clone())
    }
}

/// A question the gate needs answered before the turn goes out.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AskNeed {
    /// `checkpoint` or `unpriced`.
    pub kind: &'static str,
    pub policy_id: String,
    pub scope: Scope,
    /// The remembered answer's key.
    pub key: String,
    pub observed: f64,
    pub limit: f64,
    /// What the question says (no secret, no prompt).
    pub message: String,
}

impl AskNeed {
    /// The "tool" name the approval shows.
    pub fn action(&self) -> &'static str {
        match self.kind {
            "checkpoint" => "budget_checkpoint",
            _ => "budget_unpriced_model",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Allow,
    Deny(Block),
    Ask(Vec<AskNeed>),
}

/// A ceiling passed while a turn settled.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Crossing {
    /// `warn` or `hard`.
    pub kind: &'static str,
    pub policy: BudgetPolicy,
    pub observed: f64,
    pub incident: Option<Incident>,
}

/// One turn, as the gate and the settle see it.
#[derive(Debug, Clone, Default)]
pub struct TurnCtx {
    pub request_id: String,
    pub agent_id: String,
    /// The CLI account the turn runs on, or `provider:<id>` for a native
    /// model; `None` when unknown.
    pub account: Option<String>,
    pub model: String,
    pub conversation_id: String,
    /// The history's estimated size (a `turn` token ceiling checks it).
    pub estimate_tokens: u64,
}

impl TurnCtx {
    /// Every scope this turn counts against: global, its agent, its account,
    /// and whatever the owner of the conversation bound (job, loop, mission).
    pub fn scopes(&self) -> Vec<Scope> {
        let mut out = vec![Scope::Global, Scope::Agent(self.agent_id.clone())];
        if let Some(a) = &self.account {
            out.push(Scope::Account(a.clone()));
        }
        for s in bound_scopes(&self.conversation_id) {
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out
    }
}

/// What a finished turn used.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TurnUse {
    /// What the provider reported; `None` = it reported no price.
    pub usd: Option<f64>,
    /// `billable_input` (cache-weighted).
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl TurnUse {
    pub fn weighted(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

/// A scope's spend inside a window, for the loop cards and the budgets page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct Spend {
    /// Reported plus priced dollars.
    pub usd: f64,
    /// The part of `usd` that came from the price table (an estimate).
    pub usd_estimated: f64,
    /// Turns whose cost nobody knows (no report, no price).
    pub unpriced_turns: u32,
    pub tokens: u64,
    pub turns: u32,
}

/// Price per million tokens (input, output).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Price {
    pub input_per_m: f64,
    pub output_per_m: f64,
}

/// One quota window of an account, as its CLI last reported it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct QuotaWindow {
    pub used_pct: f64,
    /// Epoch seconds when the window rolls over.
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct QuotaReading {
    pub five_hour: Option<QuotaWindow>,
    pub seven_day: Option<QuotaWindow>,
}

impl QuotaReading {
    fn window(&self, w: Window) -> Option<QuotaWindow> {
        match w {
            Window::FiveHours => self.five_hour,
            Window::SevenDays => self.seven_day,
            _ => None,
        }
    }
}

/// Reads the last quota an account's CLI reported (the app: the in-memory
/// capacity fed by `rate_limit_event`). Never a network call.
pub type QuotaFn = Arc<dyn Fn(&str) -> Option<QuotaReading> + Send + Sync>;
/// Price of a model name (the app: the LiteLLM table, cached on disk).
pub type PriceFn = Arc<dyn Fn(String) -> BoxFuture<'static, Option<Price>> + Send + Sync>;
/// Told about every change (incident opened or settled, warning, policy
/// saved): the app turns it into `llm://budget`.
pub type Listener = Arc<dyn Fn(&BudgetEvent) + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BudgetEvent {
    /// `incident_opened | incident_changed | warn | policy_saved | policy_removed`
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub incident: Option<Incident>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<BudgetPolicy>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed: Option<f64>,
}

/// What a person decided about an incident.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// A new ceiling, above what is already spent; the scope resumes.
    Raise(f64),
    KeepPaused,
}

/// A policy with where it stands now, for the budgets page and the CLI.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PolicyStatus {
    #[serde(flatten)]
    pub policy: BudgetPolicy,
    /// `None` = cannot be measured now (a quota policy with no reading).
    pub observed: Option<f64>,
    /// `real` for a CLI's own report, `none` without one (quota only).
    pub source: &'static str,
    pub incident: Option<Incident>,
}

// ── scope bindings (who a conversation's turns belong to) ──────────────

type Bound = HashMap<String, Vec<(u64, Scope)>>;

fn bindings() -> &'static Mutex<Bound> {
    static B: OnceLock<Mutex<Bound>> = OnceLock::new();
    B.get_or_init(Default::default)
}

static NEXT_BINDING: AtomicU64 = AtomicU64::new(1);

/// While alive, the turns of `conversation` also count against `scopes`
/// (a job binds `job:<id>` and its `loop:<id>`, a mission its
/// `mission:<id>`). Dropped with the owner, however it returns.
pub struct Binding {
    conversation: String,
    id: u64,
}

impl Binding {
    pub fn new(conversation: &str, scopes: Vec<Scope>) -> Self {
        let id = NEXT_BINDING.fetch_add(1, Ordering::Relaxed);
        let mut map = bindings().lock().unwrap_or_else(|e| e.into_inner());
        let list = map.entry(conversation.to_string()).or_default();
        for s in scopes {
            list.push((id, s));
        }
        Self {
            conversation: conversation.to_string(),
            id,
        }
    }
}

impl Drop for Binding {
    fn drop(&mut self) {
        let mut map = bindings().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(list) = map.get_mut(&self.conversation) {
            list.retain(|(id, _)| *id != self.id);
            if list.is_empty() {
                map.remove(&self.conversation);
            }
        }
    }
}

pub fn bound_scopes(conversation: &str) -> Vec<Scope> {
    let map = bindings().lock().unwrap_or_else(|e| e.into_inner());
    let mut out: Vec<Scope> = Vec::new();
    for (_, s) in map.get(conversation).into_iter().flatten() {
        if !out.contains(s) {
            out.push(s.clone());
        }
    }
    out
}

/// The conversations whose turns count against `scope` right now (what a
/// hard crossing cancels when the scope is a mission or a job).
pub fn conversations_in(scope: &Scope) -> Vec<String> {
    let map = bindings().lock().unwrap_or_else(|e| e.into_inner());
    map.iter()
        .filter(|(_, list)| list.iter().any(|(_, s)| s == scope))
        .map(|(c, _)| c.clone())
        .collect()
}

// ── the global engine and its hooks ─────────────────────────────────────

static ENGINE: OnceLock<Arc<Engine>> = OnceLock::new();
static QUOTA: RwLock<Option<QuotaFn>> = RwLock::new(None);
static PRICE: RwLock<Option<PriceFn>> = RwLock::new(None);
static LISTENER: RwLock<Option<Listener>> = RwLock::new(None);

/// Opens `<llm dir>/budget.db` once for the app.
pub fn init(path: &Path) -> Result<Arc<Engine>, String> {
    if let Some(e) = ENGINE.get() {
        return Ok(e.clone());
    }
    let engine = Arc::new(Engine::open(path)?);
    let _ = ENGINE.set(engine);
    Ok(ENGINE.get().cloned().expect("set above"))
}

/// The app's engine, if the app opened one (tests and the CLI have none).
pub fn active() -> Option<Arc<Engine>> {
    ENGINE.get().cloned()
}

/// Where the app's quota readings come from (set when the CLI runtime is
/// built; an engine of its own in a test sets its own).
pub fn set_quota_source(f: QuotaFn) {
    *QUOTA.write().unwrap_or_else(|e| e.into_inner()) = Some(f);
}

/// The quota source over the CLI runtime's capacity: the last
/// `rate_limit_event` (or status line) each account's CLI reported, in
/// percent. Only [`QuotaSource::Real`](crate::core::llm::cli_runtime::QuotaSource)
/// readings count: an estimate against a plan ceiling is not a quota.
pub fn quota_from_capacity(capacity: Arc<crate::core::llm::cli_runtime::CliCapacity>) -> QuotaFn {
    Arc::new(move |account: &str| {
        let snap = capacity.get(account)?;
        if snap.source != crate::core::llm::cli_runtime::QuotaSource::Real {
            return None;
        }
        let w = |r: Option<crate::core::llm::cli_runtime::RateWindow>| {
            r.map(|r| QuotaWindow {
                used_pct: (r.used as f64 * 1000.0).round() / 10.0,
                resets_at: r.resets_at,
            })
        };
        Some(QuotaReading {
            five_hour: w(snap.window_5h),
            seven_day: w(snap.window_7d),
        })
    })
}

pub fn set_price_source(f: PriceFn) {
    *PRICE.write().unwrap_or_else(|e| e.into_inner()) = Some(f);
}

pub fn set_listener(f: Listener) {
    *LISTENER.write().unwrap_or_else(|e| e.into_inner()) = Some(f);
}

// ── the engine ──────────────────────────────────────────────────────────

pub struct Engine {
    db: Mutex<Connection>,
    /// Read before every turn; the file is only read when it changes.
    policies: RwLock<Vec<BudgetPolicy>>,
    quota: RwLock<Option<QuotaFn>>,
    price: RwLock<Option<PriceFn>>,
    listener: RwLock<Option<Listener>>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("policies", &self.policies().len())
            .finish()
    }
}

fn new_id(prefix: &str) -> String {
    format!(
        "{prefix}{}",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    )
}

impl Engine {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        Self::with(Connection::open(path).map_err(|e| e.to_string())?)
    }

    /// No file: tests.
    pub fn memory() -> Self {
        Self::with(Connection::open_in_memory().expect("in-memory sqlite")).expect("schema")
    }

    fn with(db: Connection) -> Result<Self, String> {
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             CREATE TABLE IF NOT EXISTS budget_policies (
                id TEXT PRIMARY KEY,
                scope TEXT NOT NULL,
                metric TEXT NOT NULL,
                win TEXT NOT NULL,
                body TEXT NOT NULL,
                UNIQUE(scope, metric, win));
             CREATE TABLE IF NOT EXISTS budget_ledger (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                ts_ms INTEGER NOT NULL,
                scope TEXT NOT NULL,
                request_id TEXT NOT NULL,
                agent TEXT NOT NULL,
                account TEXT,
                model TEXT NOT NULL,
                usd REAL,
                usd_source TEXT NOT NULL,
                tokens INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS budget_ledger_scope ON budget_ledger(scope, ts_ms);
             CREATE INDEX IF NOT EXISTS budget_ledger_model ON budget_ledger(model, usd_source);
             CREATE TABLE IF NOT EXISTS budget_incidents (
                id TEXT PRIMARY KEY,
                policy_id TEXT NOT NULL,
                state TEXT NOT NULL,
                opened_ms INTEGER NOT NULL,
                body TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS budget_incidents_state ON budget_incidents(state);
             CREATE TABLE IF NOT EXISTS budget_acks (
                key TEXT PRIMARY KEY,
                allow INTEGER NOT NULL,
                ts_ms INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS budget_quota (
                policy_id TEXT NOT NULL,
                account TEXT NOT NULL,
                baseline REAL NOT NULL,
                last REAL NOT NULL,
                carried REAL NOT NULL,
                resets_at INTEGER,
                PRIMARY KEY(policy_id, account));",
        )
        .map_err(|e| e.to_string())?;
        let _ = db.execute(
            "DELETE FROM budget_ledger WHERE ts_ms < ?1",
            params![now_ms() - LEDGER_KEEP_MS],
        );
        let engine = Self {
            db: Mutex::new(db),
            policies: RwLock::new(Vec::new()),
            quota: RwLock::new(None),
            price: RwLock::new(None),
            listener: RwLock::new(None),
        };
        engine.reload();
        Ok(engine)
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn reload(&self) {
        let list: Vec<BudgetPolicy> = {
            let db = self.db();
            let mut stmt = match db.prepare("SELECT body FROM budget_policies ORDER BY rowid") {
                Ok(s) => s,
                Err(_) => return,
            };
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map(|it| it.flatten().collect::<Vec<_>>())
                .unwrap_or_default();
            rows.iter()
                .filter_map(|b| serde_json::from_str(b).ok())
                .collect()
        };
        *self.policies.write().unwrap_or_else(|e| e.into_inner()) = list;
    }

    /// This engine's own quota source (tests); the app sets the global one.
    pub fn set_quota(&self, f: QuotaFn) {
        *self.quota.write().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    pub fn set_price(&self, f: PriceFn) {
        *self.price.write().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    pub fn set_listener(&self, f: Listener) {
        *self.listener.write().unwrap_or_else(|e| e.into_inner()) = Some(f);
    }

    fn quota_of(&self, account: &str) -> Option<QuotaReading> {
        let own = self.quota.read().unwrap_or_else(|e| e.into_inner()).clone();
        let f = own.or_else(|| QUOTA.read().unwrap_or_else(|e| e.into_inner()).clone())?;
        f(account)
    }

    async fn price_of(&self, model: &str) -> Option<Price> {
        let own = self.price.read().unwrap_or_else(|e| e.into_inner()).clone();
        let f = own.or_else(|| PRICE.read().unwrap_or_else(|e| e.into_inner()).clone())?;
        f(model.to_string()).await
    }

    fn tell(&self, event: BudgetEvent) {
        let own = self
            .listener
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(f) = own.or_else(|| LISTENER.read().unwrap_or_else(|e| e.into_inner()).clone())
        {
            f(&event);
        }
    }

    /// The fast path of every turn: with no enabled policy nothing else runs.
    pub fn has_policies(&self) -> bool {
        self.policies
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .any(|p| p.enabled)
    }

    pub fn policies(&self) -> Vec<BudgetPolicy> {
        self.policies
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn policy(&self, id: &str) -> Option<BudgetPolicy> {
        self.policies().into_iter().find(|p| p.id == id)
    }

    fn applicable(&self, scopes: &[Scope]) -> Vec<BudgetPolicy> {
        self.policies()
            .into_iter()
            .filter(|p| p.enabled && scopes.contains(&p.scope))
            .collect()
    }

    /// Saves a policy. One per (scope, metric, window): the same triple
    /// again replaces the old one (and keeps its id and creation time).
    pub fn upsert(&self, mut p: BudgetPolicy, actor: Actor) -> Result<BudgetPolicy, String> {
        p.validate()?;
        let now = now_ms();
        let existing = self.policies().into_iter().find(|x| {
            (!p.id.is_empty() && x.id == p.id)
                || (x.scope == p.scope && x.metric == p.metric && x.window == p.window)
        });
        match &existing {
            Some(x) => {
                p.id = x.id.clone();
                p.created_ms = x.created_ms;
            }
            None => {
                if p.id.is_empty() {
                    p.id = new_id("bp_");
                }
                p.created_ms = now;
            }
        }
        p.updated_ms = now;
        let body = serde_json::to_string(&p).map_err(|e| e.to_string())?;
        {
            let db = self.db();
            db.execute("DELETE FROM budget_policies WHERE id=?1", params![p.id])
                .map_err(|e| e.to_string())?;
            db.execute(
                "INSERT OR REPLACE INTO budget_policies (id, scope, metric, win, body) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![p.id, p.scope.key(), p.metric.as_str(), p.window.as_str(), body],
            )
            .map_err(|e| e.to_string())?;
        }
        self.reload();
        audit::log(
            actor,
            "budget.policy_set",
            Entity::budget(&p.id),
            json!({
                "scope": p.scope.key(), "metric": p.metric.as_str(), "window": p.window.as_str(),
                "amount": p.amount, "hard_stop": p.hard_stop, "enabled": p.enabled,
            }),
        );
        self.tell(BudgetEvent {
            kind: "policy_saved",
            incident: None,
            policy: Some(p.clone()),
            observed: None,
        });
        Ok(p)
    }

    /// Removes a policy; an incident it had stops blocking.
    pub fn delete(&self, id: &str, actor: Actor) -> Result<bool, String> {
        let n = self
            .db()
            .execute("DELETE FROM budget_policies WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        self.reload();
        if n > 0 {
            for inc in self.incidents(true) {
                if inc.policy_id == id {
                    self.settle_incident(inc, "resolved", "policy_removed");
                }
            }
            audit::log(
                actor,
                "budget.policy_removed",
                Entity::budget(id),
                json!({}),
            );
            self.tell(BudgetEvent {
                kind: "policy_removed",
                incident: None,
                policy: None,
                observed: None,
            });
        }
        Ok(n > 0)
    }

    // ── incidents ──────────────────────────────────────────────────────

    /// Incidents, newest first; `blocking_only` = open or kept paused.
    pub fn incidents(&self, blocking_only: bool) -> Vec<Incident> {
        let db = self.db();
        let sql = if blocking_only {
            "SELECT body FROM budget_incidents WHERE state IN ('open','paused') ORDER BY opened_ms DESC"
        } else {
            "SELECT body FROM budget_incidents ORDER BY opened_ms DESC LIMIT 200"
        };
        let Ok(mut stmt) = db.prepare(sql) else {
            return Vec::new();
        };
        stmt.query_map([], |r| r.get::<_, String>(0))
            .map(|it| {
                it.flatten()
                    .filter_map(|b| serde_json::from_str(&b).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn incident(&self, id: &str) -> Option<Incident> {
        let body: Option<String> = self
            .db()
            .query_row(
                "SELECT body FROM budget_incidents WHERE id=?1",
                params![id],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten();
        body.and_then(|b| serde_json::from_str(&b).ok())
    }

    fn save_incident(&self, inc: &Incident) {
        let Ok(body) = serde_json::to_string(inc) else {
            return;
        };
        let _ = self.db().execute(
            "INSERT OR REPLACE INTO budget_incidents (id, policy_id, state, opened_ms, body) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![inc.id, inc.policy_id, inc.state, inc.opened_ms, body],
        );
    }

    fn settle_incident(&self, mut inc: Incident, state: &str, resolution: &str) -> Incident {
        inc.state = state.to_string();
        inc.resolution = Some(resolution.to_string());
        if state == "resolved" {
            inc.resolved_ms = Some(now_ms());
        }
        self.save_incident(&inc);
        self.tell(BudgetEvent {
            kind: "incident_changed",
            incident: Some(inc.clone()),
            policy: None,
            observed: None,
        });
        inc
    }

    /// Opens the incident of a crossed policy, or returns the one it has.
    fn open_incident(
        &self,
        p: &BudgetPolicy,
        observed: f64,
        kind: &str,
        agent: Option<&str>,
    ) -> Incident {
        if let Some(i) = self
            .incidents(true)
            .into_iter()
            .find(|i| i.policy_id == p.id)
        {
            return i;
        }
        let inc = Incident {
            id: new_id("bi_"),
            policy_id: p.id.clone(),
            scope: p.scope.clone(),
            metric: p.metric,
            window: p.window,
            observed,
            limit: p.amount,
            state: if kind == "checkpoint" {
                "paused".into()
            } else {
                "open".into()
            },
            kind: kind.to_string(),
            agent: agent.map(String::from),
            opened_ms: now_ms(),
            resolved_ms: None,
            resolution: None,
        };
        self.save_incident(&inc);
        audit::log(
            Actor::system("budget"),
            "budget.hit",
            Entity::budget(&inc.id),
            json!({
                "scope": p.scope.key(), "metric": p.metric.as_str(), "window": p.window.as_str(),
                "observed": observed, "limit": p.amount, "policy": p.id, "kind": kind,
                "agent": agent,
            }),
        );
        self.tell(BudgetEvent {
            kind: "incident_opened",
            incident: Some(inc.clone()),
            policy: Some(p.clone()),
            observed: Some(observed),
        });
        inc
    }

    /// "Raise and resume" (the new ceiling must be above what is already
    /// spent) or "keep paused". Returns the settled incident; the caller
    /// resumes the work of its scope.
    pub fn resolve(&self, id: &str, what: Resolution, actor: Actor) -> Result<Incident, String> {
        let inc = self
            .incident(id)
            .ok_or_else(|| format!("{ERR_LLM_BUDGET}: no incident {id}"))?;
        if !inc.blocks() {
            return Err(format!(
                "{ERR_LLM_BUDGET}: incident {id} is already {}",
                inc.state
            ));
        }
        match what {
            Resolution::KeepPaused => {
                audit::log(
                    actor,
                    "budget.kept_paused",
                    Entity::budget(id),
                    json!({ "scope": inc.scope.key() }),
                );
                Ok(self.settle_incident(inc, "paused", "kept_paused"))
            }
            Resolution::Raise(amount) => {
                let mut p = self.policy(&inc.policy_id).ok_or_else(|| {
                    format!("{ERR_LLM_BUDGET}: the policy of incident {id} is gone")
                })?;
                let account = self.last_account(&p.scope);
                let spent = self
                    .observe(&p, account.as_deref(), now_ms())
                    .unwrap_or(inc.observed);
                if !(amount.is_finite() && amount > spent.max(inc.observed) + 1e-12) {
                    return Err(format!(
                        "{ERR_LLM_BUDGET}: raise above what is already spent ({})",
                        p.describe(spent.max(inc.observed))
                    ));
                }
                let before = p.amount;
                p.amount = amount;
                self.upsert(p.clone(), actor.clone())?;
                audit::log(
                    actor,
                    "budget.raised",
                    Entity::budget(id),
                    json!({ "scope": p.scope.key(), "from": before, "to": amount, "policy": p.id }),
                );
                Ok(self.settle_incident(inc, "resolved", &format!("raised:{amount}")))
            }
        }
    }

    // ── measuring ──────────────────────────────────────────────────────

    /// The account a scope's turns ran on most recently (for a quota
    /// policy on a loop or an agent).
    pub fn last_account(&self, scope: &Scope) -> Option<String> {
        if let Scope::Account(a) = scope {
            return Some(a.clone());
        }
        self.db()
            .query_row(
                "SELECT account FROM budget_ledger WHERE scope=?1 AND account IS NOT NULL ORDER BY ts_ms DESC, id DESC LIMIT 1",
                params![scope.key()],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .ok()
            .flatten()
            .flatten()
    }

    /// Sum of a scope's ledger since `since_ms`.
    pub fn spent_since(&self, scope: &Scope, since_ms: i64) -> Spend {
        let db = self.db();
        db.query_row(
            "SELECT
                COALESCE(SUM(CASE WHEN usd IS NOT NULL THEN usd ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN usd_source='priced' THEN usd ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN usd IS NULL THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(tokens), 0),
                COUNT(*)
             FROM budget_ledger WHERE scope=?1 AND ts_ms >= ?2",
            params![scope.key(), since_ms],
            |r| {
                Ok(Spend {
                    usd: r.get(0)?,
                    usd_estimated: r.get(1)?,
                    unpriced_turns: r.get::<_, i64>(2)? as u32,
                    tokens: r.get::<_, i64>(3)? as u64,
                    turns: r.get::<_, i64>(4)? as u32,
                })
            },
        )
        .unwrap_or_default()
    }

    /// A scope's spend over its whole life (a loop's cost).
    pub fn spent(&self, scope: &Scope) -> Spend {
        self.spent_since(scope, 0)
    }

    /// Where a policy stands now. `None` = a quota policy with no reading.
    fn observe(&self, p: &BudgetPolicy, account: Option<&str>, now: i64) -> Option<f64> {
        match p.metric {
            Metric::Usd | Metric::WeightedTokens => {
                if p.window == Window::Turn {
                    return Some(0.0);
                }
                let s = self.spent_since(&p.scope, p.window.start_ms(now, p));
                Some(if p.metric == Metric::Usd {
                    s.usd
                } else {
                    s.tokens as f64
                })
            }
            Metric::QuotaPct => {
                let account = account?;
                let w = self.quota_of(account)?.window(p.window)?;
                // An account (or everything) is measured by the window
                // itself; any other scope by what it moved since it began.
                if matches!(p.scope, Scope::Account(_) | Scope::Global) {
                    Some(w.used_pct)
                } else {
                    Some(self.quota_delta(p, account, w, false).1)
                }
            }
        }
    }

    /// Points of the window a scope used since its first reading, as of
    /// the last stored reading and as of `w`. When the window rolls over,
    /// what it used before is carried and the new window counts from zero.
    /// `record` stores `w` as the last reading.
    fn quota_delta(
        &self,
        p: &BudgetPolicy,
        account: &str,
        w: QuotaWindow,
        record: bool,
    ) -> (f64, f64) {
        let db = self.db();
        let row: Option<(f64, f64, f64, Option<i64>)> = db
            .query_row(
                "SELECT baseline, last, carried, resets_at FROM budget_quota WHERE policy_id=?1 AND account=?2",
                params![p.id, account],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .ok()
            .flatten();
        let Some((baseline, last, carried, resets)) = row else {
            let _ = db.execute(
                "INSERT OR REPLACE INTO budget_quota (policy_id, account, baseline, last, carried, resets_at) VALUES (?1, ?2, ?3, ?3, 0, ?4)",
                params![p.id, account, w.used_pct, w.resets_at],
            );
            return (0.0, 0.0);
        };
        let before = carried + (last - baseline).max(0.0);
        let rolled = matches!((resets, w.resets_at), (Some(a), Some(b)) if a != b);
        let (baseline, carried) = if rolled {
            (0.0, before)
        } else {
            (baseline, carried)
        };
        if record {
            let _ = db.execute(
                "UPDATE budget_quota SET baseline=?3, last=?4, carried=?5, resets_at=?6 WHERE policy_id=?1 AND account=?2",
                params![p.id, account, baseline, w.used_pct, carried, w.resets_at],
            );
        }
        (before, carried + (w.used_pct - baseline).max(0.0))
    }

    fn acked(&self, key: &str) -> Option<bool> {
        self.db()
            .query_row(
                "SELECT allow FROM budget_acks WHERE key=?1",
                params![key],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .ok()
            .flatten()
            .map(|v| v != 0)
    }

    /// Whether a model's dollars can be counted: the provider reported a
    /// price for it before, or the price table knows it.
    async fn priced(&self, model: &str) -> bool {
        let reported: bool = self
            .db()
            .query_row(
                "SELECT 1 FROM budget_ledger WHERE model=?1 AND usd_source='reported' LIMIT 1",
                params![model],
                |_| Ok(true),
            )
            .optional()
            .ok()
            .flatten()
            .unwrap_or(false);
        reported || self.price_of(model).await.is_some()
    }

    // ── the three moments of a turn ─────────────────────────────────────

    /// A scope paused by an incident, if any of `scopes` is. An incident
    /// whose policy no longer holds (the window rolled over, the policy was
    /// raised or removed) is settled here and blocks nothing.
    pub fn blocked(&self, scopes: &[Scope], account: Option<&str>) -> Option<Block> {
        let now = now_ms();
        for inc in self.incidents(true) {
            if !scopes.contains(&inc.scope) {
                continue;
            }
            let Some(p) = self.policy(&inc.policy_id).filter(|p| p.enabled) else {
                self.settle_incident(inc, "resolved", "policy_removed");
                continue;
            };
            let acct = account
                .map(String::from)
                .or_else(|| self.last_account(&p.scope));
            let observed = self.observe(&p, acct.as_deref(), now);
            // A checkpoint someone refused holds until they raise it; a
            // hard ceiling lifts by itself when its window starts over.
            if inc.kind == "hard" && p.window != Window::Turn {
                if let Some(v) = observed {
                    if v < p.amount {
                        self.settle_incident(inc, "resolved", "window_reset");
                        continue;
                    }
                }
            }
            let observed = observed.unwrap_or(inc.observed);
            return Some(Block {
                message: format!(
                    "budget paused: {} — incident {} (raise it on /llm/usage or `omniget agent budget raise {} <amount>`)",
                    p.describe(observed),
                    inc.id,
                    inc.id
                ),
                policy_id: p.id.clone(),
                scope: p.scope.clone(),
                metric: p.metric,
                window: p.window,
                observed,
                limit: p.amount,
                incident: Some(inc.id),
            });
        }
        None
    }

    /// Before a turn: a paused scope refuses; a hard ceiling already reached
    /// opens its incident and refuses; a checkpoint reached, or a model with
    /// no price under a dollar ceiling, asks once.
    pub async fn gate(&self, ctx: &TurnCtx) -> Verdict {
        if !self.has_policies() {
            return Verdict::Allow;
        }
        let scopes = ctx.scopes();
        let policies = self.applicable(&scopes);
        if policies.is_empty() {
            return Verdict::Allow;
        }
        if let Some(b) = self.blocked(&scopes, ctx.account.as_deref()) {
            return Verdict::Deny(b);
        }
        let now = now_ms();
        let mut asks = Vec::new();
        for p in &policies {
            // A quota policy starts measuring the scope before its first turn.
            if p.metric == Metric::QuotaPct && !matches!(p.scope, Scope::Account(_) | Scope::Global)
            {
                if let Some(w) = ctx
                    .account
                    .as_deref()
                    .and_then(|a| self.quota_of(a))
                    .and_then(|q| q.window(p.window))
                {
                    let _ = self.quota_delta(p, ctx.account.as_deref().unwrap_or(""), w, true);
                }
            }
            if p.window == Window::Turn {
                if p.metric == Metric::WeightedTokens
                    && p.hard_stop
                    && ctx.estimate_tokens as f64 > p.amount
                {
                    return Verdict::Deny(Block {
                        message: format!(
                            "budget: this turn needs about {} weighted tokens, the ceiling of {} is {} per turn",
                            ctx.estimate_tokens,
                            p.scope.key(),
                            p.amount.round() as i64
                        ),
                        policy_id: p.id.clone(),
                        scope: p.scope.clone(),
                        metric: p.metric,
                        window: p.window,
                        observed: ctx.estimate_tokens as f64,
                        limit: p.amount,
                        incident: None,
                    });
                }
            } else if let Some(observed) = self.observe(p, ctx.account.as_deref(), now) {
                if observed >= p.amount {
                    if p.hard_stop {
                        let inc = self.open_incident(p, observed, "hard", Some(&ctx.agent_id));
                        return Verdict::Deny(Block {
                            message: format!(
                                "budget reached: {} — incident {} (raise it on /llm/usage or `omniget agent budget raise {} <amount>`)",
                                p.describe(observed),
                                inc.id,
                                inc.id
                            ),
                            policy_id: p.id.clone(),
                            scope: p.scope.clone(),
                            metric: p.metric,
                            window: p.window,
                            observed,
                            limit: p.amount,
                            incident: Some(inc.id),
                        });
                    }
                    let key = format!("checkpoint:{}:{}", p.id, p.window.instance(now));
                    match self.acked(&key) {
                        Some(true) => {}
                        Some(false) => {
                            let inc =
                                self.open_incident(p, observed, "checkpoint", Some(&ctx.agent_id));
                            return Verdict::Deny(Block {
                                message: format!(
                                    "budget checkpoint refused: {} — incident {}",
                                    p.describe(observed),
                                    inc.id
                                ),
                                policy_id: p.id.clone(),
                                scope: p.scope.clone(),
                                metric: p.metric,
                                window: p.window,
                                observed,
                                limit: p.amount,
                                incident: Some(inc.id),
                            });
                        }
                        None => asks.push(AskNeed {
                            kind: "checkpoint",
                            policy_id: p.id.clone(),
                            scope: p.scope.clone(),
                            key,
                            observed,
                            limit: p.amount,
                            message: format!(
                                "Budget checkpoint reached: {}. Continue?",
                                p.describe(observed)
                            ),
                        }),
                    }
                }
            }
            if p.metric == Metric::Usd && !ctx.model.is_empty() {
                let key = format!("unpriced:{}:{}", p.id, ctx.model);
                match self.acked(&key) {
                    Some(true) => {}
                    Some(false) => {
                        return Verdict::Deny(Block {
                            message: format!(
                                "budget: the model `{}` has no price and you chose not to run it under the dollar ceiling of {}",
                                ctx.model,
                                p.scope.key()
                            ),
                            policy_id: p.id.clone(),
                            scope: p.scope.clone(),
                            metric: p.metric,
                            window: p.window,
                            observed: 0.0,
                            limit: p.amount,
                            incident: None,
                        })
                    }
                    None => {
                        if !self.priced(&ctx.model).await {
                            asks.push(AskNeed {
                                kind: "unpriced",
                                policy_id: p.id.clone(),
                                scope: p.scope.clone(),
                                key,
                                observed: 0.0,
                                limit: p.amount,
                                message: format!(
                                    "The model `{}` has no known price, so the dollar ceiling of {} cannot count its turns. Run it anyway (its turns count as unknown cost)?",
                                    ctx.model,
                                    p.scope.key()
                                ),
                            });
                        }
                    }
                }
            }
        }
        if asks.is_empty() {
            Verdict::Allow
        } else {
            Verdict::Ask(asks)
        }
    }

    /// Keeps a person's answer to a question of the gate. A refused
    /// checkpoint pauses its scope (an incident, kind `checkpoint`).
    pub fn answer(&self, need: &AskNeed, allow: bool, agent: Option<&str>) -> Option<Block> {
        let _ = self.db().execute(
            "INSERT OR REPLACE INTO budget_acks (key, allow, ts_ms) VALUES (?1, ?2, ?3)",
            params![need.key, allow as i64, now_ms()],
        );
        audit::log(
            Actor::system("ask"),
            if allow {
                "budget.continued"
            } else {
                "budget.refused"
            },
            Entity::budget(&need.policy_id),
            json!({ "kind": need.kind, "scope": need.scope.key(), "observed": need.observed, "limit": need.limit }),
        );
        if allow {
            return None;
        }
        let p = self.policy(&need.policy_id)?;
        let incident = (need.kind == "checkpoint").then(|| {
            self.open_incident(&p, need.observed, "checkpoint", agent)
                .id
        });
        Some(Block {
            message: format!(
                "budget: {} — you chose not to continue",
                p.describe(need.observed)
            ),
            policy_id: p.id.clone(),
            scope: p.scope.clone(),
            metric: p.metric,
            window: p.window,
            observed: need.observed,
            limit: p.amount,
            incident,
        })
    }

    /// After a turn: one ledger row per scope it touched, then every policy
    /// of those scopes is measured again. A hard ceiling reached opens its
    /// incident (the caller tells the owners to stop the scope's work); a
    /// `warn_pct` passed says so once.
    pub async fn charge(&self, ctx: &TurnCtx, used: TurnUse) -> Vec<Crossing> {
        let scopes = ctx.scopes();
        let policies = if self.has_policies() {
            self.applicable(&scopes)
        } else {
            Vec::new()
        };
        // Dollars: what the provider reported; else, only when a dollar
        // ceiling applies (a lookup may read the price table), the table's.
        let (usd, source) = match used.usd {
            Some(v) => (Some(v), "reported"),
            None if policies.iter().any(|p| p.metric == Metric::Usd) => {
                match self.price_of(&ctx.model).await {
                    Some(price) => (
                        Some(
                            used.input_tokens as f64 * price.input_per_m / 1e6
                                + used.output_tokens as f64 * price.output_per_m / 1e6,
                        ),
                        "priced",
                    ),
                    None => (None, "unpriced"),
                }
            }
            None => (None, "unpriced"),
        };
        let now = now_ms();
        {
            let db = self.db();
            for s in &scopes {
                let _ = db.execute(
                    "INSERT INTO budget_ledger (ts_ms, scope, request_id, agent, account, model, usd, usd_source, tokens) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        now,
                        s.key(),
                        ctx.request_id,
                        ctx.agent_id,
                        ctx.account,
                        ctx.model,
                        usd,
                        source,
                        used.weighted() as i64
                    ],
                );
            }
        }
        let mut out = Vec::new();
        for p in &policies {
            let this_turn = match p.metric {
                Metric::Usd => usd.unwrap_or(0.0),
                Metric::WeightedTokens => used.weighted() as f64,
                Metric::QuotaPct => 0.0,
            };
            let (before, after) = match (p.metric, p.window) {
                (_, Window::Turn) => (0.0, this_turn),
                (Metric::QuotaPct, _) => {
                    let Some(acct) = ctx.account.as_deref() else {
                        continue;
                    };
                    let Some(w) = self.quota_of(acct).and_then(|q| q.window(p.window)) else {
                        continue;
                    };
                    if matches!(p.scope, Scope::Account(_) | Scope::Global) {
                        (w.used_pct, w.used_pct)
                    } else {
                        self.quota_delta(p, acct, w, true)
                    }
                }
                _ => {
                    let after = self.observe(p, ctx.account.as_deref(), now).unwrap_or(0.0);
                    (after - this_turn, after)
                }
            };
            if p.hard_stop && after >= p.amount {
                let incident = self.open_incident(p, after, "hard", Some(&ctx.agent_id));
                out.push(Crossing {
                    kind: "hard",
                    policy: p.clone(),
                    observed: after,
                    incident: Some(incident),
                });
                continue;
            }
            if let Some(w) = p.warn_pct {
                let mark = p.amount * w / 100.0;
                let crossed = if p.metric == Metric::QuotaPct
                    && matches!(p.scope, Scope::Account(_) | Scope::Global)
                {
                    // An absolute reading: warn once per window instance.
                    after >= mark
                        && self
                            .acked(&format!("warn:{}:{}", p.id, p.window.instance(now)))
                            .is_none()
                } else {
                    before < mark && after >= mark
                };
                if crossed {
                    let _ = self.db().execute(
                        "INSERT OR REPLACE INTO budget_acks (key, allow, ts_ms) VALUES (?1, 1, ?2)",
                        params![format!("warn:{}:{}", p.id, p.window.instance(now)), now],
                    );
                    audit::log(
                        Actor::system("budget"),
                        "budget.warn",
                        Entity::budget(&p.id),
                        json!({ "scope": p.scope.key(), "observed": after, "limit": p.amount, "warn_pct": w }),
                    );
                    self.tell(BudgetEvent {
                        kind: "warn",
                        incident: None,
                        policy: Some(p.clone()),
                        observed: Some(after),
                    });
                    out.push(Crossing {
                        kind: "warn",
                        policy: p.clone(),
                        observed: after,
                        incident: None,
                    });
                }
            }
        }
        out
    }

    /// Every policy with where it stands, for the budgets page and the CLI.
    pub fn status(&self) -> Vec<PolicyStatus> {
        let now = now_ms();
        let blocking = self.incidents(true);
        self.policies()
            .into_iter()
            .map(|p| {
                let account = self.last_account(&p.scope);
                let observed = self.observe(&p, account.as_deref(), now);
                let source = match (p.metric, observed) {
                    (Metric::QuotaPct, Some(_)) => "real",
                    (Metric::QuotaPct, None) => "none",
                    _ => "ledger",
                };
                let incident = blocking.iter().find(|i| i.policy_id == p.id).cloned();
                PolicyStatus {
                    policy: p,
                    observed,
                    source,
                    incident,
                }
            })
            .collect()
    }

    /// The whole view: policies with their state and the incidents.
    pub fn state_view(&self) -> Value {
        json!({
            "policies": self.status(),
            "incidents": self.incidents(false),
        })
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod tests;

#[cfg(all(test, unix))]
#[path = "bn7_tests.rs"]
mod bn7_tests;
