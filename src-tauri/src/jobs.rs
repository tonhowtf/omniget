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
//! State changes go out as `llm://job` and `llm://loop`. On boot nothing
//! whose outcome is unknown is re-sent: a job or Loop that was running
//! becomes `interrupted` and waits for a person (resume, mark done,
//! discard); only a job that never left the queue starts by itself. There is
//! one scheduler (the cron ticker here), and a routine says plainly that it
//! needs the app open.

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
    /// `queued|running|waiting_approval|done|failed|cancelled|interrupted`
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
}

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
        ..
    } = event
    else {
        return;
    };
    let t = total.get_or_insert_with(|| {
        serde_json::json!({
            "model": model, "input_tokens": 0, "output_tokens": 0, "cache_read_tokens": 0, "cache_write_tokens": 0, "cost_usd": null, "calls": 0
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

/// Tokens a cap counts for a job's folded usage: input weighted by price
/// (cache read 0.1x, cache write 1.25x) plus output.
pub fn billable_of(u: &serde_json::Value) -> u64 {
    let n = |k: &str| u[k].as_u64().unwrap_or(0);
    omniget_core::core::llm::types::billable_input(
        n("input_tokens"),
        n("cache_read_tokens"),
        n("cache_write_tokens"),
    ) + n("output_tokens")
}

/// Folds one `TurnEvent::Usage` into the running total of a job.
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
    /// `running|done|failed|cancelled|interrupted`
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
    /// Silenced: runs, but never notifies.
    #[serde(default)]
    pub muted: bool,
    /// Digest of the last result that was notified: the same result again
    /// does not notify again.
    #[serde(default)]
    pub last_digest: Option<String>,
    #[serde(default)]
    pub last_notified_ms: Option<u64>,
    /// When set, firing starts a mission with these criteria (the prompt is
    /// its objective) instead of a plain job: the one scheduler starts
    /// missions too, there is no second queue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mission: Option<Value>,
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
    guards: Mutex<HashMap<String, JobGuard>>,
}

/// Per-request usage check of a job: `(tokens so far, known usd so far)`;
/// `Err(reason)` stops the job.
pub type UsageCheck = Arc<dyn Fn(u64, Option<f64>) -> Result<(), String> + Send + Sync>;

/// Limits a mission puts on one of its jobs while it runs (F7): a caller
/// check after every model request, and a wall-clock deadline. A job that
/// hits either ends `failed` with the reason (never `done`).
#[derive(Clone, Default)]
pub struct JobGuard {
    /// Epoch milliseconds after which the job is stopped.
    pub deadline_ms: Option<u64>,
    pub check: Option<UsageCheck>,
    /// Text of the error when the deadline is reached.
    pub deadline_reason: String,
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
         CREATE TABLE IF NOT EXISTS triggers (id TEXT PRIMARY KEY, body TEXT NOT NULL);",
    )
    .map_err(|e| format!("{ERR_JOBS}: {e}"))?;
    // Databases created before 0.10.0 final have no usage column.
    let _ = db.execute("ALTER TABLE jobs ADD COLUMN usage TEXT", []);
    crate::commands::llm::ensure_wired(app);
    let jobs = Arc::new(Jobs {
        db: Mutex::new(db),
        app: app.clone(),
        llm: app.state::<crate::AppState>().llm.clone(),
        slots: tokio::sync::Semaphore::new(MAX_PARALLEL_JOBS),
        cancels: Mutex::new(HashMap::new()),
        cron_running: AtomicBool::new(false),
        guards: Mutex::new(HashMap::new()),
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
    // The durable run record first: reconcile what the last session left in
    // flight before anything new can start.
    crate::commands::assist::runs::install(app);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        if let Err(e) = get(&app) {
            tracing::warn!("[jobs] boot: {e}");
        } else {
            // Missions after jobs: their reconciliation reads job outcomes.
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
    })
}

const JOB_COLS: &str = "id, kind, agent_id, conversation_id, prompt, workspace, state, loop_id, trigger_id, request_id, created_ms, started_ms, finished_ms, result, error, log, usage";

impl Jobs {
    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ── jobs ─────────────────────────────────────────────────────────

    fn save_job_checked(&self, job: &Job) -> Result<(), String> {
        let r = self.db().execute(
            &format!("INSERT OR REPLACE INTO jobs ({JOB_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17)"),
            params![
                job.id, job.kind, job.agent_id, job.conversation_id, job.prompt, job.workspace,
                job.state, job.loop_id, job.trigger_id, job.request_id, job.created_ms as i64,
                job.started_ms.map(|v| v as i64), job.finished_ms.map(|v| v as i64),
                job.result, job.error, job.log, job.usage.as_ref().map(|u| u.to_string())
            ],
        );
        r.map_err(|_| "JOB_STORAGE_UNAVAILABLE".to_owned())?;
        // The list view does not need the log on every event.
        let mut light = job.clone();
        light.log = clip(&light.log, 4096);
        let _ = self.app.emit(EVENT_JOB, &light);
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
        if self.llm.agent(agent_id).is_none() {
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

    /// Creates a job without running it, so a mission can record the job id
    /// (its effect key) before anything is dispatched.
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

    /// Runs a prepared job to the end and returns it (missions await it).
    pub async fn run_prepared(self: &Arc<Self>, id: &str) -> Option<Job> {
        self.run_job(id, None).await
    }

    /// [`Self::run_prepared`] under a mission's [`JobGuard`].
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

    pub fn cancel(&self, id: &str) -> Result<Job, String> {
        let mut job = self
            .job(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no job {id}"))?;
        // Persist before consulting the token. The runner either sees this
        // cancelled row on its re-read or has already registered its token.
        if job.state == "queued" {
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
        // Also through the manager, which drops the turn's pending questions.
        if let Some(request) = job.request_id.as_deref() {
            let _ = self.llm.cancel(request);
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
                self.save_job_checked(&job).ok()?;
                return Some(job);
            }
        }
        let cancel = CancellationToken::new();
        self.cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(job.id.clone(), cancel.clone());
        // Cancellation may have arrived after the initial read but before
        // registration. Re-read its durable state before dispatching anything.
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
        job.state = "running".into();
        job.started_ms = Some(now_ms());
        if self.save_job_checked(&job).is_err() {
            cancel.cancel();
            self.cancels
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&job.id);
            return None;
        }
        let input = resume_note.unwrap_or_else(|| job.prompt.clone());
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
        let mut storage_failed = self.save_job_checked(&job).is_err();
        if storage_failed {
            cancel.cancel();
        }

        let mut text = String::new();
        let mut saw_finished = false;
        let mut error: Option<String> = None;
        let guard = self
            .guards
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(id)
            .cloned();
        // Set when the mission's guard stopped the job (cap or deadline).
        let mut limit_hit: Option<String> = None;
        // Input tokens of the model requests so far (prune receipts), for the
        // guard's check between requests; the exact total arrives as `Usage`.
        let mut est_input: u64 = 0;
        let check = guard.as_ref().and_then(|g| g.check.clone());
        // The guard's view of the job so far: the reported usage when there is
        // one, else the request inputs plus the text streamed (chars / 4).
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
        loop {
            tokio::select! {
                event = stream.next() => {
                    let Some(event) = event else { break };
                    self.llm.note_event(&job.agent_id, &event);
                    match event {
                        TurnEvent::Finished { .. } => saw_finished = true,
                        TurnEvent::TextDelta { text: t } => text.push_str(&t),
                        TurnEvent::ToolCallStart { name, .. } => {
                            job.log.push_str(&format!("→ {name}\n"));
                            job.log = clip(&job.log, LOG_MAX);
                            if self.save_job_checked(&job).is_err() { storage_failed = true; cancel.cancel(); }
                            // A tool call ends a model request: check the cap
                            // before the next one is sent (F7).
                            if let (Some(check), None) = (&check, &limit_hit) {
                                let (tokens, usd) = spent_so_far(&job.usage, est_input, &text);
                                if let Err(why) = check(tokens, usd) {
                                    job.log.push_str(&format!("! {why}\n"));
                                    limit_hit = Some(why);
                                    cancel.cancel();
                                }
                            }
                        }
                        receipt @ TurnEvent::PruneReceipt { .. } => {
                            if let TurnEvent::PruneReceipt { input_tokens, billable_input_tokens, est_tokens_after, .. } = &receipt {
                                // Weighted input when the receipt has it (cache 0.1x/1.25x).
                                est_input += billable_input_tokens.or(*input_tokens).unwrap_or(*est_tokens_after) as u64;
                            }
                            let model = self.llm.model_label(&job.agent_id);
                            fold_receipt(&mut job.usage, &model, &receipt);
                            if let (Some(check), None) = (&check, &limit_hit) {
                                let (tokens, usd) = spent_so_far(&job.usage, est_input, &text);
                                if let Err(why) = check(tokens, usd) {
                                    job.log.push_str(&format!("! {why}\n"));
                                    limit_hit = Some(why);
                                    cancel.cancel();
                                }
                            }
                        }
                        TurnEvent::Usage { usage } => {
                            let model = self.llm.model_label(&job.agent_id);
                            fold_usage(&mut job.usage, &model, &usage);
                            // Per model request: the mission's cap holds
                            // during the job, not only before it (F7).
                            if let (Some(check), None) = (&check, &limit_hit) {
                                let u = job.usage.clone().unwrap_or_default();
                                let tokens = billable_of(&u);
                                if let Err(why) = check(tokens, u["cost_usd"].as_f64()) {
                                    job.log.push_str(&format!("! {why}\n"));
                                    limit_hit = Some(why);
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
                _ = tick.tick() => {
                    if let (Some(deadline), None) = (guard.as_ref().and_then(|g| g.deadline_ms), &limit_hit) {
                        if now_ms() > deadline {
                            let why = guard.as_ref().map(|g| g.deadline_reason.clone()).filter(|r| !r.is_empty()).unwrap_or_else(|| "the time limit was reached".into());
                            job.log.push_str(&format!("! {why}\n"));
                            limit_hit = Some(why);
                            cancel.cancel();
                        }
                    }
                    let waiting = self.llm.pending_ask_list().iter().any(|a| a["request_id"] == request_id.as_str());
                    let want = if waiting { "waiting_approval" } else { "running" };
                    if job.state != want {
                        job.state = want.into();
                        if self.save_job_checked(&job).is_err() { storage_failed = true; cancel.cancel(); }
                    }
                }
            }
        }
        self.llm.finish_turn(&request_id, &job.agent_id);
        self.cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&job.id);

        if storage_failed || !saw_finished {
            cancel.cancel();
            return None;
        }
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
        job.finished_ms = Some(now_ms());
        self.save_job_checked(&job).ok()?;
        if let Some(tid) = job.trigger_id.clone() {
            self.notify_routine(&tid, &job);
        }
        Some(job)
    }

    /// A routine's result reaches the user only when it changed and the
    /// routine is not silenced.
    fn notify_routine(&self, trigger_id: &str, job: &Job) {
        let Some(mut t) = self.trigger(trigger_id) else {
            return;
        };
        let body = match job.state.as_str() {
            "done" => job.result.clone().unwrap_or_default(),
            "cancelled" => return,
            _ => format!("error: {}", job.error.clone().unwrap_or_default()),
        };
        let Some(digest) = routine_digest(&t, &body) else {
            return;
        };
        t.last_digest = Some(digest);
        t.last_notified_ms = Some(now_ms());
        self.store_trigger(&t);
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
        if self.llm.agent(&def.agent_id).is_none() {
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
        def.rounds_done = 0;
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
        let deadline = l.max_minutes.map(|m| l.created_ms + u64::from(m) * 60_000);
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
            self.save_job(&job);
            let done = self.run_job(&job.id, None).await;
            l.rounds_done = round;
            let Some(done) = done else { break };
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
            let never_sent =
                job.state == "queued" && job.request_id.is_none() && job.started_ms.is_none();
            // Mission jobs are driven (and reconciled) by the mission driver.
            if never_sent && job.loop_id.is_none() && job.kind != "chat" && job.kind != "mission" {
                // Provably never dispatched: safe to start now.
                let this = self.clone();
                tauri::async_runtime::spawn(async move {
                    this.run_job(&job.id, None).await;
                });
                continue;
            }
            // Dispatched (or part of a Loop): the result is unknown. Never
            // re-sent by itself; a person resumes, marks done or discards.
            job.state = "interrupted".into();
            job.error =
                Some("interrupted: the app closed during this task; its result is unknown".into());
            job.finished_ms = Some(now_ms());
            self.save_job(&job);
        }
        for mut l in self.loops().into_iter().filter(|l| l.state == "running") {
            l.state = "interrupted".into();
            l.stop_reason = Some("app_closed".into());
            self.save_loop(&l);
        }
    }

    // ── explicit recovery ────────────────────────────────────────────

    /// Runs an interrupted job again, telling the agent to check what was
    /// already done first. Only a person calls this.
    pub fn job_resume(self: &Arc<Self>, id: &str) -> Result<Job, String> {
        let mut job = self
            .job(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no job {id}"))?;
        if job.state != "interrupted" {
            return Err(format!(
                "{ERR_JOBS}: job {id} is {}, not interrupted",
                job.state
            ));
        }
        job.state = "queued".into();
        job.error = None;
        job.finished_ms = None;
        self.save_job(&job);
        let note = format!(
            "The app closed in the middle of this task and its result is unknown. Check what is already done before changing anything, and do not repeat an action that already happened. Task: {}",
            job.prompt
        );
        let this = self.clone();
        let jid = job.id.clone();
        tauri::async_runtime::spawn(async move {
            this.run_job(&jid, Some(note)).await;
        });
        Ok(job)
    }

    pub fn job_mark_done(&self, id: &str) -> Result<Job, String> {
        let mut job = self
            .job(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no job {id}"))?;
        if job.state != "interrupted" {
            return Err(format!(
                "{ERR_JOBS}: job {id} is {}, not interrupted",
                job.state
            ));
        }
        job.state = "done".into();
        job.error = Some("marked done by you after an interruption".into());
        job.finished_ms = Some(now_ms());
        self.save_job(&job);
        Ok(job)
    }

    pub fn job_discard(&self, id: &str) -> Result<Job, String> {
        let mut job = self
            .job(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no job {id}"))?;
        if job.state != "interrupted" {
            return Err(format!(
                "{ERR_JOBS}: job {id} is {}, not interrupted",
                job.state
            ));
        }
        job.state = "cancelled".into();
        job.error = Some("discarded after an interruption".into());
        job.finished_ms = Some(now_ms());
        self.save_job(&job);
        Ok(job)
    }

    pub fn loop_resume(self: &Arc<Self>, id: &str) -> Result<LoopDef, String> {
        let mut l = self
            .loop_get(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no loop {id}"))?;
        if l.state != "interrupted" {
            return Err(format!(
                "{ERR_JOBS}: loop {id} is {}, not interrupted",
                l.state
            ));
        }
        l.state = "running".into();
        l.stop_reason = None;
        self.save_loop(&l);
        let this = self.clone();
        let lid = l.id.clone();
        tauri::async_runtime::spawn(async move { this.run_loop(lid).await });
        Ok(l)
    }

    pub fn loop_settle(&self, id: &str, done: bool) -> Result<LoopDef, String> {
        let mut l = self
            .loop_get(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no loop {id}"))?;
        if l.state != "interrupted" {
            return Err(format!(
                "{ERR_JOBS}: loop {id} is {}, not interrupted",
                l.state
            ));
        }
        if done {
            l.state = "done".into();
            l.stop_reason = Some("marked_done".into());
        } else {
            l.state = "cancelled".into();
            l.stop_reason = Some("discarded".into());
        }
        l.finished_ms = Some(now_ms());
        self.save_loop(&l);
        Ok(l)
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
            t.last_digest = old.last_digest;
            t.last_notified_ms = old.last_notified_ms;
        }
        if t.name.trim().is_empty() {
            t.name = t.prompt.chars().take(48).collect();
        }
        self.store_trigger(&t);
        self.clone().ensure_cron();
        Ok(t)
    }

    /// Silence (or unsilence) a routine without touching its schedule.
    pub fn trigger_mute(&self, id: &str, muted: bool) -> Result<Trigger, String> {
        let mut t = self
            .trigger(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no trigger {id}"))?;
        t.muted = muted;
        self.store_trigger(&t);
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
        if let Some(template) = t.mission.clone() {
            let mission_id = crate::missions::start_from_trigger(&self.app, &t, &prompt, template)?;
            t.last_fired_ms = Some(now_ms());
            t.fire_count += 1;
            self.store_trigger(&t);
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

impl Cron {
    /// The next minute this line fires, strictly after `from`, in `from`'s
    /// time zone (the user's, `chrono::Local`, for the UI). `None` when it
    /// never fires within a year (e.g. `0 0 31 2 *`).
    pub fn next_after<Tz: chrono::TimeZone>(
        &self,
        from: &chrono::DateTime<Tz>,
    ) -> Option<chrono::DateTime<Tz>> {
        let mut t = from.clone() + chrono::Duration::minutes(1);
        t = t.with_second(0)?.with_nanosecond(0)?;
        for _ in 0..(366 * 24 * 60) {
            if self.matches(&t) {
                return Some(t);
            }
            t = t + chrono::Duration::minutes(1);
        }
        None
    }
}

/// `Some(new digest)` when a routine result should notify: not silenced and
/// different from the last one notified.
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

/// A trigger as the UI shows it: next run in the user's zone, and the plain
/// fact that a routine only runs while OmniGet is open.
pub fn trigger_view(t: &Trigger) -> Value {
    let now = chrono::Local::now();
    let next = if t.enabled && t.kind == "cron" {
        Cron::parse(t.cron.as_deref().unwrap_or(""))
            .ok()
            .and_then(|c| c.next_after(&now))
    } else {
        None
    };
    let mut v = serde_json::to_value(t).unwrap_or(Value::Null);
    v["next_run_ms"] = json!(next.map(|d| d.timestamp_millis()));
    v["next_run_local"] = json!(next.map(|d| d.format("%Y-%m-%d %H:%M").to_string()));
    v["utc_offset"] = json!(now.format("%:z").to_string());
    v["requires_app_open"] = json!(true);
    v
}

/// `{ base_url, token }` of the local bridge, for the webhook URL in the UI.
pub fn bridge_info(app: &AppHandle) -> Value {
    let settings = crate::storage::config::load_settings(app);
    json!({
        "base_url": format!("http://127.0.0.1:{}", settings.bridge.port),
        "token": settings.bridge.token,
    })
}

#[cfg(test)]
mod routine_tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn the_next_run_is_computed_in_the_given_zone() {
        // 09:30 every weekday, from Friday 2026-09-25 18:00 at UTC-3.
        let tz = chrono::FixedOffset::west_opt(3 * 3600).unwrap();
        let from = tz.with_ymd_and_hms(2026, 9, 25, 18, 0, 0).unwrap();
        let c = Cron::parse("30 9 * * 1-5").unwrap();
        let next = c.next_after(&from).unwrap();
        assert_eq!(
            next,
            tz.with_ymd_and_hms(2026, 9, 28, 9, 30, 0).unwrap(),
            "Monday"
        );
        // The wall clock of the zone, not UTC.
        assert_eq!(next.hour(), 9);
        assert!(Cron::parse("0 0 31 2 *")
            .unwrap()
            .next_after(&from)
            .is_none());
    }

    #[test]
    fn a_silenced_or_unchanged_routine_does_not_notify() {
        let mut t = Trigger {
            name: "r".into(),
            kind: "cron".into(),
            agent_id: "a".into(),
            prompt: "p".into(),
            ..Default::default()
        };
        let d1 = routine_digest(&t, "3 new chapters").expect("first result notifies");
        t.last_digest = Some(d1);
        assert!(
            routine_digest(&t, "3 new chapters  ").is_none(),
            "same result, no repeat"
        );
        assert!(
            routine_digest(&t, "4 new chapters").is_some(),
            "a change notifies"
        );
        t.muted = true;
        assert!(routine_digest(&t, "5 new chapters").is_none(), "silenced");
    }

    #[test]
    fn a_trigger_view_says_it_needs_the_app_open() {
        let t = Trigger {
            name: "r".into(),
            kind: "cron".into(),
            cron: Some("0 8 * * *".into()),
            agent_id: "a".into(),
            prompt: "p".into(),
            enabled: true,
            ..Default::default()
        };
        let v = trigger_view(&t);
        assert_eq!(v["requires_app_open"], true);
        assert!(v["next_run_ms"].as_i64().is_some());
        assert!(v["next_run_local"].as_str().unwrap().ends_with("08:00"));
    }
}

#[cfg(test)]
mod billing_tests {
    use super::*;
    use omniget_core::core::llm::types::Usage;

    #[test]
    fn folded_usage_keeps_cache_writes_and_bills_weighted_tokens() {
        let mut total = None;
        for _ in 0..2 {
            fold_usage(
                &mut total,
                "m",
                &Usage {
                    input_tokens: 50_000,
                    cache_read_tokens: 40_000,
                    cache_write_tokens: 4_000,
                    output_tokens: 100,
                    ..Usage::default()
                },
            );
        }
        let t = total.unwrap();
        assert_eq!(t["cache_write_tokens"].as_u64(), Some(8_000));
        assert_eq!(t["cache_read_tokens"].as_u64(), Some(80_000));
        // 2 x (6k fresh + 4k read-weighted + 5k write-weighted + 100 out).
        assert_eq!(billable_of(&t), 30_200);
    }
}
