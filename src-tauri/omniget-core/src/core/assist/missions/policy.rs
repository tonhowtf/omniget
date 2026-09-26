//! Lifecycle events and the policies that react to them.
//!
//! An event is a versioned envelope (id, correlation and causation ids,
//! depth, scope, revision, bounded payload). A policy declares its trigger,
//! an optional condition (tool name glob), its effect, whether it is
//! required or advisory, a timeout, a debounce window and how many times it
//! may fire per mission. The engine:
//! - deduplicates by event id (a consumer sees an event once);
//! - never turns a required failure or timeout into a pass: the dispatch
//!   comes back `blocked` with the reason;
//! - bounds an advisory policy by its timeout and records the bypass;
//! - stops recursion: an event caused by a policy carries `depth + 1`, and
//!   past [`MAX_DEPTH`] it is dropped with a diagnostic, whatever the policy
//!   says; `max_fires` caps a policy that keeps re-triggering itself;
//! - runs a `command` effect only for policies the user wrote. Imported and
//!   proposed policies never run scripts (they come back `refused`).
//!
//! The engine decides; effects are carried out by an [`EffectRunner`] the
//! caller provides (the app: checkpoint writer, verifier, shell through the
//! broker). The domain keeps the order of mission transitions; policies do
//! not move missions between states.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::Origin;
use crate::core::assist::db::AssistDb;

pub const ENVELOPE_VERSION: u32 = 1;
/// Events caused by events caused by… stop here.
pub const MAX_DEPTH: u32 = 3;
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    MissionStarted,
    TaskClaimed,
    BeforeTool,
    AfterTool,
    ToolFailed,
    BeforeCompaction,
    CheckpointCreated,
    RunInterrupted,
    BeforeCompletion,
    VerificationFinished,
    MissionFinished,
    FeedbackReceived,
}

impl Trigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Trigger::MissionStarted => "mission_started",
            Trigger::TaskClaimed => "task_claimed",
            Trigger::BeforeTool => "before_tool",
            Trigger::AfterTool => "after_tool",
            Trigger::ToolFailed => "tool_failed",
            Trigger::BeforeCompaction => "before_compaction",
            Trigger::CheckpointCreated => "checkpoint_created",
            Trigger::RunInterrupted => "run_interrupted",
            Trigger::BeforeCompletion => "before_completion",
            Trigger::VerificationFinished => "verification_finished",
            Trigger::MissionFinished => "mission_finished",
            Trigger::FeedbackReceived => "feedback_received",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u32,
    pub event_id: String,
    pub kind: Trigger,
    pub mission_id: String,
    pub correlation_id: String,
    pub causation_id: Option<String>,
    pub depth: u32,
    pub scope: String,
    pub revision: i64,
    pub payload: Value,
    pub ts_ms: i64,
}

impl Envelope {
    pub fn new(kind: Trigger, mission_id: &str, payload: Value) -> Self {
        Self {
            v: ENVELOPE_VERSION,
            event_id: crate::core::assist::new_id(),
            kind,
            mission_id: mission_id.to_string(),
            correlation_id: mission_id.to_string(),
            causation_id: None,
            depth: 0,
            scope: String::new(),
            revision: 0,
            payload: super::clip_json(payload),
            ts_ms: crate::core::assist::now_ms(),
        }
    }

    /// An event caused by this one (one level deeper).
    pub fn child(&self, kind: Trigger, payload: Value) -> Self {
        Self {
            v: ENVELOPE_VERSION,
            event_id: crate::core::assist::new_id(),
            kind,
            mission_id: self.mission_id.clone(),
            correlation_id: self.correlation_id.clone(),
            causation_id: Some(self.event_id.clone()),
            depth: self.depth + 1,
            scope: self.scope.clone(),
            revision: self.revision,
            payload: super::clip_json(payload),
            ts_ms: crate::core::assist::now_ms(),
        }
    }

    pub fn tool(&self) -> Option<&str> {
        self.payload.get("tool").and_then(Value::as_str)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PolicyEffect {
    /// Save a structured checkpoint.
    Checkpoint,
    /// Run the verifiers (one criterion, or all).
    Verify {
        #[serde(default)]
        criterion: Option<String>,
    },
    /// Refuse the tool call (only meaningful on `before_tool`).
    DenyTool { reason: String },
    /// Record a note on the mission.
    Note { text: String },
    /// Run a command through the broker's shell tool (user policies only).
    Command { command: String },
    /// Emit another event (lets policies chain; bounded by depth).
    Emit { trigger: Trigger },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    pub id: String,
    pub trigger: Trigger,
    /// `*`-glob on the tool name (tool events only).
    #[serde(default)]
    pub tool_glob: Option<String>,
    pub effect: PolicyEffect,
    #[serde(default)]
    pub required: bool,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
    #[serde(default)]
    pub debounce_ms: u64,
    /// Most fires per mission (0 = no cap beyond depth).
    #[serde(default)]
    pub max_fires: u32,
    #[serde(default)]
    pub origin: Origin,
}

fn default_timeout() -> u64 {
    DEFAULT_TIMEOUT_MS
}

impl Policy {
    pub fn matches(&self, env: &Envelope) -> bool {
        if env.kind != self.trigger {
            return false;
        }
        match (&self.tool_glob, env.tool()) {
            (None, _) => true,
            (Some(g), Some(t)) => glob(g, t),
            (Some(_), None) => false,
        }
    }
}

/// `*` matches any run of characters; everything else is literal.
pub fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut rest = text;
    for (i, p) in parts.iter().enumerate() {
        if p.is_empty() {
            continue;
        }
        if i == 0 {
            if !rest.starts_with(p) {
                return false;
            }
            rest = &rest[p.len()..];
        } else if i == parts.len() - 1 {
            return rest.ends_with(p);
        } else if let Some(pos) = rest.find(p) {
            rest = &rest[pos + p.len()..];
        } else {
            return false;
        }
    }
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "result", rename_all = "snake_case")]
pub enum PolicyResult {
    Passed,
    Failed {
        reason: String,
    },
    TimedOut {
        after_ms: u64,
    },
    Denied {
        reason: String,
    },
    /// Within the debounce window of a previous fire: folded into it.
    Coalesced,
    /// Not run (imported script, fire cap, recursion limit).
    Refused {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyOutcome {
    pub policy_id: String,
    pub event_id: String,
    pub trigger: Trigger,
    pub depth: u32,
    pub required: bool,
    pub result: PolicyResult,
    pub ms: u64,
}

impl PolicyOutcome {
    /// Required and not passed: the step it guards must not go on.
    pub fn blocks(&self) -> bool {
        self.required && !matches!(self.result, PolicyResult::Passed | PolicyResult::Coalesced)
    }
    /// Advisory and not passed: recorded as a bypass with its reason.
    pub fn bypassed(&self) -> Option<String> {
        if self.required {
            return None;
        }
        match &self.result {
            PolicyResult::Passed | PolicyResult::Coalesced => None,
            PolicyResult::Failed { reason }
            | PolicyResult::Denied { reason }
            | PolicyResult::Refused { reason } => Some(reason.clone()),
            PolicyResult::TimedOut { after_ms } => Some(format!("timed out after {after_ms} ms")),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Dispatch {
    pub outcomes: Vec<PolicyOutcome>,
    /// Set when a required policy failed, timed out or was refused.
    pub blocked: Option<String>,
    /// Set when a `before_tool` policy denied the call.
    pub denied: Option<String>,
    /// Events dropped at the recursion limit.
    pub dropped: Vec<String>,
}

/// What carrying out an effect produced: events to dispatch next.
#[async_trait]
pub trait EffectRunner: Send + Sync {
    async fn run(&self, env: &Envelope, effect: &PolicyEffect) -> Result<Vec<Trigger>, String>;
}

#[derive(Default)]
struct EngineState {
    seen: HashSet<String>,
    last_fire: HashMap<(String, String), i64>,
    fires: HashMap<(String, String), u32>,
}

pub struct Engine {
    policies: Vec<Policy>,
    state: Mutex<EngineState>,
}

impl Engine {
    pub fn new(policies: Vec<Policy>) -> Self {
        Self {
            policies,
            state: Mutex::new(EngineState::default()),
        }
    }

    pub fn policies(&self) -> &[Policy] {
        &self.policies
    }

    /// Dispatches `env` and whatever the effects emit, breadth first.
    pub async fn dispatch(&self, env: Envelope, runner: &dyn EffectRunner) -> Dispatch {
        let mut out = Dispatch::default();
        let mut queue = vec![env];
        while let Some(env) = queue.pop() {
            if env.depth > MAX_DEPTH {
                out.dropped.push(env.event_id.clone());
                out.outcomes.push(PolicyOutcome {
                    policy_id: "-".into(),
                    event_id: env.event_id.clone(),
                    trigger: env.kind,
                    depth: env.depth,
                    required: false,
                    result: PolicyResult::Refused {
                        reason: format!(
                            "recursion limit: event at depth {} (max {MAX_DEPTH}) was dropped",
                            env.depth
                        ),
                    },
                    ms: 0,
                });
                continue;
            }
            {
                let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
                if !st.seen.insert(env.event_id.clone()) {
                    continue;
                }
            }
            for p in self.policies.iter().filter(|p| p.matches(&env)) {
                let key = (p.id.clone(), env.mission_id.clone());
                let started = std::time::Instant::now();
                // Debounce and fire cap, under the lock.
                let gate: Option<PolicyResult> = {
                    let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    let n = st.fires.get(&key).copied().unwrap_or(0);
                    if p.max_fires > 0 && n >= p.max_fires {
                        Some(PolicyResult::Refused {
                            reason: format!(
                                "fired {n} times for this mission (cap {})",
                                p.max_fires
                            ),
                        })
                    } else if p.debounce_ms > 0
                        && st
                            .last_fire
                            .get(&key)
                            .map(|t| env.ts_ms - t < p.debounce_ms as i64)
                            .unwrap_or(false)
                    {
                        Some(PolicyResult::Coalesced)
                    } else {
                        st.fires.insert(key.clone(), n + 1);
                        st.last_fire.insert(key.clone(), env.ts_ms);
                        None
                    }
                };
                let result = match gate {
                    Some(r) => r,
                    None => self.run_one(p, &env, runner, &mut queue).await,
                };
                if let (Trigger::BeforeTool, PolicyResult::Denied { reason }) = (env.kind, &result)
                {
                    out.denied.get_or_insert_with(|| reason.clone());
                }
                let o = PolicyOutcome {
                    policy_id: p.id.clone(),
                    event_id: env.event_id.clone(),
                    trigger: env.kind,
                    depth: env.depth,
                    required: p.required,
                    result,
                    ms: started.elapsed().as_millis() as u64,
                };
                if o.blocks() && out.blocked.is_none() {
                    out.blocked = Some(format!(
                        "required policy `{}` did not pass: {:?}",
                        p.id, o.result
                    ));
                }
                out.outcomes.push(o);
            }
        }
        out
    }

    async fn run_one(
        &self,
        p: &Policy,
        env: &Envelope,
        runner: &dyn EffectRunner,
        queue: &mut Vec<Envelope>,
    ) -> PolicyResult {
        match &p.effect {
            PolicyEffect::Command { .. } if !p.origin.may_execute() => {
                return PolicyResult::Refused {
                    reason: "a script from an import or a model proposal never runs by default"
                        .into(),
                }
            }
            PolicyEffect::DenyTool { reason } => {
                return PolicyResult::Denied {
                    reason: reason.clone(),
                }
            }
            _ => {}
        }
        let fut = runner.run(env, &p.effect);
        match tokio::time::timeout(Duration::from_millis(p.timeout_ms.max(1)), fut).await {
            Err(_) => PolicyResult::TimedOut {
                after_ms: p.timeout_ms,
            },
            Ok(Err(reason)) => PolicyResult::Failed {
                reason: super::clip(&reason, 600),
            },
            Ok(Ok(emitted)) => {
                for t in emitted {
                    queue.push(env.child(t, json!({ "by_policy": p.id })));
                }
                PolicyResult::Passed
            }
        }
    }
}

/// Built-in policies of every mission: checkpoint when a task is claimed and
/// before compaction (advisory, bounded), verification before completion
/// (required).
pub fn defaults() -> Vec<Policy> {
    vec![
        Policy {
            id: "checkpoint-on-claim".into(),
            trigger: Trigger::TaskClaimed,
            tool_glob: None,
            effect: PolicyEffect::Checkpoint,
            required: false,
            timeout_ms: 5_000,
            debounce_ms: 0,
            max_fires: 0,
            origin: Origin::Preset,
        },
        Policy {
            id: "checkpoint-before-compaction".into(),
            trigger: Trigger::BeforeCompaction,
            tool_glob: None,
            effect: PolicyEffect::Checkpoint,
            required: false,
            timeout_ms: 5_000,
            debounce_ms: 10_000,
            max_fires: 0,
            origin: Origin::Preset,
        },
        Policy {
            id: "verify-before-completion".into(),
            trigger: Trigger::BeforeCompletion,
            tool_glob: None,
            effect: PolicyEffect::Verify { criterion: None },
            required: true,
            timeout_ms: 15 * 60_000,
            debounce_ms: 0,
            max_fires: 0,
            origin: Origin::Preset,
        },
    ]
}

/// Writes the outcomes of a dispatch on the mission (bypasses with their
/// reason, blocks, dropped events).
pub fn persist(db: &AssistDb, mission_id: &str, d: &Dispatch) -> Result<(), String> {
    for o in &d.outcomes {
        let kind = if o.blocks() {
            "policy_blocked"
        } else if o.bypassed().is_some() {
            "policy_bypassed"
        } else {
            "policy_outcome"
        };
        super::append_event(
            db,
            mission_id,
            super::NewEvent {
                event_id: Some(format!("{}:{}", o.event_id, o.policy_id)),
                kind: kind.into(),
                correlation_id: Some(mission_id.to_string()),
                causation_id: Some(o.event_id.clone()),
                depth: o.depth as i64,
                scope: String::new(),
                payload: json!({ "policy": o.policy_id, "trigger": o.trigger, "required": o.required, "result": o.result, "ms": o.ms, "bypass_reason": o.bypassed() }),
            },
        )?;
    }
    Ok(())
}
