use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use chrono::{Datelike, Timelike};
use futures::StreamExt;
use omniget_core::core::llm::code_tools;
use omniget_core::core::llm::types::TurnEvent;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};
use tokio_util::sync::CancellationToken;

use crate::llm_manager::{now_ms, sanitize_id, LlmManager};

mod changes;
mod loop_audit;
mod review;

pub use loop_audit::LoopAudit;

pub const EVENT_JOB: &str = "llm://job";
pub const EVENT_LOOP: &str = "llm://loop";
pub const EVENT_TRIGGER: &str = "llm://trigger";
pub const EVENT_QUEUE: &str = "llm://queue";
pub const EVENT_PLAYBOOK: &str = "llm://playbook";
pub const ERR_JOBS: &str = "ERR_LLM_JOBS";
pub const ERR_WORKTREE: &str = "ERR_JOB_WORKTREE";

const LOG_MAX: usize = 64 * 1024;
const RESULT_MAX: usize = 32 * 1024;
const CRON_TICK_S: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Job {
    pub id: String,
    pub kind: String,
    pub agent_id: String,
    pub conversation_id: String,
    pub prompt: String,
    pub workspace: Option<String>,
    pub state: String,
    pub loop_id: Option<String>,
    pub trigger_id: Option<String>,
    pub request_id: Option<String>,
    pub created_ms: u64,
    pub started_ms: Option<u64>,
    pub finished_ms: Option<u64>,
    pub result: Option<String>,
    pub error: Option<String>,
    pub log: String,
    #[serde(default)]
    pub usage: Option<serde_json::Value>,
    #[serde(default)]
    pub state_seq: i64,
    #[serde(default)]
    pub seen_ms: Option<i64>,
    #[serde(default)]
    pub failure_reason: Option<String>,
    #[serde(default)]
    pub deadline_min: Option<u32>,
    #[serde(default)]
    pub deadline_ms: Option<u64>,
    #[serde(default)]
    pub wait_reason: Option<String>,
    #[serde(default)]
    pub diff_files: Option<Vec<omniget_core::core::llm::snapshot::FileChange>>,
    #[serde(default)]
    pub additions: Option<u64>,
    #[serde(default)]
    pub deletions: Option<u64>,
    #[serde(default)]
    pub diff_clipped: bool,
    #[serde(default)]
    pub diff_unavailable: Option<String>,
    #[serde(default)]
    pub spec: Option<serde_json::Value>,
    #[serde(default)]
    pub playbook_id: Option<String>,
    #[serde(default)]
    pub isolation: Isolation,
    #[serde(default)]
    pub base_ref: Option<String>,
    #[serde(default)]
    pub worktree_path: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub base_oid: Option<String>,
    #[serde(default)]
    pub head_oid: Option<String>,
    #[serde(default)]
    pub proof: Option<Value>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub run_mode: RunMode,
    #[serde(default)]
    pub outcome: Option<Value>,
    #[serde(default)]
    pub origin: Option<Value>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub pid_start: Option<u64>,
    #[serde(default)]
    pub attempt: u32,
    #[serde(default)]
    pub auto_resume: Option<bool>,
    #[serde(default)]
    pub side_effects: bool,
}

pub use omniget_core::core::llm::run_mode::RunMode;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Isolation {
    #[default]
    Shared,
    Worktree,
    Auto,
}

impl Isolation {
    pub fn as_str(self) -> &'static str {
        match self {
            Isolation::Shared => "shared",
            Isolation::Worktree => "worktree",
            Isolation::Auto => "auto",
        }
    }

    pub fn parse(s: &str) -> Result<Isolation, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "shared" => Ok(Isolation::Shared),
            "worktree" | "isolate" => Ok(Isolation::Worktree),
            "auto" => Ok(Isolation::Auto),
            other => Err(format!(
                "{ERR_JOBS}: isolation must be shared, worktree or auto (got `{other}`)"
            )),
        }
    }

    pub fn default_for(kind: &str, agent_id: &str) -> Isolation {
        if agent_id.starts_with(TOOL_PREFIX) {
            return Isolation::Shared;
        }
        match kind {
            "run" | "trigger" => Isolation::Auto,
            _ => Isolation::Shared,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    pub repo: Option<std::path::PathBuf>,
    pub force: bool,
    pub if_busy: bool,
}

pub fn placement(job: &Job, repo: Option<std::path::PathBuf>) -> Result<Placement, String> {
    if job.worktree_path.is_some() {
        return Ok(Placement {
            repo,
            force: true,
            if_busy: false,
        });
    }
    match job.isolation {
        Isolation::Shared => Ok(Placement {
            repo,
            force: false,
            if_busy: false,
        }),
        Isolation::Worktree if repo.is_none() => Err(format!(
            "{ERR_WORKTREE}: isolation=worktree needs a folder in a git repository with at least one commit"
        )),
        Isolation::Worktree => Ok(Placement {
            repo,
            force: true,
            if_busy: false,
        }),
        Isolation::Auto => Ok(Placement {
            if_busy: repo.is_some(),
            repo,
            force: false,
        }),
    }
}

pub fn effective_workspace(job: &Job) -> Option<String> {
    job.worktree_path
        .clone()
        .or_else(|| job.workspace.clone())
        .filter(|w| !w.trim().is_empty())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunSpec {
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub agent_name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub permission: Option<String>,
    #[serde(default)]
    pub sandbox: Option<omniget_core::core::agentkit_run::sandbox::SandboxOpts>,
}

pub const TOOL_PREFIX: &str = "tool:";

pub fn billable_of(u: &serde_json::Value) -> u64 {
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    omniget_core::core::llm::types::billable_input(
        n("input_tokens"),
        n("cache_read_tokens"),
        n("cache_write_tokens"),
    ) + n("output_tokens")
}

fn fold_usage(
    total: &mut Option<serde_json::Value>,
    model: &str,
    u: &omniget_core::core::llm::types::Usage,
) {
    let t = total.get_or_insert_with(|| serde_json::json!({
        "model": model, "input_tokens": 0, "output_tokens": 0, "cache_read_tokens": 0, "cache_write_tokens": 0, "cost_usd": null, "calls": 0
    }));
    let add = |t: &mut serde_json::Value, k: &str, n: u64| {
        t[k] = serde_json::json!(t[k].as_u64().unwrap_or(0) + n)
    };
    add(t, "input_tokens", u.input_tokens as u64);
    add(t, "output_tokens", u.output_tokens as u64);
    add(t, "cache_read_tokens", u.cache_read_tokens as u64);
    add(t, "cache_write_tokens", u.cache_write_tokens as u64);
    add(t, "calls", 1);
    if let Some(c) = u.cost_usd {
        t["cost_usd"] = serde_json::json!(t["cost_usd"].as_f64().unwrap_or(0.0) + c);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoopDef {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub agent_id: String,
    pub prompt: String,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub max_rounds: Option<u32>,
    #[serde(default)]
    pub max_minutes: Option<u32>,
    #[serde(default)]
    pub check_command: Option<String>,
    #[serde(default)]
    pub deadline_min: Option<u32>,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub rounds_done: u32,
    #[serde(default)]
    pub conversation_id: String,
    #[serde(default)]
    pub created_ms: u64,
    #[serde(default)]
    pub finished_ms: Option<u64>,
    #[serde(default)]
    pub last_check: Option<String>,
    #[serde(default)]
    pub stop_reason: Option<String>,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    #[serde(default)]
    pub spent_usd: f64,
    #[serde(default)]
    pub spec: Option<serde_json::Value>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub next_round_ms: Option<u64>,
    #[serde(default)]
    pub active_ms: u64,
    #[serde(default)]
    pub auto_resume: Option<bool>,
    #[serde(default)]
    pub restarted: bool,
    #[serde(default)]
    pub auto_restarts: u32,
    #[serde(default)]
    pub run_mode: Option<RunMode>,
    #[serde(default)]
    pub spent_tokens: u64,
    #[serde(default)]
    pub spent_usd_estimated: f64,
    #[serde(default)]
    pub unpriced_rounds: u32,
    #[serde(default)]
    pub token_budget: Option<u64>,
    #[serde(default)]
    pub tokens_used: u64,
    #[serde(default)]
    pub audit_model: Option<String>,
    #[serde(default)]
    pub blocked_streak: u32,
    #[serde(default)]
    pub audit_fail_streak: u32,
    #[serde(default)]
    pub disagree_streak: u32,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub audits: Vec<LoopAudit>,
    #[serde(default)]
    pub audit_usd: f64,
    #[serde(default)]
    pub rounds_guided: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlaybookStepRun {
    pub name: String,
    pub agent_id: String,
    pub prompt: String,
    #[serde(default)]
    pub spec: Option<serde_json::Value>,
    #[serde(default)]
    pub runner_label: Option<String>,
    #[serde(default)]
    pub job_id: Option<String>,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub output: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlaybookRun {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
    pub steps: Vec<PlaybookStepRun>,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub current: usize,
    #[serde(default)]
    pub created_ms: u64,
    #[serde(default)]
    pub finished_ms: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Trigger {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub cron: Option<String>,
    pub agent_id: String,
    pub prompt: String,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub last_fired_ms: Option<u64>,
    #[serde(default)]
    pub fire_count: u32,
    #[serde(default)]
    pub created_ms: u64,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub last_digest: Option<String>,
    #[serde(default)]
    pub last_notified_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_min: Option<u32>,
    #[serde(default)]
    pub next_run_ms: Option<i64>,
    #[serde(default)]
    pub catch_up: CatchUp,
    #[serde(default)]
    pub concurrency: TriggerConcurrency,
    #[serde(default = "local_zone")]
    pub tz: String,
    #[serde(default)]
    pub missed: u32,
    #[serde(default)]
    pub last_missed_ms: Option<u64>,
    #[serde(default)]
    pub backlog: u32,
    #[serde(default)]
    pub last_run_id: Option<String>,
}

fn yes() -> bool {
    true
}

fn local_zone() -> String {
    "local".into()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatchUp {
    Skip,
    #[default]
    Once,
    AllCapped,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerConcurrency {
    #[default]
    SkipIfActive,
    Queue,
}

pub const CATCH_UP_CAP: u32 = 25;

pub struct Jobs {
    db: Mutex<Connection>,
    app: AppHandle,
    llm: Arc<LlmManager>,
    gate: Arc<omniget_core::core::llm::job_gate::Gate>,
    cancels: Mutex<HashMap<String, CancellationToken>>,
    cron_running: AtomicBool,
    cron_pass: Mutex<()>,
    guards: Mutex<HashMap<String, JobGuard>>,
    live: Mutex<HashMap<String, Live>>,
    watchers: Mutex<HashMap<String, (String, tokio::sync::mpsc::UnboundedSender<AskSignal>)>>,
    pub attention: Arc<crate::llm_attention::Attention>,
    loop_waits: Mutex<HashMap<String, CancellationToken>>,
    last_worktree_gc: std::sync::atomic::AtomicU64,
    changes: tokio::sync::watch::Sender<u64>,
    round_evidence: Mutex<HashMap<String, String>>,
}

#[derive(Debug, Clone)]
pub enum AskSignal {
    Opened {
        tool_call_id: String,
        tool: String,
        preview: String,
    },
    Resolved {
        tool: String,
        outcome: String,
        waited_ms: u64,
    },
}

struct TauriSink {
    app: AppHandle,
}

impl crate::llm_attention::NotifySink for TauriSink {
    fn show(&self, title: &str, body: &str) {
        use tauri_plugin_notification::NotificationExt;
        let _ = self
            .app
            .notification()
            .builder()
            .title(title)
            .body(if body.is_empty() { " " } else { body })
            .show();
    }

    fn focus(&self) -> crate::llm_attention::UiFocus {
        let mut focus = crate::llm_attention::reported_focus();
        focus.focused = self
            .app
            .get_webview_window("main")
            .and_then(|w| w.is_focused().ok())
            .unwrap_or(false);
        focus
    }
}

pub type UsageCheck = Arc<dyn Fn(u64, Option<f64>) -> Result<(), String> + Send + Sync>;

#[derive(Clone, Default)]
pub struct JobGuard {
    pub deadline_ms: Option<u64>,
    pub check: Option<UsageCheck>,
    pub deadline_reason: String,
}

static JOBS: OnceLock<Arc<Jobs>> = OnceLock::new();

struct Unregister<'a> {
    jobs: &'a Jobs,
    id: &'a str,
}

impl Drop for Unregister<'_> {
    fn drop(&mut self) {
        self.jobs
            .cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(self.id);
    }
}

fn short_id(prefix: &str) -> String {
    format!(
        "{prefix}{}",
        &uuid::Uuid::new_v4().simple().to_string()[..10]
    )
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = s.len() - max;
    while !s.is_char_boundary(cut) {
        cut += 1;
    }
    format!("[…]\n{}", &s[cut..])
}

pub fn ensure_schema(db: &Connection, now_ms: i64) -> Result<(), String> {
    db.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS jobs (
            id TEXT PRIMARY KEY, kind TEXT NOT NULL, agent_id TEXT NOT NULL,
            conversation_id TEXT NOT NULL, prompt TEXT NOT NULL, workspace TEXT,
            state TEXT NOT NULL, loop_id TEXT, trigger_id TEXT, request_id TEXT,
            created_ms INTEGER NOT NULL, started_ms INTEGER, finished_ms INTEGER,
            result TEXT, error TEXT, log TEXT NOT NULL DEFAULT '');
         CREATE INDEX IF NOT EXISTS jobs_created ON jobs(created_ms DESC);
         CREATE TABLE IF NOT EXISTS loops (id TEXT PRIMARY KEY, body TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS triggers (id TEXT PRIMARY KEY, body TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS playbooks (id TEXT PRIMARY KEY, body TEXT NOT NULL);",
    )
    .map_err(|e| format!("{ERR_JOBS}: {e}"))?;
    let _ = db.execute("ALTER TABLE jobs ADD COLUMN usage TEXT", []);
    for column in [
        "state_seq INTEGER NOT NULL DEFAULT 0",
        "seen_ms INTEGER",
        "failure_reason TEXT",
        "deadline_min INTEGER",
        "deadline_ms INTEGER",
        "wait_reason TEXT",
        "diff_files TEXT",
        "additions INTEGER",
        "deletions INTEGER",
        "diff_clipped INTEGER NOT NULL DEFAULT 0",
        "diff_unavailable TEXT",
        "spec TEXT",
        "playbook_id TEXT",
        "isolation TEXT",
        "base_ref TEXT",
        "worktree_path TEXT",
        "branch TEXT",
        "base_oid TEXT",
        "head_oid TEXT",
        "proof TEXT",
        "title TEXT",
        "owner TEXT",
        "run_mode TEXT",
        "outcome TEXT",
        "origin TEXT",
        "session_id TEXT",
        "pid INTEGER",
        "pid_start INTEGER",
        "attempt INTEGER NOT NULL DEFAULT 0",
        "auto_resume INTEGER",
        "side_effects INTEGER NOT NULL DEFAULT 0",
    ] {
        let _ = db.execute(&format!("ALTER TABLE jobs ADD COLUMN {column}"), []);
    }
    let _ = db.execute(
        "CREATE INDEX IF NOT EXISTS jobs_request ON jobs(request_id)",
        [],
    );
    let _ = db.execute(
        "CREATE INDEX IF NOT EXISTS jobs_trigger ON jobs(trigger_id)",
        [],
    );
    let _ = db.execute("ALTER TABLE triggers ADD COLUMN next_run_ms INTEGER", []);
    migrate_triggers(db, now_ms);
    crate::bridge_tokens::ensure_schema(db).map_err(|e| format!("{ERR_JOBS}: {e}"))?;
    crate::job_commands::ensure_schema(db).map_err(|e| format!("{ERR_JOBS}: {e}"))?;
    review::ensure_notes_schema(db)?;
    changes::ensure_schema(db)?;
    Ok(())
}

pub fn get(app: &AppHandle) -> Result<Arc<Jobs>, String> {
    if let Some(j) = JOBS.get() {
        return Ok(j.clone());
    }
    let dir = omniget_core::core::llm::roster_store::llm_dir()
        .ok_or_else(|| format!("{ERR_JOBS}: no app data dir"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{ERR_JOBS}: {e}"))?;
    let db = Connection::open(dir.join("jobs.db")).map_err(|e| format!("{ERR_JOBS}: {e}"))?;
    ensure_schema(&db, now_ms() as i64)?;
    crate::commands::llm::ensure_wired(app);
    let jobs = Arc::new(Jobs {
        db: Mutex::new(db),
        app: app.clone(),
        llm: app.state::<crate::AppState>().llm.clone(),
        gate: omniget_core::core::llm::job_gate::Gate::new(gate_limits(
            &crate::storage::config::load_settings(app).llm,
        )),
        cancels: Mutex::new(HashMap::new()),
        cron_running: AtomicBool::new(false),
        cron_pass: Mutex::new(()),
        guards: Mutex::new(HashMap::new()),
        live: Mutex::new(HashMap::new()),
        watchers: Mutex::new(HashMap::new()),
        attention: crate::llm_attention::Attention::new(Arc::new(TauriSink { app: app.clone() })),
        loop_waits: Mutex::new(HashMap::new()),
        last_worktree_gc: std::sync::atomic::AtomicU64::new(now_ms()),
        changes: tokio::sync::watch::channel(0).0,
        round_evidence: Mutex::new(HashMap::new()),
    });
    if JOBS.set(jobs.clone()).is_ok() {
        let limits = crate::commands::llm::watchdog::limits(app);
        jobs.attention.set_delay(limits.notify_delay);
        jobs.attention
            .set_strings(crate::llm_attention::pushed_strings());
        let weak = Arc::downgrade(&jobs);
        jobs.llm
            .set_ask_timeout_resolver(Some(Arc::new(move |request: &str| {
                weak.upgrade()?.ask_timeout_for(request)
            })));
        let weak = Arc::downgrade(&jobs);
        omniget_core::core::llm::file_changes::set_sink(Some(Arc::new(
            move |request: &str, touch| {
                if let Some(jobs) = weak.upgrade() {
                    jobs.record_change(request, touch);
                }
            },
        )));
        let weak = Arc::downgrade(&jobs);
        let mut changed = jobs.gate.subscribe();
        tauri::async_runtime::spawn(async move {
            let mut reported: HashMap<String, (&'static str, String)> = HashMap::new();
            while changed.changed().await.is_ok() {
                let Some(jobs) = weak.upgrade() else { return };
                jobs.report_queue(&mut reported);
            }
        });
        {
            let db = jobs.db();
            let now = now_ms();
            if let Err(e) = crate::bridge_tokens::revoke_orphan_job_tokens(&db, now) {
                tracing::warn!("[jobs] job tokens: {e}");
            }
            if let Err(e) = crate::bridge_tokens::ensure_cli_token(&db, &dir, now) {
                tracing::warn!("[jobs] cli token: {e}");
            }
        }
        jobs.clone().resume();
        jobs.clone().ensure_cron();
        tauri::async_runtime::spawn(jobs.clone().gc_worktrees(true));
    }
    Ok(JOBS.get().cloned().unwrap_or(jobs))
}

pub fn try_get() -> Option<Arc<Jobs>> {
    JOBS.get().cloned()
}

pub fn on_bus(event: &omniget_core::core::omni::bus::BusEvent) {
    if let Some(jobs) = try_get() {
        jobs.handle_bus(event);
    }
}

pub fn boot(app: &AppHandle) {
    crate::commands::assist::runs::install(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        if let Err(e) = get(&app) {
            tracing::warn!("[jobs] boot: {e}");
        } else {
            crate::missions::boot(&app);
        }
    });
}

fn row_to_job(r: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    Ok(Job {
        id: r.get(0)?,
        kind: r.get(1)?,
        agent_id: r.get(2)?,
        conversation_id: r.get(3)?,
        prompt: r.get(4)?,
        workspace: r.get(5)?,
        state: r.get(6)?,
        loop_id: r.get(7)?,
        trigger_id: r.get(8)?,
        request_id: r.get(9)?,
        created_ms: r.get::<_, i64>(10)? as u64,
        started_ms: r.get::<_, Option<i64>>(11)?.map(|v| v as u64),
        finished_ms: r.get::<_, Option<i64>>(12)?.map(|v| v as u64),
        result: r.get(13)?,
        error: r.get(14)?,
        log: r.get(15)?,
        usage: r
            .get::<_, Option<String>>(16)?
            .and_then(|s| serde_json::from_str(&s).ok()),
        state_seq: r.get::<_, Option<i64>>(17)?.unwrap_or(0),
        seen_ms: r.get(18)?,
        failure_reason: r.get(19)?,
        deadline_min: r.get::<_, Option<i64>>(20)?.map(|v| v as u32),
        deadline_ms: r.get::<_, Option<i64>>(21)?.map(|v| v as u64),
        wait_reason: r.get(22)?,
        diff_files: r
            .get::<_, Option<String>>(23)?
            .and_then(|s| serde_json::from_str(&s).ok()),
        additions: r.get::<_, Option<i64>>(24)?.map(|v| v as u64),
        deletions: r.get::<_, Option<i64>>(25)?.map(|v| v as u64),
        diff_clipped: r.get::<_, Option<i64>>(26)?.unwrap_or(0) != 0,
        diff_unavailable: r.get(27)?,
        spec: r
            .get::<_, Option<String>>(28)?
            .and_then(|s| serde_json::from_str(&s).ok()),
        playbook_id: r.get(29)?,
        isolation: r
            .get::<_, Option<String>>(30)?
            .and_then(|s| Isolation::parse(&s).ok())
            .unwrap_or_default(),
        base_ref: r.get(31)?,
        worktree_path: r.get(32)?,
        branch: r.get(33)?,
        base_oid: r.get(34)?,
        head_oid: r.get(35)?,
        proof: r
            .get::<_, Option<String>>(36)?
            .and_then(|s| serde_json::from_str(&s).ok()),
        title: r.get(37)?,
        owner: r.get(38)?,
        run_mode: r
            .get::<_, Option<String>>(39)?
            .and_then(|s| RunMode::parse(&s).ok())
            .unwrap_or_default(),
        outcome: r
            .get::<_, Option<String>>(40)?
            .and_then(|s| serde_json::from_str(&s).ok()),
        origin: r
            .get::<_, Option<String>>(41)?
            .and_then(|s| serde_json::from_str(&s).ok()),
        session_id: r.get(42)?,
        pid: r
            .get::<_, Option<i64>>(43)?
            .and_then(|v| u32::try_from(v).ok()),
        pid_start: r.get::<_, Option<i64>>(44)?.map(|v| v as u64),
        attempt: r.get::<_, Option<i64>>(45)?.unwrap_or(0).max(0) as u32,
        auto_resume: r.get::<_, Option<i64>>(46)?.map(|v| v != 0),
        side_effects: r.get::<_, Option<i64>>(47)?.unwrap_or(0) != 0,
    })
}

const JOB_COLS: &str = "id, kind, agent_id, conversation_id, prompt, workspace, state, loop_id, trigger_id, request_id, created_ms, started_ms, finished_ms, result, error, log, usage, state_seq, seen_ms, failure_reason, deadline_min, deadline_ms, wait_reason, diff_files, additions, deletions, diff_clipped, diff_unavailable, spec, playbook_id, isolation, base_ref, worktree_path, branch, base_oid, head_oid, proof, title, owner, run_mode, outcome, origin, session_id, pid, pid_start, attempt, auto_resume, side_effects";

impl Jobs {
    pub(crate) fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn token_lookup(&self, token: &str) -> crate::bridge_tokens::Lookup {
        crate::bridge_tokens::lookup(&self.db(), token, now_ms())
    }

    fn job_token_lease(
        self: &Arc<Self>,
        job: &Job,
        conversation: &str,
    ) -> Option<crate::bridge_tokens::JobTokenLease> {
        let (minter, revoker) = (self.clone(), self.clone());
        let (id_m, id_r) = (job.id.clone(), job.id.clone());
        let expires = crate::bridge_tokens::job_token_expiry(job.deadline_ms, now_ms());
        let port = crate::storage::config::load_settings(&self.app).bridge.port;
        let bridge = (port > 0).then(|| format!("http://127.0.0.1:{port}"));
        let (lease, error) = crate::bridge_tokens::JobTokenLease::start_with(
            conversation,
            crate::bridge_tokens::job_identity_env(&job.id, &job.agent_id, bridge.as_deref()),
            move || {
                crate::bridge_tokens::mint(
                    &minter.db(),
                    crate::bridge_tokens::Kind::Job,
                    &format!("job {id_m}"),
                    &crate::bridge_tokens::job_scopes(),
                    Some(&id_m),
                    Some(expires),
                    now_ms(),
                )
            },
            move || {
                if let Err(e) = crate::bridge_tokens::revoke_job(&revoker.db(), &id_r, now_ms()) {
                    tracing::warn!("[jobs] revoke the token of {id_r}: {e}");
                }
            },
        );
        if let Some(e) = error {
            tracing::warn!("[jobs] token for {}: {e}", job.id);
        }
        Some(lease)
    }

    fn save_job_checked(&self, job: &Job) -> Result<(), String> {
        let state_seq = {
            let db = self.db();
            let stored: Option<(String, i64, Option<i64>)> = db
                .query_row(
                    "SELECT state, state_seq, seen_ms FROM jobs WHERE id=?1",
                    params![job.id],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                            r.get(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| "JOB_STORAGE_UNAVAILABLE".to_owned())?;
            let seq = next_state_seq(
                stored.as_ref().map(|(st, seq, _)| (st.as_str(), *seq)),
                &job.state,
            );
            let seen = stored.and_then(|(_, _, seen)| seen).or(job.seen_ms);
            let marks = (1..=JOB_COLS.split(',').count())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",");
            db.execute(
                &format!("INSERT OR REPLACE INTO jobs ({JOB_COLS}) VALUES ({marks})"),
                params![
                    job.id,
                    job.kind,
                    job.agent_id,
                    job.conversation_id,
                    job.prompt,
                    job.workspace,
                    job.state,
                    job.loop_id,
                    job.trigger_id,
                    job.request_id,
                    job.created_ms as i64,
                    job.started_ms.map(|v| v as i64),
                    job.finished_ms.map(|v| v as i64),
                    job.result,
                    job.error,
                    job.log,
                    job.usage.as_ref().map(|u| u.to_string()),
                    seq,
                    seen,
                    job.failure_reason,
                    job.deadline_min.map(i64::from),
                    job.deadline_ms.map(|v| v as i64),
                    job.wait_reason,
                    job.diff_files
                        .as_ref()
                        .and_then(|f| serde_json::to_string(f).ok()),
                    job.additions.map(|v| v as i64),
                    job.deletions.map(|v| v as i64),
                    i64::from(job.diff_clipped),
                    job.diff_unavailable,
                    job.spec.as_ref().map(|u| u.to_string()),
                    job.playbook_id,
                    job.isolation.as_str(),
                    job.base_ref,
                    job.worktree_path,
                    job.branch,
                    job.base_oid,
                    job.head_oid,
                    job.proof.as_ref().map(|p| p.to_string()),
                    job.title,
                    job.owner,
                    job.run_mode.as_str(),
                    job.outcome.as_ref().map(|o| o.to_string()),
                    job.origin.as_ref().map(|o| o.to_string()),
                    job.session_id,
                    job.pid.map(i64::from),
                    job.pid_start.map(|v| v as i64),
                    i64::from(job.attempt),
                    job.auto_resume.map(i64::from),
                    i64::from(job.side_effects)
                ],
            )
            .map_err(|_| "JOB_STORAGE_UNAVAILABLE".to_owned())?;
            (seq, seen)
        };
        let mut light = job.clone();
        light.state_seq = state_seq.0;
        light.seen_ms = state_seq.1;
        light.log = clip(&light.log, 4096);
        light.diff_files = None;
        light.proof = None;
        let _ = self.app.emit(EVENT_JOB, &light);
        self.changes.send_modify(|n| *n = n.wrapping_add(1));
        Ok(())
    }

    fn save_job(&self, job: &Job) {
        if let Err(error) = self.save_job_checked(job) {
            tracing::warn!("[jobs] save {}: {error}", job.id);
        }
    }

    pub fn job(&self, id: &str) -> Option<Job> {
        self.db()
            .query_row(
                &format!("SELECT {JOB_COLS} FROM jobs WHERE id=?1"),
                params![id],
                row_to_job,
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn list(&self, limit: u32) -> Vec<Job> {
        let db = self.db();
        let Ok(mut stmt) = db.prepare(&format!(
            "SELECT {JOB_COLS} FROM jobs ORDER BY created_ms DESC LIMIT ?1"
        )) else {
            return Vec::new();
        };
        let rows = stmt.query_map(params![limit.clamp(1, 500)], row_to_job);
        rows.map(|r| {
            r.filter_map(Result::ok)
                .map(|mut j| {
                    j.log = String::new();
                    j.diff_files = None;
                    j.proof = None;
                    j
                })
                .collect()
        })
        .unwrap_or_default()
    }

    fn new_job(
        &self,
        kind: &str,
        agent_id: &str,
        prompt: &str,
        workspace: Option<String>,
        conversation_id: Option<String>,
    ) -> Result<Job, String> {
        if let Some(tool) = agent_id.strip_prefix(TOOL_PREFIX) {
            omniget_core::core::agentkit_run::runner::runner_of(tool)?;
        } else if self.llm.agent(agent_id).is_none() {
            let ids: Vec<String> = self.llm.roster().into_iter().map(|a| a.id).collect();
            return Err(format!(
                "{ERR_JOBS}: no agent `{agent_id}` (roster: {})",
                ids.join(", ")
            ));
        }
        if prompt.trim().is_empty() {
            return Err(format!("{ERR_JOBS}: empty prompt"));
        }
        let id = short_id("j");
        let run_mode = match &conversation_id {
            Some(conv) => omniget_core::core::llm::run_mode::chosen(&sanitize_id(conv))
                .unwrap_or(RunMode::AutoEdits),
            None => RunMode::default_for(kind),
        };
        Ok(Job {
            conversation_id: conversation_id.unwrap_or_else(|| format!("job-{id}")),
            id,
            kind: kind.to_string(),
            agent_id: agent_id.to_string(),
            prompt: prompt.to_string(),
            workspace,
            state: "queued".into(),
            created_ms: now_ms(),
            isolation: Isolation::default_for(kind, agent_id),
            run_mode,
            ..Default::default()
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn submit_with_deadline(
        self: &Arc<Self>,
        kind: &str,
        agent_id: &str,
        prompt: &str,
        workspace: Option<String>,
        conversation_id: Option<String>,
        trigger_id: Option<String>,
        deadline_min: Option<u32>,
    ) -> Result<Job, String> {
        let mut job = self.new_job(kind, agent_id, prompt, workspace, conversation_id)?;
        job.trigger_id = trigger_id;
        job.deadline_min = deadline_min.map(|m| m.clamp(1, 24 * 60));
        self.save_job(&job);
        let this = self.clone();
        let id = job.id.clone();
        tauri::async_runtime::spawn(async move {
            this.run_job(&id, None).await;
        });
        Ok(job)
    }

    pub fn prepare(
        &self,
        kind: &str,
        agent_id: &str,
        prompt: &str,
        workspace: Option<String>,
        conversation_id: Option<String>,
    ) -> Result<Job, String> {
        let job = self.new_job(kind, agent_id, prompt, workspace, conversation_id)?;
        self.save_job_checked(&job)?;
        Ok(job)
    }

    pub async fn run_prepared_guarded(self: &Arc<Self>, id: &str, guard: JobGuard) -> Option<Job> {
        self.guards
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string(), guard);
        let out = self.run_job(id, None).await;
        self.guards
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
        out
    }

    pub fn submit_spec(
        self: &Arc<Self>,
        kind: &str,
        agent_id: &str,
        prompt: &str,
        workspace: Option<String>,
        spec: Option<Value>,
    ) -> Result<Job, String> {
        let mut job = self.new_job(kind, agent_id, prompt, workspace, None)?;
        job.spec = spec;
        self.save_job(&job);
        let this = self.clone();
        let id = job.id.clone();
        tauri::async_runtime::spawn(async move {
            this.run_job(&id, None).await;
        });
        Ok(job)
    }

    pub fn submit_command(
        self: &Arc<Self>,
        s: &crate::job_commands::StartSpec,
        owner: &str,
    ) -> Result<Job, String> {
        self.start_spec(s, Some(owner), RunMode::ApprovalRequired)
    }

    fn start_spec(
        self: &Arc<Self>,
        s: &crate::job_commands::StartSpec,
        owner: Option<&str>,
        default_mode: RunMode,
    ) -> Result<Job, String> {
        let earlier = match &s.resume_of {
            Some(id) => {
                let e = self
                    .job(id)
                    .ok_or_else(|| format!("{ERR_JOBS}: no job {id} to resume"))?;
                if is_active(&e.state) {
                    return Err(format!("{ERR_JOBS}: job {id} is still {}", e.state));
                }
                Some(e)
            }
            None => None,
        };
        let workspace = s
            .workspace
            .clone()
            .or_else(|| earlier.as_ref().and_then(|e| e.workspace.clone()));
        let mut job = self.new_job(
            "run",
            &s.agent_id,
            &s.prompt,
            workspace,
            earlier.as_ref().map(|e| e.conversation_id.clone()),
        )?;
        job.deadline_min = s.deadline_min.map(|m| m.clamp(1, 24 * 60));
        if let Some(i) = &s.isolation {
            job.isolation = Isolation::parse(i)?;
        }
        job.base_ref = s
            .base
            .as_ref()
            .map(|b| b.trim().to_string())
            .filter(|b| !b.is_empty());
        if job.base_ref.is_some() && job.isolation == Isolation::Shared {
            return Err(format!(
                "{ERR_JOBS}: a base ref needs a worktree (isolation worktree or auto)"
            ));
        }
        job.title = s.title.clone();
        job.owner = owner.map(str::to_string);
        job.run_mode = match s.run_mode.as_deref() {
            Some(m) => RunMode::parse(m)?,
            None => earlier.as_ref().map(|e| e.run_mode).unwrap_or(default_mode),
        };
        job.auto_resume = s.auto_resume;
        job.origin = Some(json!({
            "command_id": s.command_id,
            "purpose": s.purpose,
            "model": s.model,
            "effort": s.effort,
            "budget_usd": s.budget_usd,
            "resume_of": s.resume_of,
            "workflow": s.applied_workflow,
        }));
        if let Some(model) = s.model.as_deref() {
            let agent = self
                .llm
                .agent(&s.agent_id)
                .ok_or_else(|| format!("{ERR_JOBS}: no agent `{}`", s.agent_id))?;
            let omniget_core::core::llm::agent::ModelPolicy::Fixed { model: current } = agent.model
            else {
                return Err(format!(
                    "{ERR_JOBS}: agent `{}` routes between models; pick one in the app instead of `model`",
                    s.agent_id
                ));
            };
            self.llm.switch_model(
                &job.conversation_id,
                omniget_core::core::llm::types::ModelRef {
                    provider: current.provider,
                    model: model.to_string(),
                },
            )?;
        }
        self.save_job_checked(&job)?;
        if let Some(usd) = s.budget_usd.filter(|b| *b > 0.0) {
            budget_policy(
                omniget_core::core::llm::budget::policy::Scope::Job(job.id.clone()),
                omniget_core::core::llm::budget::policy::Metric::Usd,
                usd,
            );
        }
        let this = self.clone();
        let id = job.id.clone();
        tauri::async_runtime::spawn(async move {
            this.run_job(&id, None).await;
        });
        Ok(job)
    }

    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub fn cancel(&self, id: &str) -> Result<Job, String> {
        self.cancel_by(id, omniget_core::core::llm::audit::Actor::system("system"))
    }

    pub fn cancel_by(
        &self,
        id: &str,
        actor: omniget_core::core::llm::audit::Actor,
    ) -> Result<Job, String> {
        let mut job = self
            .job(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no job {id}"))?;
        if matches!(
            job.state.as_str(),
            "queued" | "waiting_workspace" | "running" | "waiting_approval"
        ) {
            crate::llm_audit::job_cancelled(actor, &job);
        }
        if matches!(job.state.as_str(), "queued" | "waiting_workspace") {
            job.state = "cancelled".into();
            job.finished_ms = Some(now_ms());
            self.save_job_checked(&job)?;
        }
        if let Some(token) = self
            .cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
        {
            token.cancel();
        }
        if let Some(request) = job.request_id.as_deref() {
            let _ = self.llm.cancel(request);
        }
        Ok(job)
    }

    fn diff_folder(&self, job: &Job) -> Option<std::path::PathBuf> {
        code_tools::workspace_of(&sanitize_id(&job.conversation_id)).or_else(|| {
            effective_workspace(job).map(|w| std::fs::canonicalize(&w).unwrap_or_else(|_| w.into()))
        })
    }

    async fn compute_diff(
        &self,
        job: &mut Job,
        requests: &[String],
    ) -> (Option<String>, Option<(String, String)>) {
        use omniget_core::core::llm::snapshot;
        let out = match (job.worktree_path.is_some(), job.base_oid.clone()) {
            (true, Some(base)) => match worktree_root(job).await {
                Some((_, root)) => snapshot::worktree_diff(&root, &base).await,
                None => Err(format!(
                    "{}: the job's repository is gone",
                    snapshot::ERR_DIFF_UNAVAILABLE
                )),
            },
            _ => {
                let folder = self.diff_folder(job);
                let out = match &folder {
                    Some(ws) => {
                        snapshot::job_diff(ws, &sanitize_id(&job.conversation_id), requests).await
                    }
                    None => Err(format!("{NO_WORKSPACE}: the job ran without a folder")),
                };
                match out {
                    Err(e) => {
                        let rows = self.changes(&job.id);
                        if rows.is_empty() {
                            Err(e)
                        } else {
                            Ok(changes::diff_from_baselines(folder.as_deref(), &rows))
                        }
                    }
                    ok => ok,
                }
            }
        };
        let ends = out
            .as_ref()
            .ok()
            .and_then(|d| Some((d.base.clone()?, d.tip.clone()?)));
        (apply_diff(job, out), ends)
    }

    async fn record_diff(&self, job: &mut Job, requests: &[String]) -> Option<(String, String)> {
        let (patch, ends) = self.compute_diff(job, requests).await;
        let Some(path) = patch_path(&job.id) else {
            return ends;
        };
        match patch.filter(|p| !p.is_empty()) {
            Some(text) => {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Err(e) = std::fs::write(&path, text) {
                    tracing::warn!("[jobs] patch of {}: {e}", job.id);
                }
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
        ends
    }

    fn store_diff_columns(&self, job: &Job) {
        let _ = self.db().execute(
            "UPDATE jobs SET diff_files=?2, additions=?3, deletions=?4, diff_clipped=?5, diff_unavailable=?6, head_oid=?7 WHERE id=?1",
            params![
                job.id,
                job.diff_files.as_ref().and_then(|f| serde_json::to_string(f).ok()),
                job.additions.map(|v| v as i64),
                job.deletions.map(|v| v as i64),
                i64::from(job.diff_clipped),
                job.diff_unavailable,
                job.head_oid
            ],
        );
    }

    pub async fn diff(&self, id: &str) -> Result<(Job, Option<String>, bool), String> {
        let mut job = self
            .job(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no job {id}"))?;
        let active = matches!(
            job.state.as_str(),
            "queued" | "waiting_workspace" | "running" | "waiting_approval"
        );
        let requests: Vec<String> = job.request_id.clone().into_iter().collect();
        if active {
            let (patch, _) = self.compute_diff(&mut job, &requests).await;
            return Ok((job, patch, false));
        }
        if job.diff_files.is_none() && job.diff_unavailable.is_none() {
            self.record_diff(&mut job, &requests).await;
            self.store_diff_columns(&job);
        }
        let patch = job.diff_files.as_ref().and_then(|files| {
            match patch_path(&job.id).and_then(|p| std::fs::read_to_string(p).ok()) {
                Some(text) => Some(text),
                None => files.is_empty().then(String::new),
            }
        });
        Ok((job, patch, true))
    }

    async fn run_job(self: &Arc<Self>, id: &str, resume_note: Option<String>) -> Option<Job> {
        let queued = self.job(id)?;
        if queued.state == "cancelled" {
            return Some(queued);
        }
        let earlier_request = queued.request_id.clone();
        let cancel = CancellationToken::new();
        self.cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string(), cancel.clone());
        let _unregister = Unregister { jobs: self, id };
        let repo = match queued
            .workspace
            .as_deref()
            .filter(|w| !w.trim().is_empty() && queued.worktree_path.is_none())
        {
            Some(ws) if queued.isolation != Isolation::Shared => {
                omniget_core::core::vcs::job_worktree::repo_root(std::path::Path::new(ws)).await
            }
            _ => None,
        };
        let place = match placement(&queued, repo) {
            Ok(p) => p,
            Err(e) => return self.fail_before_start(id, e),
        };
        let ticket = omniget_core::core::llm::job_gate::Ticket {
            job_id: id.to_string(),
            agent_id: queued.agent_id.clone(),
            class: if queued.kind == "mission" {
                omniget_core::core::llm::job_gate::Class::Mission
            } else {
                omniget_core::core::llm::job_gate::Class::Manual
            },
            folder: if place.force {
                None
            } else {
                self.folder_of(&queued)
            },
        };
        let (lease, isolated) = tokio::select! {
            biased;
            _ = cancel.cancelled() => return self.job(id),
            placed = self.gate.acquire_placed(ticket, place.if_busy) => placed,
        };
        if !lease.delay.is_zero() {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return self.job(id),
                _ = tokio::time::sleep(lease.delay) => {}
            }
        }
        let _lease = lease;
        let _power_hold = omniget_core::core::washmybelly::power::auto_hold_begin("agent job");
        let mut job = self.job(id)?;
        if job.state == "cancelled" {
            return Some(job);
        }
        job.wait_reason = None;
        if let Err(e) = self.enter_worktree(&mut job, &place, isolated).await {
            job.state = "failed".into();
            job.error = Some(e);
            job.finished_ms = Some(now_ms());
            self.save_job_checked(&job).ok()?;
            return Some(job);
        }
        let conv = sanitize_id(&job.conversation_id);
        if let Some(ws) = effective_workspace(&job) {
            if let Err(e) = code_tools::set_conversation_workspace(&conv, Some(ws.into())) {
                job.state = "failed".into();
                job.error = Some(e);
                job.finished_ms = Some(now_ms());
                self.save_job_checked(&job).ok()?;
                return Some(job);
            }
        }
        if self
            .job(id)
            .is_none_or(|current| current.state == "cancelled")
        {
            cancel.cancel();
            self.cancels
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&job.id);
            return self.job(id);
        }
        if let Some(block) = self.budget_block(&job) {
            job.state = "failed".into();
            job.failure_reason = Some("budget".into());
            job.error = Some(format!("{}: {}", block.error().code, block.message));
            job.finished_ms = Some(now_ms());
            self.save_job_checked(&job).ok()?;
            return Some(job);
        }
        job.state = "running".into();
        job.started_ms = Some(now_ms());
        job.pid = None;
        job.pid_start = None;
        let guard = self
            .guards
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned()
            .or_else(|| {
                let min = job.deadline_min.unwrap_or_else(|| {
                    crate::commands::llm::watchdog::limits(&self.app).job_deadline_min
                });
                default_deadline_ms(&job.kind, job.started_ms.unwrap_or_default(), min).map(
                    |deadline| JobGuard {
                        deadline_ms: Some(deadline),
                        check: None,
                        deadline_reason: format!("the job ran past its {min} min deadline"),
                    },
                )
            });
        let guard = with_budget(
            guard,
            job.origin.as_ref().and_then(|o| o["budget_usd"].as_f64()),
        );
        job.deadline_ms = guard.as_ref().and_then(|g| g.deadline_ms);
        if self.save_job_checked(&job).is_err() {
            cancel.cancel();
            self.cancels
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&job.id);
            return None;
        }
        let input = resume_note.unwrap_or_else(|| job.prompt.clone());
        if job.agent_id.starts_with(TOOL_PREFIX) {
            return Some(self.run_tool_job(job, input).await);
        }
        let input = if asks_outcome(&job.kind) {
            format!("{input}{OUTCOME_SUFFIX}")
        } else {
            input
        };
        let _job_token = self.job_token_lease(&job, &conv);
        let _run_mode = omniget_core::core::llm::run_mode::Binding::new(&conv, job.run_mode);
        let _budget_scope =
            omniget_core::core::llm::budget::policy::Binding::new(&conv, work_scopes(&job));
        let (request_id, cancel, mut stream) = match self
            .llm
            .turn_stream_with_cancel(&conv, &job.agent_id, &input, cancel.clone())
            .await
        {
            Ok(v) => v,
            Err(e) => {
                self.cancels
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&job.id);
                job.state = if cancel.is_cancelled() {
                    "cancelled"
                } else {
                    "failed"
                }
                .into();
                job.error = Some(e);
                job.finished_ms = Some(now_ms());
                self.save_job_checked(&job).ok()?;
                return Some(job);
            }
        };
        job.request_id = Some(request_id.clone());
        self.cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(job.id.clone(), cancel.clone());
        let (ask_tx, mut asks) = tokio::sync::mpsc::unbounded_channel::<AskSignal>();
        self.watchers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(request_id.clone(), (job.id.clone(), ask_tx));
        self.live.lock().unwrap_or_else(|e| e.into_inner()).insert(
            job.id.clone(),
            Live {
                request_id: request_id.clone(),
                started_ms: job.started_ms.unwrap_or_default(),
                last_event_ms: now_ms(),
                deadline_ms: job.deadline_ms,
            },
        );
        if self.llm.has_pending_ask(&request_id) {
            job.state = "waiting_approval".into();
        }
        let mut storage_failed = self.save_job_checked(&job).is_err();
        if storage_failed {
            cancel.cancel();
        }

        let mut text = String::new();
        let mut saw_finished = false;
        let mut error: Option<String> = None;
        let mut limit_hit: Option<String> = None;
        let mut limit_kind: Option<&'static str> = None;
        let est_input: u64 = 0;
        let check = guard.as_ref().and_then(|g| g.check.clone());
        let spent_so_far =
            |usage: &Option<serde_json::Value>, est_input: u64, text: &str| -> (u64, Option<f64>) {
                let u = usage.clone().unwrap_or_default();
                let reported = billable_of(&u);
                (
                    reported.max(est_input + text.len() as u64 / 4),
                    u["cost_usd"].as_f64(),
                )
            };
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        let mut first_event = true;
        let mut evidence = job
            .loop_id
            .is_some()
            .then(omniget_core::core::llm::goal_audit::Evidence::default);
        loop {
            tokio::select! {
                event = stream.next() => {
                    let Some(event) = event else { break };
                    self.touch(&job.id);
                    if first_event {
                        first_event = false;
                        if self.capture_process(&mut job, &request_id, &conv) && self.save_job_checked(&job).is_err() {
                            storage_failed = true;
                            cancel.cancel();
                        }
                    }
                    self.llm.note_event(&job.agent_id, &event);
                    if let Some(e) = evidence.as_mut() {
                        e.feed(&event);
                    }
                    match event {
                        TurnEvent::Finished { .. } => saw_finished = true,
                        TurnEvent::TextDelta { text: t } => text.push_str(&t),
                        TurnEvent::ToolCallStart { name, .. } => {
                            if tool_may_write(&name) {
                                job.side_effects = true;
                            }
                            job.log.push_str(&format!("→ {name}\n"));
                            job.log = clip(&job.log, LOG_MAX);
                            if self.save_job_checked(&job).is_err() { storage_failed = true; cancel.cancel(); }
                            if let (Some(check), None) = (&check, &limit_hit) {
                                let (tokens, usd) = spent_so_far(&job.usage, est_input, &text);
                                if let Err(why) = check(tokens, usd) {
                                    job.log.push_str(&format!("! {why}\n"));
                                    limit_hit = Some(why);
                                    limit_kind = Some("limit");
                                    cancel.cancel();
                                }
                            }
                        }
                        TurnEvent::Usage { usage } => {
                            let model = self.llm.model_label(&job.agent_id);
                            fold_usage(&mut job.usage, &model, &usage);
                            if let (Some(check), None) = (&check, &limit_hit) {
                                let u = job.usage.clone().unwrap_or_default();
                                let tokens = billable_of(&u);
                                if let Err(why) = check(tokens, u["cost_usd"].as_f64()) {
                                    job.log.push_str(&format!("! {why}\n"));
                                    limit_hit = Some(why);
                                    limit_kind = Some("limit");
                                    cancel.cancel();
                                }
                            }
                        }
                        TurnEvent::Error { error: e } => {
                            job.log.push_str(&format!("! {}: {}\n", e.code, e.message));
                            error = Some(format!("{}: {}", e.code, e.message));
                        }
                        _ => {}
                    }
                }
                Some(signal) = asks.recv() => {
                    self.apply_ask(&mut job, &request_id, signal);
                    if self.save_job_checked(&job).is_err() { storage_failed = true; cancel.cancel(); }
                }
                _ = tick.tick() => {
                    if self.capture_process(&mut job, &request_id, &conv) && self.save_job_checked(&job).is_err() {
                        storage_failed = true;
                        cancel.cancel();
                    }
                    if let (Some(deadline), None) = (guard.as_ref().and_then(|g| g.deadline_ms), &limit_hit) {
                        if now_ms() > deadline {
                            let why = guard.as_ref().map(|g| g.deadline_reason.clone()).filter(|r| !r.is_empty()).unwrap_or_else(|| "the time limit was reached".into());
                            job.log.push_str(&format!("! {why}\n"));
                            limit_hit = Some(why);
                            limit_kind = Some("deadline");
                            cancel.cancel();
                        }
                    }
                    let waiting = self.llm.has_pending_ask(&request_id);
                    let want = if waiting { "waiting_approval" } else { "running" };
                    if job.state != want {
                        tracing::debug!("[jobs] {} reconciled to {want} by the tick", job.id);
                        job.state = want.into();
                        if self.save_job_checked(&job).is_err() { storage_failed = true; cancel.cancel(); }
                    }
                }
            }
        }
        self.llm.finish_turn(&request_id, &job.agent_id);
        if let Some(e) = evidence.filter(|e| !e.is_empty()) {
            self.round_evidence
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(job.id.clone(), e.render());
        }
        self.cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&job.id);
        self.watchers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&request_id);
        self.live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&job.id);

        if storage_failed || !saw_finished {
            cancel.cancel();
            return None;
        }
        job.pid = None;
        job.pid_start = None;
        job.state = if limit_hit.is_some() {
            "failed"
        } else if cancel.is_cancelled() {
            "cancelled"
        } else if error.is_some() && text.trim().is_empty() {
            "failed"
        } else {
            "done"
        }
        .into();
        job.result = Some(clip(&text, RESULT_MAX));
        job.error = limit_hit.or(error);
        if asks_outcome(&job.kind) {
            job.outcome = parse_outcome(&text);
        }
        let requests: Vec<String> = earlier_request
            .into_iter()
            .chain(std::iter::once(request_id.clone()))
            .collect();
        self.commit_worktree(&mut job).await;
        let ends = self.record_diff(&mut job, &requests).await;
        job.state = settle_state(&job.state, job.diff_files.as_deref()).into();
        job.state = settle_outcome(&job.kind, &job.state, job.outcome.is_some()).into();
        job.failure_reason = (job.state == "failed")
            .then(|| {
                limit_kind
                    .or_else(|| job.error.as_deref().and_then(failure_reason_of))
                    .map(String::from)
            })
            .flatten();
        if job.state == "cancelled"
            && self
                .job(&job.id)
                .is_some_and(|row| row.failure_reason.as_deref() == Some("budget"))
        {
            job.failure_reason = Some("budget".into());
        }
        job.finished_ms = Some(now_ms());
        job.proof = Some(build_proof(
            &job,
            ends.as_ref(),
            &omniget_core::core::vcs::job_worktree::host_label(),
        ));
        crate::job_workflow::run_checks(crate::job_workflow::store().as_deref(), &mut job).await;
        if job.worktree_path.is_some()
            && job.diff_unavailable.is_none()
            && job.diff_files.as_ref().is_some_and(|f| f.is_empty())
        {
            match remove_job_worktree(&job).await {
                Ok(out) => {
                    job.log.push_str(&removal_note(&out));
                    job.worktree_path = None;
                }
                Err(e) => job.log.push_str(&format!("! worktree kept: {e}\n")),
            }
        }
        self.save_job_checked(&job).ok()?;
        if job.state != "cancelled" {
            crate::llm_audit::job_ended(&job);
        }
        if let Some(reason) = job
            .failure_reason
            .as_deref()
            .and_then(FailureReason::parse)
            .filter(|r| r.is_infrastructure())
        {
            if job.loop_id.is_none() && self.auto_resume(&mut job, reason, None) {
                return Some(job);
            }
        }
        if let Some(tid) = job.trigger_id.clone() {
            self.notify_routine(&tid, &job);
        } else if job.kind == "run"
            && matches!(
                job.state.as_str(),
                "done" | "review" | "done_unreported" | "failed"
            )
        {
            let ok = job.state != "failed";
            let detail = if ok {
                job.result.clone().unwrap_or_default()
            } else {
                job.error.clone().unwrap_or_default()
            };
            let first = detail.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            self.attention.job_ended(&self.target_of(&job), ok, first);
        }
        Some(job)
    }

    async fn commit_worktree(&self, job: &mut Job) {
        use omniget_core::core::vcs::job_worktree;
        if job.worktree_path.is_none() {
            return;
        }
        let Some((_, root)) = worktree_root(job).await else {
            return;
        };
        let task = job
            .origin
            .as_ref()
            .and_then(|o| o["workflow"]["task"].as_str())
            .unwrap_or(&job.prompt)
            .to_string();
        let first: String = task
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .chars()
            .take(72)
            .collect();
        match job_worktree::save_work(&root, &format!("omniget job {}: {first}", job.id)).await {
            Ok(Some(commit)) => job.log.push_str(&format!(
                "⎇ the job's work is commit {} on {}\n",
                commit.chars().take(7).collect::<String>(),
                job.branch.as_deref().unwrap_or("its branch")
            )),
            Ok(None) => {}
            Err(e) => job.log.push_str(&format!(
                "! the job's work stays uncommitted in its worktree: {e}\n"
            )),
        }
    }

    fn attach_check(&self, job_id: &str, command: &str, ok: bool, out: &str) {
        let Some(mut job) = self.job(job_id) else {
            return;
        };
        push_check(job.proof.get_or_insert_with(|| json!({})), command, ok, out);
        self.save_job(&job);
    }

    fn fail_before_start(&self, id: &str, error: String) -> Option<Job> {
        let mut job = self.job(id)?;
        job.state = "failed".into();
        job.error = Some(error);
        job.finished_ms = Some(now_ms());
        self.save_job_checked(&job).ok()?;
        crate::llm_audit::job_ended(&job);
        Some(job)
    }

    async fn enter_worktree(
        &self,
        job: &mut Job,
        place: &Placement,
        isolated: bool,
    ) -> Result<(), String> {
        use omniget_core::core::vcs::job_worktree;
        if let (Some(_), Some(branch)) = (job.worktree_path.clone(), job.branch.clone()) {
            let (repo, root) = self
                .worktree_root(job)
                .await
                .ok_or_else(|| format!("{ERR_WORKTREE}: the job's repository is gone"))?;
            let recreated = job_worktree::ensure(&repo, &root, &branch)
                .await
                .map_err(|e| format!("{ERR_WORKTREE}: {e}"))?;
            if recreated {
                job.log
                    .push_str(&format!("⎇ worktree recreated from {branch}\n"));
                crate::job_workflow::after_create(
                    crate::job_workflow::store().as_deref(),
                    job,
                    &repo,
                    &root,
                )
                .await?;
            }
            return Ok(());
        }
        if !(place.force || isolated) {
            return Ok(());
        }
        let repo = place
            .repo
            .clone()
            .ok_or_else(|| format!("{ERR_WORKTREE}: no git repository for this job"))?;
        let wt = job_worktree::create(&repo, &job.id, job.base_ref.as_deref())
            .await
            .map_err(|e| format!("{ERR_WORKTREE}: {e}"))?;
        if isolated {
            job.log
                .push_str("⎇ another job is writing in this folder: this one runs in a worktree\n");
        }
        job.log.push_str(&worktree_note(&wt));
        let sub = job
            .workspace
            .as_deref()
            .and_then(|w| std::fs::canonicalize(w).ok())
            .zip(std::fs::canonicalize(&repo).ok())
            .and_then(|(w, r)| w.strip_prefix(&r).ok().map(std::path::Path::to_path_buf))
            .unwrap_or_default();
        job.worktree_path = Some(wt.path.join(sub).to_string_lossy().into_owned());
        job.branch = Some(wt.branch);
        job.base_oid = Some(wt.base_oid);
        self.save_job_checked(job)?;
        let hooked = crate::job_workflow::after_create(
            crate::job_workflow::store().as_deref(),
            job,
            &repo,
            &wt.path,
        )
        .await;
        self.save_job_checked(job)?;
        hooked
    }

    pub async fn remove_worktree(
        &self,
        id: &str,
    ) -> Result<omniget_core::core::vcs::job_worktree::RemoveOutcome, String> {
        let mut job = self
            .job(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no job {id}"))?;
        if is_active(&job.state) {
            return Err(format!(
                "{ERR_JOBS}: job {id} is {}; cancel it before removing its worktree",
                job.state
            ));
        }
        if job.worktree_path.is_none() {
            return Err(format!("{ERR_WORKTREE}: job {id} has no worktree"));
        }
        let out = remove_job_worktree(&job).await?;
        job.log.push_str(&removal_note(&out));
        job.log = clip(&job.log, LOG_MAX);
        job.worktree_path = None;
        self.save_job_checked(&job)?;
        Ok(out)
    }

    async fn gc_worktrees(self: Arc<Self>, boot: bool) {
        use omniget_core::core::vcs::job_worktree;
        if boot {
            job_worktree::empty_trash();
        }
        let now = now_ms();
        let live = self.live_folders("");
        for (id, path) in job_worktree::list_job_worktrees() {
            if omniget_core::core::llm::snapshot::shared_with(&path, &live).is_some() {
                continue;
            }
            let job = self.job(&id);
            let Some(why) = worktree_gc_verdict(job.as_ref(), now) else {
                continue;
            };
            let result = match job {
                Some(j) if j.worktree_path.is_some() => self.remove_worktree(&j.id).await,
                _ => {
                    let branch = job_worktree::branch_of(&id);
                    if job_worktree::branch_at(&path).await.as_deref() != Some(branch.as_str()) {
                        continue;
                    }
                    match job_worktree::main_repo_of(&path).await {
                        Some(repo) => job_worktree::remove(&repo, &path, &branch, None)
                            .await
                            .map_err(|e| e.to_string()),
                        None => continue,
                    }
                }
            };
            match result {
                Ok(out) => tracing::info!(
                    "[jobs] worktree gc ({why}) {}: {}",
                    path.display(),
                    removal_note(&out).trim()
                ),
                Err(e) => tracing::warn!("[jobs] worktree gc ({why}) {}: {e}", path.display()),
            }
        }
    }

    async fn worktree_root(&self, job: &Job) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        worktree_root(job).await
    }

    fn touch(&self, job_id: &str) {
        if let Some(l) = self
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_mut(job_id)
        {
            l.last_event_ms = now_ms();
        }
    }

    fn target_of(&self, job: &Job) -> crate::llm_attention::AttentionTarget {
        crate::llm_attention::AttentionTarget {
            job_id: job.id.clone(),
            conversation_id: job.conversation_id.clone(),
            agent: self
                .llm
                .agent(&job.agent_id)
                .map(|a| a.name)
                .filter(|n| !n.trim().is_empty())
                .unwrap_or_else(|| job.agent_id.clone()),
        }
    }

    fn apply_ask(&self, job: &mut Job, request_id: &str, signal: AskSignal) {
        match signal {
            AskSignal::Opened {
                tool_call_id,
                tool,
                preview,
            } => {
                job.state = "waiting_approval".into();
                let llm = self.llm.clone();
                self.attention.ask_opened(
                    self.target_of(job),
                    tool_call_id,
                    tool,
                    preview,
                    Arc::new(move |call: &str| llm.has_pending_call(call)),
                );
            }
            AskSignal::Resolved {
                tool,
                outcome,
                waited_ms,
            } => {
                if outcome == "expired" {
                    use omniget_core::core::llm::broker::ERR_TOOL_TIMEOUT;
                    job.log.push_str(&format!(
                        "! {ERR_TOOL_TIMEOUT}: nobody answered {tool} in {} s; it did not run\n",
                        waited_ms / 1000
                    ));
                    job.log = clip(&job.log, LOG_MAX);
                    self.attention.ask_expired(
                        &self.target_of(job),
                        &tool,
                        std::time::Duration::from_millis(waited_ms),
                    );
                }
                if job.state == "waiting_approval" && !self.llm.has_pending_ask(request_id) {
                    job.state = "running".into();
                }
            }
        }
    }

    fn handle_bus(&self, event: &omniget_core::core::omni::bus::BusEvent) {
        use omniget_core::core::omni::bus::BusEvent;
        if let BusEvent::BudgetCrossed {
            scope,
            kind,
            incident,
            request,
            ..
        } = event
        {
            if kind == "hard" {
                self.cancel_scope(scope, incident.as_deref(), request);
            }
            return;
        }
        let (request_id, signal) = match event {
            BusEvent::ToolAsk {
                request_id,
                tool_call_id,
                tool,
                preview,
                ..
            } => (
                request_id,
                AskSignal::Opened {
                    tool_call_id: tool_call_id.clone(),
                    tool: tool.clone(),
                    preview: preview.clone(),
                },
            ),
            BusEvent::ToolAskResolved {
                request_id,
                tool,
                outcome,
                waited_ms,
                ..
            } => (
                request_id,
                AskSignal::Resolved {
                    tool: tool.clone(),
                    outcome: outcome.clone(),
                    waited_ms: *waited_ms,
                },
            ),
            _ => return,
        };
        let watcher = self
            .watchers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(request_id)
            .map(|(_, tx)| tx.clone());
        if let Some(tx) = watcher {
            let _ = tx.send(signal);
            return;
        }
        let row = self
            .db()
            .query_row(
                &format!(
                    "SELECT {JOB_COLS} FROM jobs WHERE request_id=?1 AND state IN ('running','waiting_approval') LIMIT 1"
                ),
                params![request_id],
                row_to_job,
            )
            .optional()
            .ok()
            .flatten();
        if let Some(mut job) = row {
            self.apply_ask(&mut job, request_id, signal);
            self.save_job(&job);
            return;
        }
        if let (
            BusEvent::ToolAskResolved { agent, .. },
            AskSignal::Resolved {
                tool,
                outcome,
                waited_ms,
            },
        ) = (event, &signal)
        {
            if outcome == "expired" {
                let target = crate::llm_attention::AttentionTarget {
                    job_id: request_id.clone(),
                    conversation_id: String::new(),
                    agent: self
                        .llm
                        .agent(agent)
                        .map(|a| a.name)
                        .unwrap_or_else(|| agent.clone()),
                };
                self.attention.ask_expired(
                    &target,
                    tool,
                    std::time::Duration::from_millis(*waited_ms),
                );
            }
        }
    }

    fn ask_timeout_for(&self, request_id: &str) -> Option<std::time::Duration> {
        let deadline = self
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .find(|l| l.request_id == request_id)
            .map(|l| l.deadline_ms)?;
        let ask_job = crate::commands::llm::watchdog::limits(&self.app).ask_job;
        job_ask_timeout(now_ms(), deadline, ask_job)
    }

    pub fn job_of_request(&self, request_id: &str) -> Option<String> {
        self.db()
            .query_row(
                "SELECT id FROM jobs WHERE request_id=?1 ORDER BY created_ms DESC LIMIT 1",
                params![request_id],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn explain(&self, id: &str) -> Option<Value> {
        let job = self.job(id)?;
        let live = self
            .live
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned();
        let mut asks = self.llm.pending_ask_list();
        for a in asks.iter_mut() {
            if let (Some(at), Some(req)) = (a["asked_ms"].as_u64(), a["request_id"].as_str()) {
                let wait = self.llm.ask_timeout_for(req).as_millis() as u64;
                a["expires_ms"] = json!(at + wait);
            }
        }
        let pid = job
            .request_id
            .as_deref()
            .and_then(|r| omniget_core::core::assist::runs::active()?.run(r))
            .and_then(|r| r.pid);
        let mut v = explain_view(&job, live.as_ref(), now_ms(), &asks, pid);
        let view = self.gate.snapshot();
        if let Some(q) = view.queued.iter().find(|q| q.job_id == id) {
            let mut queue = json!({
                "position": q.position,
                "of": view.queued.len(),
                "reason": q.reason.code(),
                "detail": wait_detail(&q.reason, &view, &q.agent_id),
            });
            if let WaitReason::Workspace { holder, .. } = &q.reason {
                queue["holder"] = json!(holder);
            }
            v["queue"] = queue;
        }
        Some(v)
    }

    fn folder_of(&self, job: &Job) -> Option<omniget_core::core::llm::folder_key::FolderKey> {
        effective_workspace(job)
            .map(std::path::PathBuf::from)
            .or_else(|| code_tools::workspace_of(&sanitize_id(&job.conversation_id)))
            .map(|p| omniget_core::core::llm::folder_key::FolderKey::of(&p))
    }

    fn report_queue(&self, reported: &mut HashMap<String, (&'static str, String)>) {
        let view = self.gate.snapshot();
        let mut still = std::collections::HashSet::new();
        for q in &view.queued {
            still.insert(q.job_id.clone());
            let want = wait_state(&q.reason);
            if reported.get(&q.job_id) == Some(&want) {
                continue;
            }
            let changed = self
                .db()
                .execute(
                    "UPDATE jobs SET state=?2, wait_reason=?3, \
                     state_seq = state_seq + (CASE WHEN state=?2 THEN 0 ELSE 1 END) \
                     WHERE id=?1 AND state IN ('queued','waiting_workspace') \
                     AND (state<>?2 OR IFNULL(wait_reason,'')<>?3)",
                    params![q.job_id, want.0, want.1],
                )
                .unwrap_or(0);
            if changed > 0 {
                if let Some(mut job) = self.job(&q.job_id) {
                    job.log = clip(&job.log, 4096);
                    let _ = self.app.emit(EVENT_JOB, &job);
                }
            }
            reported.insert(q.job_id.clone(), want);
        }
        reported.retain(|id, _| still.contains(id));
        let _ = self
            .app
            .emit(EVENT_QUEUE, state_view(&view, &|id| self.job(id)));
    }

    pub fn state_view(&self) -> Value {
        let mut v = state_view(&self.gate.snapshot(), &|id| self.job(id));
        let visible = self
            .app
            .get_webview_window("main")
            .and_then(|w| w.is_visible().ok())
            .unwrap_or(false);
        v["app"] = json!({
            "pid": std::process::id(),
            "window_visible": visible,
            "run_in_background": crate::storage::config::load_settings(&self.app).run_in_background,
        });
        v
    }

    pub fn apply_limits(&self, s: &omniget_core::models::settings::LlmSettings) {
        let l = gate_limits(s);
        self.gate.set_limits(l.max_parallel, l.max_per_agent);
    }

    pub fn max_parallel(&self) -> usize {
        self.gate.limits().max_parallel
    }

    pub fn live_folders(&self, except_conversation: &str) -> Vec<std::path::PathBuf> {
        let except = sanitize_id(except_conversation);
        let rows: Vec<(String, Option<String>)> = {
            let db = self.db();
            let Ok(mut stmt) = db.prepare(
                "SELECT conversation_id, COALESCE(worktree_path, workspace) FROM jobs WHERE state IN ('running','waiting_approval')",
            ) else {
                return Vec::new();
            };
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map(|rows| rows.filter_map(Result::ok).collect())
                .unwrap_or_default()
        };
        rows.into_iter()
            .filter(|(conv, _)| sanitize_id(conv) != except)
            .filter_map(|(conv, ws)| {
                ws.filter(|w| !w.trim().is_empty())
                    .map(std::path::PathBuf::from)
                    .or_else(|| code_tools::workspace_of(&sanitize_id(&conv)))
            })
            .collect()
    }

    fn notify_routine(&self, trigger_id: &str, job: &Job) {
        let Some(t) = self.trigger(trigger_id) else {
            return;
        };
        let body = match job.state.as_str() {
            "done" | "review" => job.result.clone().unwrap_or_default(),
            "cancelled" => return,
            _ => format!("error: {}", job.error.clone().unwrap_or_default()),
        };
        let Some(digest) = routine_digest(&t, &body) else {
            return;
        };
        let notified = now_ms();
        self.update_trigger(trigger_id, |t| {
            t.last_digest = Some(digest);
            t.last_notified_ms = Some(notified);
        });
        use tauri_plugin_notification::NotificationExt;
        let text: String = body.trim().chars().take(180).collect();
        let _ = self
            .app
            .notification()
            .builder()
            .title(format!("OmniGet · {}", t.name))
            .body(if text.is_empty() {
                "(empty)".to_string()
            } else {
                text
            })
            .show();
    }

    fn save_loop(&self, l: &LoopDef) {
        if let Ok(body) = serde_json::to_string(l) {
            let _ = self.db().execute(
                "INSERT OR REPLACE INTO loops (id, body) VALUES (?1, ?2)",
                params![l.id, body],
            );
        }
        let _ = self.app.emit(EVENT_LOOP, l);
    }

    pub fn loop_get(&self, id: &str) -> Option<LoopDef> {
        self.db()
            .query_row("SELECT body FROM loops WHERE id=?1", params![id], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .ok()
            .flatten()
            .and_then(|b| serde_json::from_str(&b).ok())
    }

    pub fn loops(&self) -> Vec<LoopDef> {
        let db = self.db();
        let Ok(mut stmt) = db.prepare("SELECT body FROM loops") else {
            return Vec::new();
        };
        let mut out: Vec<LoopDef> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map(|rows| {
                rows.filter_map(Result::ok)
                    .filter_map(|b| serde_json::from_str(&b).ok())
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by_key(|j| std::cmp::Reverse(j.created_ms));
        out
    }

    pub fn loop_create(self: &Arc<Self>, mut def: LoopDef) -> Result<LoopDef, String> {
        if let Some(tool) = def.agent_id.strip_prefix(TOOL_PREFIX) {
            omniget_core::core::agentkit_run::runner::runner_of(tool)?;
        } else if self.llm.agent(&def.agent_id).is_none() {
            return Err(format!("{ERR_JOBS}: no agent `{}`", def.agent_id));
        }
        if def.prompt.trim().is_empty() {
            return Err(format!("{ERR_JOBS}: empty prompt"));
        }
        if def.max_rounds.is_none()
            && def.max_minutes.is_none()
            && def.check_command.as_deref().unwrap_or("").trim().is_empty()
        {
            def.max_rounds = Some(10);
        }
        def.id = short_id("l");
        if def.name.trim().is_empty() {
            def.name = def.prompt.chars().take(48).collect();
        }
        def.conversation_id = format!("loop-{}", def.id);
        def.state = "running".into();
        if let Some(s) = def.schedule.as_deref() {
            if next_round_at(s, now_ms()).is_none() {
                return Err(format!(
                    "{ERR_JOBS}: bad schedule `{s}` (an interval like 10m, 2h, daily, or a 5-field cron line)"
                ));
            }
        }
        def.rounds_done = 0;
        def.spent_usd = 0.0;
        def.restarted = false;
        def.auto_restarts = 0;
        def.tokens_used = 0;
        def.audit_usd = 0.0;
        def.blocked_streak = 0;
        def.audit_fail_streak = 0;
        def.disagree_streak = 0;
        def.note = None;
        def.audits.clear();
        def.rounds_guided = 0;
        def.active_ms = 0;
        def.next_round_ms = None;
        def.created_ms = now_ms();
        def.finished_ms = None;
        def.spent_tokens = 0;
        def.spent_usd_estimated = 0.0;
        def.unpriced_rounds = 0;
        self.save_loop(&def);
        {
            use omniget_core::core::llm::budget::policy::{Metric, Scope};
            if let Some(max) = def.max_cost_usd.filter(|m| *m > 0.0) {
                budget_policy(Scope::Loop(def.id.clone()), Metric::Usd, max);
            }
            if let Some(tokens) = def.token_budget.filter(|t| *t > 0) {
                budget_policy(
                    Scope::Loop(def.id.clone()),
                    Metric::WeightedTokens,
                    tokens as f64,
                );
            }
        }
        let this = self.clone();
        let id = def.id.clone();
        tauri::async_runtime::spawn(async move { this.run_loop(id).await });
        Ok(def)
    }

    async fn run_check(&self, l: &LoopDef) -> Option<(bool, String)> {
        let command = l.check_command.clone().filter(|c| !c.trim().is_empty())?;
        let conv = sanitize_id(&l.conversation_id);
        let out = code_tools::scope(
            &conv,
            &l.agent_id,
            "loop-check",
            code_tools::shell_exec(
                json!({ "command": command, "description": "loop check", "timeout_ms": 600_000 }),
            ),
        )
        .await;
        Some(match out {
            Ok(v) => {
                let code = v["exit_code"].as_i64();
                let tail = format!(
                    "{}{}",
                    v["stdout"].as_str().unwrap_or(""),
                    v["stderr"].as_str().unwrap_or("")
                );
                (
                    code == Some(0),
                    format!(
                        "exit {}\n{}",
                        code.map(|c| c.to_string()).unwrap_or_else(|| "?".into()),
                        clip(&tail, 6000)
                    ),
                )
            }
            Err(e) => (false, e),
        })
    }

    async fn run_loop(self: Arc<Self>, id: String) {
        let Some(mut l) = self.loop_get(&id) else {
            return;
        };
        let _power_hold = omniget_core::core::washmybelly::power::auto_hold_begin("agent loop");
        let deadline = if l.schedule.is_some() {
            None
        } else {
            l.max_minutes.map(|m| l.created_ms + u64::from(m) * 60_000)
        };
        if let Some(ws) = l.workspace.clone().filter(|w| !w.is_empty()) {
            let _ = code_tools::set_conversation_workspace(
                &sanitize_id(&l.conversation_id),
                Some(ws.into()),
            );
        }
        let audited = !l.agent_id.starts_with(TOOL_PREFIX)
            && l.check_command
                .as_deref()
                .is_none_or(|c| c.trim().is_empty());
        let max_rounds = omniget_core::core::llm::goal_audit::effective_max(l.max_rounds, audited);
        let mut streaks = omniget_core::core::llm::goal_audit::Streaks {
            blocked: l.blocked_streak,
            failed: l.audit_fail_streak,
            disagree: l.disagree_streak,
        };
        let mut last_audit = loop_audit::last_audit(&l);
        if l.rounds_done == 0 {
            if let Some((true, out)) = self.run_check(&l).await {
                l.last_check = Some(out);
                l.state = "done".into();
                l.stop_reason = Some("check_passed_before_start".into());
                l.finished_ms = Some(now_ms());
                self.save_loop(&l);
                return;
            }
        }
        loop {
            match self.loop_get(&id) {
                Some(cur) if cur.state == "running" => {}
                _ => return,
            }
            if max_rounds.map(|m| l.rounds_done >= m).unwrap_or(false) {
                l.stop_reason = Some("max_rounds".into());
                if audited {
                    l.state = "blocked".into();
                }
                break;
            }
            if l.token_budget.is_some_and(|b| l.tokens_used >= b)
                && omniget_core::core::llm::budget::policy::active().is_none()
            {
                l.state = "budget_limited".into();
                l.stop_reason = Some(omniget_core::core::llm::goal_audit::STOP_TOKEN_BUDGET.into());
                break;
            }
            if deadline.map(|d| now_ms() >= d).unwrap_or(false) {
                l.stop_reason = Some("max_minutes".into());
                break;
            }
            if l.schedule.is_some()
                && l.max_minutes
                    .map(|m| l.active_ms >= u64::from(m) * 60_000)
                    .unwrap_or(false)
            {
                l.stop_reason = Some("max_minutes".into());
                break;
            }
            if let Some(block) = self.loop_budget_block(&l) {
                l.state = "paused".into();
                l.stop_reason = Some(format!(
                    "budget:{}",
                    block.incident.clone().unwrap_or_default()
                ));
                self.save_loop(&l);
                return;
            }
            if l.max_cost_usd.map(|m| l.spent_usd >= m).unwrap_or(false) {
                l.stop_reason = Some("max_cost".into());
                break;
            }
            if let Some(sched) = l.schedule.clone() {
                if l.rounds_done > 0 || l.next_round_ms.is_some() {
                    let at = l
                        .next_round_ms
                        .or_else(|| next_round_at(&sched, now_ms()))
                        .unwrap_or_else(now_ms);
                    l.next_round_ms = Some(at);
                    self.save_loop(&l);
                    if !self.wait_until(&id, at).await {
                        return;
                    }
                    match self.loop_get(&id) {
                        Some(cur) if cur.state == "running" => {}
                        _ => return,
                    }
                }
            }
            let round = l.rounds_done + 1;
            let resumed = round > 1 && self.loop_resumable(&l);
            let (prompt, guided) = omniget_core::core::llm::goal_audit::round_prompt(
                &omniget_core::core::llm::goal_audit::Round {
                    round,
                    max: max_rounds,
                    prompt: &l.prompt,
                    check_command: l.check_command.as_deref(),
                    last_check: l.last_check.as_deref(),
                    last_audit: last_audit.as_ref(),
                    audited,
                    resumed,
                },
            );
            if guided {
                l.rounds_guided += 1;
            }
            let prompt = if l.restarted {
                format!("This Loop was interrupted (the app restarted or the agent went silent) and the round in flight was stopped. Check what is already done before changing anything, and do not repeat an action that already happened.\n\n{prompt}")
            } else {
                prompt
            };
            let mut job = match self.new_job(
                "loop",
                &l.agent_id,
                &prompt,
                l.workspace.clone(),
                Some(l.conversation_id.clone()),
            ) {
                Ok(j) => j,
                Err(e) => {
                    l.stop_reason = Some(e);
                    l.state = "failed".into();
                    break;
                }
            };
            job.loop_id = Some(l.id.clone());
            job.run_mode = l.run_mode.unwrap_or_default();
            job.deadline_min = l.deadline_min;
            job.spec = l.spec.clone();
            self.save_job(&job);
            if l.restarted {
                l.restarted = false;
                self.save_loop(&l);
            }
            let round_started = now_ms();
            let done = self.run_job(&job.id, None).await;
            let evidence = self
                .round_evidence
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&job.id)
                .unwrap_or_default();
            l.rounds_done = round;
            l.active_ms += now_ms().saturating_sub(round_started);
            l.next_round_ms = l
                .schedule
                .as_deref()
                .and_then(|s| next_round_at(s, now_ms()));
            let Some(done) = done else { break };
            let round_cost = done.usage.as_ref().and_then(|u| u["cost_usd"].as_f64());
            self.note_loop_cost(&mut l, &done);
            if matches!(done.state.as_str(), "cancelled" | "failed")
                && self.loop_budget_block(&l).is_some()
            {
                self.save_loop(&l);
                continue;
            }
            let round_tokens = done.usage.as_ref().map(billable_of).unwrap_or(0);
            l.tokens_used += round_tokens;
            match done.state.as_str() {
                "cancelled" => {
                    l.state = "cancelled".into();
                    l.stop_reason = Some("cancelled".into());
                    break;
                }
                "failed" => {
                    if let Some(reason) = done
                        .failure_reason
                        .as_deref()
                        .and_then(FailureReason::parse)
                    {
                        let wanted = l.auto_resume.unwrap_or_else(|| {
                            crate::storage::config::load_settings(&self.app)
                                .llm
                                .auto_resume
                        });
                        let v = auto_resume_verdict(&done, wanted, reason, None, l.auto_restarts);
                        if v == ResumeVerdict::Resume {
                            let mut row = done.clone();
                            row.log
                                .push_str("↻ the Loop runs this round again by itself\n");
                            self.save_job(&row);
                            l.auto_restarts += 1;
                            l.rounds_done = round - 1;
                            l.restarted = true;
                            self.save_loop(&l);
                            crate::llm_audit::loop_auto_resumed(
                                &l.id,
                                reason.as_str(),
                                l.auto_restarts,
                                Some(&done.id),
                            );
                            continue;
                        }
                    }
                    if audited {
                        l.state = "blocked".into();
                        l.stop_reason = Some(format!(
                            "{}: {}",
                            omniget_core::core::llm::goal_audit::STOP_ROUND_FAILED,
                            done.error.clone().unwrap_or_default()
                        ));
                        break;
                    }
                    l.state = "failed".into();
                    l.stop_reason = done.error.clone();
                    break;
                }
                _ => l.auto_restarts = 0,
            }
            let check = self.run_check(&l).await;
            if let (Some((ok, out)), Some(cmd)) = (&check, l.check_command.as_deref()) {
                self.attach_check(&done.id, cmd, *ok, out);
            }
            match check {
                Some((true, out)) => {
                    l.last_check = Some(out);
                    l.stop_reason = Some("check_passed".into());
                    break;
                }
                Some((false, out)) => l.last_check = Some(out),
                None if audited => {
                    use omniget_core::core::llm::goal_audit::{self, Step};
                    let reply = done.result.as_deref().unwrap_or("");
                    let (audit, mut entry) = self
                        .audit_round(&l, round, reply, &evidence, last_audit.as_ref())
                        .await;
                    entry.round_cost_usd = round_cost;
                    entry.round_tokens = round_tokens;
                    entry.guided = guided;
                    l.tokens_used += entry.tokens;
                    if let Some(c) = entry.cost_usd {
                        l.audit_usd += c;
                    }
                    if !self.loop_cost_from_ledger(&mut l) {
                        l.spent_usd += entry.cost_usd.unwrap_or(0.0);
                        l.spent_tokens += entry.tokens;
                    }
                    if audit.is_none() && self.loop_budget_block(&l).is_some() {
                        crate::llm_audit::loop_audited(&l.id, &entry);
                        loop_audit::push_audit(&mut l, entry);
                        self.save_loop(&l);
                        continue;
                    }
                    crate::llm_audit::loop_audited(&l.id, &entry);
                    let step = goal_audit::step(&mut streaks, audit.as_ref(), entry.claimed_done);
                    l.blocked_streak = streaks.blocked;
                    l.audit_fail_streak = streaks.failed;
                    l.disagree_streak = streaks.disagree;
                    if let Some(a) = &audit {
                        if !a.note.is_empty() {
                            l.note = Some(a.note.clone());
                        }
                    }
                    loop_audit::push_audit(&mut l, entry);
                    last_audit = audit;
                    match step {
                        Step::Complete => {
                            l.stop_reason = Some(goal_audit::STOP_COMPLETE.into());
                            break;
                        }
                        Step::Blocked(why) => {
                            l.state = "blocked".into();
                            l.stop_reason = Some(why.into());
                            break;
                        }
                        Step::Continue => {}
                    }
                }
                None => {
                    let said_done = done
                        .result
                        .as_deref()
                        .unwrap_or("")
                        .lines()
                        .any(|line| line.trim() == "DONE");
                    if said_done {
                        l.stop_reason = Some("agent_done".into());
                        break;
                    }
                }
            }
            self.save_loop(&l);
        }
        if l.state == "running" {
            l.state = "done".into();
        }
        l.next_round_ms = None;
        if let Some(cur) = self.loop_get(&id) {
            if cur.state == "cancelled" || cur.state == "paused" {
                l.state = "cancelled".into();
                l.stop_reason = Some("cancelled".into());
            }
        }
        l.finished_ms = Some(now_ms());
        self.save_loop(&l);
        if matches!(
            l.state.as_str(),
            "done" | "failed" | "blocked" | "budget_limited"
        ) {
            let target = crate::llm_attention::AttentionTarget {
                job_id: l.id.clone(),
                conversation_id: l.conversation_id.clone(),
                agent: self
                    .llm
                    .agent(&l.agent_id)
                    .map(|a| a.name)
                    .unwrap_or_else(|| l.agent_id.clone()),
            };
            let detail = format!("{} · {}", l.name, l.stop_reason.clone().unwrap_or_default());
            self.attention
                .job_ended(&target, l.state == "done", &detail);
        }
    }

    fn account_of(&self, agent_id: &str) -> Option<String> {
        match self.llm.agent(agent_id)?.runtime {
            omniget_core::core::llm::agent::RuntimeKind::Cli { account, .. } => Some(account),
            _ => None,
        }
    }

    fn budget_block(&self, job: &Job) -> Option<omniget_core::core::llm::budget::policy::Block> {
        use omniget_core::core::llm::budget::policy::{self, Scope};
        let engine = policy::active()?;
        if !engine.has_policies() || job.agent_id.starts_with(TOOL_PREFIX) {
            return None;
        }
        let account = self.account_of(&job.agent_id);
        let mut scopes = vec![Scope::Global, Scope::Agent(job.agent_id.clone())];
        if let Some(a) = &account {
            scopes.push(Scope::Account(a.clone()));
        }
        scopes.extend(work_scopes(job));
        scopes.extend(policy::bound_scopes(&sanitize_id(&job.conversation_id)));
        engine.blocked(&scopes, account.as_deref())
    }

    fn loop_budget_block(
        &self,
        l: &LoopDef,
    ) -> Option<omniget_core::core::llm::budget::policy::Block> {
        let probe = Job {
            id: String::new(),
            agent_id: l.agent_id.clone(),
            conversation_id: l.conversation_id.clone(),
            loop_id: Some(l.id.clone()),
            ..Default::default()
        };
        self.budget_block(&probe)
    }

    fn note_loop_cost(&self, l: &mut LoopDef, done: &Job) {
        if self.loop_cost_from_ledger(l) {
            return;
        }
        let u = done.usage.clone().unwrap_or_default();
        match u["cost_usd"].as_f64() {
            Some(c) => l.spent_usd += c,
            None if done.usage.is_some() => l.unpriced_rounds += 1,
            None => {}
        }
        l.spent_tokens += billable_of(&u);
    }

    fn loop_cost_from_ledger(&self, l: &mut LoopDef) -> bool {
        use omniget_core::core::llm::budget::policy::{self, Scope};
        let Some(engine) = policy::active() else {
            return false;
        };
        let spend = engine.spent(&Scope::Loop(l.id.clone()));
        if spend.turns == 0 {
            return false;
        }
        l.spent_usd = spend.usd;
        l.spent_usd_estimated = spend.usd_estimated;
        l.unpriced_rounds = spend.unpriced_turns;
        l.spent_tokens = spend.tokens;
        true
    }

    fn cancel_scope(&self, scope: &str, incident: Option<&str>, crossing_turn: &str) {
        use omniget_core::core::llm::budget::policy::{self, Scope};
        let Ok(scope) = Scope::parse(scope) else {
            return;
        };
        let bound: Vec<String> = policy::conversations_in(&scope);
        let victims: Vec<Job> = self
            .list(500)
            .into_iter()
            .filter(|j| is_active(&j.state))
            .filter(|j| j.request_id.as_deref() != Some(crossing_turn))
            .filter(|j| in_budget_scope(j, &scope, self.account_of(&j.agent_id).as_deref(), &bound))
            .collect();
        for j in victims {
            if let Some(mut row) = self.job(&j.id).filter(|r| is_active(&r.state)) {
                row.failure_reason = Some("budget".into());
                self.save_job(&row);
            }
            tracing::info!(
                "[jobs] {}: cancelled by the budget of {} ({})",
                j.id,
                scope.key(),
                incident.unwrap_or("-")
            );
            let _ = self.cancel_by(
                &j.id,
                omniget_core::core::llm::audit::Actor::system("budget"),
            );
        }
    }

    pub fn budget_resumed(self: &Arc<Self>, incident: &str) -> Vec<String> {
        let tag = format!("budget:{incident}");
        let mut resumed = Vec::new();
        for l in self.loops() {
            if l.state == "paused" && l.stop_reason.as_deref() == Some(tag.as_str()) {
                match self.loop_resume(&l.id) {
                    Ok(_) => resumed.push(l.id),
                    Err(e) => tracing::info!("[jobs] loop {} stays paused: {e}", l.id),
                }
            }
        }
        resumed
    }

    async fn wait_until(&self, loop_id: &str, at_ms: u64) -> bool {
        let token = CancellationToken::new();
        self.loop_waits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(loop_id.to_string(), token.clone());
        let wait = at_ms.saturating_sub(now_ms());
        let woke = tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(wait)) => true,
            _ = token.cancelled() => false,
        };
        self.loop_waits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(loop_id);
        woke
    }

    async fn run_tool_job(self: &Arc<Self>, job: Job, input: String) -> Job {
        use omniget_core::core::agentkit_run::{runner, sandbox};
        let tool = job
            .agent_id
            .strip_prefix(TOOL_PREFIX)
            .unwrap_or_default()
            .to_string();
        let spec: RunSpec = job
            .spec
            .clone()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let run = runner::ToolRun {
            tool: tool.clone(),
            prompt: input,
            system_prompt: spec.system_prompt.clone(),
            model: spec.model.clone(),
            permission: runner::Permission::parse(spec.permission.as_deref().unwrap_or("")),
            cwd: job
                .workspace
                .clone()
                .filter(|w| !w.is_empty())
                .map(Into::into),
        };
        let cancel = CancellationToken::new();
        self.cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(job.id.clone(), cancel.clone());
        let log = Arc::new(Mutex::new(String::new()));
        let this = self.clone();
        let (log2, job_id) = (log.clone(), job.id.clone());
        let last_flush = Arc::new(Mutex::new(0u64));
        let mut on_log = move |line: &str| {
            let text = {
                let mut l = log2.lock().unwrap_or_else(|e| e.into_inner());
                l.push_str(line);
                l.push('\n');
                *l = clip(&l, LOG_MAX);
                l.clone()
            };
            let mut last = last_flush.lock().unwrap_or_else(|e| e.into_inner());
            if now_ms().saturating_sub(*last) >= 750 {
                *last = now_ms();
                if let Some(mut j) = this.job(&job_id) {
                    j.log = text;
                    this.save_job(&j);
                }
            }
        };
        let result = match &spec.sandbox {
            Some(opts) => sandbox::run(&job.id, &run, opts, cancel.clone(), &mut on_log)
                .await
                .map(|(out, rec)| {
                    let n = if rec.bind_original {
                        None
                    } else {
                        sandbox::diff(&rec.id).ok().map(|c| c.len())
                    };
                    on_log(&match n {
                        Some(n) => format!(
                            "· sandbox ({}): {n} changed file(s) to review in this job",
                            rec.provider
                        ),
                        None => {
                            format!("· sandbox ({}): worked on the project itself", rec.provider)
                        }
                    });
                    out
                }),
            None => runner::run_local(&run, cancel.clone(), &mut on_log).await,
        };
        self.cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&job.id);
        let mut job = self.job(&job.id).unwrap_or(job);
        job.log = log.lock().unwrap_or_else(|e| e.into_inner()).clone();
        match result {
            Ok(out) => {
                if let Some(u) = &out.usage {
                    let model = if u.model.is_empty() {
                        format!(
                            "{tool}/{}",
                            spec.model.clone().unwrap_or_else(|| "default".into())
                        )
                    } else {
                        format!("{tool}/{}", u.model)
                    };
                    job.usage = Some(json!({
                        "model": model,
                        "input_tokens": u.input_tokens,
                        "output_tokens": u.output_tokens,
                        "cache_read_tokens": u.cache_read_tokens,
                        "cost_usd": u.cost_usd,
                        "calls": u.calls,
                    }));
                }
                let errored = out.error.is_some()
                    && (out.text.trim().is_empty()
                        || out.exit_code.map(|c| c != 0).unwrap_or(true)
                        || Some(out.text.trim()) == out.error.as_deref().map(str::trim));
                job.state = if out.cancelled || cancel.is_cancelled() {
                    "cancelled"
                } else if errored {
                    "failed"
                } else {
                    "done"
                }
                .into();
                job.result = Some(clip(&out.text, RESULT_MAX));
                job.error = out.error;
            }
            Err(e) => {
                job.state = if cancel.is_cancelled() {
                    "cancelled"
                } else {
                    "failed"
                }
                .into();
                job.error = Some(e);
            }
        }
        job.finished_ms = Some(now_ms());
        self.save_job(&job);
        job
    }

    fn save_playbook(&self, p: &PlaybookRun) {
        if let Ok(body) = serde_json::to_string(p) {
            let _ = self.db().execute(
                "INSERT OR REPLACE INTO playbooks (id, body) VALUES (?1, ?2)",
                params![p.id, body],
            );
        }
        let _ = self.app.emit(EVENT_PLAYBOOK, p);
    }

    pub fn playbook_get(&self, id: &str) -> Option<PlaybookRun> {
        self.db()
            .query_row("SELECT body FROM playbooks WHERE id=?1", params![id], |r| {
                r.get::<_, String>(0)
            })
            .optional()
            .ok()
            .flatten()
            .and_then(|b| serde_json::from_str(&b).ok())
    }

    pub fn playbooks(&self) -> Vec<PlaybookRun> {
        let db = self.db();
        let Ok(mut stmt) = db.prepare("SELECT body FROM playbooks") else {
            return Vec::new();
        };
        let mut out: Vec<PlaybookRun> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map(|rows| {
                rows.filter_map(Result::ok)
                    .filter_map(|b| serde_json::from_str(&b).ok())
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by_key(|p| std::cmp::Reverse(p.created_ms));
        out
    }

    async fn run_playbook(self: Arc<Self>, id: String) {
        use omniget_core::core::agentkit_run::playbook as pb;
        let Some(mut p) = self.playbook_get(&id) else {
            return;
        };
        while p.current < p.steps.len() {
            match self.playbook_get(&id) {
                Some(cur) if cur.state == "running" => {}
                _ => return,
            }
            let i = p.current;
            let previous: Vec<(String, String)> = p.steps[..i]
                .iter()
                .filter_map(|s| s.output.clone().map(|o| (s.name.clone(), o)))
                .collect();
            let shape = pb::Playbook {
                name: p.name.clone(),
                description: p.description.clone(),
                steps: p
                    .steps
                    .iter()
                    .map(|s| pb::PlaybookStep {
                        name: s.name.clone(),
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            };
            let prompt = pb::step_prompt(&shape, i, &p.steps[i].prompt, &previous);
            let mut job = match self.new_job(
                "playbook",
                &p.steps[i].agent_id,
                &prompt,
                p.workspace.clone(),
                None,
            ) {
                Ok(j) => j,
                Err(e) => {
                    p.steps[i].state = "failed".into();
                    p.state = "failed".into();
                    p.error = Some(e);
                    break;
                }
            };
            job.playbook_id = Some(p.id.clone());
            job.spec = p.steps[i].spec.clone();
            self.save_job(&job);
            p.steps[i].job_id = Some(job.id.clone());
            p.steps[i].state = "running".into();
            self.save_playbook(&p);
            let done = self.run_job(&job.id, None).await;
            let Some(done) = done else {
                p.state = "failed".into();
                break;
            };
            p.steps[i].output = done.result.clone();
            p.steps[i].state = done.state.clone();
            if !is_success(&done.state) {
                p.state = if done.state == "cancelled" {
                    "cancelled".into()
                } else {
                    "failed".into()
                };
                p.error = done.error.clone();
                break;
            }
            p.current = i + 1;
            self.save_playbook(&p);
        }
        if p.state == "running" {
            p.state = "done".into();
        }
        if let Some(cur) = self.playbook_get(&id) {
            if cur.state == "cancelled" {
                p.state = "cancelled".into();
            }
        }
        for s in &mut p.steps {
            if s.state == "pending" {
                s.state = "skipped".into();
            }
        }
        p.finished_ms = Some(now_ms());
        self.save_playbook(&p);
    }

    fn resume(self: Arc<Self>) {
        let open: Vec<Job> = self
            .list(500)
            .into_iter()
            .filter(|j| {
                matches!(
                    j.state.as_str(),
                    "queued" | "waiting_workspace" | "running" | "waiting_approval"
                )
            })
            .collect();
        let mut rounds: HashMap<String, (Job, Option<omniget_core::core::process::StaleProcess>)> =
            HashMap::new();
        for stale in open {
            let Some(mut job) = self.job(&stale.id) else {
                continue;
            };
            let never_sent = matches!(job.state.as_str(), "queued" | "waiting_workspace")
                && job.request_id.is_none()
                && job.started_ms.is_none();
            if never_sent
                && job.loop_id.is_none()
                && job.playbook_id.is_none()
                && job.kind != "chat"
                && job.kind != "mission"
            {
                let this = self.clone();
                tauri::async_runtime::spawn(async move {
                    this.run_job(&job.id, None).await;
                });
                continue;
            }
            let process = self.reap(&mut job);
            job.state = "interrupted".into();
            job.error =
                Some("interrupted: the app closed during this task; its result is unknown".into());
            job.failure_reason = Some(FailureReason::AppRestart.as_str().into());
            job.finished_ms = Some(now_ms());
            if let Some(lid) = job.loop_id.clone() {
                self.save_job(&job);
                rounds.insert(lid, (job, process));
                continue;
            }
            if !self.auto_resume(&mut job, FailureReason::AppRestart, process) {
                self.save_job(&job);
            }
        }
        let setting = crate::storage::config::load_settings(&self.app)
            .llm
            .auto_resume;
        for mut l in self.loops().into_iter().filter(|l| l.state == "running") {
            l.state = "interrupted".into();
            l.stop_reason = Some("app_closed".into());
            let wanted = l.auto_resume.unwrap_or(setting);
            let mut lost_round = None;
            let verdict = match rounds.remove(&l.id) {
                Some((mut round, process)) => {
                    lost_round = Some(round.id.clone());
                    let v = auto_resume_verdict(
                        &round,
                        wanted,
                        FailureReason::AppRestart,
                        process,
                        l.auto_restarts,
                    );
                    if v == ResumeVerdict::Resume {
                        round.state = "failed".into();
                        round.error = Some(
                            "the app closed during this round; the Loop ran it again by itself"
                                .into(),
                        );
                        round
                            .log
                            .push_str("↻ the Loop ran this round again by itself\n");
                        self.save_job(&round);
                    } else if wanted {
                        round.log.push_str(&format!(
                            "· the Loop did not resume by itself: {}\n",
                            v.as_str()
                        ));
                        self.save_job(&round);
                    }
                    v
                }
                None if !wanted => ResumeVerdict::Off,
                None if l.auto_restarts >= AUTO_RESUME_MAX => ResumeVerdict::Attempts,
                None => ResumeVerdict::Resume,
            };
            if verdict == ResumeVerdict::Resume {
                l.state = "running".into();
                l.stop_reason = None;
                l.restarted = true;
                l.auto_restarts += 1;
                self.save_loop(&l);
                crate::llm_audit::loop_auto_resumed(
                    &l.id,
                    FailureReason::AppRestart.as_str(),
                    l.auto_restarts,
                    lost_round.as_deref(),
                );
                let this = self.clone();
                let lid = l.id.clone();
                tauri::async_runtime::spawn(async move { this.run_loop(lid).await });
            } else {
                self.save_loop(&l);
            }
        }
        for mut p in self
            .playbooks()
            .into_iter()
            .filter(|p| p.state == "running")
        {
            if let Some(s) = p.steps.get_mut(p.current) {
                s.state = "pending".into();
                s.job_id = None;
            }
            self.save_playbook(&p);
            let this = self.clone();
            tauri::async_runtime::spawn(async move { this.run_playbook(p.id).await });
        }
    }

    fn capture_process(&self, job: &mut Job, request_id: &str, conversation: &str) -> bool {
        let Some(reg) = omniget_core::core::assist::runs::active() else {
            return false;
        };
        let mut changed = false;
        if job.pid.is_none() {
            if let Some(pid) = reg
                .run(request_id)
                .and_then(|r| r.pid)
                .and_then(|p| u32::try_from(p).ok())
            {
                job.pid = Some(pid);
                job.pid_start = omniget_core::core::process::start_token(pid);
                changed = true;
            }
        }
        let handle = reg
            .session(conversation, &job.agent_id)
            .and_then(|s| s.provider_handle);
        if handle.is_some() && handle != job.session_id {
            job.session_id = handle;
            changed = true;
        }
        changed
    }

    fn reap(&self, job: &mut Job) -> Option<omniget_core::core::process::StaleProcess> {
        use omniget_core::core::process::{stop_stale, StaleProcess};
        let (pid, start) = (job.pid?, job.pid_start?);
        let out = stop_stale(pid, start);
        match out {
            StaleProcess::Killed => job.log.push_str(&format!(
                "↻ the agent process of the last session (pid {pid}) was still running: stopped, so a new run never runs beside it\n"
            )),
            StaleProcess::Survived => job.log.push_str(&format!(
                "! the agent process of the last session (pid {pid}) is still running and could not be stopped\n"
            )),
            StaleProcess::Gone | StaleProcess::NotOurs => {}
        }
        if out != StaleProcess::Survived {
            job.pid = None;
            job.pid_start = None;
        }
        Some(out)
    }

    fn wants_auto_resume(&self, job: &Job) -> bool {
        job.auto_resume.unwrap_or_else(|| {
            crate::storage::config::load_settings(&self.app)
                .llm
                .auto_resume
        })
    }

    fn auto_resume(
        self: &Arc<Self>,
        job: &mut Job,
        reason: FailureReason,
        process: Option<omniget_core::core::process::StaleProcess>,
    ) -> bool {
        let wanted = self.wants_auto_resume(job);
        let mut verdict = auto_resume_verdict(job, wanted, reason, process, job.attempt);
        if verdict == ResumeVerdict::Resume && self.budget_block(job).is_some() {
            verdict = ResumeVerdict::BudgetPaused;
        }
        if verdict != ResumeVerdict::Resume {
            if wanted && verdict != ResumeVerdict::NotThisKind {
                job.log.push_str(&format!(
                    "· not resumed by itself ({}): {}\n",
                    reason.as_str(),
                    verdict.as_str()
                ));
                self.save_job(job);
            }
            return false;
        }
        job.attempt += 1;
        job.log.push_str(&format!(
            "↻ resumed by itself after {} (attempt {} of {AUTO_RESUME_MAX}){}\n",
            reason.as_str(),
            job.attempt,
            job.session_id
                .as_deref()
                .map(|s| format!(", session {s}"))
                .unwrap_or_default()
        ));
        job.state = "queued".into();
        job.error = None;
        job.finished_ms = None;
        job.failure_reason = Some(reason.as_str().into());
        self.save_job(job);
        crate::llm_audit::job_auto_resumed(job, reason.as_str());
        let note = resume_note(reason, &job.prompt);
        let this = self.clone();
        let jid = job.id.clone();
        tauri::async_runtime::spawn(async move {
            for _ in 0..200 {
                let busy = this
                    .cancels
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .contains_key(&jid);
                if !busy {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            this.run_job(&jid, Some(note)).await;
        });
        true
    }

    pub fn loop_resume(self: &Arc<Self>, id: &str) -> Result<LoopDef, String> {
        let mut l = self
            .loop_get(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no loop {id}"))?;
        if !matches!(l.state.as_str(), "interrupted" | "paused" | "blocked") {
            return Err(format!(
                "{ERR_JOBS}: loop {id} is {}, not interrupted, paused or blocked",
                l.state
            ));
        }
        if l.state == "paused" {
            if let Some(block) = self.loop_budget_block(&l) {
                return Err(format!("{}: {}", block.error().code, block.message));
            }
        }
        l.state = "running".into();
        l.stop_reason = None;
        l.finished_ms = None;
        l.blocked_streak = 0;
        l.audit_fail_streak = 0;
        l.disagree_streak = 0;
        self.save_loop(&l);
        let this = self.clone();
        let lid = l.id.clone();
        tauri::async_runtime::spawn(async move { this.run_loop(lid).await });
        Ok(l)
    }

    pub fn triggers(&self) -> Vec<Trigger> {
        read_triggers(&self.db())
    }

    pub fn trigger(&self, id: &str) -> Option<Trigger> {
        read_trigger(&self.db(), id)
    }

    fn update_trigger(&self, id: &str, change: impl FnOnce(&mut Trigger)) -> Option<Trigger> {
        let t = {
            let db = self.db();
            let mut t = read_trigger(&db, id)?;
            change(&mut t);
            if let Err(e) = write_trigger_body(&db, &t) {
                tracing::warn!("[jobs] trigger {id}: {e}");
                return None;
            }
            t
        };
        let _ = self.app.emit(EVENT_TRIGGER, trigger_view(&t));
        Some(t)
    }

    pub fn fire(self: &Arc<Self>, id: &str, body: Option<&str>) -> Result<Job, String> {
        let t = self
            .trigger(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no trigger {id}"))?;
        if !t.enabled {
            return Err(format!("{ERR_JOBS}: trigger {id} is disabled"));
        }
        let body = clip(body.unwrap_or(""), 32 * 1024);
        let prompt = if t.prompt.contains("{{body}}") {
            t.prompt.replace("{{body}}", &body)
        } else if body.trim().is_empty() {
            t.prompt.clone()
        } else {
            format!("{}\n\nPayload:\n{body}", t.prompt)
        };
        if let Some(template) = t.mission.clone() {
            let mission_id = crate::missions::start_from_trigger(&self.app, &t, &prompt, template)?;
            self.note_fired(&t.id, &mission_id);
            return Ok(Job {
                id: mission_id,
                kind: "mission".into(),
                agent_id: t.agent_id.clone(),
                prompt,
                state: "queued".into(),
                trigger_id: Some(t.id.clone()),
                created_ms: now_ms(),
                ..Default::default()
            });
        }
        let job = self.submit_with_deadline(
            "trigger",
            &t.agent_id,
            &prompt,
            t.workspace.clone(),
            None,
            Some(t.id.clone()),
            t.deadline_min,
        )?;
        crate::llm_audit::job_created(
            omniget_core::core::llm::audit::Actor::via("trigger", &format!("trigger:{}", t.id)),
            &job,
        );
        self.note_fired(&t.id, &job.id);
        Ok(job)
    }

    fn note_fired(&self, id: &str, run_id: &str) {
        let at = now_ms();
        self.update_trigger(id, |t| {
            t.last_fired_ms = Some(at);
            t.fire_count += 1;
            t.last_run_id = Some(run_id.to_string());
        });
    }

    fn ensure_cron(self: Arc<Self>) {
        let has = |this: &Jobs| {
            this.triggers()
                .iter()
                .any(|t| t.enabled && t.kind == "cron")
        };
        if !has(&self) || self.cron_running.swap(true, Ordering::SeqCst) {
            return;
        }
        tauri::async_runtime::spawn(async move {
            self.cron_tick(0);
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(CRON_TICK_S)).await;
                if !has(&self) {
                    self.cron_running.store(false, Ordering::SeqCst);
                    return;
                }
                self.cron_tick(CRON_GRACE_MS);
            }
        });
    }

    fn cron_tick(self: &Arc<Self>, grace_ms: i64) {
        let _pass = self.cron_pass.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_ms() as i64;
        let last = self.last_worktree_gc.load(Ordering::SeqCst);
        if (now as u64).saturating_sub(last) >= WORKTREE_GC_EVERY_MS
            && self
                .last_worktree_gc
                .compare_exchange(last, now as u64, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            tauri::async_runtime::spawn(self.clone().gc_worktrees(false));
        }
        for t in self
            .triggers()
            .into_iter()
            .filter(|t| t.enabled && t.kind == "cron")
        {
            let mission_active = t.mission.is_some()
                && t.last_run_id.as_deref().is_some_and(|m| {
                    crate::missions::driver(&self.app).is_ok_and(|d| d.is_driving(m))
                });
            let claimed = claim_due(&self.db(), &t.id, now, grace_ms, mission_active);
            let Some((t, plan)) = claimed else { continue };
            if plan.missed_slots > 0 || plan.skipped_active > 0 {
                tracing::info!(
                    "[jobs] cron {}: missed={} caught_up={} skipped_active={} backlog={} policy={:?}/{:?}",
                    t.id,
                    plan.missed_slots,
                    plan.caught_up,
                    plan.skipped_active,
                    plan.backlog,
                    t.catch_up,
                    t.concurrency
                );
            }
            for _ in 0..plan.fire_now {
                if let Err(e) = self.fire(&t.id, None) {
                    tracing::warn!("[jobs] cron {}: {e}", t.id);
                }
            }
            let latest = self.trigger(&t.id).unwrap_or(t);
            let _ = self.app.emit(EVENT_TRIGGER, trigger_view(&latest));
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cron {
    minute: u64,
    hour: u64,
    dom: u64,
    month: u64,
    dow: u64,
    dom_any: bool,
    dow_any: bool,
}

impl Cron {
    fn field(raw: &str, lo: u32, hi: u32) -> Result<u64, String> {
        let bad = || format!("{ERR_JOBS}: bad cron field `{raw}` ({lo}-{hi})");
        let mut bits = 0u64;
        for part in raw.split(',') {
            let (range, step) = match part.split_once('/') {
                Some((r, s)) => (r, s.parse::<u32>().map_err(|_| bad())?),
                None => (part, 1),
            };
            if step == 0 {
                return Err(bad());
            }
            let (a, b) = if range == "*" {
                (lo, hi)
            } else if let Some((a, b)) = range.split_once('-') {
                (a.parse().map_err(|_| bad())?, b.parse().map_err(|_| bad())?)
            } else {
                let v: u32 = range.parse().map_err(|_| bad())?;
                (v, if part.contains('/') { hi } else { v })
            };
            if a < lo || b > hi || a > b {
                return Err(bad());
            }
            let mut v = a;
            while v <= b {
                bits |= 1 << v;
                v += step;
            }
        }
        Ok(bits)
    }

    pub fn parse(line: &str) -> Result<Self, String> {
        let line = match line.trim() {
            "@hourly" => "0 * * * *",
            "@daily" | "@midnight" => "0 0 * * *",
            "@weekly" => "0 0 * * 0",
            "@monthly" => "0 0 1 * *",
            other => other,
        };
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() != 5 {
            return Err(format!(
                "{ERR_JOBS}: cron needs 5 fields (minute hour day month weekday), got {}",
                f.len()
            ));
        }
        let mut dow = Self::field(f[4], 0, 7)?;
        if dow & (1 << 7) != 0 {
            dow |= 1;
        }
        Ok(Self {
            minute: Self::field(f[0], 0, 59)?,
            hour: Self::field(f[1], 0, 23)?,
            dom: Self::field(f[2], 1, 31)?,
            month: Self::field(f[3], 1, 12)?,
            dow,
            dom_any: f[2] == "*",
            dow_any: f[4] == "*",
        })
    }

    pub fn matches<Tz: chrono::TimeZone>(&self, t: &chrono::DateTime<Tz>) -> bool {
        let has = |bits: u64, v: u32| bits & (1 << v) != 0;
        let dom = has(self.dom, t.day());
        let dow = has(self.dow, t.weekday().num_days_from_sunday());
        let day = match (self.dom_any, self.dow_any) {
            (false, false) => dom || dow,
            _ => dom && dow,
        };
        has(self.minute, t.minute())
            && has(self.hour, t.hour())
            && has(self.month, t.month())
            && day
    }
}

impl Cron {
    pub fn next_after_at<Tz: chrono::TimeZone>(
        &self,
        from: &chrono::DateTime<Tz>,
    ) -> Option<chrono::DateTime<Tz>> {
        let mut t = from.clone() + chrono::Duration::minutes(1);
        t = t.with_second(0)?.with_nanosecond(0)?;
        for _ in 0..(366 * 24 * 60) {
            if self.matches(&t) {
                return Some(t);
            }
            t += chrono::Duration::minutes(1);
        }
        None
    }

    pub fn next_after(&self, ts_ms: i64, tz: &str) -> Option<i64> {
        use chrono::TimeZone;
        match zone_of(tz) {
            Zone::Local => {
                let from = chrono::Local.timestamp_millis_opt(ts_ms).single()?;
                self.next_after_at(&from).map(|d| d.timestamp_millis())
            }
            Zone::Fixed(offset) => {
                let from = offset.timestamp_millis_opt(ts_ms).single()?;
                self.next_after_at(&from).map(|d| d.timestamp_millis())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Zone {
    Local,
    Fixed(chrono::FixedOffset),
}

pub fn parse_zone(tz: &str) -> Option<Zone> {
    let tz = tz.trim();
    if tz.is_empty() || tz.eq_ignore_ascii_case("local") {
        return Some(Zone::Local);
    }
    if tz.eq_ignore_ascii_case("utc") || tz == "Z" {
        return chrono::FixedOffset::east_opt(0).map(Zone::Fixed);
    }
    let (sign, rest) = match tz.as_bytes().first()? {
        b'+' => (1, &tz[1..]),
        b'-' => (-1, &tz[1..]),
        _ => return None,
    };
    let digits: String = rest.chars().filter(|c| *c != ':').collect();
    if !digits.chars().all(|c| c.is_ascii_digit()) || !matches!(digits.len(), 2 | 4) {
        return None;
    }
    let hours: i32 = digits[..2].parse().ok()?;
    let minutes: i32 = if digits.len() == 4 {
        digits[2..].parse().ok()?
    } else {
        0
    };
    if hours > 14 || minutes > 59 {
        return None;
    }
    chrono::FixedOffset::east_opt(sign * (hours * 3600 + minutes * 60)).map(Zone::Fixed)
}

pub fn zone_of(tz: &str) -> Zone {
    parse_zone(tz).unwrap_or(Zone::Local)
}

pub const CRON_GRACE_MS: i64 = 3 * CRON_TICK_S as i64 * 1000;
const MISSED_COUNT_MAX: u32 = 10_000;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TickPlan {
    pub fire_now: u32,
    pub backlog: u32,
    pub missed_slots: u32,
    pub caught_up: u32,
    pub skipped_active: u32,
    pub not_run: u32,
    pub next_run_ms: Option<i64>,
}

pub fn plan_tick(t: &Trigger, cron: &Cron, now_ms: i64, grace_ms: i64, active: bool) -> TickPlan {
    let mut plan = TickPlan {
        backlog: t.backlog.min(CATCH_UP_CAP),
        next_run_ms: t.next_run_ms,
        ..Default::default()
    };
    let Some(first) = t.next_run_ms else {
        plan.next_run_ms = cron.next_after(now_ms, &t.tz);
        return plan;
    };
    let mut runs = 0;
    if first <= now_ms {
        let (mut due, mut last, mut slot) = (0u32, first, Some(first));
        while let Some(s) = slot.filter(|s| *s <= now_ms && due < MISSED_COUNT_MAX) {
            due += 1;
            last = s;
            slot = cron.next_after(s, &t.tz);
        }
        let on_time = u32::from(due > 0 && now_ms - last <= grace_ms);
        let missed = due - on_time;
        let catch = match t.catch_up {
            CatchUp::Skip => 0,
            CatchUp::Once => u32::from(missed > 0 && on_time == 0),
            CatchUp::AllCapped => missed.min(CATCH_UP_CAP - on_time),
        };
        plan.missed_slots = missed;
        plan.caught_up = catch;
        plan.not_run = missed - catch;
        runs = on_time + catch;
        plan.next_run_ms = cron.next_after(now_ms, &t.tz);
    }
    let pending = (runs + plan.backlog).min(CATCH_UP_CAP);
    match t.concurrency {
        TriggerConcurrency::Queue => {
            plan.fire_now = pending;
            plan.backlog = 0;
        }
        TriggerConcurrency::SkipIfActive if active => {
            plan.skipped_active = runs;
            plan.not_run += runs;
            plan.caught_up = 0;
        }
        TriggerConcurrency::SkipIfActive => {
            plan.fire_now = u32::from(pending > 0);
            plan.backlog = pending.saturating_sub(1);
        }
    }
    plan
}

fn trigger_of(body: &str, next_run_ms: Option<i64>) -> Option<Trigger> {
    let mut t: Trigger = serde_json::from_str(body).ok()?;
    t.next_run_ms = next_run_ms;
    Some(t)
}

pub fn read_triggers(db: &Connection) -> Vec<Trigger> {
    let Ok(mut stmt) = db.prepare("SELECT body, next_run_ms FROM triggers") else {
        return Vec::new();
    };
    let mut out: Vec<Trigger> = stmt
        .query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?))
        })
        .map(|rows| {
            rows.filter_map(Result::ok)
                .filter_map(|(body, next)| trigger_of(&body, next))
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|t| t.created_ms);
    out
}

pub fn read_trigger(db: &Connection, id: &str) -> Option<Trigger> {
    db.query_row(
        "SELECT body, next_run_ms FROM triggers WHERE id=?1",
        params![id],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?)),
    )
    .optional()
    .ok()
    .flatten()
    .and_then(|(body, next)| trigger_of(&body, next))
}

pub fn write_trigger_body(db: &Connection, t: &Trigger) -> Result<(), String> {
    let body = serde_json::to_string(t).map_err(|e| e.to_string())?;
    db.execute(
        "INSERT INTO triggers (id, body, next_run_ms) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET body=excluded.body",
        params![t.id, body, t.next_run_ms],
    )
    .map(|_| ())
    .map_err(|e| format!("{ERR_JOBS}: {e}"))
}

pub fn cas_schedule(db: &Connection, t: &Trigger, seen: Option<i64>) -> bool {
    let Ok(body) = serde_json::to_string(t) else {
        return false;
    };
    db.execute(
        "UPDATE triggers SET body=?2, next_run_ms=?3 WHERE id=?1 AND next_run_ms IS ?4",
        params![t.id, body, t.next_run_ms, seen],
    )
    .map(|n| n == 1)
    .unwrap_or(false)
}

fn migrate_triggers(db: &Connection, now_ms: i64) {
    let rows: Vec<(String, String)> = db
        .prepare("SELECT id, body FROM triggers WHERE next_run_ms IS NULL")
        .and_then(|mut stmt| {
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map(|rows| rows.filter_map(Result::ok).collect())
        })
        .unwrap_or_default();
    for (id, body) in rows {
        let Some(mut t) = trigger_of(&body, None) else {
            continue;
        };
        if !(t.enabled && t.kind == "cron") {
            continue;
        }
        t.next_run_ms = Cron::parse(t.cron.as_deref().unwrap_or(""))
            .ok()
            .and_then(|c| c.next_after(now_ms, &t.tz));
        if t.next_run_ms.is_some() && !cas_schedule(db, &t, None) {
            tracing::debug!("[jobs] trigger {id}: migrated by someone else");
        }
    }
}

pub fn trigger_busy(db: &Connection, id: &str) -> bool {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM jobs WHERE trigger_id=?1 AND state IN ('queued','waiting_workspace','running','waiting_approval'))",
        params![id],
        |r| r.get::<_, bool>(0),
    )
    .unwrap_or(false)
}

pub fn claim_due(
    db: &Connection,
    id: &str,
    now_ms: i64,
    grace_ms: i64,
    mission_active: bool,
) -> Option<(Trigger, TickPlan)> {
    let t = read_trigger(db, id)?;
    if !(t.enabled && t.kind == "cron") {
        return None;
    }
    let cron = Cron::parse(t.cron.as_deref().unwrap_or("")).ok()?;
    let active = mission_active || trigger_busy(db, id);
    let plan = plan_tick(&t, &cron, now_ms, grace_ms, active);
    let unchanged = plan.fire_now == 0
        && plan.not_run == 0
        && plan.backlog == t.backlog
        && plan.next_run_ms == t.next_run_ms;
    if unchanged {
        return None;
    }
    let mut next = t.clone();
    next.next_run_ms = plan.next_run_ms;
    next.backlog = plan.backlog;
    next.missed = next.missed.saturating_add(plan.not_run);
    if plan.not_run > 0 {
        next.last_missed_ms = Some(now_ms.max(0) as u64);
    }
    cas_schedule(db, &next, t.next_run_ms).then_some((next, plan))
}

pub fn routine_digest(t: &Trigger, body: &str) -> Option<String> {
    if t.muted {
        return None;
    }
    use sha2::{Digest, Sha256};
    let digest = hex::encode(Sha256::digest(body.trim().as_bytes()));
    if t.last_digest.as_deref() == Some(digest.as_str()) {
        return None;
    }
    Some(digest)
}

pub fn trigger_view(t: &Trigger) -> Value {
    use chrono::TimeZone;
    let now = chrono::Local::now();
    let next = if t.enabled && t.kind == "cron" {
        t.next_run_ms.or_else(|| {
            Cron::parse(t.cron.as_deref().unwrap_or(""))
                .ok()
                .and_then(|c| c.next_after(now.timestamp_millis(), &t.tz))
        })
    } else {
        None
    };
    let local = next.and_then(|ms| chrono::Local.timestamp_millis_opt(ms).single());
    let mut v = serde_json::to_value(t).unwrap_or(Value::Null);
    v["next_run_ms"] = json!(next);
    v["next_run_local"] = json!(local.map(|d| d.format("%Y-%m-%d %H:%M").to_string()));
    v["zone_known"] = json!(parse_zone(&t.tz).is_some());
    v["utc_offset"] = json!(now.format("%:z").to_string());
    v["requires_app_open"] = json!(true);
    v
}

pub fn next_round_at(schedule: &str, from_ms: u64) -> Option<u64> {
    let s = schedule.trim();
    if s.is_empty() {
        return None;
    }
    if s.split_whitespace().count() == 5 || s.starts_with('@') {
        let cron = Cron::parse(s).ok()?;
        let start =
            chrono::DateTime::from_timestamp_millis(from_ms as i64)?.with_timezone(&chrono::Local);
        let mut t =
            start.with_second(0).and_then(|t| t.with_nanosecond(0))? + chrono::Duration::minutes(1);
        for _ in 0..(366 * 24 * 60) {
            if cron.matches(&t) {
                return Some(t.timestamp_millis() as u64);
            }
            t += chrono::Duration::minutes(1);
        }
        return None;
    }
    omniget_core::core::agentkit::convert::loop_runbook::interval_secs(s)
        .map(|secs| from_ms + secs * 1000)
}

#[derive(Debug, Clone, Default)]
pub struct Live {
    pub request_id: String,
    pub started_ms: u64,
    pub last_event_ms: u64,
    pub deadline_ms: Option<u64>,
}

pub fn next_state_seq(stored: Option<(&str, i64)>, new_state: &str) -> i64 {
    match stored {
        None => 1,
        Some((state, seq)) if state == new_state => seq,
        Some((_, seq)) => seq + 1,
    }
}

pub fn failure_reason_of(error: &str) -> Option<&'static str> {
    use omniget_core::core::llm::broker::ERR_TOOL_TIMEOUT;
    use omniget_core::core::llm::cli_runtime::{ERR_CLI_NO_START, ERR_CLI_STALLED};
    if error.starts_with(ERR_CLI_STALLED) {
        Some("stalled")
    } else if error.starts_with(ERR_CLI_NO_START) {
        Some("no_start")
    } else if error.starts_with(ERR_TOOL_TIMEOUT) {
        Some("ask_expired")
    } else if error.starts_with(omniget_core::core::llm::error::ERR_LLM_BUDGET) {
        Some("budget")
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    AppRestart,
    Stalled,
    NoStart,
    Deadline,
    Limit,
    AskExpired,
}

impl FailureReason {
    pub fn as_str(self) -> &'static str {
        match self {
            FailureReason::AppRestart => "app_restart",
            FailureReason::Stalled => "stalled",
            FailureReason::NoStart => "no_start",
            FailureReason::Deadline => "deadline",
            FailureReason::Limit => "limit",
            FailureReason::AskExpired => "ask_expired",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "app_restart" => FailureReason::AppRestart,
            "stalled" => FailureReason::Stalled,
            "no_start" => FailureReason::NoStart,
            "deadline" => FailureReason::Deadline,
            "limit" => FailureReason::Limit,
            "ask_expired" => FailureReason::AskExpired,
            _ => return None,
        })
    }

    pub fn is_infrastructure(self) -> bool {
        matches!(
            self,
            FailureReason::AppRestart | FailureReason::Stalled | FailureReason::NoStart
        )
    }
}

pub const AUTO_RESUME_MAX: u32 = 2;

pub fn tool_may_write(name: &str) -> bool {
    const READS: &[&str] = &[
        "read",
        "glob",
        "grep",
        "ls",
        "webfetch",
        "websearch",
        "todowrite",
        "todoread",
        "notebookread",
        "bashoutput",
        "exitplanmode",
        "kb_search",
        "web_search",
        "web_fetch",
    ];
    let n = name.trim().to_ascii_lowercase();
    !(READS.contains(&n.as_str()) || code_tools::READ_TOOLS.contains(&n.as_str()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeVerdict {
    Resume,
    Off,
    NotInfrastructure,
    NotThisKind,
    Attempts,
    WroteInSharedFolder,
    ProcessAlive,
    BudgetPaused,
}

impl ResumeVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            ResumeVerdict::Resume => "resume",
            ResumeVerdict::Off => "off",
            ResumeVerdict::NotInfrastructure => "not_infrastructure",
            ResumeVerdict::NotThisKind => "not_this_kind",
            ResumeVerdict::Attempts => "attempts",
            ResumeVerdict::WroteInSharedFolder => "wrote_in_shared_folder",
            ResumeVerdict::ProcessAlive => "process_alive",
            ResumeVerdict::BudgetPaused => "budget_paused",
        }
    }
}

pub fn auto_resume_verdict(
    job: &Job,
    wanted: bool,
    reason: FailureReason,
    process: Option<omniget_core::core::process::StaleProcess>,
    attempts: u32,
) -> ResumeVerdict {
    use omniget_core::core::process::StaleProcess;
    if !wanted {
        return ResumeVerdict::Off;
    }
    if !reason.is_infrastructure() {
        return ResumeVerdict::NotInfrastructure;
    }
    if job.failure_reason.as_deref() == Some("budget") {
        return ResumeVerdict::BudgetPaused;
    }
    if job.state == "cancelled" {
        return ResumeVerdict::NotInfrastructure;
    }
    if !matches!(job.kind.as_str(), "run" | "trigger" | "loop")
        || job.agent_id.starts_with(TOOL_PREFIX)
        || job.playbook_id.is_some()
        || (job.kind != "loop" && job.loop_id.is_some())
    {
        return ResumeVerdict::NotThisKind;
    }
    if process == Some(StaleProcess::Survived) {
        return ResumeVerdict::ProcessAlive;
    }
    if attempts >= AUTO_RESUME_MAX {
        return ResumeVerdict::Attempts;
    }
    if job.side_effects && job.worktree_path.is_none() {
        return ResumeVerdict::WroteInSharedFolder;
    }
    ResumeVerdict::Resume
}

pub fn resume_note(reason: FailureReason, prompt: &str) -> String {
    let why = match reason {
        FailureReason::AppRestart => "The app closed in the middle of this task",
        _ => "This task stopped because the agent went silent",
    };
    format!(
        "{why} and its result is unknown. Check what is already done before changing anything, and do not repeat an action that already happened. Task: {prompt}"
    )
}

pub fn default_deadline_ms(kind: &str, started_ms: u64, deadline_min: u32) -> Option<u64> {
    matches!(kind, "run" | "loop" | "trigger")
        .then(|| started_ms + u64::from(deadline_min) * 60_000)
}

pub fn job_ask_timeout(
    now_ms: u64,
    deadline_ms: Option<u64>,
    ask_job: Option<std::time::Duration>,
) -> Option<std::time::Duration> {
    ask_job
        .or_else(|| deadline_ms.map(|d| std::time::Duration::from_millis(d.saturating_sub(now_ms))))
}

pub fn explain_view(
    job: &Job,
    live: Option<&Live>,
    now_ms: u64,
    asks: &[Value],
    pid: Option<i64>,
) -> Value {
    let secs = |from: u64, to: u64| to.saturating_sub(from) / 1000;
    let running = matches!(job.state.as_str(), "running" | "waiting_approval");
    let asks: Vec<Value> = asks
        .iter()
        .filter(|a| {
            job.request_id.is_some() && a["request_id"].as_str() == job.request_id.as_deref()
        })
        .map(|a| {
            json!({
                "tool": a["tool"],
                "tool_call_id": a["tool_call_id"],
                "preview": a["preview"],
                "age_s": a["asked_ms"].as_u64().map(|t| secs(t, now_ms)),
                "expires_in_s": a["expires_ms"].as_u64().map(|t| secs(now_ms, t)),
            })
        })
        .collect();
    let deadline = live.and_then(|l| l.deadline_ms).or(job.deadline_ms);
    let mut v = json!({
        "id": job.id,
        "kind": job.kind,
        "state": job.state,
        "state_seq": job.state_seq,
        "failure_reason": job.failure_reason,
        "error": job.error,
        "request_id": job.request_id,
        "started_ms": job.started_ms,
        "last_event_age_s": live.filter(|_| running).map(|l| secs(l.last_event_ms, now_ms)),
        "ask_pending": running && !asks.is_empty(),
        "asks": asks,
        "deadline_ms": deadline,
        "deadline_left_s": deadline.filter(|_| running).map(|d| secs(now_ms, d)),
        "slot": running,
        "session_id": job.session_id,
        "attempt": job.attempt,
        "auto_resume": job.auto_resume,
        "side_effects": job.side_effects,
    });
    if let Some(pid) = pid.filter(|_| running) {
        v["pid"] = json!(pid);
    }
    v
}

use omniget_core::core::llm::job_gate::{GateView, Limits, WaitReason};

pub fn gate_limits(s: &omniget_core::models::settings::LlmSettings) -> Limits {
    let (lo, hi) = omniget_core::core::llm::job_gate::MAX_PARALLEL_RANGE;
    Limits {
        max_parallel: (s.jobs_max_parallel as usize).clamp(lo, hi),
        max_per_agent: s.jobs_max_per_agent.map(|n| (n as usize).clamp(lo, hi)),
        stagger: omniget_core::core::llm::job_gate::STAGGER,
    }
}

pub fn wait_state(reason: &WaitReason) -> (&'static str, String) {
    match reason {
        WaitReason::Workspace { holder, .. } => {
            ("waiting_workspace", format!("workspace:{holder}"))
        }
        other => ("queued", other.code().to_string()),
    }
}

fn wait_detail(reason: &WaitReason, view: &GateView, agent: &str) -> String {
    match reason {
        WaitReason::Slot => format!("all {} job slots are busy", view.limits.max_parallel),
        WaitReason::AgentCap { cap } => format!("agent {agent} is at its cap of {cap} jobs"),
        WaitReason::Workspace { holder, folder } => format!("folder busy by {holder} ({folder})"),
    }
}

pub fn state_view(view: &GateView, job: &dyn Fn(&str) -> Option<Job>) -> Value {
    let mut running = Vec::new();
    let mut blocked = Vec::new();
    for h in &view.held {
        let j = job(&h.job_id);
        let state = j
            .as_ref()
            .map(|j| j.state.clone())
            .unwrap_or_else(|| "running".into());
        let row = json!({
            "id": h.job_id,
            "agent_id": h.agent_id,
            "kind": j.as_ref().map(|j| j.kind.clone()),
            "state": state,
            "workspace": j.as_ref().and_then(|j| j.workspace.clone()),
            "started_ms": j.as_ref().and_then(|j| j.started_ms),
        });
        if state == "waiting_approval" {
            let mut row = row;
            row["reason"] = json!("approval");
            blocked.push(row);
        } else {
            running.push(row);
        }
    }
    let queued: Vec<Value> = view
        .queued
        .iter()
        .map(|q| {
            let j = job(&q.job_id);
            let mut row = json!({
                "id": q.job_id,
                "position": q.position,
                "reason": q.reason.code(),
                "detail": wait_detail(&q.reason, view, &q.agent_id),
                "agent_id": q.agent_id,
                "kind": j.as_ref().map(|j| j.kind.clone()),
                "state": wait_state(&q.reason).0,
                "workspace": j.as_ref().and_then(|j| j.workspace.clone()),
            });
            if let WaitReason::Workspace { holder, folder } = &q.reason {
                row["holder"] = json!(holder);
                row["folder"] = json!(folder.as_str());
            }
            row
        })
        .collect();
    json!({
        "running": running,
        "queued": queued,
        "blocked": blocked,
        "limits": {
            "max_parallel": view.limits.max_parallel,
            "max_per_agent": view.limits.max_per_agent,
            "in_use": view.held.len(),
            "available": view.available,
            "stagger_ms": view.limits.stagger.as_millis() as u64,
        },
    })
}

pub fn is_success(state: &str) -> bool {
    matches!(state, "done" | "review" | "done_unreported")
}

pub fn is_terminal(state: &str) -> bool {
    matches!(
        state,
        "done" | "review" | "done_unreported" | "failed" | "cancelled" | "interrupted"
    )
}

pub const OUTCOME_SUFFIX: &str = "\n\nEnd your reply with one line: `OUTCOME: succeeded` if the requested change is done, `OUTCOME: failed` if not, then ` — ` and up to 3 sentences on what you did and anything left unverified.";

pub fn asks_outcome(kind: &str) -> bool {
    matches!(kind, "run" | "mission")
}

pub fn parse_outcome(text: &str) -> Option<Value> {
    text.lines().rev().find_map(|line| {
        let l = line
            .trim()
            .trim_matches(|c| c == '*' || c == '`' || c == '_' || c == '>')
            .trim();
        let rest = l
            .get(..8)
            .filter(|p| p.eq_ignore_ascii_case("outcome:"))
            .map(|_| l[8..].trim_start())?;
        let rest = rest.trim_start_matches(['*', '`']);
        let lower = rest.to_ascii_lowercase();
        let status = ["succeeded", "failed"]
            .into_iter()
            .find(|s| lower.starts_with(s))?;
        let summary = rest[status.len()..]
            .trim_start_matches(['*', '`'])
            .trim_start()
            .trim_start_matches(['—', '–', '-', ':'])
            .trim()
            .trim_end_matches(['*', '`'])
            .trim();
        Some(json!({ "status": status, "summary": clip(summary, 1_000) }))
    })
}

pub fn settle_outcome<'a>(kind: &str, state: &'a str, reported: bool) -> &'a str {
    if kind == "run" && state == "done" && !reported {
        "done_unreported"
    } else {
        state
    }
}

pub fn work_scopes(job: &Job) -> Vec<omniget_core::core::llm::budget::policy::Scope> {
    use omniget_core::core::llm::budget::policy::Scope;
    let mut out = Vec::new();
    if !job.id.is_empty() {
        out.push(Scope::Job(job.id.clone()));
    }
    if let Some(l) = &job.loop_id {
        out.push(Scope::Loop(l.clone()));
    }
    out
}

pub fn in_budget_scope(
    job: &Job,
    scope: &omniget_core::core::llm::budget::policy::Scope,
    account: Option<&str>,
    bound: &[String],
) -> bool {
    use omniget_core::core::llm::budget::policy::Scope;
    match scope {
        Scope::Global => true,
        Scope::Agent(a) => &job.agent_id == a,
        Scope::Account(a) => account == Some(a.as_str()),
        Scope::Job(id) => &job.id == id,
        Scope::Loop(id) => job.loop_id.as_deref() == Some(id.as_str()),
        Scope::Mission(_) => bound.contains(&sanitize_id(&job.conversation_id)),
    }
}

fn budget_policy(
    scope: omniget_core::core::llm::budget::policy::Scope,
    metric: omniget_core::core::llm::budget::policy::Metric,
    amount: f64,
) {
    use omniget_core::core::llm::budget::policy::{self, BudgetPolicy, Window};
    let Some(engine) = policy::active() else {
        return;
    };
    let key = scope.key();
    let saved = engine.upsert(
        BudgetPolicy {
            id: String::new(),
            scope,
            metric,
            window: Window::Run,
            amount,
            warn_pct: Some(80.0),
            hard_stop: true,
            enabled: true,
            created_ms: 0,
            updated_ms: 0,
        },
        omniget_core::core::llm::audit::Actor::window(),
    );
    if let Err(e) = saved {
        tracing::warn!("[jobs] {key} budget: {e}");
    }
}

pub fn with_budget(guard: Option<JobGuard>, budget_usd: Option<f64>) -> Option<JobGuard> {
    let Some(budget) = budget_usd.filter(|b| *b > 0.0) else {
        return guard;
    };
    let mut g = guard.unwrap_or_default();
    let before = g.check.take();
    g.check = Some(Arc::new(move |tokens, usd| {
        if let Some(check) = &before {
            check(tokens, usd)?;
        }
        match usd {
            Some(spent) if spent > budget => Err(format!(
                "the job spent US$ {spent:.4}, past its budget of US$ {budget:.4}"
            )),
            _ => Ok(()),
        }
    }));
    Some(g)
}

pub fn settle_state<'a>(
    state: &'a str,
    files: Option<&[omniget_core::core::llm::snapshot::FileChange]>,
) -> &'a str {
    if state == "done" && files.is_some_and(|f| !f.is_empty()) {
        "review"
    } else {
        state
    }
}

pub const PROOF_MAX_FILES: usize = 200;

pub fn build_proof(job: &Job, ends: Option<&(String, String)>, host: &str) -> Value {
    let short = |s: &str| s.chars().take(7).collect::<String>();
    let worktree = job.branch.is_some() && job.base_oid.is_some();
    let workspace = effective_workspace(job).unwrap_or_default();
    let (base, head) = if worktree {
        (
            ends.map(|e| e.0.clone()).or_else(|| job.base_oid.clone()),
            job.head_oid.clone(),
        )
    } else {
        (
            ends.map(|e| format!("tree:{}", e.0)),
            ends.map(|e| format!("tree:{}", e.1)),
        )
    };
    let label = |v: &Option<String>| match v {
        Some(x) => match x.strip_prefix("tree:") {
            Some(t) => format!("tree:{}", short(t)),
            None => short(x),
        },
        None => "?".into(),
    };
    let stamp = format!("{host}:{workspace}@{}→{}", label(&base), label(&head));
    let files = job.diff_files.clone().unwrap_or_default();
    let usage = job.usage.clone().unwrap_or_default();
    let duration_ms = job
        .finished_ms
        .zip(job.started_ms)
        .map(|(f, s)| f.saturating_sub(s));
    json!({
        "generated_by": "host",
        "stamp": stamp,
        "host": host,
        "workspace": workspace,
        "mode": if worktree { "worktree" } else { "shared" },
        "branch": job.branch,
        "base": base,
        "head": head,
        "state": job.state,
        "files": files.iter().take(PROOF_MAX_FILES).collect::<Vec<_>>(),
        "file_count": files.len(),
        "additions": job.additions,
        "deletions": job.deletions,
        "clipped": job.diff_clipped,
        "diff_unavailable": job.diff_unavailable,
        "checks": [],
        "tokens": {
            "input": usage["input_tokens"].as_u64(),
            "output": usage["output_tokens"].as_u64(),
            "cache_read": usage["cache_read_tokens"].as_u64(),
            "billable": job.usage.as_ref().map(billable_of),
        },
        "cost_usd": usage["cost_usd"].as_f64(),
        "duration_ms": duration_ms,
        "at_ms": job.finished_ms,
    })
}

pub fn push_check(proof: &mut Value, command: &str, ok: bool, out: &str) {
    let code = out
        .strip_prefix("exit ")
        .and_then(|r| r.split_whitespace().next())
        .and_then(|c| c.parse::<i64>().ok());
    if !proof["checks"].is_array() {
        proof["checks"] = json!([]);
    }
    if let Some(list) = proof["checks"].as_array_mut() {
        list.push(json!({ "command": command, "exit_code": code, "ok": ok, "at_ms": now_ms() }));
    }
}

pub fn proof_view(job: &Job) -> Result<Value, (u16, String)> {
    if is_active(&job.state) {
        return Err((
            409,
            format!(
                "{ERR_JOBS}: job {} is {}; the proof is stamped when it ends",
                job.id, job.state
            ),
        ));
    }
    match &job.proof {
        Some(p) => Ok(json!({ "job_id": job.id, "state": job.state, "proof": p })),
        None => Err((404, format!("{ERR_JOBS}: job {} has no proof", job.id))),
    }
}

pub fn is_active(state: &str) -> bool {
    matches!(
        state,
        "queued" | "waiting_workspace" | "running" | "waiting_approval"
    )
}

pub const WORKTREE_TTL_MS: u64 = 7 * 24 * 3600 * 1000;
pub const WORKTREE_GC_EVERY_MS: u64 = 3600 * 1000;

pub fn worktree_gc_verdict(job: Option<&Job>, now: u64) -> Option<&'static str> {
    match job {
        None => Some("orphan"),
        Some(j) if is_active(&j.state) => None,
        Some(j) => {
            let ended = j.finished_ms.or(j.started_ms).unwrap_or(j.created_ms);
            (now.saturating_sub(ended) >= WORKTREE_TTL_MS).then_some("ttl")
        }
    }
}

async fn worktree_root(job: &Job) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    use omniget_core::core::vcs::job_worktree;
    let path = job.worktree_path.as_ref()?;
    let repo = match job.workspace.as_deref().filter(|w| !w.trim().is_empty()) {
        Some(ws) => job_worktree::repo_root(std::path::Path::new(ws)).await,
        None => None,
    };
    let repo = match repo {
        Some(r) => r,
        None => job_worktree::main_repo_of(std::path::Path::new(path)).await?,
    };
    let root = job_worktree::path_of(&repo, review::worktree_owner(job)).ok()?;
    Some((repo, root))
}

async fn remove_job_worktree(
    job: &Job,
) -> Result<omniget_core::core::vcs::job_worktree::RemoveOutcome, String> {
    remove_job_worktree_with(crate::job_workflow::store().as_deref(), job).await
}

pub(crate) async fn remove_job_worktree_with(
    store: Option<&omniget_core::core::llm::workflow::Store>,
    job: &Job,
) -> Result<omniget_core::core::vcs::job_worktree::RemoveOutcome, String> {
    use omniget_core::core::vcs::job_worktree;
    let (repo, root) = worktree_root(job)
        .await
        .ok_or_else(|| format!("{ERR_WORKTREE}: the job's repository is gone"))?;
    let branch = job
        .branch
        .clone()
        .unwrap_or_else(|| job_worktree::branch_of(&job.id));
    let hook = crate::job_workflow::before_remove(store, job, &repo, &root).await?;
    let mut out = job_worktree::remove(&repo, &root, &branch, job.base_oid.as_deref())
        .await
        .map_err(|e| format!("{ERR_WORKTREE}: {e}"))?;
    out.hook_note = (!hook.is_empty()).then_some(hook);
    Ok(out)
}

pub fn removal_note(out: &omniget_core::core::vcs::job_worktree::RemoveOutcome) -> String {
    let mut line = out.hook_note.clone().unwrap_or_default();
    line.push_str("⎇ worktree removed");
    if let Some(c) = &out.saved {
        line.push_str(&format!(
            "; {} uncommitted file(s) saved as {}",
            out.dirty.len(),
            c.chars().take(7).collect::<String>()
        ));
    }
    if let Some(b) = &out.preserved_branch {
        line.push_str(&format!(
            "; branch {b} kept (it has work not merged anywhere)"
        ));
    } else if let Some(b) = &out.branch_deleted {
        line.push_str(&format!("; branch {b} deleted"));
    }
    line.push('\n');
    line
}

pub fn worktree_note(wt: &omniget_core::core::vcs::job_worktree::JobWorktree) -> String {
    let short = |s: &str| s.chars().take(7).collect::<String>();
    let mut line = format!(
        "⎇ worktree {} on {} from {} in {} ms",
        wt.path.display(),
        wt.branch,
        short(&wt.base_oid),
        wt.took_ms
    );
    if !wt.carried.is_empty() {
        line.push_str(&format!(
            "; {} uncommitted file(s) of yours carried in (your checkout untouched)",
            wt.carried.len()
        ));
    }
    if !wt.shared.is_empty() {
        line.push_str(&format!("; linked {}", wt.shared.join(", ")));
    }
    if !wt.copied.is_empty() {
        line.push_str(&format!("; copied {}", wt.copied.join(", ")));
    }
    line.push('\n');
    line
}

const NO_WORKSPACE: &str = "no_workspace";

pub fn patch_path(job_id: &str) -> Option<std::path::PathBuf> {
    let name = sanitize_id(job_id);
    if name.is_empty() || name.contains("..") {
        return None;
    }
    omniget_core::core::llm::roster_store::llm_dir()
        .map(|d| d.join("jobs").join(format!("{name}.patch")))
}

pub fn apply_diff(
    job: &mut Job,
    out: Result<omniget_core::core::llm::snapshot::JobDiff, String>,
) -> Option<String> {
    use omniget_core::core::llm::snapshot::ERR_DIFF_UNAVAILABLE;
    match out {
        Ok(d) => {
            job.diff_files = Some(d.files);
            job.additions = Some(d.additions);
            job.deletions = Some(d.deletions);
            job.diff_clipped = d.clipped;
            job.diff_unavailable = None;
            job.head_oid = d.head_oid;
            Some(d.patch)
        }
        Err(e) => {
            job.diff_files = None;
            job.additions = None;
            job.deletions = None;
            job.diff_clipped = false;
            job.diff_unavailable = Some(if let Some(rest) = e.strip_prefix(ERR_DIFF_UNAVAILABLE) {
                format!("no_snapshot{rest}")
            } else if e.starts_with(NO_WORKSPACE) {
                e
            } else {
                format!("error: {e}")
            });
            None
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum DiffBody {
    Json(Value),
    Patch(String),
}

pub fn diff_view(
    job: &Job,
    patch: Option<&str>,
    patch_path: Option<&std::path::Path>,
    format: &str,
    complete: bool,
) -> Result<DiffBody, (u16, String)> {
    let unavailable = match (&job.diff_files, &job.diff_unavailable) {
        (_, Some(u)) => Some(u.clone()),
        (Some(_), None) => None,
        (None, None) => Some("no_snapshot: the diff of this job was not computed".into()),
    };
    let (reason, detail) = match unavailable.as_deref() {
        Some(u) => match u.split_once(": ") {
            Some((code, detail)) => (Some(code), Some(detail)),
            None => (Some(u), None),
        },
        None => (None, None),
    };
    match format {
        "" | "files" => {
            let files = job.diff_files.clone().unwrap_or_default();
            Ok(DiffBody::Json(json!({
                "job_id": job.id,
                "state": job.state,
                "available": unavailable.is_none(),
                "reason": reason,
                "detail": detail,
                "file_count": files.len(),
                "files": files,
                "additions": job.additions.unwrap_or(0),
                "deletions": job.deletions.unwrap_or(0),
                "clipped": job.diff_clipped,
                "complete": complete,
                "head_oid": job.head_oid,
                "patch_path": patch.and(patch_path).map(|p| p.to_string_lossy().to_string()),
            })))
        }
        "patch" => match (unavailable.is_some(), patch) {
            (true, _) => Err((
                409,
                format!(
                    "{}: {}",
                    omniget_core::core::llm::snapshot::ERR_DIFF_UNAVAILABLE,
                    detail.or(reason).unwrap_or("no diff")
                ),
            )),
            (false, Some(p)) => Ok(DiffBody::Patch(p.to_string())),
            (false, None) => Err((
                404,
                format!("{ERR_JOBS}: the patch file of job {} is gone", job.id),
            )),
        },
        other => Err((
            400,
            format!("{ERR_JOBS}: format is `files` or `patch`, not `{other}`"),
        )),
    }
}

#[cfg(test)]
mod diff_tests {
    use super::*;

    #[test]
    fn an_old_jobs_table_migrates_to_the_diff_columns() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE jobs (
                id TEXT PRIMARY KEY, kind TEXT NOT NULL, agent_id TEXT NOT NULL,
                conversation_id TEXT NOT NULL, prompt TEXT NOT NULL, workspace TEXT,
                state TEXT NOT NULL, loop_id TEXT, trigger_id TEXT, request_id TEXT,
                created_ms INTEGER NOT NULL, started_ms INTEGER, finished_ms INTEGER,
                result TEXT, error TEXT, log TEXT NOT NULL DEFAULT '');
             INSERT INTO jobs (id, kind, agent_id, conversation_id, prompt, state, created_ms)
                VALUES ('j0', 'run', 'a', 'c', 'p', 'done', 1);",
        )
        .unwrap();
        ensure_schema(&db, 0).unwrap();
        let job = db
            .query_row(
                &format!("SELECT {JOB_COLS} FROM jobs WHERE id='j0'"),
                [],
                row_to_job,
            )
            .unwrap();
        assert_eq!(job.diff_files, None);
        assert_eq!(
            job.diff_unavailable, None,
            "not computed yet, not 'unavailable'"
        );
        assert!(!job.diff_clipped);
    }
}

#[cfg(test)]
mod cron_tests {
    use super::*;
    use chrono::TimeZone;

    const MIN: i64 = 60_000;

    fn t0() -> i64 {
        chrono::Utc
            .with_ymd_and_hms(2026, 9, 26, 10, 0, 0)
            .unwrap()
            .timestamp_millis()
    }

    fn old_body() -> String {
        json!({"id": "t1", "name": "r", "kind": "cron", "cron": "*/1 * * * *", "agent_id": "a",
               "prompt": "p", "enabled": true, "last_fired_ms": null, "fire_count": 4, "created_ms": 1})
        .to_string()
    }

    #[test]
    fn an_old_trigger_is_migrated_from_now() {
        let path =
            std::env::temp_dir().join(format!("omniget-cron-old-{}.db", uuid::Uuid::new_v4()));
        let db = Connection::open(&path).unwrap();
        db.execute_batch("CREATE TABLE triggers (id TEXT PRIMARY KEY, body TEXT NOT NULL);")
            .unwrap();
        db.execute(
            "INSERT INTO triggers (id, body) VALUES ('t1', ?1)",
            params![old_body()],
        )
        .unwrap();
        ensure_schema(&db, t0() + 30_000).unwrap();
        let t = read_trigger(&db, "t1").unwrap();
        assert_eq!(
            t.next_run_ms,
            Some(t0() + MIN),
            "from now, not from the last firing"
        );
        assert_eq!(
            (t.catch_up, t.concurrency, t.fire_count),
            (CatchUp::Once, TriggerConcurrency::SkipIfActive, 4)
        );
        assert!(parse_zone(&t.tz).is_some());
        assert!(claim_due(&db, "t1", t0() + 40_000, 0, false).is_none());
        ensure_schema(&db, t0() + 50 * MIN).unwrap();
        assert_eq!(
            read_trigger(&db, "t1").unwrap().next_run_ms,
            Some(t0() + MIN)
        );
        let _ = std::fs::remove_file(path);
    }
}
