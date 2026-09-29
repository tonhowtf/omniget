//! Capacity router: role -> candidate, by quota, budget, price, context,
//! latency and availability (plan §9.2). Owned by f2-llm-coordinator.
//!
//! The router is pure: it never touches the network and never reads a
//! credential. Everything it knows about a candidate arrives through
//! [`CapacitySource`], which the app implements on top of the CLI usage
//! window (F4), the BYOK budget ([`super::budget`]) and the telemetry.
//!
//! Rotation policy comes from plan §9.2: a candidate whose window is at or
//! past 90% used is rotated away (`ROTATE_THRESHOLD`), only comes back once it
//! is under 80% used (`HYSTERESIS_RETURN`), and cannot be picked again for
//! `COOLDOWN` after a rate error. Rate errors (`ERR_LLM_RATE`, `ERR_CLI_RATE`)
//! and a window under 5% move to the next candidate without losing the turn:
//! the coordinator resends the whole conversation and emits
//! `BusEvent::Rerouted`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::agent::{AgentDef, AgentRole, Candidate, CandidateRuntime};
use super::error::{LlmError, ERR_LLM_RATE};
use super::types::{ModelRef, ProviderId};

/// Rate limit reported by a CLI runtime (F4). Mirrors `ERR_LLM_RATE` for the
/// native providers; kept here until `error.rs` (f2-llm-providers) adopts it.
pub const ERR_CLI_RATE: &str = "ERR_CLI_RATE";
/// No candidate in the chain can serve the turn right now.
pub const ERR_LLM_NO_CANDIDATE: &str = "ERR_LLM_NO_CANDIDATE";

/// Below this share of the window left, the candidate is skipped outright.
pub const MIN_QUOTA: f32 = 0.05;
/// At or below this share left (90% used) the candidate is rotated away.
pub const ROTATE_THRESHOLD: f32 = 0.10;
/// A rotated-away candidate returns only above this share left (80% used).
pub const HYSTERESIS_RETURN: f32 = 0.20;
/// After a rate error or a rotation, a candidate is untouchable this long.
pub const COOLDOWN: Duration = Duration::from_secs(300);
/// An error this recent still counts against the candidate.
pub const ERROR_WINDOW_SECS: u32 = 60;

/// What a turn is for, so the router can prefer a candidate that is good at it.
///
/// This is **declared**, never guessed: it comes from the agent (its role, or
/// the template it was built from) or from an explicit hint at the call site.
/// Nothing here reads the user's message — a router that sniffed the prompt
/// would reroute differently for the same agent from one turn to the next, and
/// the user could not predict which account gets billed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TaskKind {
    /// Nothing declared: the chain decides on its own, as it always has.
    #[default]
    Unspecified,
    /// Writing, reading or reviewing code.
    Code,
    /// A side call (S22, 84#P3): the Loop auditor, and later titles, the
    /// walkthrough, summaries. Short, tool-less, on the cheapest model of the
    /// **same** provider/account as the session ([`resolve_side`]); never
    /// another provider's subscription. The chain order decides, like
    /// [`TaskKind::Unspecified`].
    Side,
}

/// The role a side agent is given (`AgentRole::Custom`), so the coordinator's
/// [`TaskKind::of_agent`] reads [`TaskKind::Side`] off it.
pub const SIDE_ROLE: &str = "side";

/// Conversation ids of side calls start with this: the CLI runtime launches
/// them lean (no tools, no MCP, the system prompt replaced, low effort).
pub const SIDE_PREFIX: &str = "side-";

pub fn is_side_conversation(id: &str) -> bool {
    id.starts_with(SIDE_PREFIX)
}

/// Why a side call runs on the model it runs on (shown to the user as
/// "side model: X (because …)").
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideReason {
    /// The caller named a model.
    Requested,
    /// The "side model" setting.
    Setting,
    /// The smallest model of the session's provider/account.
    ProviderSmall,
    /// No smaller model is known for this provider: the session's own.
    SessionModel,
}

impl SideReason {
    pub fn as_str(self) -> &'static str {
        match self {
            SideReason::Requested => "requested",
            SideReason::Setting => "setting",
            SideReason::ProviderSmall => "provider_small",
            SideReason::SessionModel => "session_model",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SideModel {
    pub model: String,
    pub reason: SideReason,
}

/// The provider family of a candidate, for [`small_model_of`]: `claude-cli`,
/// `codex-cli`, or the native provider id (`anthropic`, `openai`, …).
pub fn side_family(candidate: &Candidate, runtime: &super::agent::RuntimeKind) -> String {
    match (&candidate.runtime, runtime) {
        (CandidateRuntime::Cli { .. }, super::agent::RuntimeKind::Cli { cli, .. }) => {
            format!("{}-cli", cli.trim().to_ascii_lowercase())
        }
        (CandidateRuntime::Native { provider }, _) => provider.as_str().to_ascii_lowercase(),
        (CandidateRuntime::Cli { account_id }, _) => format!("cli:{account_id}"),
    }
}

/// The small model of the same family, or `None` when none is known (a
/// local model is already free; an ACP agent picks its own). Names that a
/// family does not offer only cost one failed side call: the caller retries
/// with the session model.
pub fn small_model_of(family: &str, session_model: &str) -> Option<String> {
    let m = session_model.trim().to_ascii_lowercase();
    let small = |needles: &[&str]| needles.iter().any(|n| m.contains(n));
    match family {
        // `claude --model haiku` is an alias the CLI resolves itself.
        "claude-cli" => Some("haiku".into()),
        "anthropic" => Some(if small(&["haiku"]) {
            session_model.trim().to_string()
        } else {
            "claude-haiku-4-5".into()
        }),
        "codex-cli" | "openai" => {
            if m.is_empty() || m == "default" {
                return None;
            }
            if small(&["mini", "nano", "luna"]) {
                return Some(session_model.trim().to_string());
            }
            // The tiered names Codex lists (`gpt-5.6-sol`, `gpt-6-astra`):
            // `luna` is the cheapest tier of a generation.
            if let Some((base, tier)) = m.rsplit_once('-') {
                if matches!(tier, "astra" | "sol" | "terra") && base.starts_with("gpt-") {
                    return Some(format!("{base}-luna"));
                }
            }
            if m.starts_with("gpt-") {
                return Some(format!("{m}-mini"));
            }
            None
        }
        "google" | "gemini" => Some(if small(&["flash-lite"]) {
            session_model.trim().to_string()
        } else {
            "gemini-2.5-flash-lite".into()
        }),
        _ => None,
    }
}

/// The side model chain of 84#P3: requested → setting → the smallest model
/// of the session's provider → the session's own model.
pub fn resolve_side(
    requested: Option<&str>,
    setting: Option<&str>,
    family: &str,
    session_model: &str,
) -> SideModel {
    let named = |m: Option<&str>| m.map(str::trim).filter(|m| !m.is_empty()).map(String::from);
    if let Some(model) = named(requested) {
        return SideModel {
            model,
            reason: SideReason::Requested,
        };
    }
    if let Some(model) = named(setting) {
        return SideModel {
            model,
            reason: SideReason::Setting,
        };
    }
    match small_model_of(family, session_model) {
        Some(model) => SideModel {
            model,
            reason: SideReason::ProviderSmall,
        },
        None => SideModel {
            model: session_model.trim().to_string(),
            reason: SideReason::SessionModel,
        },
    }
}

/// Template ids (and therefore agent ids, since the roster keeps them) whose
/// whole job is code. `templates.rs` owns the list of templates; this is the
/// subset that declares [`TaskKind::Code`].
pub const CODE_AGENTS: &[&str] = &["engineering-manager"];

impl TaskKind {
    /// The kind an agent declares. `AgentRole::Custom("code")` wins, then the
    /// template/agent id. Everything else is [`TaskKind::Unspecified`].
    pub fn of_agent(agent: &AgentDef) -> TaskKind {
        if let AgentRole::Custom(name) = &agent.role {
            let name = name.trim().to_ascii_lowercase();
            if name == SIDE_ROLE {
                return TaskKind::Side;
            }
            if name == "code" || name == "coder" || name == "coding" {
                return TaskKind::Code;
            }
        }
        if CODE_AGENTS.contains(&agent.id.as_str()) {
            return TaskKind::Code;
        }
        // An agent that may patch files or run a shell, with a workspace attached,
        // is doing code whatever it is called.
        let can_write = agent.tools.iter().any(|g| {
            matches!(&g.source, super::agent::ToolSource::Internal { name }
                if super::code_tools::WRITE_TOOLS.contains(&name.as_str()))
                && g.mode != super::agent::GrantMode::Deny
        });
        if can_write && super::code_tools::workspace().is_some() {
            return TaskKind::Code;
        }
        // A CLI agent (Claude Code, Codex) is a coding tool by nature.
        if matches!(
            agent.runtime,
            super::agent::RuntimeKind::Cli { .. } | super::agent::RuntimeKind::Acp { .. }
        ) {
            return TaskKind::Code;
        }
        TaskKind::Unspecified
    }
}

/// The last error a candidate returned, as the app remembers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityError {
    pub code: String,
    pub seconds_ago: u32,
}

/// What the router knows about one candidate at selection time.
#[derive(Debug, Clone, PartialEq)]
pub struct Capacity {
    /// Account logged in / key present / server reachable, as last observed.
    pub available: bool,
    /// Share of the plan window still left, 0.0..=1.0. `None` for pay-per-token.
    pub quota_remaining: Option<f32>,
    /// Dollars still available today for this candidate. `None` for flat plans.
    pub budget_remaining_usd: Option<f64>,
    /// Blended price per 1k tokens, for `Candidate::max_cost_per_1k`.
    pub cost_per_1k: Option<f64>,
    /// Usable context window of the model, in tokens.
    pub context_window: u32,
    /// Rolling average latency from telemetry.
    pub avg_latency_ms: Option<u32>,
    pub last_error: Option<CapacityError>,
    /// This candidate is a coding agent in its own right (today: a Codex CLI
    /// account). A [`TaskKind::Code`] turn prefers it over an equally usable
    /// candidate earlier in the chain. The [`CapacitySource`] sets it — the
    /// router never inspects an account, a model name or a credential.
    pub code_specialist: bool,
}

impl Default for Capacity {
    /// Everything unknown and usable: the router only rejects on evidence.
    fn default() -> Self {
        Self {
            available: true,
            quota_remaining: None,
            budget_remaining_usd: None,
            cost_per_1k: None,
            context_window: u32::MAX,
            avg_latency_ms: None,
            last_error: None,
            code_specialist: false,
        }
    }
}

impl Capacity {
    pub fn unavailable() -> Self {
        Self {
            available: false,
            ..Self::default()
        }
    }

    pub fn with_quota(quota: f32) -> Self {
        Self {
            quota_remaining: Some(quota),
            ..Self::default()
        }
    }
}

/// Where the router reads capacity from. The app wires the CLI usage window,
/// the budget store and the telemetry behind it.
pub trait CapacitySource: Send + Sync {
    fn capacity(&self, candidate: &Candidate) -> Capacity;
}

/// Everything available, always: the default for tests and for a first run
/// before any telemetry exists.
#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysAvailable;

impl CapacitySource for AlwaysAvailable {
    fn capacity(&self, _candidate: &Candidate) -> Capacity {
        Capacity::default()
    }
}

/// A fixed table keyed by [`candidate_key`]; used by the tests and by the
/// commands layer when capacity is a snapshot.
#[derive(Debug, Default, Clone)]
pub struct StaticCapacity {
    map: HashMap<String, Capacity>,
    fallback: Option<Capacity>,
}

impl StaticCapacity {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(mut self, key: impl Into<String>, cap: Capacity) -> Self {
        self.map.insert(key.into(), cap);
        self
    }

    pub fn fallback(mut self, cap: Capacity) -> Self {
        self.fallback = Some(cap);
        self
    }
}

impl CapacitySource for StaticCapacity {
    fn capacity(&self, candidate: &Candidate) -> Capacity {
        self.map
            .get(&candidate_key(candidate))
            .cloned()
            .or_else(|| self.fallback.clone())
            .unwrap_or_default()
    }
}

/// Stable identity of a candidate: `native:<provider>:<model>` or
/// `cli:<account>:<model>`. Used for telemetry, cooldown and the bus.
pub fn candidate_key(candidate: &Candidate) -> String {
    match &candidate.runtime {
        CandidateRuntime::Native { provider } => {
            format!("native:{}:{}", provider.as_str(), candidate.model)
        }
        CandidateRuntime::Cli { account_id } => {
            format!("cli:{}:{}", account_id, candidate.model)
        }
    }
}

/// The `ModelRef` a native candidate resolves to. CLI candidates have no
/// provider, so they carry a synthetic `cli` provider id.
pub fn model_ref(candidate: &Candidate) -> ModelRef {
    match &candidate.runtime {
        CandidateRuntime::Native { provider } => ModelRef {
            provider: provider.clone(),
            model: candidate.model.clone(),
        },
        CandidateRuntime::Cli { account_id } => ModelRef {
            provider: ProviderId::new(format!("cli:{account_id}")),
            model: candidate.model.clone(),
        },
    }
}

/// Should this error code move the turn to the next candidate?
pub fn is_reroutable(code: &str) -> bool {
    code == ERR_LLM_RATE || code == ERR_CLI_RATE
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    Unavailable,
    QuotaExhausted,
    NoBudget,
    TooExpensive,
    ContextTooSmall,
    TooSlow,
    RecentError,
    /// Window at or past the rotation threshold (soft: relaxed on pass 2).
    NearLimit,
    /// Rotated away less than `COOLDOWN` ago (soft).
    Cooldown,
    /// Rotated away and not yet recovered past the hysteresis mark (soft).
    Hysteresis,
    /// The coordinator already tried this one in this turn.
    AlreadyTried,
}

impl RejectReason {
    /// Soft reasons only apply on the first pass; if nothing survives them,
    /// the router tries again ignoring them rather than failing the turn.
    pub fn is_soft(self) -> bool {
        matches!(
            self,
            RejectReason::NearLimit
                | RejectReason::Cooldown
                | RejectReason::Hysteresis
                | RejectReason::TooSlow
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            RejectReason::Unavailable => "unavailable",
            RejectReason::QuotaExhausted => "quota_exhausted",
            RejectReason::NoBudget => "no_budget",
            RejectReason::TooExpensive => "too_expensive",
            RejectReason::ContextTooSmall => "context_too_small",
            RejectReason::TooSlow => "too_slow",
            RejectReason::RecentError => "recent_error",
            RejectReason::NearLimit => "near_limit",
            RejectReason::Cooldown => "cooldown",
            RejectReason::Hysteresis => "hysteresis",
            RejectReason::AlreadyTried => "already_tried",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    /// Position in the chain.
    pub index: usize,
    pub candidate: Candidate,
    pub key: String,
    pub model: ModelRef,
    /// Whether the soft rules had to be dropped to find this one.
    pub relaxed: bool,
    /// Candidates skipped before this one, in chain order.
    pub rejected: Vec<(String, RejectReason)>,
    /// True when the [`TaskKind`] preference, and not chain order, is why this
    /// candidate was picked. Purely for the timeline and the tests.
    pub preferred: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct RouterConfig {
    pub min_quota: f32,
    pub rotate_threshold: f32,
    pub hysteresis_return: f32,
    pub cooldown: Duration,
    pub error_window_secs: u32,
    /// Latency ceiling for the first pass; `None` disables the check.
    pub max_latency_ms: Option<u32>,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            min_quota: MIN_QUOTA,
            rotate_threshold: ROTATE_THRESHOLD,
            hysteresis_return: HYSTERESIS_RETURN,
            cooldown: COOLDOWN,
            error_window_secs: ERROR_WINDOW_SECS,
            max_latency_ms: None,
        }
    }
}

/// Which candidates were rotated away and when. Pure: the caller supplies the
/// clock, so the tests are deterministic.
#[derive(Debug, Default)]
pub struct RouterState {
    rotated_away: HashMap<String, Instant>,
}

impl RouterState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark a candidate as rotated away at `now` (rate error, rejected turn or
    /// window past the threshold).
    pub fn rotate_away(&mut self, key: impl Into<String>, now: Instant) {
        self.rotated_away.insert(key.into(), now);
    }

    pub fn clear(&mut self, key: &str) {
        self.rotated_away.remove(key);
    }

    pub fn rotated_at(&self, key: &str) -> Option<Instant> {
        self.rotated_away.get(key).copied()
    }

    pub fn is_cooling(&self, key: &str, now: Instant, cooldown: Duration) -> bool {
        self.rotated_at(key)
            .map(|at| now.duration_since(at) < cooldown)
            .unwrap_or(false)
    }
}

/// Why a candidate is not usable right now, or `None` when it is.
fn reject(
    candidate: &Candidate,
    cap: &Capacity,
    cfg: &RouterConfig,
    state: &RouterState,
    now: Instant,
) -> Option<RejectReason> {
    let key = candidate_key(candidate);
    if !cap.available {
        return Some(RejectReason::Unavailable);
    }
    if let Some(q) = cap.quota_remaining {
        if q < cfg.min_quota {
            return Some(RejectReason::QuotaExhausted);
        }
    }
    if let Some(usd) = cap.budget_remaining_usd {
        if usd <= 0.0 {
            return Some(RejectReason::NoBudget);
        }
    }
    if let (Some(max), Some(price)) = (candidate.max_cost_per_1k, cap.cost_per_1k) {
        if price > max {
            return Some(RejectReason::TooExpensive);
        }
    }
    if candidate.min_context > 0 && cap.context_window < candidate.min_context {
        return Some(RejectReason::ContextTooSmall);
    }
    if let Some(err) = &cap.last_error {
        if err.seconds_ago <= cfg.error_window_secs {
            return Some(RejectReason::RecentError);
        }
    }
    // Soft rules below: pass 2 ignores them.
    if let (Some(max), Some(ms)) = (cfg.max_latency_ms, cap.avg_latency_ms) {
        if ms > max {
            return Some(RejectReason::TooSlow);
        }
    }
    if state.is_cooling(&key, now, cfg.cooldown) {
        return Some(RejectReason::Cooldown);
    }
    if let Some(q) = cap.quota_remaining {
        let was_rotated = state.rotated_at(&key).is_some();
        if was_rotated && q < cfg.hysteresis_return {
            return Some(RejectReason::Hysteresis);
        }
        if !was_rotated && q <= cfg.rotate_threshold {
            return Some(RejectReason::NearLimit);
        }
    }
    None
}

/// Pick the first candidate that can serve the turn. `tried` holds the keys the
/// coordinator already burned in this turn (a reroute never goes backwards).
///
/// Equivalent to [`select_for`] with [`TaskKind::Unspecified`].
pub fn select(
    chain: &[Candidate],
    source: &dyn CapacitySource,
    state: &RouterState,
    cfg: &RouterConfig,
    now: Instant,
    tried: &[String],
) -> Result<Route, LlmError> {
    select_for(chain, source, state, cfg, now, tried, TaskKind::Unspecified)
}

/// [`select`] plus the task preference: a [`TaskKind::Code`] turn takes the
/// first usable `code_specialist` in the chain (a Codex account, today) even if
/// a plain candidate sits before it.
///
/// The preference is *only* over candidates that already passed every rule, so
/// a rate limit takes it away by itself: an account out of window, inside the
/// `ERR_CLI_RATE` error window, on cooldown or past the rotation threshold is
/// not usable, so it cannot be preferred either. Nothing here reads a token,
/// a credential or an account file — the flag arrives through
/// [`CapacitySource`].
#[allow(clippy::too_many_arguments)]
pub fn select_for(
    chain: &[Candidate],
    source: &dyn CapacitySource,
    state: &RouterState,
    cfg: &RouterConfig,
    now: Instant,
    tried: &[String],
    task: TaskKind,
) -> Result<Route, LlmError> {
    if chain.is_empty() {
        return Err(LlmError::new(
            ERR_LLM_NO_CANDIDATE,
            "the model policy has an empty chain",
        ));
    }
    let mut caps: Vec<(usize, String, Capacity)> = Vec::with_capacity(chain.len());
    for (i, candidate) in chain.iter().enumerate() {
        caps.push((i, candidate_key(candidate), source.capacity(candidate)));
    }

    // Every rejection, in chain order; the `Route` only carries the ones that
    // come before whichever candidate wins, so a preferred pick does not report
    // the candidates it jumped over as "rejected".
    let mut rejections: Vec<(usize, String, RejectReason)> = Vec::new();
    let mut soft_only: Vec<(usize, String)> = Vec::new();
    let mut first_ok: Option<(usize, String)> = None;
    let mut preferred: Option<(usize, String)> = None;

    for (i, key, cap) in &caps {
        if tried.iter().any(|t| t == key) {
            rejections.push((*i, key.clone(), RejectReason::AlreadyTried));
            continue;
        }
        match reject(&chain[*i], cap, cfg, state, now) {
            None => {
                if first_ok.is_none() {
                    first_ok = Some((*i, key.clone()));
                }
                if task == TaskKind::Code && cap.code_specialist {
                    preferred = Some((*i, key.clone()));
                    break;
                }
                // With no preference to look for, the first usable one wins,
                // exactly as before.
                if matches!(task, TaskKind::Unspecified | TaskKind::Side) {
                    break;
                }
            }
            Some(reason) => {
                if reason.is_soft() {
                    soft_only.push((*i, key.clone()));
                }
                rejections.push((*i, key.clone(), reason));
            }
        }
    }

    let before = |index: usize| -> Vec<(String, RejectReason)> {
        rejections
            .iter()
            .filter(|(i, _, _)| *i < index)
            .map(|(_, k, r)| (k.clone(), *r))
            .collect()
    };

    // `preferred` only *means* something when it moved the pick; a specialist
    // that was already the head of the chain was not preferred, it was first.
    let by_preference = preferred.is_some() && preferred != first_ok;
    if let Some((i, key)) = preferred.or(first_ok) {
        return Ok(Route {
            index: i,
            candidate: chain[i].clone(),
            key,
            model: model_ref(&chain[i]),
            relaxed: false,
            rejected: before(i),
            preferred: by_preference,
        });
    }

    // Pass 2: nothing passed the soft rules, so take the first candidate that
    // only failed them. Better a slow or nearly exhausted account than no turn.
    if let Some((i, key)) = soft_only.first().cloned() {
        return Ok(Route {
            index: i,
            candidate: chain[i].clone(),
            key,
            model: model_ref(&chain[i]),
            relaxed: true,
            rejected: before(i),
            preferred: false,
        });
    }

    let detail = rejections
        .iter()
        .map(|(_, k, r)| format!("{k}={}", r.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    Err(LlmError::new(
        ERR_LLM_NO_CANDIDATE,
        format!("no candidate can serve the turn: {detail}"),
    ))
}

/// The router as the coordinator holds it: config, capacity source and the
/// rotation state behind a mutex.
pub struct Router {
    source: Arc<dyn CapacitySource>,
    cfg: RouterConfig,
    state: Mutex<RouterState>,
}

impl std::fmt::Debug for Router {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Router").field("cfg", &self.cfg).finish()
    }
}

impl Router {
    pub fn new(source: Arc<dyn CapacitySource>) -> Self {
        Self {
            source,
            cfg: RouterConfig::default(),
            state: Mutex::new(RouterState::new()),
        }
    }

    pub fn with_config(mut self, cfg: RouterConfig) -> Self {
        self.cfg = cfg;
        self
    }

    pub fn config(&self) -> &RouterConfig {
        &self.cfg
    }

    pub fn pick(&self, chain: &[Candidate], tried: &[String]) -> Result<Route, LlmError> {
        self.pick_for(chain, tried, TaskKind::Unspecified)
    }

    /// [`Router::pick`] with a declared task. A code turn prefers a usable
    /// Codex account; anything else behaves exactly like `pick`.
    pub fn pick_for(
        &self,
        chain: &[Candidate],
        tried: &[String],
        task: TaskKind,
    ) -> Result<Route, LlmError> {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        select_for(
            chain,
            self.source.as_ref(),
            &state,
            &self.cfg,
            Instant::now(),
            tried,
            task,
        )
    }

    /// Called by the coordinator when a candidate fails with a reroutable code
    /// or when its window crosses the threshold.
    pub fn rotate_away(&self, key: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.rotate_away(key, Instant::now());
    }

    pub fn clear(&self, key: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.clear(key);
    }

    pub fn is_cooling(&self, key: &str) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.is_cooling(key, Instant::now(), self.cfg.cooldown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native(provider: &str, model: &str) -> Candidate {
        Candidate {
            runtime: CandidateRuntime::Native {
                provider: ProviderId::new(provider),
            },
            model: model.into(),
            max_cost_per_1k: None,
            min_context: 0,
        }
    }

    fn cli(account: &str, model: &str) -> Candidate {
        Candidate {
            runtime: CandidateRuntime::Cli {
                account_id: account.into(),
            },
            model: model.into(),
            max_cost_per_1k: None,
            min_context: 0,
        }
    }

    fn pick(chain: &[Candidate], source: &dyn CapacitySource) -> Route {
        select(
            chain,
            source,
            &RouterState::new(),
            &RouterConfig::default(),
            Instant::now(),
            &[],
        )
        .expect("a route")
    }

    #[test]
    fn candidate_key_separates_runtimes() {
        assert_eq!(
            candidate_key(&native("openai", "gpt-5")),
            "native:openai:gpt-5"
        );
        assert_eq!(candidate_key(&cli("max-1", "sonnet")), "cli:max-1:sonnet");
    }

    #[test]
    fn picks_the_first_candidate_when_everything_is_fine() {
        let chain = vec![native("anthropic", "a"), native("openai", "b")];
        let route = pick(&chain, &AlwaysAvailable);
        assert_eq!(route.index, 0);
        assert!(!route.relaxed);
        assert!(route.rejected.is_empty());
        assert_eq!(route.model.model, "a");
    }

    #[test]
    fn empty_chain_is_an_error() {
        let err = select(
            &[],
            &AlwaysAvailable,
            &RouterState::new(),
            &RouterConfig::default(),
            Instant::now(),
            &[],
        )
        .unwrap_err();
        assert_eq!(err.code, ERR_LLM_NO_CANDIDATE);
    }

    #[test]
    fn skips_the_unavailable_candidate() {
        let chain = vec![native("anthropic", "a"), native("openai", "b")];
        let source = StaticCapacity::new().set("native:anthropic:a", Capacity::unavailable());
        let route = pick(&chain, &source);
        assert_eq!(route.index, 1);
        assert_eq!(route.rejected[0].1, RejectReason::Unavailable);
    }

    #[test]
    fn quota_under_five_percent_is_skipped() {
        let chain = vec![cli("max-1", "sonnet"), cli("max-2", "sonnet")];
        let source = StaticCapacity::new()
            .set("cli:max-1:sonnet", Capacity::with_quota(0.04))
            .set("cli:max-2:sonnet", Capacity::with_quota(0.80));
        let route = pick(&chain, &source);
        assert_eq!(route.key, "cli:max-2:sonnet");
        assert_eq!(route.rejected[0].1, RejectReason::QuotaExhausted);
    }

    #[test]
    fn window_at_ninety_percent_used_rotates_to_the_next() {
        let chain = vec![cli("max-1", "sonnet"), cli("max-2", "sonnet")];
        let source = StaticCapacity::new()
            .set("cli:max-1:sonnet", Capacity::with_quota(0.10))
            .set("cli:max-2:sonnet", Capacity::with_quota(0.90));
        let route = pick(&chain, &source);
        assert_eq!(route.key, "cli:max-2:sonnet");
        assert_eq!(route.rejected[0].1, RejectReason::NearLimit);
    }

    #[test]
    fn exhausted_budget_is_skipped() {
        let chain = vec![native("openai", "a"), native("openrouter", "b")];
        let source = StaticCapacity::new().set(
            "native:openai:a",
            Capacity {
                budget_remaining_usd: Some(0.0),
                ..Capacity::default()
            },
        );
        let route = pick(&chain, &source);
        assert_eq!(route.index, 1);
        assert_eq!(route.rejected[0].1, RejectReason::NoBudget);
    }

    #[test]
    fn price_above_the_ceiling_is_skipped() {
        let mut expensive = native("anthropic", "opus");
        expensive.max_cost_per_1k = Some(0.01);
        let chain = vec![expensive, native("ollama", "qwen")];
        let source = StaticCapacity::new().set(
            "native:anthropic:opus",
            Capacity {
                cost_per_1k: Some(0.075),
                ..Capacity::default()
            },
        );
        let route = pick(&chain, &source);
        assert_eq!(route.index, 1);
        assert_eq!(route.rejected[0].1, RejectReason::TooExpensive);
    }

    #[test]
    fn context_smaller_than_required_is_skipped() {
        let mut big = native("ollama", "qwen");
        big.min_context = 128_000;
        let chain = vec![big, native("anthropic", "sonnet")];
        let source = StaticCapacity::new().set(
            "native:ollama:qwen",
            Capacity {
                context_window: 8_192,
                ..Capacity::default()
            },
        );
        let route = pick(&chain, &source);
        assert_eq!(route.index, 1);
        assert_eq!(route.rejected[0].1, RejectReason::ContextTooSmall);
    }

    #[test]
    fn an_error_inside_the_sixty_second_window_is_skipped() {
        let chain = vec![native("openai", "a"), native("openrouter", "b")];
        let source = StaticCapacity::new().set(
            "native:openai:a",
            Capacity {
                last_error: Some(CapacityError {
                    code: ERR_LLM_RATE.into(),
                    seconds_ago: 12,
                }),
                ..Capacity::default()
            },
        );
        let route = pick(&chain, &source);
        assert_eq!(route.index, 1);
        assert_eq!(route.rejected[0].1, RejectReason::RecentError);
    }

    #[test]
    fn an_error_older_than_the_window_does_not_count() {
        let chain = vec![native("openai", "a"), native("openrouter", "b")];
        let source = StaticCapacity::new().set(
            "native:openai:a",
            Capacity {
                last_error: Some(CapacityError {
                    code: ERR_LLM_RATE.into(),
                    seconds_ago: 61,
                }),
                ..Capacity::default()
            },
        );
        assert_eq!(pick(&chain, &source).index, 0);
    }

    #[test]
    fn latency_above_the_target_is_soft_and_relaxes() {
        let chain = vec![native("ollama", "qwen")];
        let source = StaticCapacity::new().set(
            "native:ollama:qwen",
            Capacity {
                avg_latency_ms: Some(30_000),
                ..Capacity::default()
            },
        );
        let cfg = RouterConfig {
            max_latency_ms: Some(5_000),
            ..RouterConfig::default()
        };
        let route = select(
            &chain,
            &source,
            &RouterState::new(),
            &cfg,
            Instant::now(),
            &[],
        )
        .unwrap();
        assert!(
            route.relaxed,
            "the only candidate is slow, but it is the only one"
        );
    }

    #[test]
    fn cooldown_keeps_a_rotated_candidate_out_for_five_minutes() {
        let chain = vec![cli("max-1", "sonnet"), cli("max-2", "sonnet")];
        let now = Instant::now();
        let mut state = RouterState::new();
        state.rotate_away("cli:max-1:sonnet", now);
        let cfg = RouterConfig::default();
        let route = select(&chain, &AlwaysAvailable, &state, &cfg, now, &[]).unwrap();
        assert_eq!(route.key, "cli:max-2:sonnet");
        assert_eq!(route.rejected[0].1, RejectReason::Cooldown);
        // Past the cooldown, with no quota reading, the first one is back.
        let later = now + COOLDOWN + Duration::from_secs(1);
        let route = select(&chain, &AlwaysAvailable, &state, &cfg, later, &[]).unwrap();
        assert_eq!(route.key, "cli:max-1:sonnet");
    }

    #[test]
    fn hysteresis_holds_a_rotated_candidate_until_it_recovers() {
        let chain = vec![cli("max-1", "sonnet"), cli("max-2", "sonnet")];
        let now = Instant::now();
        let mut state = RouterState::new();
        state.rotate_away("cli:max-1:sonnet", now);
        // Cooldown already over: only the hysteresis rule is left.
        let cfg = RouterConfig {
            cooldown: Duration::ZERO,
            ..RouterConfig::default()
        };
        // 85% used: past the rotation threshold but short of the return mark.
        let source = StaticCapacity::new()
            .set("cli:max-1:sonnet", Capacity::with_quota(0.15))
            .set("cli:max-2:sonnet", Capacity::with_quota(0.70));
        let route = select(&chain, &source, &state, &cfg, now, &[]).unwrap();
        assert_eq!(route.key, "cli:max-2:sonnet");
        assert_eq!(route.rejected[0].1, RejectReason::Hysteresis);
        // 75% used: above the return mark, it comes back.
        let source = StaticCapacity::new()
            .set("cli:max-1:sonnet", Capacity::with_quota(0.25))
            .set("cli:max-2:sonnet", Capacity::with_quota(0.70));
        let route = select(&chain, &source, &state, &cfg, now, &[]).unwrap();
        assert_eq!(route.key, "cli:max-1:sonnet");
    }

    #[test]
    fn tried_candidates_are_never_picked_again_in_the_same_turn() {
        let chain = vec![native("a", "m"), native("b", "m"), native("c", "m")];
        let route = select(
            &chain,
            &AlwaysAvailable,
            &RouterState::new(),
            &RouterConfig::default(),
            Instant::now(),
            &["native:a:m".into(), "native:b:m".into()],
        )
        .unwrap();
        assert_eq!(route.key, "native:c:m");
        assert_eq!(route.rejected[0].1, RejectReason::AlreadyTried);
    }

    #[test]
    fn chain_of_four_with_injected_failures_walks_to_the_end() {
        let chain = vec![
            cli("max-1", "sonnet"),
            cli("max-2", "sonnet"),
            native("anthropic", "sonnet"),
            native("ollama", "qwen"),
        ];
        let source = StaticCapacity::new()
            // #1 out of window
            .set("cli:max-1:sonnet", Capacity::with_quota(0.01))
            // #2 rate limited a moment ago
            .set(
                "cli:max-2:sonnet",
                Capacity {
                    last_error: Some(CapacityError {
                        code: ERR_CLI_RATE.into(),
                        seconds_ago: 3,
                    }),
                    ..Capacity::default()
                },
            )
            // #3 out of money
            .set(
                "native:anthropic:sonnet",
                Capacity {
                    budget_remaining_usd: Some(0.0),
                    ..Capacity::default()
                },
            )
            // #4 local, always there
            .set("native:ollama:qwen", Capacity::default());
        let route = pick(&chain, &source);
        assert_eq!(route.index, 3);
        assert_eq!(route.key, "native:ollama:qwen");
        let reasons: Vec<RejectReason> = route.rejected.iter().map(|(_, r)| *r).collect();
        assert_eq!(
            reasons,
            vec![
                RejectReason::QuotaExhausted,
                RejectReason::RecentError,
                RejectReason::NoBudget
            ]
        );
    }

    #[test]
    fn everything_hard_blocked_is_an_error_not_a_pick() {
        let chain = vec![native("a", "m"), native("b", "m")];
        let source = StaticCapacity::new().fallback(Capacity::unavailable());
        let err = select(
            &chain,
            &source,
            &RouterState::new(),
            &RouterConfig::default(),
            Instant::now(),
            &[],
        )
        .unwrap_err();
        assert_eq!(err.code, ERR_LLM_NO_CANDIDATE);
        assert!(err.message.contains("unavailable"));
    }

    #[test]
    fn reroutable_codes_are_the_rate_limits_only() {
        assert!(is_reroutable(ERR_LLM_RATE));
        assert!(is_reroutable(ERR_CLI_RATE));
        assert!(!is_reroutable(super::super::error::ERR_LLM_AUTH));
    }

    #[test]
    fn router_handle_rotates_and_clears() {
        let chain = vec![native("a", "m"), native("b", "m")];
        let router = Router::new(Arc::new(AlwaysAvailable));
        assert_eq!(router.pick(&chain, &[]).unwrap().key, "native:a:m");
        router.rotate_away("native:a:m");
        assert!(router.is_cooling("native:a:m"));
        assert_eq!(router.pick(&chain, &[]).unwrap().key, "native:b:m");
        router.clear("native:a:m");
        assert_eq!(router.pick(&chain, &[]).unwrap().key, "native:a:m");
    }

    // ── Task preference (Codex for code) ─────────────────────────────

    /// A Codex account, as `CliCapacitySource` reports it.
    fn codex_cap(quota: Option<f32>) -> Capacity {
        Capacity {
            quota_remaining: quota,
            code_specialist: true,
            ..Capacity::default()
        }
    }

    fn pick_code(chain: &[Candidate], source: &dyn CapacitySource) -> Route {
        select_for(
            chain,
            source,
            &RouterState::new(),
            &RouterConfig::default(),
            Instant::now(),
            &[],
            TaskKind::Code,
        )
        .expect("a route")
    }

    #[test]
    fn a_code_turn_prefers_the_codex_account_over_the_head_of_the_chain() {
        let chain = vec![cli("claude-1", "sonnet"), cli("codex-1", "gpt-5-codex")];
        let source = StaticCapacity::new()
            .set("cli:claude-1:sonnet", Capacity::with_quota(0.90))
            .set("cli:codex-1:gpt-5-codex", codex_cap(Some(0.90)));
        let route = pick_code(&chain, &source);
        assert_eq!(route.key, "cli:codex-1:gpt-5-codex");
        assert!(route.preferred);
        // The candidate it jumped over was never rejected, so it is not listed.
        assert!(route.rejected.is_empty(), "{:?}", route.rejected);
        assert!(!route.relaxed);
    }

    #[test]
    fn a_non_code_turn_keeps_the_chain_order() {
        let chain = vec![cli("claude-1", "sonnet"), cli("codex-1", "gpt-5-codex")];
        let source = StaticCapacity::new()
            .set("cli:claude-1:sonnet", Capacity::with_quota(0.90))
            .set("cli:codex-1:gpt-5-codex", codex_cap(Some(0.90)));
        let route = pick(&chain, &source);
        assert_eq!(route.key, "cli:claude-1:sonnet");
        assert!(!route.preferred);
    }

    #[test]
    fn a_rate_limited_codex_loses_the_preference() {
        let chain = vec![cli("claude-1", "sonnet"), cli("codex-1", "gpt-5-codex")];
        // Window exhausted: hard reject, so it cannot even be considered.
        let source = StaticCapacity::new()
            .set("cli:claude-1:sonnet", Capacity::with_quota(0.90))
            .set("cli:codex-1:gpt-5-codex", codex_cap(Some(0.01)));
        let route = pick_code(&chain, &source);
        assert_eq!(route.key, "cli:claude-1:sonnet");
        assert!(!route.preferred);

        // An ERR_CLI_RATE seconds ago does the same thing.
        let source = StaticCapacity::new()
            .set("cli:claude-1:sonnet", Capacity::with_quota(0.90))
            .set(
                "cli:codex-1:gpt-5-codex",
                Capacity {
                    last_error: Some(CapacityError {
                        code: ERR_CLI_RATE.into(),
                        seconds_ago: 3,
                    }),
                    code_specialist: true,
                    ..Capacity::default()
                },
            );
        let route = pick_code(&chain, &source);
        assert_eq!(route.key, "cli:claude-1:sonnet");
        assert!(!route.preferred);
    }

    /// 90% used: the rotation rule is soft, but a candidate that failed it is
    /// still not usable on pass 1, so the preference does not apply either.
    #[test]
    fn a_codex_past_the_rotation_threshold_is_not_preferred() {
        let chain = vec![cli("claude-1", "sonnet"), cli("codex-1", "gpt-5-codex")];
        let source = StaticCapacity::new()
            .set("cli:claude-1:sonnet", Capacity::with_quota(0.90))
            .set("cli:codex-1:gpt-5-codex", codex_cap(Some(0.10)));
        let route = pick_code(&chain, &source);
        assert_eq!(route.key, "cli:claude-1:sonnet");
        assert!(!route.preferred);
    }

    #[test]
    fn without_a_codex_account_a_code_turn_routes_exactly_as_before() {
        let chain = vec![
            cli("claude-1", "sonnet"),
            native("anthropic", "sonnet"),
            native("ollama", "qwen"),
        ];
        let source = StaticCapacity::new()
            .set("cli:claude-1:sonnet", Capacity::unavailable())
            .set("native:anthropic:sonnet", Capacity::default());
        let code = pick_code(&chain, &source);
        let plain = pick(&chain, &source);
        assert_eq!(code.key, plain.key);
        assert_eq!(code.index, plain.index);
        assert_eq!(code.rejected, plain.rejected);
        assert!(!code.preferred);
        assert_eq!(code.rejected[0].1, RejectReason::Unavailable);
    }

    /// A cooling Codex is skipped, and the preference survives a reroute: the
    /// key already tried never comes back.
    #[test]
    fn the_preference_never_resurrects_a_tried_or_cooling_candidate() {
        let chain = vec![
            cli("claude-1", "sonnet"),
            cli("codex-1", "gpt-5-codex"),
            cli("codex-2", "gpt-5-codex"),
        ];
        let source = StaticCapacity::new()
            .set("cli:claude-1:sonnet", Capacity::default())
            .set("cli:codex-1:gpt-5-codex", codex_cap(None))
            .set("cli:codex-2:gpt-5-codex", codex_cap(None));
        let now = Instant::now();
        let route = select_for(
            &chain,
            &source,
            &RouterState::new(),
            &RouterConfig::default(),
            now,
            &["cli:codex-1:gpt-5-codex".into()],
            TaskKind::Code,
        )
        .unwrap();
        assert_eq!(route.key, "cli:codex-2:gpt-5-codex");
        assert!(route.preferred);

        let mut state = RouterState::new();
        state.rotate_away("cli:codex-1:gpt-5-codex", now);
        state.rotate_away("cli:codex-2:gpt-5-codex", now);
        let route = select_for(
            &chain,
            &source,
            &state,
            &RouterConfig::default(),
            now,
            &[],
            TaskKind::Code,
        )
        .unwrap();
        assert_eq!(route.key, "cli:claude-1:sonnet");
        assert!(!route.preferred);
    }

    #[test]
    fn the_router_handle_takes_the_task_kind_too() {
        let chain = vec![cli("claude-1", "sonnet"), cli("codex-1", "gpt-5-codex")];
        let source = StaticCapacity::new()
            .set("cli:claude-1:sonnet", Capacity::default())
            .set("cli:codex-1:gpt-5-codex", codex_cap(None));
        let router = Router::new(Arc::new(source));
        assert_eq!(router.pick(&chain, &[]).unwrap().key, "cli:claude-1:sonnet");
        assert_eq!(
            router.pick_for(&chain, &[], TaskKind::Code).unwrap().key,
            "cli:codex-1:gpt-5-codex"
        );
    }

    #[test]
    fn the_task_kind_comes_from_the_agent_not_from_the_prompt() {
        use super::super::agent::{Budget, ModelPolicy, RuntimeKind};
        let agent = |id: &str, role: AgentRole| AgentDef {
            id: id.into(),
            name: id.into(),
            role,
            // The prompt is full of the word "code" and must change nothing.
            system_prompt: "write code, review code, ship code".into(),
            model: ModelPolicy::Route { chain: vec![] },
            tools: vec![],
            skills: vec![],
            budget: Budget::default(),
            runtime: RuntimeKind::Native,
            skin: None,
        };
        assert_eq!(
            TaskKind::of_agent(&agent("engineering-manager", AgentRole::Worker)),
            TaskKind::Code
        );
        assert_eq!(
            TaskKind::of_agent(&agent("whatever", AgentRole::Custom("Code".into()))),
            TaskKind::Code
        );
        assert_eq!(
            TaskKind::of_agent(&agent("chief-of-staff", AgentRole::Coordinator)),
            TaskKind::Unspecified
        );
        assert_eq!(
            TaskKind::of_agent(&agent("team-advisor", AgentRole::Custom("poet".into()))),
            TaskKind::Unspecified
        );
        assert_eq!(TaskKind::default(), TaskKind::Unspecified);
    }

    #[test]
    fn model_ref_of_a_cli_candidate_is_namespaced() {
        let m = model_ref(&cli("max-1", "sonnet"));
        assert_eq!(m.provider.as_str(), "cli:max-1");
        assert_eq!(m.model, "sonnet");
    }

    // ── TaskKind::Side (3.7, 84#P3) ────────────────────────────────────

    #[test]
    fn a_side_call_stays_on_the_session_provider_small_model() {
        use crate::core::llm::agent::RuntimeKind;
        let claude = RuntimeKind::Cli {
            cli: "claude".into(),
            account: "max-1".into(),
        };
        let codex = RuntimeKind::Cli {
            cli: "codex".into(),
            account: "pro".into(),
        };
        assert_eq!(side_family(&cli("max-1", "sonnet"), &claude), "claude-cli");
        assert_eq!(side_family(&cli("pro", "gpt-5.6-sol"), &codex), "codex-cli");
        assert_eq!(
            side_family(
                &native("anthropic", "claude-sonnet-5"),
                &RuntimeKind::Native
            ),
            "anthropic"
        );
        let side = |family: &str, model: &str| resolve_side(None, None, family, model);
        assert_eq!(side("claude-cli", "opus").model, "haiku");
        assert_eq!(side("claude-cli", "").model, "haiku");
        assert_eq!(side("claude-cli", "opus").reason, SideReason::ProviderSmall);
        assert_eq!(
            side("anthropic", "claude-sonnet-5").model,
            "claude-haiku-4-5"
        );
        assert_eq!(side("codex-cli", "gpt-5.6-sol").model, "gpt-5.6-luna");
        assert_eq!(side("codex-cli", "gpt-6-astra").model, "gpt-6-luna");
        assert_eq!(side("codex-cli", "gpt-5.6-luna").model, "gpt-5.6-luna");
        assert_eq!(side("openai", "gpt-4o").model, "gpt-4o-mini");
        assert_eq!(side("openai", "gpt-4o-mini").model, "gpt-4o-mini");
        assert_eq!(
            side("google", "gemini-2.5-pro").model,
            "gemini-2.5-flash-lite"
        );
        // Nothing smaller known: the session's own model, and the reason says so.
        let local = side("ollama", "qwen3:8b");
        assert_eq!(local.model, "qwen3:8b");
        assert_eq!(local.reason, SideReason::SessionModel);
        let default_codex = side("codex-cli", "");
        assert_eq!(default_codex.reason, SideReason::SessionModel);
    }

    #[test]
    fn a_requested_side_model_wins_then_the_setting() {
        let r = resolve_side(Some("sonnet"), Some("haiku"), "claude-cli", "opus");
        assert_eq!(
            (r.model.as_str(), r.reason),
            ("sonnet", SideReason::Requested)
        );
        let r = resolve_side(Some("  "), Some("haiku"), "claude-cli", "opus");
        assert_eq!((r.model.as_str(), r.reason), ("haiku", SideReason::Setting));
    }

    #[test]
    fn a_side_agent_declares_side_and_takes_the_first_usable_candidate() {
        use crate::core::llm::agent::{AgentRole, Budget, ModelPolicy, RuntimeKind};
        let agent = AgentDef {
            id: "claude-code".into(),
            name: "Claude Code".into(),
            role: AgentRole::Custom(SIDE_ROLE.into()),
            system_prompt: String::new(),
            model: ModelPolicy::Route { chain: vec![] },
            tools: vec![],
            skills: vec![],
            budget: Budget::default(),
            runtime: RuntimeKind::Cli {
                cli: "claude".into(),
                account: "max-1".into(),
            },
            skin: None,
        };
        assert_eq!(TaskKind::of_agent(&agent), TaskKind::Side);
        assert!(is_side_conversation("side-audit-l1"));
        assert!(!is_side_conversation("loop-l1"));
        let chain = vec![cli("max-1", "haiku"), cli("max-2", "haiku")];
        let caps = StaticCapacity::new();
        let route = select_for(
            &chain,
            &caps,
            &RouterState::new(),
            &RouterConfig::default(),
            Instant::now(),
            &[],
            TaskKind::Side,
        )
        .unwrap();
        assert_eq!(route.index, 0);
        assert!(!route.preferred);
    }
}
