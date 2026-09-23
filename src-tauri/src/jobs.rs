//! The system around the agent: durable jobs, Loops and triggers.
//!
//! Every unit is an object with an id in SQLite (`<llm dir>/jobs.db`), never a
//! script: a **job** is one agent turn that outlives the window, a **Loop** is
//! a sequence of jobs on one conversation that stops on a criterion (rounds,
//! minutes, or a check command that exits 0), a **trigger** starts a job from a
//! cron line or from `POST /v1/hooks/<id>` on the local bridge. Shapes read
//! from `compozy/compozy` (durable sessions), `compozy/codex-loop` and
//! `compozy/cc-loop` (loop until the check passes). The cron parser is ours.
//!
//! State changes go out as `llm://job` and `llm://loop`; on boot whatever was
//! `queued` or `running` is picked up again.

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

pub const EVENT_JOB: &str = "llm://job";
pub const EVENT_LOOP: &str = "llm://loop";
pub const EVENT_PLAYBOOK: &str = "llm://playbook";
pub const ERR_JOBS: &str = "ERR_LLM_JOBS";

const MAX_PARALLEL_JOBS: usize = 2;
const LOG_MAX: usize = 64 * 1024;
const RESULT_MAX: usize = 32 * 1024;
const CRON_TICK_S: u64 = 30;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Job {
    pub id: String,
    /// `run` (CLI, UI, bridge), `chat` (a turn of the conversation tab),
    /// `loop` (one round of a Loop), `trigger`.
    pub kind: String,
    pub agent_id: String,
    pub conversation_id: String,
    pub prompt: String,
    pub workspace: Option<String>,
    /// `queued|running|waiting_approval|done|failed|cancelled`
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
    /// Summed over the model calls of the job: `{model, input_tokens,
    /// output_tokens, cache_read_tokens, cost_usd, calls}`. `cost_usd` is
    /// `null` when the provider has no price (local models, CLI accounts).
    #[serde(default)]
    pub usage: Option<serde_json::Value>,
    /// Run spec of a job whose `agent_id` is `tool:<id>` (a coding CLI run
    /// headless, maybe in a sandbox): [`RunSpec`] as JSON.
    #[serde(default)]
    pub spec: Option<serde_json::Value>,
    /// The playbook run this job is a step of.
    #[serde(default)]
    pub playbook_id: Option<String>,
}

/// How a `tool:<id>` job runs: the agent as the system prompt, model and
/// permission level of the CLI, and the optional sandbox.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RunSpec {
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Name of the agent used as the role (for the UI).
    #[serde(default)]
    pub agent_name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// `default | plan | accept_edits | bypass`
    #[serde(default)]
    pub permission: Option<String>,
    #[serde(default)]
    pub sandbox: Option<omniget_core::core::agentkit_run::sandbox::SandboxOpts>,
}

/// Prefix of the agent id of a job run by a coding CLI instead of the roster.
pub const TOOL_PREFIX: &str = "tool:";

/// Appends one prune receipt to a job's usage: per model request, what the
/// history was estimated at before and after the omissions, and what the
/// provider billed as input. Numbers as measured, nothing derived.
pub fn fold_receipt(total: &mut Option<serde_json::Value>, model: &str, event: &TurnEvent) {
    let TurnEvent::PruneReceipt {
        request,
        omitted,
        est_tokens_before,
        est_tokens_after,
        input_tokens,
    } = event
    else {
        return;
    };
    let t = total.get_or_insert_with(|| {
        serde_json::json!({
            "model": model, "input_tokens": 0, "output_tokens": 0, "cache_read_tokens": 0, "cost_usd": null, "calls": 0
        })
    });
    if !t["prune"].is_object() {
        t["prune"] = serde_json::json!({ "omitted": 0, "requests": [] });
    }
    t["prune"]["omitted"] =
        serde_json::json!((*omitted as u64).max(t["prune"]["omitted"].as_u64().unwrap_or(0)));
    if let Some(list) = t["prune"]["requests"].as_array_mut() {
        list.push(serde_json::json!({
            "request": request,
            "omitted": omitted,
            "est_tokens_before": est_tokens_before,
            "est_tokens_after": est_tokens_after,
            "input_tokens": input_tokens,
        }));
    }
}

/// Folds one `TurnEvent::Usage` into the running total of a job.
fn fold_usage(
    total: &mut Option<serde_json::Value>,
    model: &str,
    u: &omniget_core::core::llm::types::Usage,
) {
    let t = total.get_or_insert_with(|| serde_json::json!({
        "model": model, "input_tokens": 0, "output_tokens": 0, "cache_read_tokens": 0, "cost_usd": null, "calls": 0
    }));
    let add = |t: &mut serde_json::Value, k: &str, n: u64| {
        t[k] = serde_json::json!(t[k].as_u64().unwrap_or(0) + n)
    };
    add(t, "input_tokens", u.input_tokens as u64);
    add(t, "output_tokens", u.output_tokens as u64);
    add(t, "cache_read_tokens", u.cache_read_tokens as u64);
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
    /// `running|done|failed|cancelled`
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
    /// Rounds spaced by an interval (`10m`, `daily`) or a cron line
    /// (`*/15 * * * *`); `None` = back to back.
    #[serde(default)]
    pub schedule: Option<String>,
    /// Stop when the rounds together cost this much (USD).
    #[serde(default)]
    pub max_cost_usd: Option<f64>,
    #[serde(default)]
    pub spent_usd: f64,
    /// [`RunSpec`] of each round when `agent_id` is `tool:<id>`.
    #[serde(default)]
    pub spec: Option<serde_json::Value>,
    /// Catalog id of the loop it came from.
    #[serde(default)]
    pub source: Option<String>,
    /// When the next scheduled round starts.
    #[serde(default)]
    pub next_round_ms: Option<u64>,
    /// Time spent inside rounds (the minute budget of a scheduled loop).
    #[serde(default)]
    pub active_ms: u64,
}

/// One step of a running playbook.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlaybookStepRun {
    pub name: String,
    pub agent_id: String,
    /// The step's own task (the chained context is added at run time).
    pub prompt: String,
    #[serde(default)]
    pub spec: Option<serde_json::Value>,
    #[serde(default)]
    pub runner_label: Option<String>,
    #[serde(default)]
    pub job_id: Option<String>,
    /// `pending|running|done|failed|cancelled|skipped`
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub output: Option<String>,
}

/// A workflow run as a chain of jobs: the output of each step becomes the
/// context of the next.
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
    /// `running|done|failed|cancelled`
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
    /// `cron|webhook`
    pub kind: String,
    #[serde(default)]
    pub cron: Option<String>,
    pub agent_id: String,
    /// `{{body}}` is replaced by the webhook body.
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
}

fn yes() -> bool {
    true
}

pub struct Jobs {
    db: Mutex<Connection>,
    app: AppHandle,
    llm: Arc<LlmManager>,
    slots: tokio::sync::Semaphore,
    cancels: Mutex<HashMap<String, CancellationToken>>,
    cron_running: AtomicBool,
    /// Wakes a Loop sleeping until its next scheduled round when it is cancelled.
    loop_waits: Mutex<HashMap<String, CancellationToken>>,
}

static JOBS: OnceLock<Arc<Jobs>> = OnceLock::new();

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

/// The one instance. The first caller opens the database and resumes what the
/// last run left behind.
pub fn get(app: &AppHandle) -> Result<Arc<Jobs>, String> {
    if let Some(j) = JOBS.get() {
        return Ok(j.clone());
    }
    let dir = omniget_core::core::llm::roster_store::llm_dir()
        .ok_or_else(|| format!("{ERR_JOBS}: no app data dir"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{ERR_JOBS}: {e}"))?;
    let db = Connection::open(dir.join("jobs.db")).map_err(|e| format!("{ERR_JOBS}: {e}"))?;
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
    // Databases created before 0.10.0 final have no usage column.
    let _ = db.execute("ALTER TABLE jobs ADD COLUMN usage TEXT", []);
    // Catalog runs (0.11): the tool run spec and the playbook of a step.
    let _ = db.execute("ALTER TABLE jobs ADD COLUMN spec TEXT", []);
    let _ = db.execute("ALTER TABLE jobs ADD COLUMN playbook_id TEXT", []);
    crate::commands::llm::ensure_wired(app);
    let jobs = Arc::new(Jobs {
        db: Mutex::new(db),
        app: app.clone(),
        llm: app.state::<crate::AppState>().llm.clone(),
        slots: tokio::sync::Semaphore::new(MAX_PARALLEL_JOBS),
        cancels: Mutex::new(HashMap::new()),
        cron_running: AtomicBool::new(false),
        loop_waits: Mutex::new(HashMap::new()),
    });
    if JOBS.set(jobs.clone()).is_ok() {
        jobs.clone().resume();
        jobs.clone().ensure_cron();
    }
    Ok(JOBS.get().cloned().unwrap_or(jobs))
}

/// Called from the app setup so a Loop left running comes back without anyone
/// opening `/llm`.
pub fn boot(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        if let Err(e) = get(&app) {
            tracing::warn!("[jobs] boot: {e}");
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
        spec: r
            .get::<_, Option<String>>(17)?
            .and_then(|s| serde_json::from_str(&s).ok()),
        playbook_id: r.get(18)?,
    })
}

const JOB_COLS: &str = "id, kind, agent_id, conversation_id, prompt, workspace, state, loop_id, trigger_id, request_id, created_ms, started_ms, finished_ms, result, error, log, usage, spec, playbook_id";

impl Jobs {
    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ── jobs ─────────────────────────────────────────────────────────

    fn save_job(&self, job: &Job) {
        let r = self.db().execute(
            &format!("INSERT OR REPLACE INTO jobs ({JOB_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19)"),
            params![
                job.id, job.kind, job.agent_id, job.conversation_id, job.prompt, job.workspace,
                job.state, job.loop_id, job.trigger_id, job.request_id, job.created_ms as i64,
                job.started_ms.map(|v| v as i64), job.finished_ms.map(|v| v as i64),
                job.result, job.error, job.log, job.usage.as_ref().map(|u| u.to_string()),
                job.spec.as_ref().map(|u| u.to_string()), job.playbook_id
            ],
        );
        if let Err(e) = r {
            tracing::warn!("[jobs] save {}: {e}", job.id);
        }
        // The list view does not need the log on every event.
        let mut light = job.clone();
        light.log = clip(&light.log, 4096);
        let _ = self.app.emit(EVENT_JOB, &light);
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
        Ok(Job {
            conversation_id: conversation_id.unwrap_or_else(|| format!("job-{id}")),
            id,
            kind: kind.to_string(),
            agent_id: agent_id.to_string(),
            prompt: prompt.to_string(),
            workspace,
            state: "queued".into(),
            created_ms: now_ms(),
            ..Default::default()
        })
    }

    /// Queue one turn and return at once.
    pub fn submit(
        self: &Arc<Self>,
        kind: &str,
        agent_id: &str,
        prompt: &str,
        workspace: Option<String>,
        conversation_id: Option<String>,
        trigger_id: Option<String>,
    ) -> Result<Job, String> {
        let mut job = self.new_job(kind, agent_id, prompt, workspace, conversation_id)?;
        job.trigger_id = trigger_id;
        self.save_job(&job);
        let this = self.clone();
        let id = job.id.clone();
        tauri::async_runtime::spawn(async move {
            this.run_job(&id, None).await;
        });
        Ok(job)
    }

    /// Like [`Jobs::submit`] with a run spec (tool runners, sandbox).
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

    pub fn cancel(&self, id: &str) -> Result<Job, String> {
        let mut job = self
            .job(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no job {id}"))?;
        if let Some(token) = self
            .cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
        {
            token.cancel();
        }
        // Also through the manager, which drops the turn's pending questions.
        if let Some(request) = job.request_id.as_deref() {
            let _ = self.llm.cancel(request);
        }
        if job.state == "queued" {
            job.state = "cancelled".into();
            job.finished_ms = Some(now_ms());
            self.save_job(&job);
        }
        Ok(job)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let _ = self.cancel(id);
        self.db()
            .execute("DELETE FROM jobs WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// A turn of the conversation tab, mirrored as a job so `/llm/jobs` shows
    /// everything the agents did.
    pub fn chat_started(
        &self,
        agent_id: &str,
        conversation_id: &str,
        prompt: &str,
        request_id: &str,
    ) -> String {
        let id = short_id("j");
        let job = Job {
            id: id.clone(),
            kind: "chat".into(),
            agent_id: agent_id.into(),
            conversation_id: conversation_id.into(),
            prompt: clip(prompt, 4000),
            workspace: code_tools::workspace_of(&sanitize_id(conversation_id))
                .map(|p| p.to_string_lossy().to_string()),
            state: "running".into(),
            request_id: Some(request_id.into()),
            created_ms: now_ms(),
            started_ms: Some(now_ms()),
            ..Default::default()
        };
        self.save_job(&job);
        id
    }

    pub fn chat_finished(
        &self,
        id: &str,
        text: &str,
        error: Option<String>,
        cancelled: bool,
        usage: &[omniget_core::core::llm::types::Usage],
        receipts: &[TurnEvent],
    ) {
        let Some(mut job) = self.job(id) else { return };
        let model = self.llm.model_label(&job.agent_id);
        for u in usage {
            fold_usage(&mut job.usage, &model, u);
        }
        for r in receipts {
            fold_receipt(&mut job.usage, &model, r);
        }
        job.state = if cancelled {
            "cancelled"
        } else if error.is_some() && text.is_empty() {
            "failed"
        } else {
            "done"
        }
        .into();
        job.result = Some(clip(text, RESULT_MAX));
        job.error = error;
        job.finished_ms = Some(now_ms());
        self.save_job(&job);
    }

    /// Runs one job to the end. `resume_note` replaces the prompt when the job
    /// was already running when the app died.
    async fn run_job(self: &Arc<Self>, id: &str, resume_note: Option<String>) -> Option<Job> {
        let _slot = self.slots.acquire().await.ok()?;
        let mut job = self.job(id)?;
        if job.state == "cancelled" {
            return Some(job);
        }
        let conv = sanitize_id(&job.conversation_id);
        if let Some(ws) = job.workspace.clone().filter(|w| !w.is_empty()) {
            if let Err(e) = code_tools::set_conversation_workspace(&conv, Some(ws.into())) {
                job.state = "failed".into();
                job.error = Some(e);
                job.finished_ms = Some(now_ms());
                self.save_job(&job);
                return Some(job);
            }
        }
        job.state = "running".into();
        job.started_ms = Some(now_ms());
        self.save_job(&job);

        let input = resume_note.unwrap_or_else(|| job.prompt.clone());
        if job.agent_id.starts_with(TOOL_PREFIX) {
            return Some(self.run_tool_job(job, input).await);
        }
        let (request_id, cancel, mut stream) =
            match self.llm.turn_stream(&conv, &job.agent_id, &input).await {
                Ok(v) => v,
                Err(e) => {
                    job.state = "failed".into();
                    job.error = Some(e);
                    job.finished_ms = Some(now_ms());
                    self.save_job(&job);
                    return Some(job);
                }
            };
        job.request_id = Some(request_id.clone());
        self.cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(job.id.clone(), cancel.clone());
        self.save_job(&job);

        let mut text = String::new();
        let mut error: Option<String> = None;
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            tokio::select! {
                event = stream.next() => {
                    let Some(event) = event else { break };
                    self.llm.note_event(&job.agent_id, &event);
                    match event {
                        TurnEvent::TextDelta { text: t } => text.push_str(&t),
                        TurnEvent::ToolCallStart { name, .. } => {
                            job.log.push_str(&format!("→ {name}\n"));
                            job.log = clip(&job.log, LOG_MAX);
                            self.save_job(&job);
                        }
                        receipt @ TurnEvent::PruneReceipt { .. } => {
                            let model = self.llm.model_label(&job.agent_id);
                            fold_receipt(&mut job.usage, &model, &receipt);
                        }
                        TurnEvent::Usage { usage } => {
                            let model = self.llm.model_label(&job.agent_id);
                            fold_usage(&mut job.usage, &model, &usage);
                        }
                        TurnEvent::Error { error: e } => {
                            job.log.push_str(&format!("! {}: {}\n", e.code, e.message));
                            error = Some(format!("{}: {}", e.code, e.message));
                        }
                        _ => {}
                    }
                }
                _ = tick.tick() => {
                    let waiting = self.llm.pending_ask_list().iter().any(|a| a["request_id"] == request_id.as_str());
                    let want = if waiting { "waiting_approval" } else { "running" };
                    if job.state != want {
                        job.state = want.into();
                        self.save_job(&job);
                    }
                }
            }
        }
        self.llm.finish_turn(&request_id, &job.agent_id);
        self.cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&job.id);

        job.state = if cancel.is_cancelled() {
            "cancelled"
        } else if error.is_some() && text.trim().is_empty() {
            "failed"
        } else {
            "done"
        }
        .into();
        job.result = Some(clip(&text, RESULT_MAX));
        job.error = error;
        job.finished_ms = Some(now_ms());
        self.save_job(&job);
        Some(job)
    }

    // ── loops ────────────────────────────────────────────────────────

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
            // A Loop with no way out is a bug waiting for a bill.
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
        def.active_ms = 0;
        def.next_round_ms = None;
        def.created_ms = now_ms();
        def.finished_ms = None;
        self.save_loop(&def);
        let this = self.clone();
        let id = def.id.clone();
        tauri::async_runtime::spawn(async move { this.run_loop(id).await });
        Ok(def)
    }

    pub fn loop_cancel(&self, id: &str) -> Result<LoopDef, String> {
        let mut l = self
            .loop_get(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no loop {id}"))?;
        if l.state == "running" {
            l.state = "cancelled".into();
            l.stop_reason = Some("cancelled".into());
            l.finished_ms = Some(now_ms());
            self.save_loop(&l);
        }
        if let Some(t) = self
            .loop_waits
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id)
        {
            t.cancel();
        }
        let running: Vec<String> = self
            .list(200)
            .into_iter()
            .filter(|j| {
                j.loop_id.as_deref() == Some(id)
                    && matches!(j.state.as_str(), "queued" | "running" | "waiting_approval")
            })
            .map(|j| j.id)
            .collect();
        for j in running {
            let _ = self.cancel(&j);
        }
        Ok(l)
    }

    pub fn loop_delete(&self, id: &str) -> Result<(), String> {
        let _ = self.loop_cancel(id);
        self.db()
            .execute("DELETE FROM loops WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
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
        // A scheduled loop counts the minutes spent in rounds, not the wall clock.
        let deadline = if l.schedule.is_some() {
            None
        } else {
            l.max_minutes.map(|m| l.created_ms + u64::from(m) * 60_000)
        };
        if let Some(ws) = l.workspace.clone().filter(|w| !w.is_empty()) {
            // The check may run before the first round sets the workspace.
            let _ = code_tools::set_conversation_workspace(
                &sanitize_id(&l.conversation_id),
                Some(ws.into()),
            );
        }
        // Already green? Then there is nothing to do.
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
            // The row is the source of truth: a cancel lands there.
            match self.loop_get(&id) {
                Some(cur) if cur.state == "running" => {}
                _ => return,
            }
            if l.max_rounds.map(|m| l.rounds_done >= m).unwrap_or(false) {
                l.stop_reason = Some("max_rounds".into());
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
            let prompt = if round == 1 {
                match &l.check_command {
                    Some(c) if !c.trim().is_empty() => format!("{}\n\nYou are done when this command exits 0: `{c}`. Run it yourself before you stop.", l.prompt),
                    _ => l.prompt.clone(),
                }
            } else {
                match &l.last_check {
                    Some(out) => format!("Round {round}. The check still fails:\n```\n{out}\n```\nKeep working on the task: {}", l.prompt),
                    None => format!("Round {round}. Continue the task; say DONE on a line of its own when nothing is left. Task: {}", l.prompt),
                }
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
            job.spec = l.spec.clone();
            self.save_job(&job);
            let round_started = now_ms();
            let done = self.run_job(&job.id, None).await;
            l.rounds_done = round;
            l.active_ms += now_ms().saturating_sub(round_started);
            l.next_round_ms = l
                .schedule
                .as_deref()
                .and_then(|s| next_round_at(s, now_ms()));
            let Some(done) = done else { break };
            if let Some(c) = done.usage.as_ref().and_then(|u| u["cost_usd"].as_f64()) {
                l.spent_usd += c;
            }
            match done.state.as_str() {
                "cancelled" => {
                    l.state = "cancelled".into();
                    l.stop_reason = Some("cancelled".into());
                    break;
                }
                "failed" => {
                    // Budget cut, no provider, no candidate: another round would fail the same way.
                    l.state = "failed".into();
                    l.stop_reason = done.error.clone();
                    break;
                }
                _ => {}
            }
            match self.run_check(&l).await {
                Some((true, out)) => {
                    l.last_check = Some(out);
                    l.stop_reason = Some("check_passed".into());
                    break;
                }
                Some((false, out)) => l.last_check = Some(out),
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
        // A cancel may have landed while the last round ran.
        if let Some(cur) = self.loop_get(&id) {
            if cur.state == "cancelled" {
                l.state = "cancelled".into();
                l.stop_reason = Some("cancelled".into());
            }
        }
        l.finished_ms = Some(now_ms());
        self.save_loop(&l);
    }

    /// Sleeps until `at_ms`; `false` when the loop was cancelled meanwhile.
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

    // ── tool runs (coding CLIs headless, maybe in a sandbox) ─────────

    /// Runs a `tool:<id>` job: the CLI from the tool manifest's `[runner]`,
    /// the agent as the system prompt, locally or in the sandbox of the spec.
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
        // Log lines land in the row at most once per 750 ms.
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
                // A CLI that failed (auth, crash) may still print its error as
                // the result: a non-zero exit with an error is a failure.
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

    // ── playbooks (workflows as chained jobs) ────────────────────────

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

    /// Starts a playbook whose steps are already resolved to runners.
    pub fn playbook_start(self: &Arc<Self>, mut p: PlaybookRun) -> Result<PlaybookRun, String> {
        if p.steps.is_empty() {
            return Err(format!("{ERR_JOBS}: a playbook needs at least one step"));
        }
        for s in &p.steps {
            if let Some(tool) = s.agent_id.strip_prefix(TOOL_PREFIX) {
                omniget_core::core::agentkit_run::runner::runner_of(tool)?;
            } else if self.llm.agent(&s.agent_id).is_none() {
                return Err(format!(
                    "{ERR_JOBS}: step `{}`: no agent `{}`",
                    s.name, s.agent_id
                ));
            }
        }
        p.id = short_id("p");
        p.state = "running".into();
        p.current = 0;
        p.created_ms = now_ms();
        p.finished_ms = None;
        for s in &mut p.steps {
            s.state = "pending".into();
            s.job_id = None;
            s.output = None;
        }
        self.save_playbook(&p);
        let this = self.clone();
        let id = p.id.clone();
        tauri::async_runtime::spawn(async move { this.run_playbook(id).await });
        Ok(p)
    }

    pub fn playbook_cancel(&self, id: &str) -> Result<PlaybookRun, String> {
        let mut p = self
            .playbook_get(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no playbook {id}"))?;
        if p.state == "running" {
            p.state = "cancelled".into();
            p.finished_ms = Some(now_ms());
            for s in &mut p.steps {
                if s.state == "pending" {
                    s.state = "skipped".into();
                }
            }
            self.save_playbook(&p);
        }
        for s in &p.steps {
            if let Some(j) = &s.job_id {
                let _ = self.cancel(j);
            }
        }
        Ok(p)
    }

    pub fn playbook_delete(&self, id: &str) -> Result<(), String> {
        let _ = self.playbook_cancel(id);
        self.db()
            .execute("DELETE FROM playbooks WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
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
            if done.state != "done" {
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

    // ── boot ─────────────────────────────────────────────────────────

    fn resume(self: Arc<Self>) {
        let open: Vec<Job> = self
            .list(500)
            .into_iter()
            .filter(|j| matches!(j.state.as_str(), "queued" | "running" | "waiting_approval"))
            .collect();
        for stale in open {
            let Some(mut job) = self.job(&stale.id) else {
                continue;
            };
            if job.kind == "chat" || job.loop_id.is_some() || job.playbook_id.is_some() {
                // The Loop itself resumes below with a fresh round.
                job.state = "failed".into();
                job.error = Some("interrupted: the app closed during this turn".into());
                job.finished_ms = Some(now_ms());
                self.save_job(&job);
                continue;
            }
            let note = (job.state != "queued").then(|| {
                format!("The app restarted in the middle of this task. Check what is already done and finish it. Task: {}", job.prompt)
            });
            let this = self.clone();
            tauri::async_runtime::spawn(async move {
                this.run_job(&job.id, note).await;
            });
        }
        for l in self.loops().into_iter().filter(|l| l.state == "running") {
            let this = self.clone();
            tauri::async_runtime::spawn(async move { this.run_loop(l.id).await });
        }
        // A playbook resumes at the step that was running (it starts again).
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

    // ── triggers ─────────────────────────────────────────────────────

    pub fn triggers(&self) -> Vec<Trigger> {
        let db = self.db();
        let Ok(mut stmt) = db.prepare("SELECT body FROM triggers") else {
            return Vec::new();
        };
        let mut out: Vec<Trigger> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map(|rows| {
                rows.filter_map(Result::ok)
                    .filter_map(|b| serde_json::from_str(&b).ok())
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by_key(|j| j.created_ms);
        out
    }

    pub fn trigger(&self, id: &str) -> Option<Trigger> {
        self.triggers().into_iter().find(|t| t.id == id)
    }

    fn store_trigger(&self, t: &Trigger) {
        if let Ok(body) = serde_json::to_string(t) {
            let _ = self.db().execute(
                "INSERT OR REPLACE INTO triggers (id, body) VALUES (?1, ?2)",
                params![t.id, body],
            );
        }
    }

    pub fn trigger_save(self: &Arc<Self>, mut t: Trigger) -> Result<Trigger, String> {
        if self.llm.agent(&t.agent_id).is_none() {
            return Err(format!("{ERR_JOBS}: no agent `{}`", t.agent_id));
        }
        match t.kind.as_str() {
            "cron" => {
                Cron::parse(t.cron.as_deref().unwrap_or(""))?;
            }
            "webhook" => t.cron = None,
            other => return Err(format!("{ERR_JOBS}: unknown trigger kind `{other}`")),
        }
        if t.id.is_empty() {
            t.id = short_id("t");
            t.created_ms = now_ms();
        } else if let Some(old) = self.trigger(&t.id) {
            t.created_ms = old.created_ms;
            t.fire_count = old.fire_count;
            t.last_fired_ms = old.last_fired_ms;
        }
        if t.name.trim().is_empty() {
            t.name = t.prompt.chars().take(48).collect();
        }
        self.store_trigger(&t);
        self.clone().ensure_cron();
        Ok(t)
    }

    pub fn trigger_delete(&self, id: &str) -> Result<(), String> {
        self.db()
            .execute("DELETE FROM triggers WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn fire(self: &Arc<Self>, id: &str, body: Option<&str>) -> Result<Job, String> {
        let mut t = self
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
        let job = self.submit(
            "trigger",
            &t.agent_id,
            &prompt,
            t.workspace.clone(),
            None,
            Some(t.id.clone()),
        )?;
        t.last_fired_ms = Some(now_ms());
        t.fire_count += 1;
        self.store_trigger(&t);
        Ok(job)
    }

    /// The ticker only exists while some cron trigger is enabled.
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
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(CRON_TICK_S)).await;
                if !has(&self) {
                    self.cron_running.store(false, Ordering::SeqCst);
                    return;
                }
                let now = chrono::Local::now();
                let minute = now.timestamp() / 60;
                for t in self
                    .triggers()
                    .into_iter()
                    .filter(|t| t.enabled && t.kind == "cron")
                {
                    let fired_this_minute = t
                        .last_fired_ms
                        .map(|ms| (ms / 60_000) as i64 == minute)
                        .unwrap_or(false);
                    let due = Cron::parse(t.cron.as_deref().unwrap_or(""))
                        .map(|c| c.matches(&now))
                        .unwrap_or(false);
                    if due && !fired_this_minute {
                        if let Err(e) = self.fire(&t.id, None) {
                            tracing::warn!("[jobs] cron {}: {e}", t.id);
                        }
                    }
                }
            }
        });
    }
}

// ── cron: five fields, our own parser ─────────────────────────────────

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
        // Vixie rule: when both day fields are restricted, either one fires.
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

/// When the next round of a scheduled Loop starts: a 5-field cron line (the
/// next matching minute, local time) or an interval (`10m`, `2h`, `daily`).
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

/// `{ base_url, token }` of the local bridge, for the webhook URL in the UI.
pub fn bridge_info(app: &AppHandle) -> Value {
    let settings = crate::storage::config::load_settings(app);
    json!({
        "base_url": format!("http://127.0.0.1:{}", settings.bridge.port),
        "token": settings.bridge.token,
    })
}
