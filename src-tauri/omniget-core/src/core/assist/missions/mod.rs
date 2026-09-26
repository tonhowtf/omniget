//! Missions: a durable objective with versioned criteria, dependent tasks,
//! receipts tied to the artifact they checked, a journal of external effects
//! and structured checkpoints. Built on top of what already exists: a task is
//! dispatched as a job (`jobs.db`), each attempt is a run (`runs_*`), a group
//! room can own the mission, and the day budget pools of `llm::budget` hold a
//! pool per mission.
//!
//! The rules this module enforces, whoever calls it:
//! - a run that ended (Stop, Length, "DONE", a convincing summary) is not an
//!   objective met. The only way into `succeeded` is [`complete`], which
//!   needs every required criterion of the current version to have a `pass`
//!   receipt for the artifact as it is now ([`verify::verdict`]);
//! - a receipt whose artifact changed afterwards is invalidated, never
//!   reused (A03);
//! - a task is claimed by one owner at a time, with a lease; an expired lease
//!   is recoverable, unless the task left an effect whose outcome is unknown,
//!   in which case the task blocks and says why (A04, A06);
//! - dependents of a failed or cancelled task are never dispatched (A07);
//! - cancelling stops new dispatches and reports the live attempts to cancel;
//!   pausing is not failing;
//! - every state change is written in one transaction with its event, and
//!   repeating a transition to the state a mission is already in is a no-op.

pub mod checkpoint;
pub mod diag;
pub mod hooks;
pub mod policy;
pub mod progress;
pub mod verify;

#[cfg(test)]
mod tests;

use rusqlite::{params, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::db::{AssistDb, Migration};
use super::{new_id, now_ms};

pub const ERR_MISSION: &str = "ERR_MISSION";
pub const ERR_MISSION_NOT_FOUND: &str = "ERR_MISSION_NOT_FOUND";
pub const ERR_MISSION_STATE: &str = "ERR_MISSION_STATE";
pub const ERR_MISSION_CLAIMED: &str = "ERR_MISSION_CLAIMED";
pub const ERR_MISSION_INPUT: &str = "ERR_MISSION_INPUT";
pub const ERR_MISSION_REVISION: &str = "ERR_MISSION_REVISION";
pub const ERR_MISSION_EFFECT_UNKNOWN: &str = "ERR_MISSION_EFFECT_UNKNOWN";
pub const ERR_MISSION_PIN_CHANGED: &str = "ERR_MISSION_PIN_CHANGED";
pub const ERR_MISSION_BUDGET: &str = "ERR_MISSION_BUDGET";
pub const ERR_MISSION_CRITERIA: &str = "ERR_MISSION_CRITERIA";
pub const ERR_MISSION_STAGNANT: &str = "ERR_MISSION_STAGNANT";
pub const ERR_MISSION_UNSUPPORTED: &str = "ERR_MISSION_UNSUPPORTED";

/// Payloads (events, evidence) are clipped to this many bytes.
pub const PAYLOAD_MAX: usize = 4 * 1024;
/// Events kept per mission; later ones only bump `events_dropped`.
pub const EVENTS_PER_MISSION_MAX: i64 = 5_000;
/// Default lease of a claimed task.
pub const DEFAULT_LEASE_MS: i64 = 5 * 60 * 1000;
/// Tasks a mission may hold (a plan is small by design).
pub const MAX_TASKS: usize = 24;
/// Criteria a mission may hold.
pub const MAX_CRITERIA: usize = 16;

pub const MIGRATIONS: &[Migration] = &[Migration {
    module: "missions",
    version: 1,
    sql: "
CREATE TABLE missions_missions (
    id TEXT PRIMARY KEY,
    principal TEXT NOT NULL,
    bot_id TEXT,
    room_id TEXT,
    conversation_id TEXT NOT NULL,
    auth_origin TEXT NOT NULL,
    objective TEXT NOT NULL,
    state TEXT NOT NULL,
    criteria_version INTEGER NOT NULL DEFAULT 1,
    budget TEXT NOT NULL DEFAULT '{}',
    spent TEXT NOT NULL DEFAULT '{}',
    workspace TEXT,
    context_kind TEXT NOT NULL DEFAULT 'projectless',
    pins TEXT,
    allow_replay INTEGER NOT NULL DEFAULT 0,
    block TEXT,
    result TEXT,
    revision INTEGER NOT NULL DEFAULT 1,
    events_dropped INTEGER NOT NULL DEFAULT 0,
    created_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL,
    started_ms INTEGER,
    finished_ms INTEGER
);
CREATE INDEX missions_missions_state ON missions_missions(state, updated_ms DESC);
CREATE INDEX missions_missions_conversation ON missions_missions(conversation_id);
CREATE TABLE missions_criteria (
    mission_id TEXT NOT NULL REFERENCES missions_missions(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    id TEXT NOT NULL,
    position INTEGER NOT NULL,
    kind TEXT NOT NULL,
    severity TEXT NOT NULL,
    title TEXT NOT NULL,
    spec TEXT NOT NULL DEFAULT '{}',
    origin TEXT NOT NULL,
    acceptance TEXT NOT NULL,
    PRIMARY KEY (mission_id, version, id)
);
CREATE TABLE missions_tasks (
    id TEXT PRIMARY KEY,
    mission_id TEXT NOT NULL REFERENCES missions_missions(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    title TEXT NOT NULL,
    input TEXT NOT NULL,
    deps TEXT NOT NULL DEFAULT '[]',
    bot_id TEXT,
    state TEXT NOT NULL,
    owner TEXT,
    lease_until_ms INTEGER,
    attempts INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    job_ids TEXT NOT NULL DEFAULT '[]',
    result TEXT,
    error TEXT,
    created_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL
);
CREATE INDEX missions_tasks_mission ON missions_tasks(mission_id, position);
CREATE TABLE missions_receipts (
    id TEXT PRIMARY KEY,
    mission_id TEXT NOT NULL REFERENCES missions_missions(id) ON DELETE CASCADE,
    criterion_id TEXT NOT NULL,
    criterion_version INTEGER NOT NULL,
    task_id TEXT,
    run_id TEXT,
    artifact_ref TEXT NOT NULL DEFAULT '',
    artifact_digest TEXT NOT NULL,
    verifier TEXT NOT NULL,
    verifier_version TEXT NOT NULL,
    status TEXT NOT NULL,
    exit_code INTEGER,
    confidence TEXT,
    evidence TEXT NOT NULL DEFAULT '',
    created_ms INTEGER NOT NULL,
    invalidated_ms INTEGER,
    invalidated_reason TEXT
);
CREATE INDEX missions_receipts_mission ON missions_receipts(mission_id, criterion_id, created_ms DESC);
CREATE TABLE missions_effects (
    key TEXT PRIMARY KEY,
    mission_id TEXT NOT NULL REFERENCES missions_missions(id) ON DELETE CASCADE,
    task_id TEXT,
    kind TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    state TEXT NOT NULL,
    instance_id TEXT NOT NULL,
    detail TEXT,
    created_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL
);
CREATE INDEX missions_effects_mission ON missions_effects(mission_id, state);
CREATE TABLE missions_checkpoints (
    id TEXT PRIMARY KEY,
    mission_id TEXT NOT NULL REFERENCES missions_missions(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    reason TEXT NOT NULL,
    state TEXT NOT NULL,
    created_ms INTEGER NOT NULL,
    UNIQUE(mission_id, seq)
);
CREATE TABLE missions_events (
    event_id TEXT PRIMARY KEY,
    mission_id TEXT NOT NULL REFERENCES missions_missions(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    v INTEGER NOT NULL DEFAULT 1,
    kind TEXT NOT NULL,
    correlation_id TEXT,
    causation_id TEXT,
    depth INTEGER NOT NULL DEFAULT 0,
    scope TEXT NOT NULL DEFAULT '',
    revision INTEGER NOT NULL DEFAULT 0,
    payload TEXT NOT NULL DEFAULT '{}',
    ts_ms INTEGER NOT NULL,
    UNIQUE(mission_id, seq)
);
",
}];

// ── States ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissionState {
    Draft,
    Queued,
    Running,
    Verifying,
    Succeeded,
    Partial,
    Blocked,
    Paused,
    Failed,
    Cancelled,
}

impl MissionState {
    pub const ALL: [MissionState; 10] = [
        MissionState::Draft,
        MissionState::Queued,
        MissionState::Running,
        MissionState::Verifying,
        MissionState::Succeeded,
        MissionState::Partial,
        MissionState::Blocked,
        MissionState::Paused,
        MissionState::Failed,
        MissionState::Cancelled,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            MissionState::Draft => "draft",
            MissionState::Queued => "queued",
            MissionState::Running => "running",
            MissionState::Verifying => "verifying",
            MissionState::Succeeded => "succeeded",
            MissionState::Partial => "partial",
            MissionState::Blocked => "blocked",
            MissionState::Paused => "paused",
            MissionState::Failed => "failed",
            MissionState::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.as_str() == s)
    }

    /// Nothing moves these on (a new mission is the way to try again).
    pub fn is_final(self) -> bool {
        matches!(self, MissionState::Succeeded | MissionState::Cancelled)
    }

    /// States in which the driver may dispatch work.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            MissionState::Queued | MissionState::Running | MissionState::Verifying
        )
    }

    /// The state machine. Same state is a no-op, not an error. `succeeded`
    /// is only reachable from `verifying`, and [`complete`] is the only
    /// caller that asks for it.
    pub fn can_go(self, to: MissionState) -> bool {
        use MissionState::*;
        if self == to {
            return true;
        }
        match self {
            Draft => matches!(to, Queued | Cancelled),
            Queued => matches!(to, Running | Paused | Blocked | Cancelled | Failed),
            Running => matches!(
                to,
                Verifying | Blocked | Paused | Partial | Failed | Cancelled
            ),
            Verifying => matches!(
                to,
                Running | Succeeded | Blocked | Partial | Failed | Paused | Cancelled
            ),
            // A person (or a resolved block) puts these back in the queue.
            Blocked | Paused | Partial | Failed => matches!(to, Queued | Cancelled),
            Succeeded | Cancelled => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// Waiting for its dependencies or a claim.
    Pending,
    Claimed,
    Running,
    Done,
    Failed,
    Cancelled,
    /// Never dispatched because a dependency failed or was cancelled.
    Skipped,
    /// Needs a person: an effect of unknown outcome, a missing capability.
    Blocked,
}

impl TaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskState::Pending => "pending",
            TaskState::Claimed => "claimed",
            TaskState::Running => "running",
            TaskState::Done => "done",
            TaskState::Failed => "failed",
            TaskState::Cancelled => "cancelled",
            TaskState::Skipped => "skipped",
            TaskState::Blocked => "blocked",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "claimed" => TaskState::Claimed,
            "running" => TaskState::Running,
            "done" => TaskState::Done,
            "failed" => TaskState::Failed,
            "cancelled" => TaskState::Cancelled,
            "skipped" => TaskState::Skipped,
            "blocked" => TaskState::Blocked,
            "pending" => TaskState::Pending,
            // A state this build does not know is never dispatchable: it
            // needs a person, like a blocked task (and never allows success).
            _ => TaskState::Blocked,
        }
    }
    pub fn is_live(self) -> bool {
        matches!(self, TaskState::Claimed | TaskState::Running)
    }
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskState::Done | TaskState::Failed | TaskState::Cancelled | TaskState::Skipped
        )
    }
    /// A dependency in this state stops its dependents for good.
    pub fn poisons(self) -> bool {
        matches!(
            self,
            TaskState::Failed | TaskState::Cancelled | TaskState::Skipped
        )
    }
}

// ── Criteria ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterionKind {
    /// A command/test run in the mission workspace; pass = exit 0.
    Command,
    /// A file (or files) with checks on its content or shape.
    Artifact,
    /// A fact recorded by an assistant tool (a reading round, …).
    ToolResult,
    /// A subjective judgement with a visible rubric. Never shown as proof.
    Rubric,
    /// A person accepts or rejects.
    Human,
}

impl CriterionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CriterionKind::Command => "command",
            CriterionKind::Artifact => "artifact",
            CriterionKind::ToolResult => "tool_result",
            CriterionKind::Rubric => "rubric",
            CriterionKind::Human => "human",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "command" => CriterionKind::Command,
            "artifact" => CriterionKind::Artifact,
            "tool_result" => CriterionKind::ToolResult,
            "rubric" => CriterionKind::Rubric,
            "human" => CriterionKind::Human,
            _ => return None,
        })
    }
    /// Whether a machine can decide it without judgement.
    pub fn is_objective(self) -> bool {
        matches!(
            self,
            CriterionKind::Command | CriterionKind::Artifact | CriterionKind::ToolResult
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    #[default]
    Required,
    Advisory,
}

/// Who decides a criterion once its verifier ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Acceptance {
    /// The verifier's receipt decides.
    #[default]
    Auto,
    /// A person decides (rubric shown; a review can inform, not decide).
    Human,
    /// One recorded review with confidence is enough (opt-in per criterion).
    SingleReview,
}

/// Where a criterion came from. Only `user` and `preset` criteria may run a
/// command: text from a page, a skill or an import is data, never authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    #[default]
    User,
    Preset,
    /// Proposed by a model: shown, not executable until a person approves it.
    Proposed,
    Import,
}

impl Origin {
    pub fn may_execute(self) -> bool {
        matches!(self, Origin::User | Origin::Preset)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Criterion {
    pub id: String,
    #[serde(default)]
    pub version: i64,
    pub kind: CriterionKind,
    #[serde(default)]
    pub severity: Severity,
    pub title: String,
    #[serde(default)]
    pub spec: Value,
    #[serde(default)]
    pub origin: Origin,
    #[serde(default)]
    pub acceptance: Acceptance,
}

impl Criterion {
    /// Paths (relative to the workspace) whose content the verdict depends on.
    pub fn artifact_paths(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        if let Some(p) = self.spec.get("path").and_then(Value::as_str) {
            out.push(p.to_string());
        }
        if let Some(arr) = self.spec.get("artifacts").and_then(Value::as_array) {
            out.extend(arr.iter().filter_map(Value::as_str).map(str::to_string));
        }
        out.sort();
        out.dedup();
        out
    }
}

// ── Records ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MissionBudget {
    pub usd: Option<f64>,
    pub tokens: Option<u64>,
    pub turns: Option<u32>,
    pub max_minutes: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MissionSpent {
    pub usd_known: f64,
    /// Calls whose cost the provider did not report: never shown as free.
    pub unknown_cost_calls: u32,
    pub tokens: u64,
    pub turns: u32,
    /// Spend by purpose (`work`, `review`, `summary`, `learning`).
    pub by_purpose: std::collections::BTreeMap<String, u64>,
    /// Milliseconds of work in finished activations: the clock
    /// `max_minutes` counts. Running and automated verification are work;
    /// paused/blocked/queued time and time in `verifying` waiting only for a
    /// person's decision are not.
    pub active_ms: u64,
    /// Start of the current activation; `None` while the clock is stopped
    /// (including a `verifying` mission that waits for a person).
    pub active_since_ms: Option<i64>,
}

/// Folds the current activation into `active_ms` and stops the clock.
pub(crate) fn stop_clock(spent: &mut MissionSpent, now: i64) {
    if let Some(since) = spent.active_since_ms.take() {
        spent.active_ms += (now - since).max(0) as u64;
    }
}

/// Restarts the work clock of a `verifying` mission whose clock stopped while
/// it waited for a person, because automated verification runs again (a
/// re-check after an acceptance, a boot). No-op in any other state.
pub fn start_clock(db: &AssistDb, mission_id: &str) -> Result<Mission, String> {
    db.tx(|tx| {
        let mut m = get_tx(tx, mission_id)?;
        if m.state == MissionState::Verifying && m.spent.active_since_ms.is_none() {
            m.spent.active_since_ms = Some(now_ms());
            tx.execute(
                "UPDATE missions_missions SET spent = ?2 WHERE id = ?1",
                params![mission_id, serde_json::to_string(&m.spent).map_err(e)?],
            )
            .map_err(e)?;
        }
        Ok(m)
    })
}

/// Work time of the mission up to `now` (F7): running, plus verifying while
/// automated checks run. Time waiting for a person's decision is not work.
pub fn active_ms(m: &Mission, now: i64) -> u64 {
    m.spent.active_ms
        + m.spent
            .active_since_ms
            .map(|s| (now - s).max(0) as u64)
            .unwrap_or(0)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mission {
    pub id: String,
    pub principal: String,
    pub bot_id: Option<String>,
    pub room_id: Option<String>,
    pub conversation_id: String,
    /// `user_ui`, `chat:<conversation>`, `trigger:<id>`, `mcp`.
    pub auth_origin: String,
    pub objective: String,
    pub state: MissionState,
    pub criteria_version: i64,
    pub budget: MissionBudget,
    pub spent: MissionSpent,
    pub workspace: Option<String>,
    pub context_kind: String,
    /// Account/cwd/runtime pinned when the mission first ran.
    pub pins: Option<Value>,
    /// The user allowed a replay on a different account/folder/runtime.
    pub allow_replay: bool,
    /// Why the mission is blocked/partial/failed ([`diag::Diagnosis`]).
    pub block: Option<Value>,
    pub result: Option<Value>,
    pub revision: i64,
    pub events_dropped: i64,
    pub created_ms: i64,
    pub updated_ms: i64,
    pub started_ms: Option<i64>,
    pub finished_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissionTask {
    pub id: String,
    pub mission_id: String,
    pub position: i64,
    pub title: String,
    pub input: String,
    pub deps: Vec<String>,
    pub bot_id: Option<String>,
    pub state: TaskState,
    pub owner: Option<String>,
    pub lease_until_ms: Option<i64>,
    pub attempts: i64,
    pub max_attempts: i64,
    pub job_ids: Vec<String>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissionReceipt {
    pub id: String,
    pub mission_id: String,
    pub criterion_id: String,
    pub criterion_version: i64,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub artifact_ref: String,
    pub artifact_digest: String,
    pub verifier: String,
    pub verifier_version: String,
    /// `pass|fail|unknown`
    pub status: String,
    pub exit_code: Option<i64>,
    /// Rubric reviews only: `low|medium|high`.
    pub confidence: Option<String>,
    pub evidence: String,
    pub created_ms: i64,
    pub invalidated_ms: Option<i64>,
    pub invalidated_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectState {
    Pending,
    Running,
    Confirmed,
    Failed,
    Unknown,
    /// Outcome still unknown, and a person chose to go on anyway: settled
    /// by that recorded decision (never shown as confirmed).
    UnknownAccepted,
}

impl EffectState {
    pub fn as_str(self) -> &'static str {
        match self {
            EffectState::Pending => "pending",
            EffectState::Running => "running",
            EffectState::Confirmed => "confirmed",
            EffectState::Failed => "failed",
            EffectState::Unknown => "unknown",
            EffectState::UnknownAccepted => "unknown_accepted",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "pending" => EffectState::Pending,
            "running" => EffectState::Running,
            "confirmed" => EffectState::Confirmed,
            "failed" => EffectState::Failed,
            "unknown_accepted" => EffectState::UnknownAccepted,
            _ => EffectState::Unknown,
        }
    }
    /// Settled: nothing is waiting to learn what happened.
    pub fn is_settled(self) -> bool {
        matches!(
            self,
            EffectState::Confirmed | EffectState::Failed | EffectState::UnknownAccepted
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    pub key: String,
    pub mission_id: String,
    pub task_id: Option<String>,
    pub kind: String,
    pub fingerprint: String,
    pub state: EffectState,
    pub instance_id: String,
    pub detail: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissionEvent {
    pub event_id: String,
    pub mission_id: String,
    pub seq: i64,
    pub v: i64,
    pub kind: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    pub depth: i64,
    pub scope: String,
    pub revision: i64,
    pub payload: Value,
    pub ts_ms: i64,
}

/// Everything the UI shows for one mission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissionDetail {
    pub mission: Mission,
    pub criteria: Vec<Criterion>,
    pub tasks: Vec<MissionTask>,
    pub receipts: Vec<MissionReceipt>,
    pub effects: Vec<Effect>,
    pub verdict: verify::Verdict,
    pub last_checkpoint: Option<checkpoint::Checkpoint>,
    pub events: Vec<MissionEvent>,
}

// ── Input ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NewTask {
    /// Caller-chosen key, so `deps` can name tasks of the same draft.
    pub key: String,
    pub title: String,
    pub input: String,
    pub deps: Vec<String>,
    pub bot_id: Option<String>,
    pub max_attempts: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NewMission {
    pub objective: String,
    pub bot_id: Option<String>,
    pub room_id: Option<String>,
    pub conversation_id: Option<String>,
    pub auth_origin: String,
    pub criteria: Vec<Criterion>,
    pub tasks: Vec<NewTask>,
    pub budget: MissionBudget,
    pub workspace: Option<String>,
    /// Start in `queued` instead of `draft`.
    pub start: bool,
}

// ── Helpers ───────────────────────────────────────────────────────────────

pub(crate) fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

pub(crate) fn clip_json(v: Value) -> Value {
    let text = v.to_string();
    if text.len() <= PAYLOAD_MAX {
        return v;
    }
    json!({ "clipped": true, "bytes": text.len(), "head": clip(&text, PAYLOAD_MAX - 64) })
}

fn e<E: std::fmt::Display>(err: E) -> String {
    format!("{ERR_MISSION}: {err}")
}

fn input_err(msg: impl std::fmt::Display) -> String {
    format!("{ERR_MISSION_INPUT}: {msg}")
}

const MISSION_COLS: &str = "id, principal, bot_id, room_id, conversation_id, auth_origin, objective, state, criteria_version, budget, spent, workspace, context_kind, pins, allow_replay, block, result, revision, events_dropped, created_ms, updated_ms, started_ms, finished_ms";

fn row_mission(r: &Row<'_>) -> rusqlite::Result<Mission> {
    let json_opt = |s: Option<String>| s.and_then(|t| serde_json::from_str::<Value>(&t).ok());
    Ok(Mission {
        id: r.get(0)?,
        principal: r.get(1)?,
        bot_id: r.get(2)?,
        room_id: r.get(3)?,
        conversation_id: r.get(4)?,
        auth_origin: r.get(5)?,
        objective: r.get(6)?,
        state: MissionState::parse(&r.get::<_, String>(7)?).unwrap_or(MissionState::Blocked),
        criteria_version: r.get(8)?,
        budget: serde_json::from_str(&r.get::<_, String>(9)?).unwrap_or_default(),
        spent: serde_json::from_str(&r.get::<_, String>(10)?).unwrap_or_default(),
        workspace: r.get(11)?,
        context_kind: r.get(12)?,
        pins: json_opt(r.get(13)?),
        allow_replay: r.get::<_, i64>(14)? != 0,
        block: json_opt(r.get(15)?),
        result: json_opt(r.get(16)?),
        revision: r.get(17)?,
        events_dropped: r.get(18)?,
        created_ms: r.get(19)?,
        updated_ms: r.get(20)?,
        started_ms: r.get(21)?,
        finished_ms: r.get(22)?,
    })
}

const TASK_COLS: &str = "id, mission_id, position, title, input, deps, bot_id, state, owner, lease_until_ms, attempts, max_attempts, job_ids, result, error, created_ms, updated_ms";

fn row_task(r: &Row<'_>) -> rusqlite::Result<MissionTask> {
    Ok(MissionTask {
        id: r.get(0)?,
        mission_id: r.get(1)?,
        position: r.get(2)?,
        title: r.get(3)?,
        input: r.get(4)?,
        deps: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
        bot_id: r.get(6)?,
        state: TaskState::parse(&r.get::<_, String>(7)?),
        owner: r.get(8)?,
        lease_until_ms: r.get(9)?,
        attempts: r.get(10)?,
        max_attempts: r.get(11)?,
        job_ids: serde_json::from_str(&r.get::<_, String>(12)?).unwrap_or_default(),
        result: r.get(13)?,
        error: r.get(14)?,
        created_ms: r.get(15)?,
        updated_ms: r.get(16)?,
    })
}

const CRITERION_COLS: &str = "id, version, kind, severity, title, spec, origin, acceptance";

fn row_criterion(r: &Row<'_>) -> rusqlite::Result<Criterion> {
    let parse = |s: String, what: &str| -> Value { Value::String(format!("{what}:{s}")) };
    let _ = parse;
    Ok(Criterion {
        id: r.get(0)?,
        version: r.get(1)?,
        kind: CriterionKind::parse(&r.get::<_, String>(2)?).unwrap_or(CriterionKind::Human),
        severity: serde_json::from_value(Value::String(r.get(3)?)).unwrap_or_default(),
        title: r.get(4)?,
        spec: serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or(Value::Null),
        origin: serde_json::from_value(Value::String(r.get(6)?)).unwrap_or(Origin::Import),
        acceptance: serde_json::from_value(Value::String(r.get(7)?)).unwrap_or(Acceptance::Human),
    })
}

const RECEIPT_COLS: &str = "id, mission_id, criterion_id, criterion_version, task_id, run_id, artifact_ref, artifact_digest, verifier, verifier_version, status, exit_code, confidence, evidence, created_ms, invalidated_ms, invalidated_reason";

fn row_receipt(r: &Row<'_>) -> rusqlite::Result<MissionReceipt> {
    Ok(MissionReceipt {
        id: r.get(0)?,
        mission_id: r.get(1)?,
        criterion_id: r.get(2)?,
        criterion_version: r.get(3)?,
        task_id: r.get(4)?,
        run_id: r.get(5)?,
        artifact_ref: r.get(6)?,
        artifact_digest: r.get(7)?,
        verifier: r.get(8)?,
        verifier_version: r.get(9)?,
        status: r.get(10)?,
        exit_code: r.get(11)?,
        confidence: r.get(12)?,
        evidence: r.get(13)?,
        created_ms: r.get(14)?,
        invalidated_ms: r.get(15)?,
        invalidated_reason: r.get(16)?,
    })
}

const EFFECT_COLS: &str =
    "key, mission_id, task_id, kind, fingerprint, state, instance_id, detail, created_ms, updated_ms";

fn row_effect(r: &Row<'_>) -> rusqlite::Result<Effect> {
    Ok(Effect {
        key: r.get(0)?,
        mission_id: r.get(1)?,
        task_id: r.get(2)?,
        kind: r.get(3)?,
        fingerprint: r.get(4)?,
        state: EffectState::parse(&r.get::<_, String>(5)?),
        instance_id: r.get(6)?,
        detail: r.get(7)?,
        created_ms: r.get(8)?,
        updated_ms: r.get(9)?,
    })
}

const EVENT_COLS: &str = "event_id, mission_id, seq, v, kind, correlation_id, causation_id, depth, scope, revision, payload, ts_ms";

fn row_event(r: &Row<'_>) -> rusqlite::Result<MissionEvent> {
    Ok(MissionEvent {
        event_id: r.get(0)?,
        mission_id: r.get(1)?,
        seq: r.get(2)?,
        v: r.get(3)?,
        kind: r.get(4)?,
        correlation_id: r.get(5)?,
        causation_id: r.get(6)?,
        depth: r.get(7)?,
        scope: r.get(8)?,
        revision: r.get(9)?,
        payload: serde_json::from_str(&r.get::<_, String>(10)?).unwrap_or(Value::Null),
        ts_ms: r.get(11)?,
    })
}

fn enum_str<T: Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

// ── Validation ────────────────────────────────────────────────────────────

fn valid_key(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Checks a criteria set before it is stored. Returns the normalised list.
pub fn validate_criteria(criteria: &[Criterion]) -> Result<Vec<Criterion>, String> {
    if criteria.len() > MAX_CRITERIA {
        return Err(input_err(format!("at most {MAX_CRITERIA} criteria")));
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (i, c) in criteria.iter().enumerate() {
        let mut c = c.clone();
        if c.id.trim().is_empty() {
            c.id = format!("c{}", i + 1);
        }
        if !valid_key(&c.id) {
            return Err(input_err(format!(
                "criterion id `{}` must be [A-Za-z0-9_-]",
                c.id
            )));
        }
        if !seen.insert(c.id.clone()) {
            return Err(input_err(format!("criterion `{}` appears twice", c.id)));
        }
        if c.title.trim().is_empty() {
            return Err(input_err(format!("criterion `{}` needs a title", c.id)));
        }
        match c.kind {
            CriterionKind::Command => {
                let cmd = c.spec.get("command").and_then(Value::as_str).unwrap_or("");
                if cmd.trim().is_empty() {
                    return Err(input_err(format!("criterion `{}`: command is empty", c.id)));
                }
            }
            CriterionKind::Artifact => {
                if c.spec
                    .get("path")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
                {
                    return Err(input_err(format!(
                        "criterion `{}`: artifact needs a path",
                        c.id
                    )));
                }
            }
            CriterionKind::ToolResult => {
                if c.spec
                    .get("check")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .is_empty()
                {
                    return Err(input_err(format!(
                        "criterion `{}`: tool_result needs `check`",
                        c.id
                    )));
                }
            }
            CriterionKind::Rubric => {
                let items = c
                    .spec
                    .get("rubric")
                    .and_then(Value::as_array)
                    .map(Vec::len)
                    .unwrap_or(0);
                if items == 0 {
                    return Err(input_err(format!(
                        "criterion `{}`: a rubric lists what is judged",
                        c.id
                    )));
                }
                // A rubric is judgement: it never decides alone unless the
                // criterion says one review is enough.
                if c.acceptance == Acceptance::Auto {
                    c.acceptance = Acceptance::Human;
                }
            }
            CriterionKind::Human => c.acceptance = Acceptance::Human,
        }
        for p in c.artifact_paths() {
            verify::check_rel_path(&p).map_err(input_err)?;
        }
        out.push(c);
    }
    Ok(out)
}

fn plan_tasks(tasks: &[NewTask]) -> Result<Vec<(String, NewTask, Vec<String>)>, String> {
    if tasks.len() > MAX_TASKS {
        return Err(input_err(format!("at most {MAX_TASKS} tasks")));
    }
    let mut ids: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut keyed = Vec::new();
    for (i, t) in tasks.iter().enumerate() {
        let key = if t.key.trim().is_empty() {
            format!("t{}", i + 1)
        } else {
            t.key.clone()
        };
        if ids.contains_key(&key) {
            return Err(input_err(format!("task key `{key}` appears twice")));
        }
        if t.input.trim().is_empty() && t.title.trim().is_empty() {
            return Err(input_err(format!("task `{key}` is empty")));
        }
        ids.insert(key.clone(), new_id());
        keyed.push((key, t.clone()));
    }
    // Dependencies name earlier keys only: no cycles by construction.
    let mut out = Vec::new();
    let mut earlier: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (key, t) in keyed {
        let mut deps = Vec::new();
        for d in &t.deps {
            if !earlier.contains(d) {
                return Err(input_err(format!(
                    "task `{key}` depends on `{d}`, which is not an earlier task"
                )));
            }
            deps.push(ids[d].clone());
        }
        earlier.insert(key.clone());
        out.push((ids[&key].clone(), t, deps));
    }
    Ok(out)
}

// ── Events ────────────────────────────────────────────────────────────────

/// One event to append. `event_id` makes it idempotent: the same id twice
/// is stored once.
#[derive(Debug, Clone, Default)]
pub struct NewEvent {
    pub event_id: Option<String>,
    pub kind: String,
    pub correlation_id: Option<String>,
    pub causation_id: Option<String>,
    pub depth: i64,
    pub scope: String,
    pub payload: Value,
}

impl NewEvent {
    pub fn new(kind: &str, payload: Value) -> Self {
        Self {
            kind: kind.to_string(),
            payload,
            ..Default::default()
        }
    }
}

/// Appends inside a transaction. Returns `None` when the event id was
/// already stored or the mission reached its event cap.
pub fn append_event_tx(
    tx: &Transaction,
    mission_id: &str,
    ev: NewEvent,
) -> Result<Option<String>, String> {
    let id = ev.event_id.clone().unwrap_or_else(new_id);
    let exists: bool = tx
        .query_row(
            "SELECT 1 FROM missions_events WHERE event_id = ?1",
            params![id],
            |_| Ok(()),
        )
        .optional()
        .map_err(e)?
        .is_some();
    if exists {
        return Ok(None);
    }
    let (n, revision): (i64, i64) = tx
        .query_row(
            "SELECT (SELECT count(*) FROM missions_events WHERE mission_id = ?1), \
             (SELECT revision FROM missions_missions WHERE id = ?1)",
            params![mission_id],
            |r| Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0))),
        )
        .map_err(e)?;
    // Lifecycle events are never dropped: a client following the cursor must
    // always see the state changes and the end (F10). Chatty kinds
    // (after_tool, …) are what the cap drops.
    if n >= EVENTS_PER_MISSION_MAX && !is_lifecycle_event(&ev.kind) {
        tx.execute(
            "UPDATE missions_missions SET events_dropped = events_dropped + 1 WHERE id = ?1",
            params![mission_id],
        )
        .map_err(e)?;
        return Ok(None);
    }
    let seq: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM missions_events WHERE mission_id = ?1",
            params![mission_id],
            |r| r.get(0),
        )
        .map_err(e)?;
    tx.execute(
        &format!("INSERT INTO missions_events ({EVENT_COLS}) VALUES (?1,?2,?3,1,?4,?5,?6,?7,?8,?9,?10,?11)"),
        params![
            id,
            mission_id,
            seq,
            ev.kind,
            ev.correlation_id,
            ev.causation_id,
            ev.depth,
            ev.scope,
            revision,
            clip_json(redact_strings(ev.payload)).to_string(),
            now_ms()
        ],
    )
    .map_err(e)?;
    Ok(Some(id))
}

/// Kinds the event cap never drops.
pub fn is_lifecycle_event(kind: &str) -> bool {
    matches!(
        kind,
        "mission_created"
            | "mission_started"
            | "state_changed"
            | "mission_finished"
            | "diagnosis"
            | "round_observed"
            | "rounds_reset"
            | "effect_accepted_unknown"
    )
}

/// Secrets and home paths out of every string of a payload before it is
/// stored (C11). Keys are kept: event keys such as `key` (an effect key) or
/// `policy` are identifiers, not secrets.
pub(crate) fn redact_strings(v: Value) -> Value {
    match v {
        Value::String(s) => Value::String(diag::redact(&s)),
        Value::Array(a) => Value::Array(a.into_iter().map(redact_strings).collect()),
        Value::Object(o) => {
            Value::Object(o.into_iter().map(|(k, v)| (k, redact_strings(v))).collect())
        }
        other => other,
    }
}

pub fn append_event(
    db: &AssistDb,
    mission_id: &str,
    ev: NewEvent,
) -> Result<Option<String>, String> {
    db.tx(|tx| append_event_tx(tx, mission_id, ev))
}

pub fn events(
    db: &AssistDb,
    mission_id: &str,
    after_seq: i64,
    limit: u32,
) -> Result<Vec<MissionEvent>, String> {
    db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {EVENT_COLS} FROM missions_events WHERE mission_id = ?1 AND seq > ?2 ORDER BY seq LIMIT ?3"
        ))?;
        let rows = st.query_map(params![mission_id, after_seq, limit.clamp(1, 1000)], row_event)?;
        rows.collect()
    })
}

/// The newest `limit` events, oldest first.
pub fn recent_events(
    db: &AssistDb,
    mission_id: &str,
    limit: u32,
) -> Result<Vec<MissionEvent>, String> {
    let mut out: Vec<MissionEvent> = db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {EVENT_COLS} FROM missions_events WHERE mission_id = ?1 ORDER BY seq DESC LIMIT ?2"
        ))?;
        let rows = st.query_map(params![mission_id, limit.clamp(1, 1000)], row_event)?;
        rows.collect()
    })?;
    out.reverse();
    Ok(out)
}

// ── Missions ──────────────────────────────────────────────────────────────

/// Creates a mission with its criteria (version 1) and tasks, in one
/// transaction. A mission needs at least one required criterion: without
/// one there is nothing that could ever make it `succeeded`.
pub fn create(db: &AssistDb, new: NewMission) -> Result<MissionDetail, String> {
    create_inner(db, new, None)
}

/// One regular file with explicit checks; directory manifests and executable
/// predicates cannot widen an external grant through the verifier.
pub fn external_criterion_allowed(c: &Criterion) -> Result<(), String> {
    if c.kind != CriterionKind::Artifact || c.acceptance != Acceptance::Auto {
        return Err("EXTERNAL_CRITERION_NOT_GRANTED".into());
    }
    let spec = c.spec.as_object().ok_or("INVALID_ARTIFACT_CRITERION")?;
    if spec
        .keys()
        .any(|k| !matches!(k.as_str(), "path" | "contains" | "must_exist"))
    {
        return Err("EXTERNAL_CRITERION_NOT_GRANTED".into());
    }
    super::external_files::validate_path(c.spec["path"].as_str().ok_or("INVALID_ARTIFACT_PATH")?)
}

struct ExternalBinding {
    principal: String,
    grant: super::authority::Ceiling,
    key: String,
    fingerprint: String,
}
/// External creation uses the existing mission transaction: identity, grant
/// binding, criteria, tasks and idempotency receipt are committed together.
/// The caller authenticates the principal; a locally issued live ceiling is
/// still mandatory here, independently of any projection/schema.
pub fn create_external(
    db: &AssistDb,
    mut new: NewMission,
    principal: &str,
    grant_id: &str,
    key: &str,
) -> Result<MissionDetail, String> {
    use sha2::{Digest, Sha256};
    if key.is_empty() || key.len() > 100 {
        return Err("INVALID_IDEMPOTENCY_KEY".into());
    }
    let grant = super::authority::get(db, grant_id, principal)?;
    let bot = new.bot_id.as_deref().ok_or("EXECUTOR_REQUIRED")?;
    // A derived bot (C02) runs under the same grant and ledger as its source.
    let granted =
        grant.bots.iter().any(|b| b == bot) || super::authority::for_bot(db, &grant, bot).is_ok();
    if !granted
        || new
            .tasks
            .iter()
            .any(|t| t.bot_id.as_ref().is_some_and(|b| b != bot))
    {
        return Err("EXECUTOR_NOT_GRANTED".into());
    }
    if new.room_id.is_some()
        || new.conversation_id.is_some()
        || new.workspace.as_deref() != Some(&grant.workspace)
    {
        return Err("EXTERNAL_CONTEXT_NOT_GRANTED".into());
    }
    if new
        .budget
        .tokens
        .is_none_or(|n| n == 0 || n > grant.max_tokens)
    {
        return Err("EXTERNAL_BUDGET_EXCEEDED".into());
    }
    if let Some(cap) = grant.max_usd {
        if new
            .budget
            .usd
            .is_none_or(|n| !n.is_finite() || n < 0.0 || n > cap)
        {
            return Err("EXTERNAL_BUDGET_EXCEEDED".into());
        }
    }
    if new
        .criteria
        .iter()
        .any(|c| external_criterion_allowed(c).is_err())
    {
        return Err("EXTERNAL_CRITERION_NOT_GRANTED".into());
    }
    for criterion in &mut new.criteria {
        criterion.origin = Origin::Proposed;
    }
    new.auth_origin = "external_mcp".into();
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(grant_id, &new)).map_err(|_| "INVALID_MISSION")?)
    );
    create_inner(
        db,
        new,
        Some(ExternalBinding {
            principal: principal.into(),
            grant,
            key: key.into(),
            fingerprint,
        }),
    )
}

fn create_inner(
    db: &AssistDb,
    new: NewMission,
    external: Option<ExternalBinding>,
) -> Result<MissionDetail, String> {
    let objective = new.objective.trim();
    if objective.is_empty() {
        return Err(input_err("the objective is empty"));
    }
    let criteria = validate_criteria(&new.criteria)?;
    if !criteria.iter().any(|c| c.severity == Severity::Required) {
        return Err(format!(
            "{ERR_MISSION_CRITERIA}: a mission needs at least one required criterion; a run finishing is not an objective met"
        ));
    }
    let mut tasks_in = new.tasks.clone();
    if tasks_in.is_empty() {
        tasks_in.push(NewTask {
            key: "work".into(),
            title: clip(objective, 120),
            input: objective.to_string(),
            ..Default::default()
        });
    }
    let tasks = plan_tasks(&tasks_in)?;
    let id = new_id();
    let now = now_ms();
    let conversation = if external.is_some() {
        format!("{}{}", super::authority::PREFIX, new_id())
    } else {
        new.conversation_id
            .clone()
            .filter(|c| !c.trim().is_empty())
            .unwrap_or_else(|| format!("mission-{}", &id[..8]))
    };
    let context_kind = if new
        .workspace
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .is_empty()
    {
        "projectless"
    } else {
        "project"
    };
    let state = if new.start {
        MissionState::Queued
    } else {
        MissionState::Draft
    };
    let origin = if new.auth_origin.trim().is_empty() {
        "user_ui".to_string()
    } else {
        new.auth_origin.clone()
    };
    let committed_id=db.tx(|tx| {
        if let Some(binding)=&external {
            let previous:Option<(String,String)>=tx.query_row("SELECT fingerprint,mission FROM external_mission_intents WHERE principal=?1 AND key=?2",params![binding.principal,binding.key],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(e)?;
            if let Some((fingerprint,mission))=previous {if fingerprint!=binding.fingerprint{return Err("IDEMPOTENCY_CONFLICT".into());}return Ok(mission);}
            let live:Option<String>=tx.query_row("SELECT body FROM external_grants WHERE id=?1 AND principal=?2 AND revoked=0",params![binding.grant.id,binding.principal],|r|r.get(0)).optional().map_err(e)?;
            if live.as_deref()!=Some(serde_json::to_string(&binding.grant).map_err(e)?.as_str()){return Err("EXECUTION_NOT_GRANTED".into());}
        }
        tx.execute(
            &format!("INSERT INTO missions_missions ({MISSION_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,1,?9,'{{}}',?10,?11,NULL,0,NULL,NULL,1,0,?12,?12,NULL,NULL)"),
            params![
                id,
                external.as_ref().map(|b|b.principal.as_str()).unwrap_or(super::ctx::LOCAL_USER),
                new.bot_id,
                new.room_id,
                conversation,
                origin,
                objective,
                state.as_str(),
                serde_json::to_string(&new.budget).unwrap_or_else(|_| "{}".into()),
                new.workspace.clone().filter(|w| !w.trim().is_empty()).map(|w| {
                    // Canonical: descriptor-pinned checks refuse symlinked
                    // ancestors (`/var` → `/private/var` on macOS).
                    std::fs::canonicalize(&w).map(|p| p.to_string_lossy().to_string()).unwrap_or(w)
                }),
                context_kind,
                now
            ],
        )
        .map_err(e)?;
        insert_criteria_tx(tx, &id, 1, &criteria)?;
        for (pos, (tid, t, deps)) in tasks.iter().enumerate() {
            tx.execute(
                &format!("INSERT INTO missions_tasks ({TASK_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,'pending',NULL,NULL,0,?8,'[]',NULL,NULL,?9,?9)"),
                params![
                    tid,
                    id,
                    pos as i64,
                    if t.title.trim().is_empty() { clip(&t.input, 120) } else { t.title.trim().to_string() },
                    if t.input.trim().is_empty() { t.title.clone() } else { t.input.clone() },
                    serde_json::to_string(deps).unwrap_or_else(|_| "[]".into()),
                    t.bot_id.clone().or_else(|| new.bot_id.clone()),
                    t.max_attempts.unwrap_or(3).clamp(1, 10),
                    now
                ],
            )
            .map_err(e)?;
        }
        append_event_tx(
            tx,
            &id,
            NewEvent::new(
                "mission_created",
                json!({ "state": state.as_str(), "criteria": criteria.len(), "tasks": tasks.len(), "origin": origin }),
            ),
        )?;
        if let Some(binding)=&external {
            tx.execute("INSERT INTO external_executions(conversation,grant_id,mission,bot) VALUES(?1,?2,?3,?4)",params![conversation,binding.grant.id,id,new.bot_id]).map_err(e)?;
            tx.execute("INSERT INTO external_mission_intents(principal,key,fingerprint,mission) VALUES(?1,?2,?3,?4)",params![binding.principal,binding.key,binding.fingerprint,id]).map_err(e)?;
        }
        Ok(id.clone())
    })?;
    detail(db, &committed_id)
}

fn insert_criteria_tx(
    tx: &Transaction,
    mission: &str,
    version: i64,
    criteria: &[Criterion],
) -> Result<(), String> {
    for (pos, c) in criteria.iter().enumerate() {
        tx.execute(
            &format!("INSERT INTO missions_criteria (mission_id, position, {CRITERION_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)"),
            params![
                mission,
                pos as i64,
                c.id,
                version,
                c.kind.as_str(),
                enum_str(&c.severity),
                c.title.trim(),
                c.spec.to_string(),
                enum_str(&c.origin),
                enum_str(&c.acceptance)
            ],
        )
        .map_err(e)?;
    }
    Ok(())
}

pub fn get(db: &AssistDb, id: &str) -> Result<Mission, String> {
    db.with(|c| {
        c.query_row(
            &format!("SELECT {MISSION_COLS} FROM missions_missions WHERE id = ?1"),
            params![id],
            row_mission,
        )
        .optional()
    })?
    .ok_or_else(|| format!("{ERR_MISSION_NOT_FOUND}: no mission {id}"))
}

fn get_tx(tx: &Transaction, id: &str) -> Result<Mission, String> {
    tx.query_row(
        &format!("SELECT {MISSION_COLS} FROM missions_missions WHERE id = ?1"),
        params![id],
        row_mission,
    )
    .optional()
    .map_err(e)?
    .ok_or_else(|| format!("{ERR_MISSION_NOT_FOUND}: no mission {id}"))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ListFilter {
    pub state: Option<String>,
    pub bot_id: Option<String>,
    pub conversation_id: Option<String>,
    pub limit: Option<u32>,
}

pub fn list(db: &AssistDb, f: &ListFilter) -> Result<Vec<Mission>, String> {
    db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {MISSION_COLS} FROM missions_missions \
             WHERE (?1 IS NULL OR state = ?1) AND (?2 IS NULL OR bot_id = ?2) AND (?3 IS NULL OR conversation_id = ?3) \
             ORDER BY updated_ms DESC LIMIT ?4"
        ))?;
        let rows = st.query_map(
            params![f.state, f.bot_id, f.conversation_id, f.limit.unwrap_or(100).clamp(1, 500)],
            row_mission,
        )?;
        rows.collect()
    })
}

/// Criteria of one version (the current one when `version` is `None`).
pub fn criteria(
    db: &AssistDb,
    mission_id: &str,
    version: Option<i64>,
) -> Result<Vec<Criterion>, String> {
    let v = match version {
        Some(v) => v,
        None => get(db, mission_id)?.criteria_version,
    };
    db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {CRITERION_COLS} FROM missions_criteria WHERE mission_id = ?1 AND version = ?2 ORDER BY position"
        ))?;
        let rows = st.query_map(params![mission_id, v], row_criterion)?;
        rows.collect()
    })
}

/// Replaces the criteria with a new version. Receipts of the old version stay
/// in the history but no longer count; the mission goes back to work.
pub fn revise_criteria(
    db: &AssistDb,
    mission_id: &str,
    new: &[Criterion],
    reason: &str,
) -> Result<Mission, String> {
    let criteria = validate_criteria(new)?;
    if !criteria.iter().any(|c| c.severity == Severity::Required) {
        return Err(format!(
            "{ERR_MISSION_CRITERIA}: at least one criterion must be required"
        ));
    }
    db.tx(|tx| {
        let m = get_tx(tx, mission_id)?;
        if m.state.is_final() {
            return Err(format!("{ERR_MISSION_STATE}: mission is {}", m.state.as_str()));
        }
        let v = m.criteria_version + 1;
        insert_criteria_tx(tx, mission_id, v, &criteria)?;
        tx.execute(
            "UPDATE missions_missions SET criteria_version = ?2, revision = revision + 1, updated_ms = ?3 WHERE id = ?1",
            params![mission_id, v, now_ms()],
        )
        .map_err(e)?;
        append_event_tx(
            tx,
            mission_id,
            NewEvent::new("criteria_revised", json!({ "version": v, "reason": clip(reason, 400) })),
        )?;
        get_tx(tx, mission_id)
    })
}

/// Moves a mission to `to`. `expected_revision` guards against a stale
/// writer (the UI passes what it showed). Same state = no-op.
pub fn transition(
    db: &AssistDb,
    mission_id: &str,
    to: MissionState,
    reason: &str,
    expected_revision: Option<i64>,
) -> Result<Mission, String> {
    if to == MissionState::Succeeded {
        return Err(format!(
            "{ERR_MISSION_STATE}: `succeeded` is reached only through verification (complete)"
        ));
    }
    db.tx(|tx| transition_tx(tx, mission_id, to, reason, expected_revision, None))
}

fn transition_tx(
    tx: &Transaction,
    mission_id: &str,
    to: MissionState,
    reason: &str,
    expected_revision: Option<i64>,
    block: Option<&Value>,
) -> Result<Mission, String> {
    let m = get_tx(tx, mission_id)?;
    if let Some(rev) = expected_revision {
        if rev != m.revision {
            return Err(format!(
                "{ERR_MISSION_REVISION}: the mission changed (revision {} ≠ {rev}); reload it",
                m.revision
            ));
        }
    }
    if m.state == to {
        return Ok(m);
    }
    if !m.state.can_go(to) {
        return Err(format!(
            "{ERR_MISSION_STATE}: {} → {} is not allowed",
            m.state.as_str(),
            to.as_str()
        ));
    }
    let now = now_ms();
    let finished = matches!(
        to,
        MissionState::Succeeded
            | MissionState::Cancelled
            | MissionState::Failed
            | MissionState::Partial
    );
    let keep_block = matches!(
        to,
        MissionState::Blocked | MissionState::Partial | MissionState::Failed
    );
    let block_text: Option<String> = match block {
        Some(b) => Some(b.to_string()),
        None if keep_block => m.block.as_ref().map(|b| b.to_string()),
        None => None,
    };
    // The active clock: only running time and automated verification count
    // (F7). Entering running/verifying starts it (unless it already runs);
    // leaving them stops it. Waiting for a person inside `verifying` stops it
    // too ([`complete`] calls [`stop_clock`]), and whatever resumes the
    // mission from there (another round, a re-check) restarts it here.
    let working = |s: MissionState| matches!(s, MissionState::Running | MissionState::Verifying);
    let mut spent = m.spent.clone();
    if !working(to) {
        stop_clock(&mut spent, now);
    } else if spent.active_since_ms.is_none() {
        spent.active_since_ms = Some(now);
    }
    tx.execute(
        "UPDATE missions_missions SET state = ?2, revision = revision + 1, updated_ms = ?3, \
         started_ms = COALESCE(started_ms, CASE WHEN ?2 = 'running' THEN ?3 END), \
         finished_ms = CASE WHEN ?4 THEN ?3 ELSE NULL END, block = ?5, spent = ?6 WHERE id = ?1",
        params![
            mission_id,
            to.as_str(),
            now,
            finished,
            block_text,
            serde_json::to_string(&spent).map_err(e)?
        ],
    )
    .map_err(e)?;
    append_event_tx(
        tx,
        mission_id,
        NewEvent::new(
            "state_changed",
            json!({ "from": m.state.as_str(), "to": to.as_str(), "reason": clip(reason, 600) }),
        ),
    )?;
    get_tx(tx, mission_id)
}

/// Moves the mission to `blocked`/`partial`/`failed` with a structured
/// diagnosis the UI can show and act on.
pub fn block(
    db: &AssistDb,
    mission_id: &str,
    to: MissionState,
    diagnosis: &diag::Diagnosis,
) -> Result<Mission, String> {
    if !matches!(
        to,
        MissionState::Blocked | MissionState::Partial | MissionState::Failed
    ) {
        return Err(format!(
            "{ERR_MISSION_STATE}: block() takes blocked, partial or failed"
        ));
    }
    let value = serde_json::to_value(diagnosis).map_err(e)?;
    db.tx(|tx| {
        let m = transition_tx(tx, mission_id, to, &diagnosis.summary, None, Some(&value))?;
        append_event_tx(tx, mission_id, NewEvent::new("diagnosis", value.clone()))?;
        Ok(m)
    })
}

/// Pins the account/cwd/runtime the first time the mission runs and refuses
/// a later resume on different pins unless the user allowed a replay (A11).
pub fn check_pins(db: &AssistDb, mission_id: &str, current: &Value) -> Result<Mission, String> {
    db.tx(|tx| {
        let m = get_tx(tx, mission_id)?;
        match &m.pins {
            None => {
                tx.execute(
                    "UPDATE missions_missions SET pins = ?2, updated_ms = ?3 WHERE id = ?1",
                    params![mission_id, current.to_string(), now_ms()],
                )
                .map_err(e)?;
                get_tx(tx, mission_id)
            }
            Some(pinned) if pins_compatible(pinned, current) => Ok(m),
            Some(pinned) if m.allow_replay => {
                tx.execute(
                    "UPDATE missions_missions SET pins = ?2, allow_replay = 0, updated_ms = ?3 WHERE id = ?1",
                    params![mission_id, current.to_string(), now_ms()],
                )
                .map_err(e)?;
                append_event_tx(
                    tx,
                    mission_id,
                    NewEvent::new(
                        "resume_replay",
                        json!({ "from": pinned, "to": current, "why": "pins changed; replay authorised by the user" }),
                    ),
                )?;
                get_tx(tx, mission_id)
            }
            Some(pinned) => Err(format!(
                "{ERR_MISSION_PIN_CHANGED}: this mission ran on {} and now would run on {}; a provider session is never reused across account, folder or runtime. Allow a replay to continue with a fresh session",
                pin_label(pinned),
                pin_label(current)
            )),
        }
    })
}

fn pin_label(v: &Value) -> String {
    let get = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("-").to_string();
    format!(
        "{} / account {} / folder {}",
        get("runtime"),
        get("account"),
        get("cwd")
    )
}

/// Same runtime, account and folder (missing keys compare as "none").
pub fn pins_compatible(a: &Value, b: &Value) -> bool {
    ["runtime", "account", "cwd"]
        .iter()
        .all(|k| a.get(*k).and_then(Value::as_str) == b.get(*k).and_then(Value::as_str))
}

pub fn allow_replay(db: &AssistDb, mission_id: &str) -> Result<Mission, String> {
    db.tx(|tx| {
        tx.execute(
            "UPDATE missions_missions SET allow_replay = 1, revision = revision + 1, updated_ms = ?2 WHERE id = ?1",
            params![mission_id, now_ms()],
        )
        .map_err(e)?;
        append_event_tx(tx, mission_id, NewEvent::new("replay_allowed", json!({})))?;
        get_tx(tx, mission_id)
    })
}

/// Books what one call really used, by purpose. Unknown cost stays unknown.
pub fn book_spend(
    db: &AssistDb,
    mission_id: &str,
    purpose: &str,
    usd: Option<f64>,
    tokens: u64,
) -> Result<MissionSpent, String> {
    db.tx(|tx| {
        let m = get_tx(tx, mission_id)?;
        let mut s = m.spent.clone();
        match usd {
            Some(v) => s.usd_known += v,
            None => s.unknown_cost_calls += 1,
        }
        s.tokens += tokens;
        s.turns += 1;
        *s.by_purpose.entry(purpose.to_string()).or_default() += tokens;
        tx.execute(
            "UPDATE missions_missions SET spent = ?2, updated_ms = ?3 WHERE id = ?1",
            params![mission_id, serde_json::to_string(&s).map_err(e)?, now_ms()],
        )
        .map_err(e)?;
        Ok(s)
    })
}

/// A person changes the mission's limits (e.g. more minutes after the time
/// limit blocked it). Local decision, recorded; never lowers what was spent.
pub fn revise_budget(
    db: &AssistDb,
    mission_id: &str,
    budget: &MissionBudget,
) -> Result<Mission, String> {
    db.tx(|tx| {
        let m = get_tx(tx, mission_id)?;
        if m.state.is_final() {
            return Err(format!("{ERR_MISSION_STATE}: mission is {}", m.state.as_str()));
        }
        tx.execute(
            "UPDATE missions_missions SET budget = ?2, revision = revision + 1, updated_ms = ?3 WHERE id = ?1",
            params![mission_id, serde_json::to_string(budget).map_err(e)?, now_ms()],
        )
        .map_err(e)?;
        append_event_tx(tx, mission_id, NewEvent::new("budget_revised", json!({ "from": m.budget, "to": budget })))?;
        get_tx(tx, mission_id)
    })
}

/// After a person raised the limits: tasks that were stopped only by a
/// budget/time limit (never by an unknown effect) go back to pending with
/// fresh attempts, so resuming actually continues. Returns how many.
pub fn reopen_budget_blocked(db: &AssistDb, mission_id: &str) -> Result<usize, String> {
    db.tx(|tx| {
        let n = tx
            .execute(
                "UPDATE missions_tasks SET state = 'pending', error = NULL, attempts = 0, owner = NULL, lease_until_ms = NULL, updated_ms = ?2 \
                 WHERE mission_id = ?1 AND state IN ('blocked','failed') \
                 AND (error LIKE '%ERR_MISSION_BUDGET%' OR error LIKE '%EXTERNAL_BUDGET_EXCEEDED%' OR error LIKE '%EXTERNAL_MISSION_BUDGET_EXCEEDED%' OR error LIKE '%ERR_LLM_BUDGET%') \
                 AND NOT EXISTS (SELECT 1 FROM missions_effects x WHERE x.task_id = missions_tasks.id AND x.state IN ('pending','running','unknown'))",
                params![mission_id, now_ms()],
            )
            .map_err(e)?;
        if n > 0 {
            append_event_tx(tx, mission_id, NewEvent::new("budget_tasks_reopened", json!({ "tasks": n })))?;
        }
        Ok(n)
    })
}

/// Budget pool id of a mission in `llm::budget` (work, review, summary and
/// learning calls all reserve here).
pub fn budget_pool(mission_id: &str) -> String {
    format!("mission:{mission_id}")
}

pub fn pool_limits(b: &MissionBudget) -> crate::core::llm::budget::PoolLimits {
    crate::core::llm::budget::PoolLimits {
        usd: b.usd,
        tokens: b.tokens,
        turns: b.turns,
        strict_unknown: true,
    }
}

pub fn set_result(db: &AssistDb, mission_id: &str, result: &Value) -> Result<(), String> {
    db.with(|c| {
        c.execute(
            "UPDATE missions_missions SET result = ?2, updated_ms = ?3 WHERE id = ?1",
            params![mission_id, clip_json(result.clone()).to_string(), now_ms()],
        )
    })?;
    Ok(())
}

/// Deletes a mission and everything under it (cascade).
pub fn delete(db: &AssistDb, mission_id: &str) -> Result<(), String> {
    let m = get(db, mission_id)?;
    if m.state.is_active() {
        return Err(format!(
            "{ERR_MISSION_STATE}: cancel the mission before deleting it"
        ));
    }
    db.with(|c| {
        c.execute(
            "DELETE FROM missions_missions WHERE id = ?1",
            params![mission_id],
        )
    })?;
    Ok(())
}

pub fn detail(db: &AssistDb, id: &str) -> Result<MissionDetail, String> {
    let mission = get(db, id)?;
    let criteria = criteria(db, id, Some(mission.criteria_version))?;
    let tasks = tasks(db, id)?;
    let receipts = receipts(db, id)?;
    let effects = effects(db, id)?;
    // The artifacts as they are now: a quick view that trusts stored
    // receipts would show a stale pass (or hide a real one).
    let ws = mission.workspace.clone().map(std::path::PathBuf::from);
    let since = mission.created_ms;
    let dflt = |c: &Criterion| verify::digest_for(Some(db), ws.as_deref(), c, since);
    let verdict = verify::verdict(&criteria, &receipts, &with_revision(db, id, &dflt));
    let last_checkpoint = checkpoint::latest(db, id)?;
    let events = recent_events(db, id, 200)?;
    Ok(MissionDetail {
        mission,
        criteria,
        tasks,
        receipts,
        effects,
        verdict,
        last_checkpoint,
        events,
    })
}

// ── Tasks ─────────────────────────────────────────────────────────────────

pub fn tasks(db: &AssistDb, mission_id: &str) -> Result<Vec<MissionTask>, String> {
    db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {TASK_COLS} FROM missions_tasks WHERE mission_id = ?1 ORDER BY position"
        ))?;
        let rows = st.query_map(params![mission_id], row_task)?;
        rows.collect()
    })
}

pub fn task(db: &AssistDb, task_id: &str) -> Result<MissionTask, String> {
    db.with(|c| {
        c.query_row(
            &format!("SELECT {TASK_COLS} FROM missions_tasks WHERE id = ?1"),
            params![task_id],
            row_task,
        )
        .optional()
    })?
    .ok_or_else(|| format!("{ERR_MISSION_NOT_FOUND}: no task {task_id}"))
}

fn task_tx(tx: &Transaction, task_id: &str) -> Result<MissionTask, String> {
    tx.query_row(
        &format!("SELECT {TASK_COLS} FROM missions_tasks WHERE id = ?1"),
        params![task_id],
        row_task,
    )
    .optional()
    .map_err(e)?
    .ok_or_else(|| format!("{ERR_MISSION_NOT_FOUND}: no task {task_id}"))
}

/// Marks every pending task whose dependency failed/was cancelled/skipped as
/// `skipped`, so it is never dispatched (A07). Returns the ids changed.
pub fn skip_poisoned(db: &AssistDb, mission_id: &str) -> Result<Vec<String>, String> {
    db.tx(|tx| skip_poisoned_tx(tx, mission_id))
}

// Task/effect mutations participate in completion's optimistic revision check.
fn bump_work_revision_tx(tx: &Transaction, mission_id: &str) -> Result<(), String> {
    tx.execute(
        "UPDATE missions_missions SET revision=revision+1, updated_ms=?2 WHERE id=?1",
        params![mission_id, now_ms()],
    )
    .map_err(e)?;
    Ok(())
}

fn skip_poisoned_tx(tx: &Transaction, mission_id: &str) -> Result<Vec<String>, String> {
    let mut changed = Vec::new();
    loop {
        let all: Vec<MissionTask> = {
            let mut st = tx
                .prepare(&format!(
                    "SELECT {TASK_COLS} FROM missions_tasks WHERE mission_id = ?1"
                ))
                .map_err(e)?;
            let rows = st.query_map(params![mission_id], row_task).map_err(e)?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)?
        };
        let state_of: std::collections::HashMap<&str, TaskState> =
            all.iter().map(|t| (t.id.as_str(), t.state)).collect();
        let mut round = Vec::new();
        for t in all.iter().filter(|t| t.state == TaskState::Pending) {
            if let Some(bad) = t.deps.iter().find(|d| {
                state_of
                    .get(d.as_str())
                    .map(|s| s.poisons())
                    .unwrap_or(true)
            }) {
                round.push((t.id.clone(), bad.clone()));
            }
        }
        if round.is_empty() {
            break;
        }
        for (id, bad) in round {
            tx.execute(
                "UPDATE missions_tasks SET state = 'skipped', error = ?2, updated_ms = ?3 WHERE id = ?1",
                params![id, format!("not dispatched: dependency {bad} did not finish"), now_ms()],
            )
            .map_err(e)?;
            bump_work_revision_tx(tx, mission_id)?;
            append_event_tx(
                tx,
                mission_id,
                NewEvent::new("task_skipped", json!({ "task": id, "dependency": bad })),
            )?;
            changed.push(id);
        }
    }
    Ok(changed)
}

/// Pending tasks whose dependencies are all done (dispatch order).
pub fn ready_tasks(db: &AssistDb, mission_id: &str) -> Result<Vec<MissionTask>, String> {
    let all = tasks(db, mission_id)?;
    let done: std::collections::HashSet<&str> = all
        .iter()
        .filter(|t| t.state == TaskState::Done)
        .map(|t| t.id.as_str())
        .collect();
    Ok(all
        .iter()
        .filter(|t| {
            t.state == TaskState::Pending && t.deps.iter().all(|d| done.contains(d.as_str()))
        })
        .cloned()
        .collect())
}

/// Claims a task for `owner` with a lease. One owner at a time: the update
/// only matches a pending task whose dependencies are done, or a live task
/// whose lease ran out. A task whose previous attempt left an effect of
/// unknown outcome is not reclaimed: it blocks (A04, A06).
pub fn claim(
    db: &AssistDb,
    task_id: &str,
    owner: &str,
    lease_ms: i64,
    now: i64,
) -> Result<MissionTask, String> {
    db.tx(|tx| {
        let t = task_tx(tx, task_id)?;
        let m = get_tx(tx, &t.mission_id)?;
        if !m.state.is_active() {
            return Err(format!(
                "{ERR_MISSION_STATE}: mission is {}, nothing is dispatched",
                m.state.as_str()
            ));
        }
        let expired = t.state.is_live() && t.lease_until_ms.map(|l| l < now).unwrap_or(true);
        if !(t.state == TaskState::Pending || expired) {
            return Err(format!(
                "{ERR_MISSION_CLAIMED}: task {task_id} is {} (owner {})",
                t.state.as_str(),
                t.owner.as_deref().unwrap_or("-")
            ));
        }
        // Dependencies must be done.
        for d in &t.deps {
            let s: Option<String> = tx
                .query_row("SELECT state FROM missions_tasks WHERE id = ?1", params![d], |r| r.get(0))
                .optional()
                .map_err(e)?;
            if s.as_deref() != Some("done") {
                return Err(format!("{ERR_MISSION_STATE}: task {task_id} waits for {d}"));
            }
        }
        if expired {
            let open: Vec<Effect> = effects_of_task_tx(tx, task_id)?
                .into_iter()
                .filter(|x| matches!(x.state, EffectState::Running | EffectState::Unknown | EffectState::Pending))
                .collect();
            if !open.is_empty() {
                let keys: Vec<String> = open.iter().map(|x| x.key.clone()).collect();
                for x in &open {
                    tx.execute(
                        "UPDATE missions_effects SET state = 'unknown', updated_ms = ?2 WHERE key = ?1 AND state IN ('pending','running')",
                        params![x.key, now],
                    )
                    .map_err(e)?;
                }
                tx.execute(
                    "UPDATE missions_tasks SET state = 'blocked', owner = NULL, lease_until_ms = NULL, error = ?2, updated_ms = ?3 WHERE id = ?1",
                    params![task_id, format!("{ERR_MISSION_EFFECT_UNKNOWN}: the last attempt may have acted ({}) and its outcome is unknown", keys.join(", ")), now],
                )
                .map_err(e)?;
                append_event_tx(tx, &t.mission_id, NewEvent::new("task_blocked", json!({ "task": task_id, "effects": keys, "why": "lease expired with an effect of unknown outcome" })))?;
                return Err(format!(
                    "{ERR_MISSION_EFFECT_UNKNOWN}: task {task_id} is blocked; the previous attempt may have acted and nobody confirmed it"
                ));
            }
        }
        if t.attempts >= t.max_attempts {
            tx.execute(
                "UPDATE missions_tasks SET state = 'failed', owner = NULL, lease_until_ms = NULL, error = COALESCE(error, 'attempts exhausted'), updated_ms = ?2 WHERE id = ?1",
                params![task_id, now],
            )
            .map_err(e)?;
            skip_poisoned_tx(tx, &t.mission_id)?;
            return Err(format!("{ERR_MISSION_STATE}: task {task_id} used its {} attempts", t.max_attempts));
        }
        let n = tx
            .execute(
                "UPDATE missions_tasks SET state = 'claimed', owner = ?2, lease_until_ms = ?3, attempts = attempts + 1, updated_ms = ?4 \
                 WHERE id = ?1 AND (state = 'pending' OR (state IN ('claimed','running') AND (lease_until_ms IS NULL OR lease_until_ms < ?4)))",
                params![task_id, owner, now + lease_ms.max(1_000), now],
            )
            .map_err(e)?;
        if n != 1 {
            return Err(format!("{ERR_MISSION_CLAIMED}: task {task_id} was claimed by someone else"));
        }
        bump_work_revision_tx(tx, &t.mission_id)?;
        append_event_tx(
            tx,
            &t.mission_id,
            NewEvent::new("task_claimed", json!({ "task": task_id, "owner": owner, "attempt": t.attempts + 1, "reclaimed": expired })),
        )?;
        task_tx(tx, task_id)
    })
}

/// Extends the lease of a task still owned by `owner`.
pub fn heartbeat(
    db: &AssistDb,
    task_id: &str,
    owner: &str,
    lease_ms: i64,
    now: i64,
) -> Result<bool, String> {
    let n = db.with(|c| {
        c.execute(
            "UPDATE missions_tasks SET lease_until_ms = ?3, updated_ms = ?4 WHERE id = ?1 AND owner = ?2 AND state IN ('claimed','running')",
            params![task_id, owner, now + lease_ms, now],
        )
    })?;
    Ok(n == 1)
}

/// The owner reports that the attempt is running (with the job it started).
pub fn task_started(
    db: &AssistDb,
    task_id: &str,
    owner: &str,
    job_id: &str,
) -> Result<MissionTask, String> {
    db.tx(|tx| {
        let t = task_tx(tx, task_id)?;
        if t.owner.as_deref() != Some(owner) || !t.state.is_live() {
            return Err(format!("{ERR_MISSION_CLAIMED}: task {task_id} is not owned by {owner}"));
        }
        // Checked in the same transaction that records the job: a pause or
        // cancel either lands before (this refuses, nothing is dispatched)
        // or after (it sees the job in `live_jobs` and stops it). F2.
        let m = get_tx(tx, &t.mission_id)?;
        if !matches!(m.state, MissionState::Running | MissionState::Verifying) {
            return Err(format!(
                "{ERR_MISSION_STATE}: mission is {}, the prepared job is not dispatched",
                m.state.as_str()
            ));
        }
        let mut jobs = t.job_ids.clone();
        if !jobs.iter().any(|j| j == job_id) {
            jobs.push(job_id.to_string());
        }
        tx.execute(
            "UPDATE missions_tasks SET state = 'running', job_ids = ?2, updated_ms = ?3 WHERE id = ?1",
            params![task_id, serde_json::to_string(&jobs).map_err(e)?, now_ms()],
        )
        .map_err(e)?;
        bump_work_revision_tx(tx, &t.mission_id)?;
        append_event_tx(tx, &t.mission_id, NewEvent::new("task_started", json!({ "task": task_id, "job": job_id })))?;
        task_tx(tx, task_id)
    })
}

/// How one attempt ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TaskEnd {
    /// The run finished. This says nothing about the criteria.
    Done {
        result: String,
    },
    /// Failed; `retryable` puts it back to pending while attempts remain.
    Failed {
        error: String,
        retryable: bool,
    },
    Cancelled,
    Blocked {
        reason: String,
    },
    /// Stopped by a pause (or before it was dispatched): back to pending
    /// without spending the attempt, dependents untouched. F1.
    Interrupted {
        reason: String,
    },
}

/// Gives a claimed task back without spending its attempt (it never ran:
/// the mission was paused/cancelled before dispatch, or the budget asked it
/// to wait for a reservation in flight). A late call from a non-owner is a
/// no-op error.
pub fn release_claim(
    db: &AssistDb,
    task_id: &str,
    owner: &str,
    reason: &str,
) -> Result<MissionTask, String> {
    finish_task(
        db,
        task_id,
        owner,
        TaskEnd::Interrupted {
            reason: reason.to_string(),
        },
    )
}

pub fn finish_task(
    db: &AssistDb,
    task_id: &str,
    owner: &str,
    end: TaskEnd,
) -> Result<MissionTask, String> {
    db.tx(|tx| {
        let t = task_tx(tx, task_id)?;
        if t.owner.as_deref() != Some(owner) {
            // A late report from an owner who lost the lease changes nothing.
            return Err(format!("{ERR_MISSION_CLAIMED}: task {task_id} is not owned by {owner}"));
        }
        if !t.state.is_live() {
            return Ok(t);
        }
        let now = now_ms();
        // A job stopped because its mission was paused is an interruption,
        // not a cancellation: pausing is not failing (F1).
        let paused = get_tx(tx, &t.mission_id)?.state == MissionState::Paused;
        let end = match end {
            TaskEnd::Cancelled if paused => TaskEnd::Interrupted { reason: "paused".into() },
            other => other,
        };
        let (state, result, error) = match &end {
            TaskEnd::Interrupted { reason } => ("pending", None, Some(clip(&diag::redact(reason), 4000))),
            TaskEnd::Done { result } => ("done", Some(clip(result, 32 * 1024)), None),
            TaskEnd::Failed { error, retryable } => {
                if *retryable && t.attempts < t.max_attempts {
                    ("pending", None, Some(clip(&diag::redact(error), 4000)))
                } else {
                    ("failed", None, Some(clip(&diag::redact(error), 4000)))
                }
            }
            TaskEnd::Cancelled => ("cancelled", None, Some("cancelled".to_string())),
            TaskEnd::Blocked { reason } => ("blocked", None, Some(clip(&diag::redact(reason), 4000))),
        };
        let refund = matches!(end, TaskEnd::Interrupted { .. });
        tx.execute(
            "UPDATE missions_tasks SET state = ?2, result = COALESCE(?3, result), error = ?4, owner = NULL, lease_until_ms = NULL, updated_ms = ?5, \
             attempts = CASE WHEN ?6 AND attempts > 0 THEN attempts - 1 ELSE attempts END WHERE id = ?1",
            params![task_id, state, result, error, now, refund],
        )
        .map_err(e)?;
        bump_work_revision_tx(tx, &t.mission_id)?;
        append_event_tx(tx, &t.mission_id, NewEvent::new("task_finished", json!({ "task": task_id, "state": state, "error": error })))?;
        if matches!(state, "failed" | "cancelled") {
            skip_poisoned_tx(tx, &t.mission_id)?;
        }
        task_tx(tx, task_id)
    })
}

/// A person puts a blocked task back in the queue (after reviewing what the
/// unknown effect did). `confirmed` settles the open effects: `Some(true)`
/// = it happened, `Some(false)` = it did not, `None` = leave them unknown
/// and continue anyway (explicit choice, recorded).
pub fn unblock_task(
    db: &AssistDb,
    task_id: &str,
    confirmed: Option<bool>,
    note: &str,
) -> Result<MissionTask, String> {
    db.tx(|tx| {
        let t = task_tx(tx, task_id)?;
        if t.state != TaskState::Blocked {
            return Err(format!("{ERR_MISSION_STATE}: task {task_id} is {}", t.state.as_str()));
        }
        let now = now_ms();
        // `None` ("continue anyway") is a decision too: the open effects are
        // settled as `unknown_accepted`, recorded, so completion and resume
        // can go on (F4). The outcome stays unknown in the journal.
        let settle_as = match confirmed {
            Some(true) => "confirmed",
            Some(false) => "failed",
            None => "unknown_accepted",
        };
        let keys: Vec<String> = effects_of_task_tx(tx, task_id)?
            .into_iter()
            .filter(|x| !x.state.is_settled())
            .map(|x| x.key)
            .collect();
        tx.execute(
            "UPDATE missions_effects SET state = ?2, detail = ?3, updated_ms = ?4 WHERE task_id = ?1 AND state IN ('unknown','running','pending')",
            params![task_id, settle_as, format!("settled by the user: {}", clip(note, 300)), now],
        )
        .map_err(e)?;
        if confirmed.is_none() && !keys.is_empty() {
            append_event_tx(tx, &t.mission_id, NewEvent::new("effect_accepted_unknown", json!({ "task": task_id, "effects": keys, "note": clip(note, 300) })))?;
        }
        let next = if confirmed == Some(true) { "done" } else { "pending" };
        tx.execute(
            "UPDATE missions_tasks SET state = ?2, error = NULL, attempts = CASE WHEN ?2 = 'pending' AND attempts >= max_attempts THEN max_attempts - 1 ELSE attempts END, updated_ms = ?3 WHERE id = ?1",
            params![task_id, next, now],
        )
        .map_err(e)?;
        bump_work_revision_tx(tx, &t.mission_id)?;
        append_event_tx(
            tx,
            &t.mission_id,
            NewEvent::new("task_unblocked", json!({ "task": task_id, "confirmed": confirmed, "note": clip(note, 300), "next": next })),
        )?;
        task_tx(tx, task_id)
    })
}

/// A person settles one effect by key (for an effect left open by a task
/// that is no longer blocked). `decision` is `confirmed`, `failed` or
/// `unknown_accepted`; recorded as an event.
pub fn settle_effect_by_person(
    db: &AssistDb,
    key: &str,
    decision: &str,
    note: &str,
) -> Result<Effect, String> {
    if !matches!(decision, "confirmed" | "failed" | "unknown_accepted") {
        return Err(input_err(
            "decision is confirmed, failed or unknown_accepted",
        ));
    }
    db.tx(|tx| {
        let x = tx
            .query_row(
                &format!("SELECT {EFFECT_COLS} FROM missions_effects WHERE key = ?1"),
                params![key],
                row_effect,
            )
            .optional()
            .map_err(e)?
            .ok_or_else(|| format!("{ERR_MISSION_NOT_FOUND}: no effect {key}"))?;
        if x.state.is_settled() {
            return Ok(x);
        }
        if let Some(tid) = &x.task_id {
            if task_tx(tx, tid)?.state.is_live() {
                return Err(format!(
                    "{ERR_MISSION_STATE}: the task that owns {key} is still running"
                ));
            }
        }
        tx.execute(
            "UPDATE missions_effects SET state = ?2, detail = ?3, updated_ms = ?4 WHERE key = ?1",
            params![
                key,
                decision,
                format!("settled by the user: {}", clip(note, 300)),
                now_ms()
            ],
        )
        .map_err(e)?;
        bump_work_revision_tx(tx, &x.mission_id)?;
        append_event_tx(
            tx,
            &x.mission_id,
            NewEvent::new(
                if decision == "unknown_accepted" {
                    "effect_accepted_unknown"
                } else {
                    "effect_settled"
                },
                json!({ "key": key, "decision": decision, "by": "user", "note": clip(note, 300) }),
            ),
        )?;
        tx.query_row(
            &format!("SELECT {EFFECT_COLS} FROM missions_effects WHERE key = ?1"),
            params![key],
            row_effect,
        )
        .map_err(e)
    })
}

/// Result of [`cancel`]: the jobs still running that the caller must stop.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CancelOutcome {
    pub mission: Option<Mission>,
    pub cancelled_tasks: Vec<String>,
    /// Jobs of tasks that were live: stop them (the driver does).
    pub live_jobs: Vec<String>,
}

/// Cancels the mission: no new dispatch, pending and live tasks cancelled,
/// finished tasks kept as they are (A07).
pub fn cancel(db: &AssistDb, mission_id: &str, reason: &str) -> Result<CancelOutcome, String> {
    db.tx(|tx| {
        let m = get_tx(tx, mission_id)?;
        if m.state == MissionState::Cancelled {
            return Ok(CancelOutcome { mission: Some(m), ..Default::default() });
        }
        if m.state.is_final() {
            return Err(format!("{ERR_MISSION_STATE}: mission is {}", m.state.as_str()));
        }
        let all: Vec<MissionTask> = {
            let mut st = tx
                .prepare(&format!("SELECT {TASK_COLS} FROM missions_tasks WHERE mission_id = ?1"))
                .map_err(e)?;
            let rows = st.query_map(params![mission_id], row_task).map_err(e)?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)?
        };
        let mut out = CancelOutcome::default();
        let now = now_ms();
        for t in all.iter().filter(|t| !t.state.is_terminal()) {
            if t.state == TaskState::Running {
                if let Some(j) = t.job_ids.last() {
                    out.live_jobs.push(j.clone());
                }
            }
            if t.state == TaskState::Claimed {
                // Claimed, never started: its job was prepared at most, and
                // `task_started` now refuses it. Its effect did not happen.
                settle_undispatched_tx(tx, &t.id, "not dispatched: mission cancelled", now)?;
            }
            tx.execute(
                "UPDATE missions_tasks SET state = 'cancelled', owner = NULL, lease_until_ms = NULL, error = 'mission cancelled', updated_ms = ?2 WHERE id = ?1",
                params![t.id, now],
            )
            .map_err(e)?;
            out.cancelled_tasks.push(t.id.clone());
        }
        let m = transition_tx(tx, mission_id, MissionState::Cancelled, reason, None, None)?;
        append_event_tx(
            tx,
            mission_id,
            NewEvent::new("mission_finished", json!({ "state": "cancelled", "tasks": out.cancelled_tasks, "live_jobs": out.live_jobs })),
        )?;
        out.mission = Some(m);
        Ok(out)
    })
}

/// Settles the open effects of a task that was claimed but never started:
/// `task_started` is the only way to dispatch, so they did not happen.
fn settle_undispatched_tx(
    tx: &Transaction,
    task_id: &str,
    why: &str,
    now: i64,
) -> Result<(), String> {
    let open: Vec<Effect> = effects_of_task_tx(tx, task_id)?
        .into_iter()
        .filter(|x| matches!(x.state, EffectState::Pending | EffectState::Running))
        .collect();
    for x in open {
        tx.execute(
            "UPDATE missions_effects SET state = 'failed', detail = ?2, updated_ms = ?3 WHERE key = ?1",
            params![x.key, why, now],
        )
        .map_err(e)?;
        append_event_tx(
            tx,
            &x.mission_id,
            NewEvent::new(
                "effect_settled",
                json!({ "key": x.key, "ok": false, "why": why }),
            ),
        )?;
    }
    Ok(())
}

/// Pauses: live attempts are reported to stop, pending tasks wait. Resuming
/// puts the mission back in the queue.
pub fn pause(db: &AssistDb, mission_id: &str) -> Result<CancelOutcome, String> {
    db.tx(|tx| {
        let m = get_tx(tx, mission_id)?;
        let live: Vec<MissionTask> = {
            let mut st = tx
                .prepare(&format!("SELECT {TASK_COLS} FROM missions_tasks WHERE mission_id = ?1 AND state IN ('claimed','running')"))
                .map_err(e)?;
            let rows = st.query_map(params![mission_id], row_task).map_err(e)?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)?
        };
        let mut out = CancelOutcome::default();
        if !m.state.can_go(MissionState::Paused) {
            return Err(format!("{ERR_MISSION_STATE}: cannot pause a {} mission", m.state.as_str()));
        }
        let now = now_ms();
        for t in &live {
            if t.state == TaskState::Running {
                // Running: the driver stops the job; the task comes back as
                // pending when it reports (finish_task, F1).
                if let Some(j) = t.job_ids.last() {
                    out.live_jobs.push(j.clone());
                }
            } else {
                // Claimed, not started: nothing was dispatched. Back to
                // pending now, attempt refunded, effect settled (F1/F2).
                settle_undispatched_tx(tx, &t.id, "not dispatched: mission paused", now)?;
                tx.execute(
                    "UPDATE missions_tasks SET state = 'pending', owner = NULL, lease_until_ms = NULL, error = 'paused before dispatch', \
                     attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END, updated_ms = ?2 WHERE id = ?1",
                    params![t.id, now],
                )
                .map_err(e)?;
                bump_work_revision_tx(tx, mission_id)?;
                append_event_tx(tx, mission_id, NewEvent::new("task_interrupted", json!({ "task": t.id, "why": "paused before dispatch" })))?;
            }
        }
        out.mission = Some(transition_tx(tx, mission_id, MissionState::Paused, "paused by the user", None, None)?);
        Ok(out)
    })
}

/// Blocked/paused/partial/failed → queued. Failed tasks with attempts left
/// go back to pending; skipped dependents are reconsidered.
pub fn resume(db: &AssistDb, mission_id: &str, note: &str) -> Result<Mission, String> {
    db.tx(|tx| {
        let m = get_tx(tx, mission_id)?;
        if m.state.is_active() {
            return Ok(m);
        }
        if !m.state.can_go(MissionState::Queued) {
            return Err(format!("{ERR_MISSION_STATE}: cannot resume a {} mission", m.state.as_str()));
        }
        let now = now_ms();
        // A person resuming is a decision to try again: the newest failed
        // task gets a fresh set of attempts (older failed rounds stay as
        // history).
        tx.execute(
            "UPDATE missions_tasks SET state = 'pending', error = NULL, attempts = 0, updated_ms = ?2 \
             WHERE id = (SELECT id FROM missions_tasks WHERE mission_id = ?1 ORDER BY position DESC LIMIT 1) AND state = 'failed'",
            params![mission_id, now],
        )
        .map_err(e)?;
        if m.state == MissionState::Failed || m.state == MissionState::Partial {
            tx.execute(
                "UPDATE missions_tasks SET state = 'pending', error = NULL, updated_ms = ?2 \
                 WHERE mission_id = ?1 AND state IN ('failed','skipped') AND attempts < max_attempts",
                params![mission_id, now],
            )
            .map_err(e)?;
            tx.execute(
                "UPDATE missions_tasks SET state = 'pending', updated_ms = ?2 WHERE mission_id = ?1 AND state = 'skipped'",
                params![mission_id, now],
            )
            .map_err(e)?;
        }
        // A task left live by a paused driver starts over only if nothing unknown.
        tx.execute(
            "UPDATE missions_tasks SET state = 'pending', owner = NULL, lease_until_ms = NULL, updated_ms = ?2 \
             WHERE mission_id = ?1 AND state IN ('claimed','running') \
             AND NOT EXISTS (SELECT 1 FROM missions_effects x WHERE x.task_id = missions_tasks.id AND x.state IN ('pending','running','unknown'))",
            params![mission_id, now],
        )
        .map_err(e)?;
        skip_poisoned_tx(tx, mission_id)?;
        transition_tx(tx, mission_id, MissionState::Queued, if note.is_empty() { "resumed" } else { note }, None, None)
    })
}

// ── Effects journal ───────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum EffectAdmission {
    /// First time: go ahead, then settle it.
    New(Effect),
    /// Already confirmed: do not act again; here is what happened.
    Done(Effect),
    /// A previous attempt may have acted; nobody knows. Do not act.
    Unknown(Effect),
    /// It failed before: acting again is a new attempt (allowed).
    Retry(Effect),
}

/// Records the intent to cause an external effect under a stable key
/// (idempotency key). Same key with a different fingerprint is refused.
pub fn effect_begin(
    db: &AssistDb,
    mission_id: &str,
    task_id: Option<&str>,
    key: &str,
    kind: &str,
    fingerprint: &str,
    instance: &str,
) -> Result<EffectAdmission, String> {
    db.tx(|tx| {
        let mission = get_tx(tx, mission_id)?;
        if !matches!(mission.state, MissionState::Running | MissionState::Verifying) {
            return Err(format!("{ERR_MISSION_STATE}: effects require a running or verifying mission"));
        }
        let now = now_ms();
        let existing = tx
            .query_row(
                &format!("SELECT {EFFECT_COLS} FROM missions_effects WHERE key = ?1"),
                params![key],
                row_effect,
            )
            .optional()
            .map_err(e)?;
        if let Some(x) = existing {
            if x.mission_id != mission_id || x.task_id.as_deref() != task_id {
                return Err(format!("{ERR_MISSION}: effect key conflicts with another owner"));
            }
            if x.fingerprint != fingerprint {
                return Err(format!(
                    "{ERR_MISSION}: effect key {key} was used for different content; refused, never merged"
                ));
            }
            return Ok(match x.state {
                EffectState::Confirmed => EffectAdmission::Done(x),
                EffectState::Failed | EffectState::UnknownAccepted => {
                    tx.execute(
                        "UPDATE missions_effects SET state = 'running', instance_id = ?2, updated_ms = ?3 WHERE key = ?1",
                        params![key, instance, now],
                    )
                    .map_err(e)?;
                    bump_work_revision_tx(tx, &x.mission_id)?;
                    EffectAdmission::Retry(x)
                }
                // Running under this very process: still ours, same answer.
                EffectState::Running | EffectState::Pending if x.instance_id == instance => {
                    EffectAdmission::Unknown(x)
                }
                _ => {
                    tx.execute(
                        "UPDATE missions_effects SET state = 'unknown', updated_ms = ?2 WHERE key = ?1",
                        params![key, now],
                    )
                    .map_err(e)?;
                    bump_work_revision_tx(tx, &x.mission_id)?;
                    EffectAdmission::Unknown(Effect { state: EffectState::Unknown, ..x })
                }
            });
        }
        if let Some(task_id) = task_id {
            if task_tx(tx, task_id)?.mission_id != mission_id {
                return Err(format!("{ERR_MISSION}: effect task does not belong to this mission"));
            }
        }
        let x = Effect {
            key: key.to_string(),
            mission_id: mission_id.to_string(),
            task_id: task_id.map(str::to_string),
            kind: kind.to_string(),
            fingerprint: fingerprint.to_string(),
            state: EffectState::Running,
            instance_id: instance.to_string(),
            detail: None,
            created_ms: now,
            updated_ms: now,
        };
        tx.execute(
            &format!("INSERT INTO missions_effects ({EFFECT_COLS}) VALUES (?1,?2,?3,?4,?5,'running',?6,NULL,?7,?7)"),
            params![x.key, x.mission_id, x.task_id, x.kind, x.fingerprint, x.instance_id, now],
        )
        .map_err(e)?;
        bump_work_revision_tx(tx, mission_id)?;
        append_event_tx(tx, mission_id, NewEvent::new("effect_started", json!({ "key": key, "kind": kind, "task": task_id })))?;
        Ok(EffectAdmission::New(x))
    })
}

pub fn effect_settle(db: &AssistDb, key: &str, ok: bool, detail: &str) -> Result<Effect, String> {
    db.tx(|tx| {
        let now = now_ms();
        tx.execute(
            "UPDATE missions_effects SET state = ?2, detail = ?3, updated_ms = ?4 WHERE key = ?1",
            params![
                key,
                if ok { "confirmed" } else { "failed" },
                clip(detail, 2000),
                now
            ],
        )
        .map_err(e)?;
        let x = tx
            .query_row(
                &format!("SELECT {EFFECT_COLS} FROM missions_effects WHERE key = ?1"),
                params![key],
                row_effect,
            )
            .map_err(e)?;
        bump_work_revision_tx(tx, &x.mission_id)?;
        append_event_tx(
            tx,
            &x.mission_id,
            NewEvent::new("effect_settled", json!({ "key": key, "ok": ok })),
        )?;
        Ok(x)
    })
}

fn effects_of_task_tx(tx: &Transaction, task_id: &str) -> Result<Vec<Effect>, String> {
    let mut st = tx
        .prepare(&format!(
            "SELECT {EFFECT_COLS} FROM missions_effects WHERE task_id = ?1"
        ))
        .map_err(e)?;
    let rows = st.query_map(params![task_id], row_effect).map_err(e)?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(e)
}

pub fn effects(db: &AssistDb, mission_id: &str) -> Result<Vec<Effect>, String> {
    db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {EFFECT_COLS} FROM missions_effects WHERE mission_id = ?1 ORDER BY created_ms"
        ))?;
        let rows = st.query_map(params![mission_id], row_effect)?;
        rows.collect()
    })
}

/// What [`reconcile`] did after a restart.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Reconciled {
    /// Effects the probe settled (confirmed or failed).
    pub settled: Vec<String>,
    /// Effects left unknown; their tasks are blocked.
    pub unknown: Vec<String>,
    pub blocked_tasks: Vec<String>,
    /// Missions that were active and are now blocked or re-queued.
    pub missions: Vec<String>,
}

/// After a restart: every effect another process left running is probed
/// (`probe` asks the outside world; `None` = cannot tell). Confirmed/failed
/// settle; the rest become unknown and their tasks block, with the reason.
/// Active missions whose tasks were live go back to the queue (their
/// unaffected tasks restart) or block when something is unknown.
pub fn reconcile(
    db: &AssistDb,
    instance: &str,
    probe: &dyn Fn(&Effect) -> Option<bool>,
) -> Result<Reconciled, String> {
    let open: Vec<Effect> = db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {EFFECT_COLS} FROM missions_effects WHERE state IN ('pending','running') AND instance_id <> ?1"
        ))?;
        let rows = st.query_map(params![instance], row_effect)?;
        rows.collect()
    })?;
    let mut out = Reconciled::default();
    for x in open {
        match probe(&x) {
            Some(ok) => {
                effect_settle(db, &x.key, ok, "reconciled after restart")?;
                out.settled.push(x.key.clone());
            }
            None => {
                db.tx(|tx| {
                    tx.execute(
                        "UPDATE missions_effects SET state = 'unknown', updated_ms = ?2 WHERE key = ?1",
                        params![x.key, now_ms()],
                    ).map_err(e)?;
                    bump_work_revision_tx(tx, &x.mission_id)
                })?;
                out.unknown.push(x.key.clone());
            }
        }
    }
    // Live tasks of a dead process.
    let live: Vec<MissionTask> = db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {TASK_COLS} FROM missions_tasks WHERE state IN ('claimed','running')"
        ))?;
        let rows = st.query_map([], row_task)?;
        rows.collect()
    })?;
    for t in live {
        if t.owner
            .as_deref()
            .map(|o| o.starts_with(instance))
            .unwrap_or(false)
        {
            continue;
        }
        let mine: Vec<Effect> = effects(db, &t.mission_id)?
            .into_iter()
            .filter(|x| x.task_id.as_deref() == Some(t.id.as_str()))
            .collect();
        let unknown: Vec<String> = mine
            .iter()
            .filter(|x| x.state == EffectState::Unknown)
            .map(|x| x.key.clone())
            .collect();
        // The newest attempt finished and the probe confirmed it: the task
        // is done (its result is read back from the job), not re-run.
        let confirmed = mine
            .iter()
            .max_by_key(|x| x.created_ms)
            .filter(|x| x.state == EffectState::Confirmed)
            .map(|x| x.key.clone());
        db.tx(|tx| {
            if let (true, Some(key)) = (unknown.is_empty(), &confirmed) {
                tx.execute(
                    "UPDATE missions_tasks SET state = 'done', owner = NULL, lease_until_ms = NULL, result = COALESCE(result, ?2), updated_ms = ?3 WHERE id = ?1",
                    params![t.id, format!("(finished while the app was closing; recovered from {key})"), now_ms()],
                )
                .map_err(e)?;
            } else if unknown.is_empty() {
                tx.execute(
                    "UPDATE missions_tasks SET state = 'pending', owner = NULL, lease_until_ms = NULL, updated_ms = ?2 WHERE id = ?1",
                    params![t.id, now_ms()],
                )
                .map_err(e)?;
            } else {
                tx.execute(
                    "UPDATE missions_tasks SET state = 'blocked', owner = NULL, lease_until_ms = NULL, error = ?2, updated_ms = ?3 WHERE id = ?1",
                    params![t.id, format!("{ERR_MISSION_EFFECT_UNKNOWN}: the app closed while this task was acting ({}); its outcome is unknown and it is not repeated by itself", unknown.join(", ")), now_ms()],
                )
                .map_err(e)?;
            }
            bump_work_revision_tx(tx, &t.mission_id)?;
            append_event_tx(
                tx,
                &t.mission_id,
                NewEvent::new("run_interrupted", json!({ "task": t.id, "unknown_effects": unknown })),
            )?;
            Ok(())
        })?;
        if !unknown.is_empty() {
            out.blocked_tasks.push(t.id.clone());
        }
        if !out.missions.contains(&t.mission_id) {
            out.missions.push(t.mission_id.clone());
        }
    }
    // Missions that were running: blocked if a task is blocked, else queued.
    let active: Vec<Mission> = db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {MISSION_COLS} FROM missions_missions WHERE state IN ('running','verifying')"
        ))?;
        let rows = st.query_map([], row_mission)?;
        rows.collect()
    })?;
    for m in active {
        let ts = tasks(db, &m.id)?;
        if let Some(b) = ts.iter().find(|t| t.state == TaskState::Blocked) {
            let d = diag::Diagnosis::effect_unknown(&m.id, &b.id, b.error.as_deref().unwrap_or(""));
            block(db, &m.id, MissionState::Blocked, &d)?;
        } else {
            db.tx(|tx| {
                // running → blocked → queued keeps the state machine honest.
                transition_tx(
                    tx,
                    &m.id,
                    MissionState::Blocked,
                    "app restarted",
                    None,
                    None,
                )?;
                transition_tx(
                    tx,
                    &m.id,
                    MissionState::Queued,
                    "resumed after restart (nothing unknown)",
                    None,
                    None,
                )
            })?;
        }
        if !out.missions.contains(&m.id) {
            out.missions.push(m.id.clone());
        }
    }
    Ok(out)
}

// ── Receipts ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct NewReceipt {
    pub mission_id: String,
    pub criterion_id: String,
    pub criterion_version: i64,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub artifact_ref: String,
    pub artifact_digest: String,
    pub verifier: String,
    pub verifier_version: String,
    pub status: String,
    pub exit_code: Option<i64>,
    pub confidence: Option<String>,
    pub evidence: String,
}

/// Stores one verification receipt. Evidence is redacted and clipped here,
/// whoever produced it.
pub fn record_receipt(db: &AssistDb, r: NewReceipt) -> Result<MissionReceipt, String> {
    if !matches!(r.status.as_str(), "pass" | "fail" | "unknown" | "partial") {
        return Err(input_err(
            "receipt status is pass, fail, unknown or partial",
        ));
    }
    let id = new_id();
    let now = now_ms();
    let evidence = clip(&diag::redact(&r.evidence), 8 * 1024);
    // A judgement with no declared artifact (a person accepting, a rubric
    // review) is bound to the revision of what the mission delivered when it
    // was recorded, never to the constant "none": the next round makes it
    // stale (F3).
    let mut r = r;
    if let Some(c) = criteria(db, &r.mission_id, Some(r.criterion_version))?
        .iter()
        .find(|c| c.id == r.criterion_id)
    {
        if judged_without_artifact(c) {
            let m = get(db, &r.mission_id)?;
            let ws = m.workspace.clone().map(std::path::PathBuf::from);
            let since = m.created_ms;
            let dflt = |c: &Criterion| verify::digest_for(Some(db), ws.as_deref(), c, since);
            r.artifact_digest = revision_digest(db, &r.mission_id, &dflt)
                .unwrap_or_else(|| "rev:unavailable".into());
        }
    }
    db.tx(|tx| {
        tx.execute(
            &format!("INSERT INTO missions_receipts ({RECEIPT_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,NULL,NULL)"),
            params![
                id,
                r.mission_id,
                r.criterion_id,
                r.criterion_version,
                r.task_id,
                r.run_id,
                r.artifact_ref,
                r.artifact_digest,
                r.verifier,
                r.verifier_version,
                r.status,
                r.exit_code,
                r.confidence,
                evidence,
                now
            ],
        )
        .map_err(e)?;
        append_event_tx(
            tx,
            &r.mission_id,
            NewEvent::new(
                "verification_finished",
                json!({ "criterion": r.criterion_id, "version": r.criterion_version, "status": r.status, "verifier": r.verifier, "digest": r.artifact_digest }),
            ),
        )?;
        tx.execute("UPDATE missions_missions SET revision=revision+1 WHERE id=?1",params![r.mission_id]).map_err(e)?;
        tx.query_row(
            &format!("SELECT {RECEIPT_COLS} FROM missions_receipts WHERE id = ?1"),
            params![id],
            row_receipt,
        )
        .map_err(e)
    })
}

pub fn receipts(db: &AssistDb, mission_id: &str) -> Result<Vec<MissionReceipt>, String> {
    db.with(|c| {
        let mut st = c.prepare(&format!(
            "SELECT {RECEIPT_COLS} FROM missions_receipts WHERE mission_id = ?1 ORDER BY created_ms DESC LIMIT 500"
        ))?;
        let rows = st.query_map(params![mission_id], row_receipt)?;
        rows.collect()
    })
}

/// Invalidates every live receipt whose artifact changed since it was
/// recorded (`current` gives the digest now; `None` = cannot tell, which
/// also invalidates: an unverifiable artifact is not a pass). Returns the
/// receipts invalidated (A03).
pub fn invalidate_stale(
    db: &AssistDb,
    mission_id: &str,
    current: &dyn Fn(&Criterion) -> Option<String>,
) -> Result<Vec<String>, String> {
    let wrapped = with_revision(db, mission_id, current);
    let current: &dyn Fn(&Criterion) -> Option<String> = &wrapped;
    let m = get(db, mission_id)?;
    let crit = criteria(db, mission_id, Some(m.criteria_version))?;
    let live: Vec<MissionReceipt> = receipts(db, mission_id)?
        .into_iter()
        .filter(|r| r.invalidated_ms.is_none())
        .collect();
    let mut out = Vec::new();
    for r in live {
        let reason = match crit.iter().find(|c| c.id == r.criterion_id) {
            None => Some("criterion no longer exists".to_string()),
            Some(_) if r.criterion_version != m.criteria_version => Some(format!(
                "criteria changed to version {}",
                m.criteria_version
            )),
            Some(c) => match current(c) {
                Some(d) if d == r.artifact_digest => None,
                Some(d) => Some(format!(
                    "artifact changed ({} → {})",
                    short(&r.artifact_digest),
                    short(&d)
                )),
                None => Some("artifact can no longer be read".to_string()),
            },
        };
        if let Some(reason) = reason {
            db.tx(|tx| {
                tx.execute(
                    "UPDATE missions_receipts SET invalidated_ms = ?2, invalidated_reason = ?3 WHERE id = ?1",
                    params![r.id, now_ms(), reason],
                )
                .map_err(e)?;
                append_event_tx(tx, mission_id, NewEvent::new("receipt_invalidated", json!({ "receipt": r.id, "criterion": r.criterion_id, "reason": reason })))?;
                tx.execute("UPDATE missions_missions SET revision=revision+1 WHERE id=?1",params![mission_id]).map_err(e)?;
                Ok(())
            })?;
            out.push(r.id);
        }
    }
    Ok(out)
}

fn short(d: &str) -> &str {
    &d[..d.len().min(10)]
}

/// A criterion decided by judgement (a person, a rubric review) that names
/// no artifact: what it judged is "what the mission delivered".
pub fn judged_without_artifact(c: &Criterion) -> bool {
    matches!(c.kind, CriterionKind::Human | CriterionKind::Rubric) && c.artifact_paths().is_empty()
}

/// Revision of what the mission delivered: the digests of every other
/// criterion's artifact now, plus each finished task (its attempts, jobs and
/// result). Another round, a changed artifact or a recovered result gives a
/// different revision. `None` when an artifact cannot be read (never a
/// match).
pub fn revision_digest(
    db: &AssistDb,
    mission_id: &str,
    current: &dyn Fn(&Criterion) -> Option<String>,
) -> Option<String> {
    use sha2::{Digest, Sha256};
    let m = get(db, mission_id).ok()?;
    let crit = criteria(db, mission_id, Some(m.criteria_version)).ok()?;
    let mut parts: Vec<(String, String)> = Vec::new();
    for c in crit.iter().filter(|c| !judged_without_artifact(c)) {
        parts.push((format!("criterion:{}:{}", c.id, c.version), current(c)?));
    }
    for t in tasks(db, mission_id)
        .ok()?
        .iter()
        .filter(|t| t.state == TaskState::Done)
    {
        let body = format!(
            "{}|{}|{}",
            t.attempts,
            t.job_ids.join(","),
            t.result.as_deref().unwrap_or("")
        );
        parts.push((
            format!("task:{}", t.id),
            hex::encode(Sha256::digest(body.as_bytes())),
        ));
    }
    let bytes = serde_json::to_vec(&parts).ok()?;
    Some(format!("rev:{}", hex::encode(Sha256::digest(&bytes))))
}

/// `current`, with judged-without-artifact criteria bound to
/// [`revision_digest`] instead of whatever the caller computed for them.
pub fn with_revision<'a>(
    db: &'a AssistDb,
    mission_id: &'a str,
    current: &'a dyn Fn(&Criterion) -> Option<String>,
) -> impl Fn(&Criterion) -> Option<String> + 'a {
    move |c: &Criterion| {
        if judged_without_artifact(c) {
            revision_digest(db, mission_id, current)
        } else {
            current(c)
        }
    }
}

/// Tries to finish the mission. Moves `running → verifying`, then:
/// every required criterion passes for the current artifacts → `succeeded`;
/// something waits for a person → stays `verifying` with the work clock
/// stopped (waiting is not work, F7); otherwise → back to
/// `running` (the caller decides whether another round or a block follows).
/// The model's own words are not an input here.
pub fn complete(
    db: &AssistDb,
    mission_id: &str,
    current: &dyn Fn(&Criterion) -> Option<String>,
) -> Result<(Mission, verify::Verdict), String> {
    let wrapped = with_revision(db, mission_id, current);
    let current: &dyn Fn(&Criterion) -> Option<String> = &wrapped;
    invalidate_stale(db, mission_id, current)?;
    let m = get(db, mission_id)?;
    let crit = criteria(db, mission_id, Some(m.criteria_version))?;
    let recs = receipts(db, mission_id)?;
    let verdict = verify::verdict(&crit, &recs, current);
    let checked_revision = m.revision;
    let checked_criteria_version = m.criteria_version;
    let m = db.tx(|tx| {
        let m = get_tx(tx, mission_id)?;
        if m.revision != checked_revision || m.criteria_version != checked_criteria_version {
            return Err(format!("{ERR_MISSION_REVISION}: mission changed during verification; verify the current revision"));
        }
        if verdict.passed {
            // Terminal failed/cancelled/skipped tasks do not override passing
            // required criteria. Pending/live/blocked or unrecognized states
            // can still change artifacts and must never permit success.
            let unfinished: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM missions_tasks WHERE mission_id=?1 AND state NOT IN ('done','failed','cancelled','skipped'))",
                params![mission_id], |r| r.get(0)).map_err(e)?;
            if unfinished {
                return Err(format!("{ERR_MISSION_STATE}: unfinished tasks prevent completion"));
            }
            let unsettled: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM missions_effects WHERE mission_id=?1 AND state NOT IN ('confirmed','failed','unknown_accepted'))",
                params![mission_id], |r| r.get(0)).map_err(e)?;
            if unsettled {
                return Err(format!("{ERR_MISSION_EFFECT_UNKNOWN}: unsettled effects prevent completion"));
            }
        }
        if m.state != MissionState::Verifying {
            transition_tx(tx, mission_id, MissionState::Verifying, "checking criteria", None, None)?;
        }
        append_event_tx(tx, mission_id, NewEvent::new("before_completion", serde_json::to_value(&verdict).unwrap_or_default()))?;
        if verdict.passed {
            let m = transition_tx(tx, mission_id, MissionState::Succeeded, "every required criterion passed for the current artifacts", None, None)?;
            append_event_tx(tx, mission_id, NewEvent::new("mission_finished", json!({ "state": "succeeded" })))?;
            Ok(m)
        } else if !verdict.awaiting_human.is_empty() && verdict.failing.is_empty() && verdict.missing.is_empty() {
            // Only a person's decision is left: the automated part is done,
            // so the work clock stops until something resumes the mission.
            let mut m = get_tx(tx, mission_id)?;
            if m.spent.active_since_ms.is_some() {
                stop_clock(&mut m.spent, now_ms());
                tx.execute(
                    "UPDATE missions_missions SET spent = ?2 WHERE id = ?1",
                    params![mission_id, serde_json::to_string(&m.spent).map_err(e)?],
                )
                .map_err(e)?;
            }
            Ok(m)
        } else if !verdict.partial.is_empty() && verdict.failing.is_empty() && verdict.missing.is_empty() && verdict.awaiting_human.is_empty() {
            // Delivered less than asked, with the reason recorded: an honest
            // partial result, not a success and not something to retry blindly.
            let m = transition_tx(tx, mission_id, MissionState::Partial, &verdict.summary(), None, None)?;
            append_event_tx(tx, mission_id, NewEvent::new("mission_finished", json!({ "state": "partial" })))?;
            Ok(m)
        } else {
            transition_tx(tx, mission_id, MissionState::Running, &verdict.summary(), None, None)
        }
    })?;
    Ok((m, verdict))
}

/// Appends a task (a new round after failing checks, a follow-up the person
/// adds). Its dependencies name existing task ids.
pub fn add_task(db: &AssistDb, mission_id: &str, t: NewTask) -> Result<MissionTask, String> {
    if t.input.trim().is_empty() {
        return Err(input_err("the task is empty"));
    }
    db.tx(|tx| {
        let m = get_tx(tx, mission_id)?;
        if m.state.is_final() {
            return Err(format!("{ERR_MISSION_STATE}: mission is {}", m.state.as_str()));
        }
        let (n, pos): (i64, i64) = tx
            .query_row("SELECT count(*), COALESCE(MAX(position), -1) + 1 FROM missions_tasks WHERE mission_id = ?1", params![mission_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(e)?;
        if n as usize >= MAX_TASKS * 2 {
            return Err(format!("{ERR_MISSION_STATE}: this mission already has {n} tasks"));
        }
        for d in &t.deps {
            let ok: Option<String> = tx.query_row("SELECT id FROM missions_tasks WHERE id = ?1 AND mission_id = ?2", params![d, mission_id], |r| r.get(0)).optional().map_err(e)?;
            if ok.is_none() {
                return Err(input_err(format!("dependency {d} is not a task of this mission")));
            }
        }
        let id = new_id();
        let now = now_ms();
        tx.execute(
            &format!("INSERT INTO missions_tasks ({TASK_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,'pending',NULL,NULL,0,?8,'[]',NULL,NULL,?9,?9)"),
            params![id, mission_id, pos, if t.title.trim().is_empty() { clip(&t.input, 120) } else { t.title.clone() }, t.input, serde_json::to_string(&t.deps).map_err(e)?, t.bot_id.clone().or(m.bot_id.clone()), t.max_attempts.unwrap_or(2).clamp(1, 10), now],
        )
        .map_err(e)?;
        bump_work_revision_tx(tx, mission_id)?;
        append_event_tx(tx, mission_id, NewEvent::new("task_added", json!({ "task": id, "title": clip(&t.title, 120) })))?;
        task_tx(tx, &id)
    })
}

/// What a mission needs from the runtime that runs it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Needs {
    /// The work calls OmniGet tools (project files, reading, web).
    pub tools: bool,
}

/// Whether the runtime can do what the mission needs. A runtime that cannot
/// reach OmniGet's tools is not asked to pretend (A27).
pub fn capability_gap(
    caps: &crate::core::llm::caps::RuntimeCaps,
    needs: &Needs,
    mission_id: &str,
) -> Option<diag::Diagnosis> {
    if caps.missing {
        return Some(
            diag::Diagnosis::new(
                ERR_MISSION_UNSUPPORTED,
                "dispatch",
                false,
                format!("{} is not installed on this machine", caps.label),
            )
            .with_correlation(mission_id)
            .with_action(diag::SuggestedAction::new(
                "change_bot",
                "Use another bot",
                "Pick a bot whose runtime is installed.",
                false,
            )),
        );
    }
    if needs.tools && !caps.managed_tools && !caps.mcp_projection {
        return Some(
            diag::Diagnosis::new(
                ERR_MISSION_UNSUPPORTED,
                "dispatch",
                false,
                format!("{} cannot reach OmniGet's tools (no managed tools, no MCP projection); this mission needs them", caps.label),
            )
            .with_correlation(mission_id)
            .with_detail(&caps.limits.join("; "))
            .with_action(diag::SuggestedAction::new("change_bot", "Use another bot", "Pick a bot whose runtime can call tools.", false)),
        );
    }
    None
}

/// What a mission needs, from its criteria and workspace.
pub fn needs_of(m: &Mission, criteria: &[Criterion]) -> Needs {
    Needs {
        tools: m.workspace.is_some()
            || criteria.iter().any(|c| c.kind == CriterionKind::ToolResult),
    }
}

/// Rounds the progress guard observed since the last reset, rebuilt from the
/// events (the counter survives a restart). F9.
pub fn round_observations(
    db: &AssistDb,
    mission_id: &str,
) -> Result<Vec<progress::RoundObservation>, String> {
    let rows: Vec<(String, String)> = db.with(|c| {
        let mut st = c.prepare(
            "SELECT kind, payload FROM missions_events WHERE mission_id = ?1 AND kind IN ('round_observed','rounds_reset') ORDER BY seq",
        )?;
        let rows = st.query_map(params![mission_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect()
    })?;
    let mut out = Vec::new();
    for (kind, payload) in rows {
        if kind == "rounds_reset" {
            out.clear();
        } else if let Ok(o) = serde_json::from_str::<progress::RoundObservation>(&payload) {
            out.push(o);
        }
    }
    Ok(out)
}

pub fn record_round(
    db: &AssistDb,
    mission_id: &str,
    o: &progress::RoundObservation,
) -> Result<(), String> {
    append_event(
        db,
        mission_id,
        NewEvent::new("round_observed", serde_json::to_value(o).map_err(e)?),
    )?;
    Ok(())
}

pub fn reset_rounds(db: &AssistDb, mission_id: &str) -> Result<(), String> {
    append_event(db, mission_id, NewEvent::new("rounds_reset", json!({})))?;
    Ok(())
}

/// [`clip`] for callers outside the crate (the app's driver).
pub fn clip_pub(s: &str, max: usize) -> String {
    clip(s, max)
}
