use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::ctx::AssistCtx;
use super::db::{AssistDb, Migration};

pub const ERR_RUNS: &str = "ERR_RUNS";
pub const ERR_RUNS_FINGERPRINT: &str = "ERR_RUNS_FINGERPRINT";
pub const ERR_RUNS_STATE: &str = "ERR_RUNS_STATE";
pub const ERR_RUNS_NOT_FOUND: &str = "ERR_RUNS_NOT_FOUND";
pub const ERR_PERMISSION_EXPIRED: &str = "ERR_PERMISSION_EXPIRED";
pub const ERR_PERMISSION_RESOLVED: &str = "ERR_PERMISSION_RESOLVED";

pub const EVENT_PAYLOAD_MAX: usize = 4 * 1024;
pub const EVENTS_PER_RUN_MAX: i64 = 2_000;

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        module: "runs",
        version: 1,
        sql: "
CREATE TABLE runs_sessions (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    bot_id TEXT NOT NULL,
    runtime TEXT NOT NULL,
    account TEXT,
    exe_version TEXT,
    cwd TEXT,
    context_kind TEXT NOT NULL DEFAULT 'projectless',
    provider_handle TEXT,
    generation INTEGER NOT NULL DEFAULT 1,
    state TEXT NOT NULL DEFAULT 'active',
    native_resume INTEGER NOT NULL DEFAULT 0,
    created_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL,
    UNIQUE(conversation_id, bot_id)
);
CREATE TABLE runs_runs (
    id TEXT PRIMARY KEY,
    session_id TEXT,
    conversation_id TEXT NOT NULL,
    bot_id TEXT NOT NULL,
    parent_run_id TEXT,
    state TEXT NOT NULL,
    resume_kind TEXT,
    runtime TEXT,
    cwd TEXT,
    instance_id TEXT NOT NULL,
    launch_id TEXT,
    pid INTEGER,
    input_preview TEXT NOT NULL DEFAULT '',
    summary TEXT,
    error TEXT,
    resolution TEXT,
    tokens_in INTEGER NOT NULL DEFAULT 0,
    tokens_out INTEGER NOT NULL DEFAULT 0,
    cost_usd REAL,
    events_dropped INTEGER NOT NULL DEFAULT 0,
    created_ms INTEGER NOT NULL,
    started_ms INTEGER,
    ended_ms INTEGER,
    updated_ms INTEGER NOT NULL
);
CREATE INDEX runs_runs_conversation ON runs_runs(conversation_id, created_ms DESC);
CREATE INDEX runs_runs_state ON runs_runs(state);
CREATE TABLE runs_events (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs_runs(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    provider_id TEXT,
    payload TEXT NOT NULL DEFAULT '{}',
    v INTEGER NOT NULL DEFAULT 1,
    ts_ms INTEGER NOT NULL,
    UNIQUE(run_id, seq)
);
CREATE UNIQUE INDEX runs_events_provider ON runs_events(run_id, provider_id)
    WHERE provider_id IS NOT NULL;
CREATE TABLE runs_receipts (
    operation_id TEXT PRIMARY KEY,
    run_id TEXT,
    fingerprint TEXT NOT NULL,
    state TEXT NOT NULL,
    result TEXT,
    error TEXT,
    created_ms INTEGER NOT NULL,
    updated_ms INTEGER NOT NULL
);
CREATE TABLE runs_permissions (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL,
    tool_call_id TEXT NOT NULL,
    action TEXT NOT NULL,
    scope TEXT NOT NULL DEFAULT '',
    preview TEXT NOT NULL DEFAULT '',
    options TEXT NOT NULL DEFAULT '[]',
    deadline_ms INTEGER NOT NULL,
    state TEXT NOT NULL,
    answer TEXT,
    version INTEGER NOT NULL DEFAULT 1,
    created_ms INTEGER NOT NULL,
    resolved_ms INTEGER
);
CREATE INDEX runs_permissions_run ON runs_permissions(run_id, state);
",
    },
    Migration {
        module: "runs",
        version: 2,
        sql: "
ALTER TABLE runs_permissions ADD COLUMN agent_id TEXT NOT NULL DEFAULT '';
ALTER TABLE runs_permissions ADD COLUMN instance_id TEXT NOT NULL DEFAULT '';
ALTER TABLE runs_permissions ADD COLUMN payload TEXT NOT NULL DEFAULT '{}';
ALTER TABLE runs_permissions ADD COLUMN channel TEXT;
ALTER TABLE runs_permissions ADD COLUMN decided_by TEXT;
ALTER TABLE runs_permissions ADD COLUMN note TEXT;
CREATE INDEX runs_permissions_state ON runs_permissions(state, created_ms);
",
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Queued,
    Preparing,
    Running,
    WaitingUser,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    Unknown,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Queued => "queued",
            RunState::Preparing => "preparing",
            RunState::Running => "running",
            RunState::WaitingUser => "waiting_user",
            RunState::Completed => "completed",
            RunState::Failed => "failed",
            RunState::Cancelled => "cancelled",
            RunState::Interrupted => "interrupted",
            RunState::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "queued" => RunState::Queued,
            "preparing" => RunState::Preparing,
            "running" => RunState::Running,
            "waiting_user" => RunState::WaitingUser,
            "completed" => RunState::Completed,
            "failed" => RunState::Failed,
            "cancelled" => RunState::Cancelled,
            "interrupted" => RunState::Interrupted,
            "unknown" => RunState::Unknown,
            _ => return None,
        })
    }

    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            RunState::Completed | RunState::Failed | RunState::Cancelled
        )
    }

    pub fn is_live(self) -> bool {
        matches!(
            self,
            RunState::Queued | RunState::Preparing | RunState::Running | RunState::WaitingUser
        )
    }

    pub fn can_go(self, to: RunState) -> bool {
        use RunState::*;
        if self == to {
            return true;
        }
        match self {
            Queued => matches!(to, Preparing | Running | Cancelled | Failed | Interrupted),
            Preparing => matches!(to, Running | Failed | Cancelled | Interrupted | Unknown),
            Running => matches!(
                to,
                WaitingUser | Completed | Failed | Cancelled | Interrupted | Unknown
            ),
            WaitingUser => matches!(
                to,
                Running | Completed | Failed | Cancelled | Interrupted | Unknown
            ),
            Interrupted | Unknown => matches!(to, Running | Completed | Cancelled | Failed),
            Completed | Failed | Cancelled => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeKind {
    Native,
    Replay,
    New,
}

impl ResumeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ResumeKind::Native => "native",
            ResumeKind::Replay => "replay",
            ResumeKind::New => "new",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunUpdate {
    pub run_id: String,
    pub conversation_id: String,
    pub bot_id: String,
    pub state: RunState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_kind: Option<ResumeKind>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeSession {
    pub id: String,
    pub conversation_id: String,
    pub bot_id: String,
    pub runtime: String,
    pub account: Option<String>,
    pub exe_version: Option<String>,
    pub cwd: Option<String>,
    pub context_kind: String,
    pub provider_handle: Option<String>,
    pub generation: i64,
    pub state: String,
    pub native_resume: bool,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionPins {
    pub runtime: String,
    pub account: Option<String>,
    pub exe_version: Option<String>,
    pub cwd: Option<String>,
    pub context_kind: String,
    pub native_resume: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: String,
    pub session_id: Option<String>,
    pub conversation_id: String,
    pub bot_id: String,
    pub parent_run_id: Option<String>,
    pub state: RunState,
    pub resume_kind: Option<ResumeKind>,
    pub runtime: Option<String>,
    pub cwd: Option<String>,
    pub instance_id: String,
    pub launch_id: Option<String>,
    pub pid: Option<i64>,
    pub input_preview: String,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub resolution: Option<String>,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost_usd: Option<f64>,
    pub events_dropped: i64,
    pub created_ms: i64,
    pub started_ms: Option<i64>,
    pub ended_ms: Option<i64>,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunEvent {
    pub id: String,
    pub run_id: String,
    pub seq: i64,
    pub kind: String,
    pub provider_id: Option<String>,
    pub payload: Value,
    pub ts_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    pub operation_id: String,
    pub run_id: Option<String>,
    pub fingerprint: String,
    pub state: String,
    pub result: Option<String>,
    pub error: Option<String>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Admission {
    New(Receipt),
    Existing(Receipt),
}

impl Admission {
    pub fn receipt(&self) -> &Receipt {
        match self {
            Admission::New(r) | Admission::Existing(r) => r,
        }
    }
    pub fn is_new(&self) -> bool {
        matches!(self, Admission::New(_))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Pending,
    Approved,
    Denied,
    Expired,
    Cancelled,
    Stale,
}

impl PermissionState {
    pub fn as_str(self) -> &'static str {
        match self {
            PermissionState::Pending => "pending",
            PermissionState::Approved => "approved",
            PermissionState::Denied => "denied",
            PermissionState::Expired => "expired",
            PermissionState::Cancelled => "cancelled",
            PermissionState::Stale => "stale",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "approved" => PermissionState::Approved,
            "denied" => PermissionState::Denied,
            "expired" => PermissionState::Expired,
            "cancelled" => PermissionState::Cancelled,
            "stale" => PermissionState::Stale,
            _ => PermissionState::Pending,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub id: String,
    pub run_id: String,
    pub tool_call_id: String,
    pub action: String,
    pub scope: String,
    pub preview: String,
    pub options: Vec<String>,
    pub deadline_ms: i64,
    pub state: PermissionState,
    pub answer: Option<String>,
    pub version: i64,
    pub created_ms: i64,
    pub resolved_ms: Option<i64>,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub instance_id: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub decided_by: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Actor {
    pub channel: String,
    pub decided_by: String,
}

impl Actor {
    pub fn new(channel: &str, decided_by: &str) -> Self {
        Self {
            channel: channel.to_string(),
            decided_by: decided_by.to_string(),
        }
    }

    pub fn window() -> Self {
        Self::new("window", "window")
    }
}

#[derive(Debug, Clone, Default)]
pub struct NewAsk {
    pub run_id: String,
    pub tool_call_id: String,
    pub action: String,
    pub preview: String,
    pub options: Vec<String>,
    pub deadline_ms: i64,
    pub agent_id: String,
    pub payload: Value,
}

pub const STALE_NOTE: &str =
    "the app closed while this question waited; nothing ran. Restart the turn to ask again";

pub fn new_ask_id() -> String {
    let mut buf = [0u8; 16];
    rand::Rng::fill_bytes(&mut rand::rng(), &mut buf);
    format!("apr_{}", hex::encode(buf))
}

type Sink = Arc<dyn Fn(&RunUpdate) + Send + Sync>;

static ENABLED: AtomicBool = AtomicBool::new(false);
static SINK: RwLock<Option<Sink>> = RwLock::new(None);
static INSTANCE: OnceLock<String> = OnceLock::new();

tokio::task_local! {
    static CURRENT: Registry;
}

pub fn instance_id() -> String {
    INSTANCE
        .get_or_init(|| format!("boot-{}", super::new_id()))
        .clone()
}

pub fn enable() {
    ENABLED.store(true, Ordering::SeqCst);
}

pub fn set_sink(f: Sink) {
    *SINK.write().unwrap_or_else(|e| e.into_inner()) = Some(f);
}

pub fn active() -> Option<Registry> {
    if let Ok(r) = CURRENT.try_with(Clone::clone) {
        return Some(r);
    }
    if ENABLED.load(Ordering::SeqCst) {
        return super::db::global().ok().map(Registry::new);
    }
    None
}

pub async fn scope<F: std::future::Future>(reg: Option<Registry>, fut: F) -> F::Output {
    match reg {
        Some(r) => CURRENT.scope(r, fut).await,
        None => fut.await,
    }
}

pub fn note_tool_use(ctx: &AssistCtx, tool: &str, ok: bool) {
    let (Some(run), Some(reg)) = (ctx.run_id.as_deref(), active()) else {
        return;
    };
    let _ = reg.add_event(
        run,
        "tool_used",
        None,
        json!({ "tool": tool, "ok": ok, "bot": ctx.bot_id }),
    );
}

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{ERR_RUNS}: {e}")
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn clip_json(v: Value) -> Value {
    let text = v.to_string();
    if text.len() <= EVENT_PAYLOAD_MAX {
        return v;
    }
    json!({ "clipped": true, "bytes": text.len(), "head": clip(&text, EVENT_PAYLOAD_MAX - 64) })
}

fn event_id(run_id: &str, provider_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(run_id.as_bytes());
    h.update([0u8]);
    h.update(provider_id.as_bytes());
    format!("ev-{}", &hex::encode(h.finalize())[..32])
}

pub fn fingerprint(v: &Value) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(v.to_string().as_bytes());
    hex::encode(h.finalize())
}

const RUN_COLS: &str = "id, session_id, conversation_id, bot_id, parent_run_id, state, resume_kind, runtime, cwd, instance_id, launch_id, pid, input_preview, summary, error, resolution, tokens_in, tokens_out, cost_usd, events_dropped, created_ms, started_ms, ended_ms, updated_ms";

fn row_run(r: &rusqlite::Row<'_>) -> rusqlite::Result<RunRecord> {
    let resume: Option<String> = r.get(6)?;
    Ok(RunRecord {
        id: r.get(0)?,
        session_id: r.get(1)?,
        conversation_id: r.get(2)?,
        bot_id: r.get(3)?,
        parent_run_id: r.get(4)?,
        state: RunState::parse(&r.get::<_, String>(5)?).unwrap_or(RunState::Unknown),
        resume_kind: resume.and_then(|s| match s.as_str() {
            "native" => Some(ResumeKind::Native),
            "replay" => Some(ResumeKind::Replay),
            "new" => Some(ResumeKind::New),
            _ => None,
        }),
        runtime: r.get(7)?,
        cwd: r.get(8)?,
        instance_id: r.get(9)?,
        launch_id: r.get(10)?,
        pid: r.get(11)?,
        input_preview: r.get(12)?,
        summary: r.get(13)?,
        error: r.get(14)?,
        resolution: r.get(15)?,
        tokens_in: r.get(16)?,
        tokens_out: r.get(17)?,
        cost_usd: r.get(18)?,
        events_dropped: r.get(19)?,
        created_ms: r.get(20)?,
        started_ms: r.get(21)?,
        ended_ms: r.get(22)?,
        updated_ms: r.get(23)?,
    })
}

const SESSION_COLS: &str = "id, conversation_id, bot_id, runtime, account, exe_version, cwd, context_kind, provider_handle, generation, state, native_resume, created_ms, updated_ms";

fn row_session(r: &rusqlite::Row<'_>) -> rusqlite::Result<RuntimeSession> {
    Ok(RuntimeSession {
        id: r.get(0)?,
        conversation_id: r.get(1)?,
        bot_id: r.get(2)?,
        runtime: r.get(3)?,
        account: r.get(4)?,
        exe_version: r.get(5)?,
        cwd: r.get(6)?,
        context_kind: r.get(7)?,
        provider_handle: r.get(8)?,
        generation: r.get(9)?,
        state: r.get(10)?,
        native_resume: r.get::<_, i64>(11)? != 0,
        created_ms: r.get(12)?,
        updated_ms: r.get(13)?,
    })
}

const PERM_COLS: &str = "id, run_id, tool_call_id, action, scope, preview, options, deadline_ms, state, answer, version, created_ms, resolved_ms, agent_id, instance_id, payload, channel, decided_by, note";

fn row_perm(r: &rusqlite::Row<'_>) -> rusqlite::Result<PermissionRequest> {
    let options: String = r.get(6)?;
    Ok(PermissionRequest {
        id: r.get(0)?,
        run_id: r.get(1)?,
        tool_call_id: r.get(2)?,
        action: r.get(3)?,
        scope: r.get(4)?,
        preview: r.get(5)?,
        options: serde_json::from_str(&options).unwrap_or_default(),
        deadline_ms: r.get(7)?,
        state: PermissionState::parse(&r.get::<_, String>(8)?),
        answer: r.get(9)?,
        version: r.get(10)?,
        created_ms: r.get(11)?,
        resolved_ms: r.get(12)?,
        agent_id: r.get(13)?,
        instance_id: r.get(14)?,
        payload: serde_json::from_str(&r.get::<_, String>(15)?).unwrap_or(Value::Null),
        channel: r.get(16)?,
        decided_by: r.get(17)?,
        note: r.get(18)?,
    })
}

fn row_receipt(r: &rusqlite::Row<'_>) -> rusqlite::Result<Receipt> {
    Ok(Receipt {
        operation_id: r.get(0)?,
        run_id: r.get(1)?,
        fingerprint: r.get(2)?,
        state: r.get(3)?,
        result: r.get(4)?,
        error: r.get(5)?,
        created_ms: r.get(6)?,
        updated_ms: r.get(7)?,
    })
}

const RECEIPT_COLS: &str =
    "operation_id, run_id, fingerprint, state, result, error, created_ms, updated_ms";

#[derive(Debug, Clone, Default)]
pub struct NewRun {
    pub id: String,
    pub conversation_id: String,
    pub bot_id: String,
    pub parent_run_id: Option<String>,
    pub runtime: Option<String>,
    pub input_preview: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Reconciled {
    pub interrupted: Vec<String>,
    pub unknown: Vec<String>,
    pub permissions_stale: usize,
    pub receipts_unknown: usize,
}

#[derive(Clone)]
pub struct Registry {
    db: Arc<AssistDb>,
    instance: Arc<str>,
    sink: Option<Sink>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("instance", &self.instance)
            .finish()
    }
}

impl Registry {
    pub fn new(db: Arc<AssistDb>) -> Self {
        Self {
            db,
            instance: instance_id().into(),
            sink: None,
        }
    }

    pub fn db(&self) -> &Arc<AssistDb> {
        &self.db
    }

    pub fn instance(&self) -> &str {
        &self.instance
    }

    fn emit(&self, run: &RunRecord) {
        let update = RunUpdate {
            run_id: run.id.clone(),
            conversation_id: run.conversation_id.clone(),
            bot_id: run.bot_id.clone(),
            state: run.state,
            error: run.error.clone(),
            resume_kind: run.resume_kind,
        };
        let sink = self
            .sink
            .clone()
            .or_else(|| SINK.read().unwrap_or_else(|e| e.into_inner()).clone());
        if let Some(sink) = sink {
            sink(&update);
        }
    }

    pub fn session(&self, conversation: &str, bot: &str) -> Option<RuntimeSession> {
        self.db
            .with(|c| {
                c.query_row(
                    &format!("SELECT {SESSION_COLS} FROM runs_sessions WHERE conversation_id=?1 AND bot_id=?2"),
                    params![conversation, bot],
                    row_session,
                )
                .optional()
            })
            .ok()
            .flatten()
    }

    pub fn open_session(
        &self,
        conversation: &str,
        bot: &str,
        pins: &SessionPins,
    ) -> Result<(RuntimeSession, bool), String> {
        let now = super::now_ms();
        self.db.tx(|tx| {
            let found = tx
                .query_row(
                    &format!("SELECT {SESSION_COLS} FROM runs_sessions WHERE conversation_id=?1 AND bot_id=?2"),
                    params![conversation, bot],
                    row_session,
                )
                .optional()
                .map_err(err)?;
            let changed = match &found {
                None => {
                    tx.execute(
                        "INSERT INTO runs_sessions (id, conversation_id, bot_id, runtime, account, exe_version, cwd, context_kind, generation, state, native_resume, created_ms, updated_ms)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,1,'active',?9,?10,?10)",
                        params![
                            super::new_id(), conversation, bot, pins.runtime, pins.account,
                            pins.exe_version, pins.cwd, pins.context_kind, pins.native_resume as i64, now
                        ],
                    )
                    .map_err(err)?;
                    false
                }
                Some(s) => {
                    let same = s.runtime == pins.runtime
                        && s.account == pins.account
                        && s.exe_version == pins.exe_version
                        && s.cwd == pins.cwd
                        && s.context_kind == pins.context_kind;
                    if same {
                        tx.execute(
                            "UPDATE runs_sessions SET native_resume=?2, updated_ms=?3 WHERE id=?1",
                            params![s.id, pins.native_resume as i64, now],
                        )
                        .map_err(err)?;
                    } else {
                        tx.execute(
                            "UPDATE runs_sessions SET runtime=?2, account=?3, exe_version=?4, cwd=?5, context_kind=?6,
                                 provider_handle=NULL, generation=generation+1, native_resume=?7, state='active', updated_ms=?8
                             WHERE id=?1",
                            params![
                                s.id, pins.runtime, pins.account, pins.exe_version, pins.cwd,
                                pins.context_kind, pins.native_resume as i64, now
                            ],
                        )
                        .map_err(err)?;
                    }
                    !same
                }
            };
            let s = tx
                .query_row(
                    &format!("SELECT {SESSION_COLS} FROM runs_sessions WHERE conversation_id=?1 AND bot_id=?2"),
                    params![conversation, bot],
                    row_session,
                )
                .map_err(err)?;
            Ok((s, changed))
        })
    }

    pub fn set_handle(&self, session_id: &str, handle: &str) -> Result<(), String> {
        self.db.with(|c| {
            c.execute(
                "UPDATE runs_sessions SET provider_handle=?2, updated_ms=?3 WHERE id=?1",
                params![session_id, handle, super::now_ms()],
            )
            .map(|_| ())
        })
    }

    pub fn drop_handle(&self, session_id: &str) -> Result<(), String> {
        self.db.with(|c| {
            c.execute(
                "UPDATE runs_sessions SET provider_handle=NULL, generation=generation+1, updated_ms=?2 WHERE id=?1",
                params![session_id, super::now_ms()],
            )
            .map(|_| ())
        })
    }

    pub fn open_run(&self, new: NewRun) -> Result<RunRecord, String> {
        let now = super::now_ms();
        let preview = clip(&new.input_preview, 400);
        let run = self.db.tx(|tx| {
            tx.execute(
                "INSERT OR IGNORE INTO runs_runs (id, conversation_id, bot_id, parent_run_id, state, runtime, instance_id, input_preview, created_ms, updated_ms)
                 VALUES (?1,?2,?3,?4,'queued',?5,?6,?7,?8,?8)",
                params![new.id, new.conversation_id, new.bot_id, new.parent_run_id, new.runtime, &*self.instance, preview, now],
            )
            .map_err(err)?;
            tx.query_row(
                &format!("SELECT {RUN_COLS} FROM runs_runs WHERE id=?1"),
                params![new.id],
                row_run,
            )
            .map_err(err)
        })?;
        self.emit(&run);
        Ok(run)
    }

    pub fn run(&self, id: &str) -> Option<RunRecord> {
        self.db
            .with(|c| {
                c.query_row(
                    &format!("SELECT {RUN_COLS} FROM runs_runs WHERE id=?1"),
                    params![id],
                    row_run,
                )
                .optional()
            })
            .ok()
            .flatten()
    }

    pub fn list_runs(&self, conversation: Option<&str>, limit: u32) -> Vec<RunRecord> {
        let limit = limit.clamp(1, 500) as i64;
        self.db
            .with(|c| {
                let (sql, conv) = match conversation {
                    Some(conv) => (
                        format!("SELECT {RUN_COLS} FROM runs_runs WHERE conversation_id=?1 ORDER BY created_ms DESC LIMIT ?2"),
                        conv.to_string(),
                    ),
                    None => (
                        format!("SELECT {RUN_COLS} FROM runs_runs WHERE ?1 = ?1 ORDER BY created_ms DESC LIMIT ?2"),
                        String::new(),
                    ),
                };
                let mut stmt = c.prepare(&sql)?;
                let rows = stmt.query_map(params![conv, limit], row_run)?;
                rows.collect()
            })
            .unwrap_or_default()
    }

    pub fn transition(
        &self,
        id: &str,
        to: RunState,
        error: Option<&str>,
    ) -> Result<RunRecord, String> {
        let now = super::now_ms();
        let (run, changed) = self.db.tx(|tx| {
            let cur = tx
                .query_row(
                    &format!("SELECT {RUN_COLS} FROM runs_runs WHERE id=?1"),
                    params![id],
                    row_run,
                )
                .optional()
                .map_err(err)?
                .ok_or_else(|| format!("{ERR_RUNS_NOT_FOUND}: run {id}"))?;
            if cur.state == to && error.is_none() {
                return Ok((cur, false));
            }
            if !cur.state.can_go(to) {
                return Err(format!(
                    "{ERR_RUNS_STATE}: run {id} cannot go from {} to {}",
                    cur.state.as_str(),
                    to.as_str()
                ));
            }
            let started = if to == RunState::Running && cur.started_ms.is_none() {
                Some(now)
            } else {
                cur.started_ms
            };
            let ended = if to.is_terminal() { Some(now) } else { cur.ended_ms };
            tx.execute(
                "UPDATE runs_runs SET state=?2, error=COALESCE(?3, error), started_ms=?4, ended_ms=?5, updated_ms=?6 WHERE id=?1",
                params![id, to.as_str(), error.map(|e| clip(e, 2000)), started, ended, now],
            )
            .map_err(err)?;
            let run = tx
                .query_row(
                    &format!("SELECT {RUN_COLS} FROM runs_runs WHERE id=?1"),
                    params![id],
                    row_run,
                )
                .map_err(err)?;
            Ok((run, true))
        })?;
        if changed {
            if to == RunState::Cancelled {
                let _ = self.cancel_permissions(id);
            }
            let _ = self.add_event(
                id,
                "state",
                None,
                json!({ "state": to.as_str(), "error": error }),
            );
            self.emit(&run);
        }
        Ok(run)
    }

    pub fn bind_session(
        &self,
        run_id: &str,
        session_id: &str,
        resume: ResumeKind,
        cwd: Option<&str>,
    ) -> Result<(), String> {
        self.db.with(|c| {
            c.execute(
                "UPDATE runs_runs SET session_id=?2, resume_kind=?3, cwd=COALESCE(?4, cwd), updated_ms=?5 WHERE id=?1",
                params![run_id, session_id, resume.as_str(), cwd, super::now_ms()],
            )
            .map(|_| ())
        })?;
        let _ = self.add_event(
            run_id,
            "session",
            None,
            json!({ "session": session_id, "resume_kind": resume.as_str() }),
        );
        if let Some(run) = self.run(run_id) {
            self.emit(&run);
        }
        Ok(())
    }

    pub fn set_process(
        &self,
        run_id: &str,
        launch_id: &str,
        pid: Option<u32>,
    ) -> Result<(), String> {
        self.db.with(|c| {
            c.execute(
                "UPDATE runs_runs SET launch_id=?2, pid=?3, updated_ms=?4 WHERE id=?1",
                params![run_id, launch_id, pid.map(|p| p as i64), super::now_ms()],
            )
            .map(|_| ())
        })
    }

    pub fn set_outcome(
        &self,
        run_id: &str,
        summary: Option<&str>,
        tokens_in: u64,
        tokens_out: u64,
        cost: Option<f64>,
    ) -> Result<(), String> {
        self.db.with(|c| {
            c.execute(
                "UPDATE runs_runs SET summary=COALESCE(?2, summary), tokens_in=?3, tokens_out=?4, cost_usd=?5, updated_ms=?6 WHERE id=?1",
                params![run_id, summary.map(|s| clip(s, 600)), tokens_in as i64, tokens_out as i64, cost, super::now_ms()],
            )
            .map(|_| ())
        })
    }

    pub fn resolve_run(
        &self,
        run_id: &str,
        resolution: RunResolution,
    ) -> Result<RunRecord, String> {
        let run = self
            .run(run_id)
            .ok_or_else(|| format!("{ERR_RUNS_NOT_FOUND}: run {run_id}"))?;
        if !matches!(run.state, RunState::Interrupted | RunState::Unknown) {
            return Err(format!(
                "{ERR_RUNS_STATE}: run {run_id} is {}, not interrupted",
                run.state.as_str()
            ));
        }
        let (to, text) = match &resolution {
            RunResolution::MarkedDone => (Some(RunState::Completed), "marked_done".to_string()),
            RunResolution::Discarded => (Some(RunState::Cancelled), "discarded".to_string()),
            RunResolution::ContinuedBy(next) => (None, format!("continued:{next}")),
        };
        self.db.with(|c| {
            c.execute(
                "UPDATE runs_runs SET resolution=?2, updated_ms=?3 WHERE id=?1",
                params![run_id, text, super::now_ms()],
            )
            .map(|_| ())
        })?;
        match to {
            Some(state) => self.transition(run_id, state, None),
            None => {
                let run = self
                    .run(run_id)
                    .ok_or_else(|| format!("{ERR_RUNS_NOT_FOUND}: run {run_id}"))?;
                self.emit(&run);
                Ok(run)
            }
        }
    }

    pub fn reconcile(&self) -> Result<Reconciled, String> {
        let now = super::now_ms();
        let stale: Vec<RunRecord> = self.db.with(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {RUN_COLS} FROM runs_runs WHERE state IN ('queued','preparing','running','waiting_user') AND instance_id <> ?1"
            ))?;
            let rows = stmt.query_map(params![&*self.instance], row_run)?;
            rows.collect()
        })?;
        let mut out = Reconciled::default();
        for run in stale {
            let to = match run.state {
                RunState::Queued | RunState::Preparing => RunState::Interrupted,
                _ => RunState::Unknown,
            };
            let why = "the app closed while this run was in flight";
            if let Ok(r) = self.transition(&run.id, to, Some(why)) {
                if r.state == RunState::Interrupted {
                    out.interrupted.push(r.id.clone());
                } else {
                    out.unknown.push(r.id.clone());
                }
            }
            out.receipts_unknown += self.db.with(|c| {
                c.execute(
                    "UPDATE runs_receipts SET state='unknown', updated_ms=?2 WHERE run_id=?1 AND state='admitted'",
                    params![run.id, now],
                )
            })?;
        }
        out.permissions_stale += self.db.with(|c| {
            c.execute(
                "UPDATE runs_permissions SET state='stale', channel='boot', note=?3, resolved_ms=?2, version=version+1 WHERE state='pending' AND instance_id <> ?1",
                params![&*self.instance, now, STALE_NOTE],
            )
        })?;
        if out.permissions_stale > 0 {
            crate::core::llm::audit::log(
                crate::core::llm::audit::Actor::system("boot"),
                "ask.stale",
                crate::core::llm::audit::Entity::ask("boot"),
                serde_json::json!({ "count": out.permissions_stale }),
            );
        }
        Ok(out)
    }

    pub fn add_event(
        &self,
        run_id: &str,
        kind: &str,
        provider_id: Option<&str>,
        payload: Value,
    ) -> Result<Option<i64>, String> {
        let now = super::now_ms();
        let payload = clip_json(payload).to_string();
        self.db.tx(|tx| {
            let exists: Option<i64> = tx
                .query_row(
                    "SELECT events_dropped FROM runs_runs WHERE id=?1",
                    params![run_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(err)?;
            if exists.is_none() {
                return Ok(None);
            }
            let id = match provider_id {
                Some(p) => {
                    let id = event_id(run_id, p);
                    let dup: Option<i64> = tx
                        .query_row(
                            "SELECT seq FROM runs_events WHERE id=?1",
                            params![id],
                            |r| r.get(0),
                        )
                        .optional()
                        .map_err(err)?;
                    if dup.is_some() {
                        return Ok(None);
                    }
                    id
                }
                None => format!("ev-{}", super::new_id()),
            };
            let (count, max): (i64, Option<i64>) = tx
                .query_row(
                    "SELECT count(*), max(seq) FROM runs_events WHERE run_id=?1",
                    params![run_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(err)?;
            if count >= EVENTS_PER_RUN_MAX {
                tx.execute(
                    "UPDATE runs_runs SET events_dropped=events_dropped+1 WHERE id=?1",
                    params![run_id],
                )
                .map_err(err)?;
                return Ok(None);
            }
            let seq = max.unwrap_or(0) + 1;
            tx.execute(
                "INSERT INTO runs_events (id, run_id, seq, kind, provider_id, payload, ts_ms) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![id, run_id, seq, kind, provider_id, payload, now],
            )
            .map_err(err)?;
            Ok(Some(seq))
        })
    }

    pub fn events(&self, run_id: &str) -> Vec<RunEvent> {
        self.db
            .with(|c| {
                let mut stmt = c.prepare(
                    "SELECT id, run_id, seq, kind, provider_id, payload, ts_ms FROM runs_events WHERE run_id=?1 ORDER BY seq",
                )?;
                let rows = stmt.query_map(params![run_id], |r| {
                    let payload: String = r.get(5)?;
                    Ok(RunEvent {
                        id: r.get(0)?,
                        run_id: r.get(1)?,
                        seq: r.get(2)?,
                        kind: r.get(3)?,
                        provider_id: r.get(4)?,
                        payload: serde_json::from_str(&payload).unwrap_or(Value::Null),
                        ts_ms: r.get(6)?,
                    })
                })?;
                rows.collect()
            })
            .unwrap_or_default()
    }

    pub fn admit(
        &self,
        operation_id: &str,
        run_id: Option<&str>,
        fingerprint: &str,
    ) -> Result<Admission, String> {
        if operation_id.trim().is_empty() {
            return Err(format!("{ERR_RUNS}: empty operation id"));
        }
        let now = super::now_ms();
        self.db.tx(|tx| {
            let found = tx
                .query_row(
                    &format!("SELECT {RECEIPT_COLS} FROM runs_receipts WHERE operation_id=?1"),
                    params![operation_id],
                    row_receipt,
                )
                .optional()
                .map_err(err)?;
            if let Some(r) = found {
                if r.fingerprint != fingerprint {
                    return Err(format!(
                        "{ERR_RUNS_FINGERPRINT}: operation {operation_id} was admitted with other content"
                    ));
                }
                return Ok(Admission::Existing(r));
            }
            tx.execute(
                "INSERT INTO runs_receipts (operation_id, run_id, fingerprint, state, created_ms, updated_ms) VALUES (?1,?2,?3,'admitted',?4,?4)",
                params![operation_id, run_id, fingerprint, now],
            )
            .map_err(err)?;
            let r = tx
                .query_row(
                    &format!("SELECT {RECEIPT_COLS} FROM runs_receipts WHERE operation_id=?1"),
                    params![operation_id],
                    row_receipt,
                )
                .map_err(err)?;
            Ok(Admission::New(r))
        })
    }

    pub fn receipt(&self, operation_id: &str) -> Option<Receipt> {
        self.db
            .with(|c| {
                c.query_row(
                    &format!("SELECT {RECEIPT_COLS} FROM runs_receipts WHERE operation_id=?1"),
                    params![operation_id],
                    row_receipt,
                )
                .optional()
            })
            .ok()
            .flatten()
    }

    pub fn open_ask(&self, ask: NewAsk) -> Result<PermissionRequest, String> {
        let id = new_ask_id();
        let now = super::now_ms();
        let options = serde_json::to_string(&ask.options).unwrap_or_else(|_| "[]".into());
        let payload = if ask.payload.is_null() {
            "{}".to_string()
        } else {
            ask.payload.to_string()
        };
        let req = self.db.tx(|tx| {
            tx.execute(
                "INSERT INTO runs_permissions (id, run_id, tool_call_id, action, preview, options, deadline_ms, state, created_ms, agent_id, instance_id, payload) VALUES (?1,?2,?3,?4,?5,?6,?7,'pending',?8,?9,?10,?11)",
                params![id, ask.run_id, ask.tool_call_id, ask.action, clip(&ask.preview, 2000), options, ask.deadline_ms, now, ask.agent_id, &*self.instance, clip(&payload, 8000)],
            )
            .map_err(err)?;
            tx.query_row(
                &format!("SELECT {PERM_COLS} FROM runs_permissions WHERE id=?1"),
                params![id],
                row_perm,
            )
            .map_err(err)
        })?;
        if let Some(run) = self.run(&ask.run_id) {
            if matches!(run.state, RunState::Running) {
                let _ = self.transition(&ask.run_id, RunState::WaitingUser, None);
            }
            let _ = self.add_event(
                &ask.run_id,
                "permission_requested",
                Some(&format!("perm:{}", req.id)),
                json!({ "permission": req.id, "action": ask.action, "tool_call_id": ask.tool_call_id }),
            );
        }
        Ok(req)
    }

    pub fn permission(&self, id: &str) -> Option<PermissionRequest> {
        self.db
            .with(|c| {
                c.query_row(
                    &format!("SELECT {PERM_COLS} FROM runs_permissions WHERE id=?1"),
                    params![id],
                    row_perm,
                )
                .optional()
            })
            .ok()
            .flatten()
    }

    pub fn permissions(&self, run_id: Option<&str>, only_pending: bool) -> Vec<PermissionRequest> {
        self.db
            .with(|c| {
                let sql = format!(
                    "SELECT {PERM_COLS} FROM runs_permissions WHERE (?1 IS NULL OR run_id=?1) AND (?2 = 0 OR state='pending') ORDER BY created_ms DESC LIMIT 200"
                );
                let mut stmt = c.prepare(&sql)?;
                let rows = stmt.query_map(params![run_id, only_pending as i64], row_perm)?;
                rows.collect()
            })
            .unwrap_or_default()
    }

    pub fn permissions_in(&self, state: PermissionState, limit: usize) -> Vec<PermissionRequest> {
        self.db
            .with(|c| {
                let sql = format!(
                    "SELECT {PERM_COLS} FROM runs_permissions WHERE state=?1 ORDER BY created_ms DESC LIMIT ?2"
                );
                let mut stmt = c.prepare(&sql)?;
                let rows = stmt.query_map(params![state.as_str(), limit as i64], row_perm)?;
                rows.collect()
            })
            .unwrap_or_default()
    }

    pub fn answer_permission(&self, id: &str, answer: &str) -> Result<PermissionRequest, String> {
        self.answer_permission_at(id, answer, super::now_ms())
    }

    pub fn answer_permission_at(
        &self,
        id: &str,
        answer: &str,
        now: i64,
    ) -> Result<PermissionRequest, String> {
        self.answer_permission_by_at(id, answer, &Actor::window(), now)
    }

    pub fn answer_permission_by(
        &self,
        id: &str,
        answer: &str,
        actor: &Actor,
    ) -> Result<PermissionRequest, String> {
        self.answer_permission_by_at(id, answer, actor, super::now_ms())
    }

    pub fn answer_permission_by_at(
        &self,
        id: &str,
        answer: &str,
        actor: &Actor,
        now: i64,
    ) -> Result<PermissionRequest, String> {
        let state = match answer {
            "once" | "session" | "always" => PermissionState::Approved,
            "deny" => PermissionState::Denied,
            "cancel" => PermissionState::Cancelled,
            other => return Err(format!("{ERR_RUNS}: unknown answer `{other}`")),
        };
        let out = self.db.tx(|tx| {
            let cur = tx
                .query_row(
                    &format!("SELECT {PERM_COLS} FROM runs_permissions WHERE id=?1"),
                    params![id],
                    row_perm,
                )
                .optional()
                .map_err(err)?
                .ok_or_else(|| format!("{ERR_RUNS_NOT_FOUND}: permission {id}"))?;
            if cur.state != PermissionState::Pending {
                return Err(format!(
                    "{}: permission {id} is already {}",
                    if cur.state == PermissionState::Expired {
                        ERR_PERMISSION_EXPIRED
                    } else {
                        ERR_PERMISSION_RESOLVED
                    },
                    cur.state.as_str()
                ));
            }
            if now >= cur.deadline_ms {
                tx.execute(
                    "UPDATE runs_permissions SET state='expired', channel='timeout', resolved_ms=?2, version=version+1 WHERE id=?1",
                    params![id, now],
                )
                .map_err(err)?;
                return Ok(Err(format!(
                    "{ERR_PERMISSION_EXPIRED}: permission {id} expired before the answer"
                )));
            }
            tx.execute(
                "UPDATE runs_permissions SET state=?2, answer=?3, resolved_ms=?4, channel=?5, decided_by=?6, version=version+1 WHERE id=?1 AND state='pending'",
                params![id, state.as_str(), answer, now, clip(&actor.channel, 40), clip(&actor.decided_by, 200)],
            )
            .map_err(err)?;
            tx.query_row(
                &format!("SELECT {PERM_COLS} FROM runs_permissions WHERE id=?1"),
                params![id],
                row_perm,
            )
            .map(Ok)
            .map_err(err)
        })?;
        let req = out?;
        self.after_permission(&req);
        Ok(req)
    }

    pub fn dismiss_stale(&self, id: &str, actor: &Actor) -> Result<PermissionRequest, String> {
        let now = super::now_ms();
        let n = self.db.with(|c| {
            c.execute(
                "UPDATE runs_permissions SET state='cancelled', answer='cancel', channel=?3, decided_by=?4, resolved_ms=?2, version=version+1 WHERE id=?1 AND state='stale'",
                params![id, now, clip(&actor.channel, 40), clip(&actor.decided_by, 200)],
            )
        })?;
        let req = self
            .permission(id)
            .ok_or_else(|| format!("{ERR_RUNS_NOT_FOUND}: permission {id}"))?;
        if n == 0 {
            return Err(format!(
                "{ERR_PERMISSION_RESOLVED}: permission {id} is {}",
                req.state.as_str()
            ));
        }
        Ok(req)
    }

    pub fn expire_permission(&self, id: &str) -> Result<Option<PermissionRequest>, String> {
        let now = super::now_ms();
        let n = self.db.with(|c| {
            c.execute(
                "UPDATE runs_permissions SET state='expired', channel='timeout', resolved_ms=?2, version=version+1 WHERE id=?1 AND state='pending'",
                params![id, now],
            )
        })?;
        let req = self.permission(id);
        if n > 0 {
            if let Some(r) = &req {
                self.after_permission(r);
            }
        }
        Ok(req)
    }

    pub fn cancel_permissions(&self, run_id: &str) -> Result<usize, String> {
        let now = super::now_ms();
        let n = self.db.with(|c| {
            c.execute(
                "UPDATE runs_permissions SET state='cancelled', note='the turn ended before an answer', resolved_ms=?2, version=version+1 WHERE run_id=?1 AND state='pending'",
                params![run_id, now],
            )
        })?;
        if n > 0 {
            let _ = self.add_event(run_id, "permissions_cancelled", None, json!({ "count": n }));
        }
        Ok(n)
    }

    fn after_permission(&self, req: &PermissionRequest) {
        let _ = self.add_event(
            &req.run_id,
            "permission_resolved",
            Some(&format!("perm:{}:{}", req.id, req.version)),
            json!({ "permission": req.id, "state": req.state.as_str(), "answer": req.answer }),
        );
        let still = self.permissions(Some(&req.run_id), true);
        if still.is_empty() {
            if let Some(run) = self.run(&req.run_id) {
                if run.state == RunState::WaitingUser {
                    let _ = self.transition(&req.run_id, RunState::Running, None);
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunResolution {
    ContinuedBy(String),
    MarkedDone,
    Discarded,
}
