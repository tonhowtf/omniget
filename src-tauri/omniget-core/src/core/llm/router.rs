use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::agent::{AgentDef, AgentRole, Candidate, CandidateRuntime};
use super::error::{LlmError, ERR_LLM_RATE};
use super::types::{ModelRef, ProviderId};

pub const ERR_CLI_RATE: &str = "ERR_CLI_RATE";
pub const ERR_LLM_NO_CANDIDATE: &str = "ERR_LLM_NO_CANDIDATE";

pub const MIN_QUOTA: f32 = 0.05;
pub const ROTATE_THRESHOLD: f32 = 0.10;
pub const HYSTERESIS_RETURN: f32 = 0.20;
pub const COOLDOWN: Duration = Duration::from_secs(300);
pub const ERROR_WINDOW_SECS: u32 = 60;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TaskKind {
    #[default]
    Unspecified,
    Code,
    Side,
}

pub const SIDE_ROLE: &str = "side";

pub const SIDE_PREFIX: &str = "side-";

pub fn is_side_conversation(id: &str) -> bool {
    id.starts_with(SIDE_PREFIX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideReason {
    Requested,
    Setting,
    ProviderSmall,
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

pub fn small_model_of(family: &str, session_model: &str) -> Option<String> {
    let m = session_model.trim().to_ascii_lowercase();
    let small = |needles: &[&str]| needles.iter().any(|n| m.contains(n));
    match family {
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

pub const CODE_AGENTS: &[&str] = &["engineering-manager"];

impl TaskKind {
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
        let can_write = agent.tools.iter().any(|g| {
            matches!(&g.source, super::agent::ToolSource::Internal { name }
                if super::code_tools::WRITE_TOOLS.contains(&name.as_str()))
                && g.mode != super::agent::GrantMode::Deny
        });
        if can_write && super::code_tools::workspace().is_some() {
            return TaskKind::Code;
        }
        if matches!(
            agent.runtime,
            super::agent::RuntimeKind::Cli { .. } | super::agent::RuntimeKind::Acp { .. }
        ) {
            return TaskKind::Code;
        }
        TaskKind::Unspecified
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityError {
    pub code: String,
    pub seconds_ago: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Capacity {
    pub available: bool,
    pub quota_remaining: Option<f32>,
    pub budget_remaining_usd: Option<f64>,
    pub cost_per_1k: Option<f64>,
    pub context_window: u32,
    pub avg_latency_ms: Option<u32>,
    pub last_error: Option<CapacityError>,
    pub code_specialist: bool,
}

impl Default for Capacity {
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
}

pub trait CapacitySource: Send + Sync {
    fn capacity(&self, candidate: &Candidate) -> Capacity;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AlwaysAvailable;

impl CapacitySource for AlwaysAvailable {
    fn capacity(&self, _candidate: &Candidate) -> Capacity {
        Capacity::default()
    }
}

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
    NearLimit,
    Cooldown,
    Hysteresis,
    AlreadyTried,
}

impl RejectReason {
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
    pub index: usize,
    pub candidate: Candidate,
    pub key: String,
    pub model: ModelRef,
    pub relaxed: bool,
    pub rejected: Vec<(String, RejectReason)>,
    pub preferred: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct RouterConfig {
    pub min_quota: f32,
    pub rotate_threshold: f32,
    pub hysteresis_return: f32,
    pub cooldown: Duration,
    pub error_window_secs: u32,
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

#[derive(Debug, Default)]
pub struct RouterState {
    rotated_away: HashMap<String, Instant>,
}

impl RouterState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rotate_away(&mut self, key: impl Into<String>, now: Instant) {
        self.rotated_away.insert(key.into(), now);
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

    pub fn rotate_away(&self, key: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.rotate_away(key, Instant::now());
    }
}
