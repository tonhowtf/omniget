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

mod changes;
mod loop_audit;
mod review;

pub use loop_audit::LoopAudit;

pub const EVENT_JOB: &str = "llm://job";
pub const EVENT_LOOP: &str = "llm://loop";
/// A routine's schedule or counters changed (fired, missed, saved): the
/// payload is [`trigger_view`].
pub const EVENT_TRIGGER: &str = "llm://trigger";
/// The queue changed (a job was admitted, finished or the cap moved): the
/// payload is [`state_view`], so the jobs page shows positions without asking.
pub const EVENT_QUEUE: &str = "llm://queue";
pub const EVENT_PLAYBOOK: &str = "llm://playbook";
pub const ERR_JOBS: &str = "ERR_LLM_JOBS";
/// A job's worktree could not be made, found or recreated (2.3).
pub const ERR_WORKTREE: &str = "ERR_JOB_WORKTREE";

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
    /// `queued|waiting_workspace|running|waiting_approval|done|failed|cancelled|interrupted`.
    /// `waiting_workspace`: another job holds an overlapping folder (1.5).
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
    /// Grows by one on every state change of this job (never on a log line),
    /// so a listener can drop an event older than what it already has.
    #[serde(default)]
    pub state_seq: i64,
    /// When the user last opened this job (epoch ms); a finished job with a
    /// later `finished_ms` is "not seen yet".
    #[serde(default)]
    pub seen_ms: Option<i64>,
    /// Why a failed job failed, in one word the UI can translate:
    /// `stalled`, `no_start`, `deadline`, `limit`, `ask_expired`.
    #[serde(default)]
    pub failure_reason: Option<String>,
    /// Per-job wall-clock ceiling in minutes; `None` = the setting
    /// `job_default_deadline_min`.
    #[serde(default)]
    pub deadline_min: Option<u32>,
    /// The deadline actually applied, set when the job starts.
    #[serde(default)]
    pub deadline_ms: Option<u64>,
    /// Why a `queued`/`waiting_workspace` job has not started: `slot`,
    /// `agent_cap` or `workspace:<job id>` (the job holding the folder).
    /// Cleared when it starts.
    #[serde(default)]
    pub wait_reason: Option<String>,
    /// What the job changed in its folder, recorded when it ended (1.7):
    /// the files (status, `+/-`, binary), totals, and whether the patch at
    /// `<llm dir>/jobs/<id>.patch` or the list was cut. `None` with
    /// `diff_unavailable` also `None` = not computed yet.
    #[serde(default)]
    pub diff_files: Option<Vec<omniget_core::core::llm::snapshot::FileChange>>,
    #[serde(default)]
    pub additions: Option<u64>,
    #[serde(default)]
    pub deletions: Option<u64>,
    #[serde(default)]
    pub diff_clipped: bool,
    /// Why the job has no diff, `code: detail`, code one of `no_workspace`,
    /// `no_snapshot`, `error`. Never an empty list standing for "unknown".
    #[serde(default)]
    pub diff_unavailable: Option<String>,
    /// Run spec of a job whose `agent_id` is `tool:<id>` (a coding CLI run
    /// headless, maybe in a sandbox): [`RunSpec`] as JSON.
    #[serde(default)]
    pub spec: Option<serde_json::Value>,
    /// The playbook run this job is a step of.
    #[serde(default)]
    pub playbook_id: Option<String>,
    /// Where the job writes (2.3, D3): the folder itself, a git worktree of
    /// its own, or `auto` (a worktree only when another job is already
    /// writing in the same canonical folder of a git repository).
    #[serde(default)]
    pub isolation: Isolation,
    /// The ref a worktree starts from (`--base`); `None` = the user's
    /// checkout as it is on disk, uncommitted work included.
    #[serde(default)]
    pub base_ref: Option<String>,
    /// The job's worktree (`<app data>/worktrees/<repo>-job-<id>`), set when
    /// it runs in one; the folder its turns see.
    #[serde(default)]
    pub worktree_path: Option<String>,
    /// `omniget/job-<id>`: holds the job's work once the worktree is gone.
    #[serde(default)]
    pub branch: Option<String>,
    /// The commit the worktree started from.
    #[serde(default)]
    pub base_oid: Option<String>,
    /// The worktree's `HEAD` when the diff was recorded: the commit that
    /// holds the job's work on its branch (2.4, worktree mode).
    #[serde(default)]
    pub head_oid: Option<String>,
    /// What the job did, stamped by the host when it ended (2.4, 83#P4):
    /// `host:workspace@base→head`, the files, the checks with their exit
    /// code, tokens, cost and time. Never written by the agent; 0 tokens.
    #[serde(default)]
    pub proof: Option<Value>,
    /// A short name for lists, given by whoever started the job (2.5).
    #[serde(default)]
    pub title: Option<String>,
    /// Who started the job through the command surface (2.5): `token:<id>`
    /// for a bridge token, `mcp:<client>` for an MCP client. An MCP client
    /// reads, waits on, cancels and answers the asks of the jobs it owns.
    #[serde(default)]
    pub owner: Option<String>,
    /// How much the job's turns may do without asking (2.7, D2): a job from
    /// outside the window asks before every edit and command unless the
    /// caller chose otherwise; older rows keep what they did (`auto_edits`).
    #[serde(default)]
    pub run_mode: RunMode,
    /// The agent's own last word (2.6): `{status: succeeded|failed, summary}`
    /// from its `OUTCOME:` line. `None` when it wrote none (a `run` job then
    /// ends `done_unreported`).
    #[serde(default)]
    pub outcome: Option<Value>,
    /// The rest of the `job.start` command: `command_id`, `purpose`,
    /// `model`, `effort`, `budget_usd`, `resume_of`.
    #[serde(default)]
    pub origin: Option<Value>,
    /// The CLI's own session id, copied onto the row at the first event of
    /// the turn that names it (3.1, S16), not at the end: a job the app
    /// loses mid-turn still knows which session `--resume` continues.
    #[serde(default)]
    pub session_id: Option<String>,
    /// The agent process of the turn running now, with the start token the
    /// OS gives it ([`omniget_core::core::process::start_token`]): on boot a
    /// process that is still alive is stopped before anything runs again,
    /// and a pid the OS reused (another start token) is never touched.
    /// Cleared when the turn ends.
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub pid_start: Option<u64>,
    /// How many times the host started this job again by itself after an
    /// infrastructure failure (3.1); at most [`AUTO_RESUME_MAX`].
    #[serde(default)]
    pub attempt: u32,
    /// This job's own answer to "resume by itself after a restart";
    /// `None` = the setting `llm.auto_resume` (a Loop round takes its Loop's).
    #[serde(default)]
    pub auto_resume: Option<bool>,
    /// A tool that can change something (anything but reading and
    /// searching) started in one of the job's turns: a job that ran in the
    /// shared folder is then never re-run by itself.
    #[serde(default)]
    pub side_effects: bool,
}

pub use omniget_core::core::llm::run_mode::RunMode;

/// How a job is placed in its folder (2.3). `Auto` is the default of a job
/// a person starts in the background (`run`, `trigger`); missions, Loop
/// rounds, chat turns and catalog tool runs stay `Shared` unless asked.
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

    /// `shared | worktree | auto`; anything else is an error the caller
    /// shows (a typo must not silently mean "shared").
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

    /// The default for a new job of `kind` run by `agent_id`.
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

/// Where a job's worktree comes from and why, decided before the gate.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// The git repository of the job's folder (with a commit), when there is one.
    pub repo: Option<std::path::PathBuf>,
    /// A worktree whatever the gate says (`isolation = worktree`, or a job
    /// that already has one).
    pub force: bool,
    /// Ask the gate for a worktree only if the folder is busy (`auto`).
    pub if_busy: bool,
}

/// The placement of `job` given the repository of its folder (`repo`,
/// `None` = no git repository with a commit). `Err` = the job cannot run as
/// asked (a worktree without a repository).
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

/// The folder a job's turns see: its worktree when it has one, else the
/// folder it was given.
pub fn effective_workspace(job: &Job) -> Option<String> {
    job.worktree_path
        .clone()
        .or_else(|| job.workspace.clone())
        .filter(|w| !w.trim().is_empty())
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
    /// Ceiling of each round (one job), in minutes; `None` = the setting.
    #[serde(default)]
    pub deadline_min: Option<u32>,
    /// `running|done|failed|cancelled|interrupted|blocked|budget_limited`
    /// (`blocked` and `budget_limited` come from the auditor's rules, 3.7).
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
    /// "Resume by itself after a restart" for this Loop (3.1); `None` = the
    /// setting `llm.auto_resume`.
    #[serde(default)]
    pub auto_resume: Option<bool>,
    /// The host restarted this Loop by itself: the next round starts with
    /// the "check before repeating" note. Cleared once that round starts.
    #[serde(default)]
    pub restarted: bool,
    /// Restarts by the host since the last round that finished; at most
    /// [`AUTO_RESUME_MAX`] in a row.
    #[serde(default)]
    pub auto_restarts: u32,
    /// The run mode of every round (2.7). `None` = a Loop saved before the
    /// field: its rounds keep `auto_edits`. The bridge fills
    /// `approval_required` when the caller sends none.
    #[serde(default)]
    pub run_mode: Option<RunMode>,
    /// Weighted tokens of all rounds (3.3, PROB C5): what the loop cost when
    /// its dollars are unknown.
    #[serde(default)]
    pub spent_tokens: u64,
    /// The part of `spent_usd` that is the price table's estimate (a CLI
    /// that reported no price), not what the provider billed.
    #[serde(default)]
    pub spent_usd_estimated: f64,
    /// Rounds whose cost nobody knows (no report, no price).
    #[serde(default)]
    pub unpriced_rounds: u32,
    /// Stop when the rounds and their audits spent this many billable tokens
    /// (`billable_of`: input weighted by cache, plus output) → state
    /// `budget_limited` (3.7, 84#P1).
    #[serde(default)]
    pub token_budget: Option<u64>,
    #[serde(default)]
    pub tokens_used: u64,
    /// The auditor's model; `None` = the side model of the agent's own
    /// provider/account (`router::resolve_side`).
    #[serde(default)]
    pub audit_model: Option<String>,
    /// `blocked` verdicts in a row (3 end the Loop).
    #[serde(default)]
    pub blocked_streak: u32,
    /// Audits in a row without a verdict (2 end the Loop).
    #[serde(default)]
    pub audit_fail_streak: u32,
    /// Rounds in a row the agent said done and the auditor `continue` (3
    /// end the Loop as `blocked`: a person settles it).
    #[serde(default)]
    pub disagree_streak: u32,
    /// The auditor's last note: the Loop's progress line.
    #[serde(default)]
    pub note: Option<String>,
    /// One entry per audited round, newest last.
    #[serde(default)]
    pub audits: Vec<LoopAudit>,
    /// What the audits cost together (also inside `spent_usd`).
    #[serde(default)]
    pub audit_usd: f64,
    /// Rounds that went out as the short continuation guide (83#P7).
    #[serde(default)]
    pub rounds_guided: u32,
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
    /// Ceiling of the job each firing starts, in minutes; `None` = the
    /// setting `job_default_deadline_min`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_min: Option<u32>,
    /// Next scheduled time of a cron routine (epoch ms); `None` for webhooks,
    /// disabled routines and lines that never fire. The `triggers` column of
    /// the same name is the truth: only a compare-and-set moves it (1.4).
    #[serde(default)]
    pub next_run_ms: Option<i64>,
    /// What happens to the times missed while the app was closed or asleep.
    #[serde(default)]
    pub catch_up: CatchUp,
    /// What happens when a time comes while the last run is still going.
    #[serde(default)]
    pub concurrency: TriggerConcurrency,
    /// Zone of the cron line: `local` (this machine), `UTC` or `+HH:MM`.
    #[serde(default = "local_zone")]
    pub tz: String,
    /// Scheduled times that did not run (missed and not caught up, or
    /// skipped because the last run was still going), all time.
    #[serde(default)]
    pub missed: u32,
    #[serde(default)]
    pub last_missed_ms: Option<u64>,
    /// Catch-up runs still to start: `skip_if_active` runs them one at a
    /// time, never two of one routine together.
    #[serde(default)]
    pub backlog: u32,
    /// Job (or mission) of the last firing.
    #[serde(default)]
    pub last_run_id: Option<String>,
}

fn yes() -> bool {
    true
}

fn local_zone() -> String {
    "local".into()
}

/// The times a cron routine missed while nothing could run it (80#P2).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CatchUp {
    /// Counted and logged as missed; nothing runs.
    Skip,
    /// One run for all of them (a daily 9:00 routine opened at 10:00 runs
    /// once).
    #[default]
    Once,
    /// One run per missed time, at most [`CATCH_UP_CAP`].
    AllCapped,
}

/// A scheduled time while the routine's last run is still going.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerConcurrency {
    /// Not started (counted as missed); catch-up runs wait their turn.
    #[default]
    SkipIfActive,
    /// Started anyway; the gate orders it.
    Queue,
}

/// Most catch-up runs one routine starts after a long gap (80#P2).
pub const CATCH_UP_CAP: u32 = 25;

pub struct Jobs {
    db: Mutex<Connection>,
    app: AppHandle,
    llm: Arc<LlmManager>,
    /// Slots, per-agent caps and one writer per folder (1.5, 1.6).
    gate: Arc<omniget_core::core::llm::job_gate::Gate>,
    cancels: Mutex<HashMap<String, CancellationToken>>,
    cron_running: AtomicBool,
    /// One cron pass at a time (the boot pass and the ticker): a pass claims
    /// and starts its runs before the next one reads who is busy.
    cron_pass: Mutex<()>,
    guards: Mutex<HashMap<String, JobGuard>>,
    /// Per running job: its turn, when it last heard from it, its deadline
    /// (what `explain` and the job ask timeout read).
    live: Mutex<HashMap<String, Live>>,
    /// Per running turn (request id): the job it belongs to and the channel
    /// its runner reads questions from. The runner is the only writer of its
    /// row, so the bus hands it the event instead of writing the row itself.
    watchers: Mutex<HashMap<String, (String, tokio::sync::mpsc::UnboundedSender<AskSignal>)>>,
    /// "Needs you" / "finished" notifications.
    pub attention: Arc<crate::llm_attention::Attention>,
    /// Wakes a Loop sleeping until its next scheduled round when it is cancelled.
    loop_waits: Mutex<HashMap<String, CancellationToken>>,
    /// When the cron ticker last ran the worktree GC (epoch ms).
    last_worktree_gc: std::sync::atomic::AtomicU64,
    /// Bumped on every saved row, so `wait` (2.5) sleeps until something
    /// changed instead of polling.
    changes: tokio::sync::watch::Sender<u64>,
}

/// A question of a job's turn, as the bus forwarder hands it to the runner.
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

/// The OS side of [`crate::llm_attention::Attention`]: the notification
/// plugin, and the focus of the main window plus the route the front reports.
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

/// Drops a job's cancel token however `run_job` returns (the early returns
/// of a cancel at the gate included).
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

/// The tables of `jobs.db` and every migration, safe on every open: a
/// column that already exists is skipped.
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
    // Databases created before 0.10.0 final have no usage column.
    let _ = db.execute("ALTER TABLE jobs ADD COLUMN usage TEXT", []);
    // Watchdog, deadline and state events (1.2, 1.3): older rows read as
    // seq 0, never seen, no reason, the default deadline.
    for column in [
        "state_seq INTEGER NOT NULL DEFAULT 0",
        "seen_ms INTEGER",
        "failure_reason TEXT",
        "deadline_min INTEGER",
        "deadline_ms INTEGER",
        // Why a queued job waits (1.6).
        "wait_reason TEXT",
        // What the job changed (1.7): older rows have no diff yet, and one
        // is computed the first time someone asks.
        "diff_files TEXT",
        "additions INTEGER",
        "deletions INTEGER",
        "diff_clipped INTEGER NOT NULL DEFAULT 0",
        "diff_unavailable TEXT",
        // Catalog runs (0.11): the tool run spec and the playbook of a step.
        "spec TEXT",
        "playbook_id TEXT",
        // Worktree per job (2.3): older rows are `shared`, no worktree.
        "isolation TEXT",
        "base_ref TEXT",
        "worktree_path TEXT",
        "branch TEXT",
        "base_oid TEXT",
        // Diff by base..tip and the host's proof (2.4).
        "head_oid TEXT",
        "proof TEXT",
        // One run mode per job (2.7): older rows read as `auto_edits`.
        // Command surface and outcome (2.5, 2.6).
        "title TEXT",
        "owner TEXT",
        "run_mode TEXT",
        "outcome TEXT",
        "origin TEXT",
        // Resume after a restart (3.1): the CLI session, the process of the
        // turn in flight, attempts, the per-job switch, whether it wrote.
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
    // The durable schedule (1.4): routines saved before it start from now.
    let _ = db.execute("ALTER TABLE triggers ADD COLUMN next_run_ms INTEGER", []);
    migrate_triggers(db, now_ms);
    // Scoped control tokens of the bridge (2.1).
    crate::bridge_tokens::ensure_schema(db).map_err(|e| format!("{ERR_JOBS}: {e}"))?;
    // Receipts of `POST /v1/commands` and the MCP job tools (2.5).
    crate::job_commands::ensure_schema(db).map_err(|e| format!("{ERR_JOBS}: {e}"))?;
    // Review notes (3.5).
    review::ensure_notes_schema(db)?;
    // Per-file baselines of the CLI's own writes (3.4).
    changes::ensure_schema(db)?;
    Ok(())
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
        // The boot pass counts as a GC run.
        last_worktree_gc: std::sync::atomic::AtomicU64::new(now_ms()),
        changes: tokio::sync::watch::channel(0).0,
    });
    if JOBS.set(jobs.clone()).is_ok() {
        let limits = crate::commands::llm::watchdog::limits(app);
        jobs.attention.set_delay(limits.notify_delay);
        jobs.attention
            .set_strings(crate::llm_attention::pushed_strings());
        // A job's question waits as long as the job may still run (1.3).
        let weak = Arc::downgrade(&jobs);
        jobs.llm
            .set_ask_timeout_resolver(Some(Arc::new(move |request: &str| {
                weak.upgrade()?.ask_timeout_for(request)
            })));
        // The "before" of every file a CLI turn writes lands under its job
        // (3.4).
        let weak = Arc::downgrade(&jobs);
        omniget_core::core::llm::file_changes::set_sink(Some(Arc::new(
            move |request: &str, touch| {
                if let Some(jobs) = weak.upgrade() {
                    jobs.record_change(request, touch);
                }
            },
        )));
        // Waiting rows follow the gate: state, reason and `llm://queue`.
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
            // No job runs yet: a token a crash left live dies here, and the
            // CLI finds a valid token of its own (2.1).
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

/// The instance, if something already opened it. Never opens it (the boot
/// order matters: runs are reconciled before jobs resume).
pub fn try_get() -> Option<Arc<Jobs>> {
    JOBS.get().cloned()
}

/// Called by the bus forwarder for every bus event: questions of a job's
/// turn change its state the moment they happen (A7), and the ones nobody
/// answers become notifications (A4).
pub fn on_bus(event: &omniget_core::core::omni::bus::BusEvent) {
    if let Some(jobs) = try_get() {
        jobs.handle_bus(event);
    }
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

    // ── bridge tokens (2.1) ──────────────────────────────────────────

    /// What a bearer presented to an agent route is (the bridge's guard).
    pub fn token_lookup(&self, token: &str) -> crate::bridge_tokens::Lookup {
        crate::bridge_tokens::lookup(&self.db(), token, now_ms())
    }

    pub fn token_mint(
        &self,
        name: &str,
        scopes: &[String],
        expires_ms: Option<u64>,
    ) -> Result<crate::bridge_tokens::Minted, String> {
        crate::bridge_tokens::mint(
            &self.db(),
            crate::bridge_tokens::Kind::User,
            name,
            scopes,
            None,
            expires_ms,
            now_ms(),
        )
    }

    pub fn token_list(&self) -> Vec<crate::bridge_tokens::TokenView> {
        crate::bridge_tokens::list(&self.db(), now_ms())
    }

    pub fn token_revoke(&self, id: &str) -> Result<bool, String> {
        crate::bridge_tokens::revoke(&self.db(), id, now_ms())
    }

    /// The job's own token for the length of its turn (see
    /// [`crate::bridge_tokens::JobTokenLease`]). A failure to mint only
    /// costs the child its token, never the job.
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

    // ── jobs ─────────────────────────────────────────────────────────

    fn save_job_checked(&self, job: &Job) -> Result<(), String> {
        let state_seq = {
            let db = self.db();
            // The stored row decides the sequence, not the caller's copy: a
            // runner, the bus handler and a cancel may each hold one, and the
            // number must still only grow, by one per real state change.
            // `seen_ms` is owned by the user's side (`mark_seen`): kept.
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
        // The list view does not need the log on every event. The event is
        // the whole row, so it carries `{id, state, state_seq}` too.
        let mut light = job.clone();
        light.state_seq = state_seq.0;
        light.seen_ms = state_seq.1;
        light.log = clip(&light.log, 4096);
        // Up to 1000 file entries: the totals travel, the list is read on
        // demand (`llm_job_diff`, `GET /v1/jobs/{id}/diff`).
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
                    // Rows of a list carry the diff totals, not the file
                    // list (see `save_job_checked`).
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
        // A job that continues an existing conversation (a chat turn the app
        // closed on) keeps that conversation's mode, the chat default when
        // it never chose one; a fresh job takes the default of its kind.
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
        self.submit_with_deadline(
            kind,
            agent_id,
            prompt,
            workspace,
            conversation_id,
            trigger_id,
            None,
        )
    }

    /// [`Self::submit`] with the job's own deadline in minutes (`None` = the
    /// `job_default_deadline_min` setting).
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

    /// Starts the job of a `job.start` command (2.5), owned by `owner`. The
    /// command's fields land on the row; `resume_of` continues that job's
    /// conversation (and folder, unless another is named); `model` becomes
    /// the conversation's model on the agent's own provider; `budget_usd`
    /// guards the turn ([`with_budget`]). `effort` is recorded only.
    pub fn submit_command(
        self: &Arc<Self>,
        s: &crate::job_commands::StartSpec,
        owner: &str,
    ) -> Result<Job, String> {
        self.start_spec(s, Some(owner), RunMode::ApprovalRequired)
    }

    /// The job of a start spec from the window (3.10: the same path as a
    /// command, so the folder's workflow applies), with no owner and the
    /// window's default mode when none was chosen.
    pub fn submit_window(
        self: &Arc<Self>,
        s: &crate::job_commands::StartSpec,
    ) -> Result<Job, String> {
        self.start_spec(s, None, RunMode::AutoEdits)
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
        // The mode asked for (2.7); none = the resumed job's, else a job from
        // outside asks first (D2).
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
            // What `.omniget/workflow.md` filled in and held back (3.10);
            // its hooks and checks append their runs here.
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
        // The US$ budget of the start (the caller's, or the folder's
        // workflow `budget:`, 3.10) is also a hard policy on `job:<id>`
        // (3.3): one budget mechanism, priced even when the CLI reports no
        // cost, with an incident when crossed. The guard below still stops
        // the turn in flight.
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

    /// Something saved a job row since `seen` (see [`Self::changes`]).
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub fn cancel(&self, id: &str) -> Result<Job, String> {
        self.cancel_by(id, omniget_core::core::llm::audit::Actor::system("system"))
    }

    /// [`Self::cancel`] on someone's behalf: the audit line (2.8) says who
    /// asked and through what, when the job was still alive.
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
        // Persist before consulting the token. The runner either sees this
        // cancelled row on its re-read or has already registered its token
        // (a job waiting at the gate has one: the cancel wakes the wait).
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
        // Also through the manager, which drops the turn's pending questions.
        if let Some(request) = job.request_id.as_deref() {
            let _ = self.llm.cancel(request);
        }
        Ok(job)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let _ = self.cancel(id);
        // The worktree goes too, its work saved on the branch first (2.3),
        // unless a follow-up job (3.5) still works in it.
        let shared_worktree = |j: &Job| -> bool {
            self.db()
                .query_row(
                    "SELECT COUNT(*) FROM jobs WHERE branch=?1 AND id<>?2",
                    params![j.branch, j.id],
                    |r| r.get::<_, i64>(0),
                )
                .unwrap_or(0)
                > 0
        };
        if let Some(job) = self
            .job(id)
            .filter(|j| j.worktree_path.is_some() && !shared_worktree(j))
        {
            tauri::async_runtime::spawn(async move {
                match remove_job_worktree(&job).await {
                    Ok(out) => tracing::info!("[jobs] {}: {}", job.id, removal_note(&out).trim()),
                    Err(e) => tracing::warn!("[jobs] {}: worktree kept: {e}", job.id),
                }
            });
        }
        forget_budget_policies(&omniget_core::core::llm::budget::policy::Scope::Job(
            id.to_string(),
        ));
        self.db()
            .execute("DELETE FROM jobs WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        let _ = self
            .db()
            .execute("DELETE FROM job_notes WHERE job_id=?1", params![id]);
        changes::forget(&self.db(), id);
        if let Some(path) = patch_path(id) {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }

    // ── diff (1.7) ───────────────────────────────────────────────────

    /// The folder the job's turns wrote in: the conversation's bound folder
    /// (canonical, which is what the snapshots are keyed by), else the
    /// job's own.
    fn diff_folder(&self, job: &Job) -> Option<std::path::PathBuf> {
        code_tools::workspace_of(&sanitize_id(&job.conversation_id)).or_else(|| {
            effective_workspace(job).map(|w| std::fs::canonicalize(&w).unwrap_or_else(|_| w.into()))
        })
    }

    /// Fills the job's diff fields; returns the patch when there is one,
    /// and the two ends compared (for the proof). A job in a worktree is
    /// diffed `merge-base(base_oid, HEAD)..worktree` (2.4); any other from
    /// its shadow snapshots (1.7).
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
                // No snapshot to diff: the baselines the stream gave (3.4).
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

    /// [`Self::compute_diff`] and the patch file next to `jobs.db`.
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

    /// The job's diff as stored, its patch, and whether it is final. A
    /// finished job recorded before 1.7 (or a chat turn) gets its diff
    /// computed and stored now; a running one is diffed against the folder
    /// as it is (`complete = false`), nothing stored.
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
                // No file: nothing changed, or the file is gone.
                None => files.is_empty().then(String::new),
            }
        });
        Ok((job, patch, true))
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
        let queued = self.job(id)?;
        if queued.state == "cancelled" {
            return Some(queued);
        }
        // A resumed job keeps the turn it had before: its diff spans both.
        let earlier_request = queued.request_id.clone();
        // Registered before the wait, so a cancel while queued wakes it (the
        // gate drops it from the queue) instead of waiting for a slot first.
        let cancel = CancellationToken::new();
        self.cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string(), cancel.clone());
        let _unregister = Unregister { jobs: self, id };
        // Where the job writes (2.3): its folder, or a worktree of its own
        // (asked, or `auto` with the folder busy). Only a folder in a git
        // repository with a commit can have one.
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
        // One gate for a slot, the agent's share and the folder (1.5, 1.6):
        // the same for native, CLI and ACP agents, since it holds the job,
        // not a tool. Chat turns never come here. A job going to a worktree
        // takes no folder lock: nobody else writes there.
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
        // The slot is this job's; the spawn waits its turn (500 ms apart).
        if !lease.delay.is_zero() {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => return self.job(id),
                _ = tokio::time::sleep(lease.delay) => {}
            }
        }
        let _lease = lease;
        // The computer stays awake while an agent job runs (washmybelly).
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
        // Cancellation may have landed between the gate and here. Re-read
        // its durable state before dispatching anything.
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
        // A scope paused by a budget incident starts nothing (3.3): the job
        // fails before its turn, and says which incident holds it.
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
        // The process of an earlier attempt is not this one (3.1).
        job.pid = None;
        job.pid_start = None;
        // A mission brings its own guard; every other background job gets the
        // default deadline (D4: a stuck run, Loop round or trigger can no
        // longer hold a slot until the app closes).
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
        // A budget from `job.start` (2.5) rides on whatever guard the job has.
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
        // The agent ends with its own verdict (2.6); the host still decides
        // the state from what it saw.
        let input = if asks_outcome(&job.kind) {
            format!("{input}{OUTCOME_SUFFIX}")
        } else {
            input
        };
        // Held to the end of the job: however this returns, the token is
        // revoked and leaves the conversation's env (2.1).
        let _job_token = self.job_token_lease(&job, &conv);
        // The turns of this job run in its run mode (2.7), whatever the
        // conversation had chosen; the binding goes with the job.
        let _run_mode = omniget_core::core::llm::run_mode::Binding::new(&conv, job.run_mode);
        // Its turns count against `job:<id>` (and its Loop's) budgets (3.3).
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
        // Questions of this turn reach this loop through the bus forwarder
        // (`on_bus`), the moment they are raised and resolved.
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
        // A question raised before the watcher existed is still seen.
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
        // Set when a guard stopped the job (cap or deadline), with its kind.
        let mut limit_hit: Option<String> = None;
        let mut limit_kind: Option<&'static str> = None;
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
        let mut first_event = true;
        loop {
            tokio::select! {
                event = stream.next() => {
                    let Some(event) = event else { break };
                    self.touch(&job.id);
                    // The session and the process go on the row at the
                    // first event, not at the end (3.1).
                    if first_event {
                        first_event = false;
                        if self.capture_process(&mut job, &request_id, &conv) && self.save_job_checked(&job).is_err() {
                            storage_failed = true;
                            cancel.cancel();
                        }
                    }
                    self.llm.note_event(&job.agent_id, &event);
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
                            // A tool call ends a model request: check the cap
                            // before the next one is sent (F7).
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
                                    limit_kind = Some("limit");
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
                    // A7: `waiting_approval` is born with the question itself.
                    self.apply_ask(&mut job, &request_id, signal);
                    if self.save_job_checked(&job).is_err() { storage_failed = true; cancel.cancel(); }
                }
                _ = tick.tick() => {
                    // A session named after the first event (the CLI's init
                    // line comes a moment after the spawn) still lands.
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
                    // Only a safety net for a bus event lost to lag: the
                    // state itself follows the ask events above.
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
        // The turn's process is over (3.1).
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
        // What the job changed, from the snapshots its turns took: host-side
        // post-processing, not a token more (1.7). The Coordinator records
        // the tree after the turn before the stream says `Finished`.
        let requests: Vec<String> = earlier_request
            .into_iter()
            .chain(std::iter::once(request_id.clone()))
            .collect();
        // In a worktree the job's work becomes a commit on its branch, so
        // `head_oid` names the result (2.4); the diff still counts anything
        // left uncommitted, through a private index.
        self.commit_worktree(&mut job).await;
        let ends = self.record_diff(&mut job, &requests).await;
        // A job that wrote waits for a person: `review`, not `done` (2.4).
        job.state = settle_state(&job.state, job.diff_files.as_deref()).into();
        // A run that ended without its `OUTCOME:` line says so (2.6).
        job.state = settle_outcome(&job.kind, &job.state, job.outcome.is_some()).into();
        job.failure_reason = (job.state == "failed")
            .then(|| {
                limit_kind
                    .or_else(|| job.error.as_deref().and_then(failure_reason_of))
                    .map(String::from)
            })
            .flatten();
        // Cancelled by a budget crossing (`cancel_scope`): the mark stays.
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
        // The repository's checks from its workflow (3.10), when trusted:
        // each exit code lands in the proof.
        crate::job_workflow::run_checks(crate::job_workflow::store().as_deref(), &mut job).await;
        // A worktree the job did not change has nothing to review: it goes
        // now, with its branch (2.3).
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
        // How it ended (2.8); a cancel already has its line, with who asked.
        // A run the watchdog stopped ends here too, even when it goes
        // again by itself below (that start gets its own line, 3.1).
        if job.state != "cancelled" {
            crate::llm_audit::job_ended(&job);
        }
        // Stopped by the watchdog: the host's problem, not the agent's. When
        // asked, and when it is safe (3.1), the job goes again by itself.
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
            // Routines keep their own rule (only a changed result, muted).
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

    /// Commits what the job left in its worktree onto its branch (2.4), as
    /// OmniGet through a private index: no hook of the repository runs and
    /// nothing is signed. A failure leaves the work uncommitted in the
    /// worktree (the diff still counts it) and says so in the log.
    async fn commit_worktree(&self, job: &mut Job) {
        use omniget_core::core::vcs::job_worktree;
        if job.worktree_path.is_none() {
            return;
        }
        let Some((_, root)) = worktree_root(job).await else {
            return;
        };
        // The task as asked, not the workflow's template around it (3.10).
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

    /// Adds a check that ran after the job (a Loop's `check_command`) to its
    /// proof, with the exit code.
    fn attach_check(&self, job_id: &str, command: &str, ok: bool, out: &str) {
        let Some(mut job) = self.job(job_id) else {
            return;
        };
        push_check(job.proof.get_or_insert_with(|| json!({})), command, ok, out);
        self.save_job(&job);
    }

    /// Ends a job that cannot start as asked (before the gate).
    fn fail_before_start(&self, id: &str, error: String) -> Option<Job> {
        let mut job = self.job(id)?;
        job.state = "failed".into();
        job.error = Some(error);
        job.finished_ms = Some(now_ms());
        self.save_job_checked(&job).ok()?;
        crate::llm_audit::job_ended(&job);
        Some(job)
    }

    /// Puts the job in its worktree when it goes to one (2.3): made now
    /// (with the user's uncommitted work carried in, or from `base_ref`), or
    /// recreated from the job's branch when a resumed job finds it gone.
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
                // A fresh checkout again: the workflow's hook runs again (3.10).
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
        // A job given a subfolder of the repository works in the same
        // subfolder of its worktree.
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
        // The repository's `after_create` hook, only when its workflow is
        // trusted (3.10; the hooks 2.3 deferred). A failure fails the job;
        // the worktree stays until it is removed like any other.
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

    /// Removes the job's worktree (2.3), never losing work: what is not
    /// committed is saved on its branch first, and a branch holding work is
    /// kept and named in the job's log. Refused while the job runs.
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

    /// Removes the worktrees nobody needs any more (2.3): an orphan (its job
    /// row is gone) and one of a job that ended more than
    /// [`WORKTREE_TTL_MS`] ago. Work is saved on the branch first, as in
    /// every removal. On boot the trash left by a crash is emptied too.
    async fn gc_worktrees(self: Arc<Self>, boot: bool) {
        use omniget_core::core::vcs::job_worktree;
        if boot {
            job_worktree::empty_trash();
        }
        let now = now_ms();
        let live = self.live_folders("");
        for (id, path) in job_worktree::list_job_worktrees() {
            // A follow-up job (3.5) may be working in a finished job's worktree.
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
                    // Only a folder on the job's own branch is taken for it.
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

    /// The repository of a job's worktree and the worktree's root folder
    /// (`worktree_path` may be a subfolder of it).
    async fn worktree_root(&self, job: &Job) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
        worktree_root(job).await
    }

    /// Marks that the turn of a running job was just heard from.
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

    /// Who a notification about this job names, and where it lives.
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

    /// One question event applied to a job's row (in memory; the caller
    /// saves): the state follows the question at once, an unanswered one is
    /// scheduled as a notification, and an expired one is written down with
    /// its reason and said out loud — never a silent "no" (A3).
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

    /// Routes a bus event to the job it belongs to: the runner of a
    /// background job (through its channel, so it stays the only writer of
    /// its row), or the row of a chat turn directly.
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
        // A chat turn has no runner here: its mirror row is updated in place.
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
        // A turn with no job at all (Help, the stateless bridge): there is
        // no row to write the reason to, but an expiry is still said out
        // loud — the agent got ERR_TOOL_TIMEOUT, and so does the user.
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

    /// How long a question of this turn waits, when the turn is a running
    /// background job; `None` = not one (the chat's timeout applies).
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

    /// The job whose turn is `request_id` (the run id of its asks, 2.2).
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

    /// `GET /v1/jobs/{id}/explain` and `omniget agent explain`.
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
        // Still at the gate: where it stands and why (1.6).
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

    // ── the gate: folder, queue, limits (1.5, 1.6) ─────────────────────

    /// The folder a job may write in: its own, or the one its conversation
    /// is already bound to. `None` = a personal job (the agent's private
    /// sandbox, no project tools), which takes no folder lock.
    fn folder_of(&self, job: &Job) -> Option<omniget_core::core::llm::folder_key::FolderKey> {
        effective_workspace(job)
            .map(std::path::PathBuf::from)
            .or_else(|| code_tools::workspace_of(&sanitize_id(&job.conversation_id)))
            .map(|p| omniget_core::core::llm::folder_key::FolderKey::of(&p))
    }

    /// Writes the state and reason of every job still at the gate, only when
    /// they changed, and sends `llm://queue`. Only a row still `queued` or
    /// `waiting_workspace` is touched, so a cancel is never overwritten.
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

    /// `GET /v1/state`, `llm_jobs_state`, `omniget agent status`.
    pub fn state_view(&self) -> Value {
        let mut v = state_view(&self.gate.snapshot(), &|id| self.job(id));
        // Whether the app is running with no window (3.2): the CLI and the
        // daemon benchmark read it; `pid` stays on the local bridge.
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

    /// New caps from the settings, applied to running jobs at once (no
    /// permit lost or made up, see `job_gate`).
    pub fn apply_limits(&self, s: &omniget_core::models::settings::LlmSettings) {
        let l = gate_limits(s);
        self.gate.set_limits(l.max_parallel, l.max_per_agent);
    }

    /// The parallel cap now: how many tasks a mission sends to the gate at
    /// once (the gate still orders them behind manual jobs).
    pub fn max_parallel(&self) -> usize {
        self.gate.limits().max_parallel
    }

    /// Folders of the turns running right now in other conversations: jobs
    /// and chat turns (mirrored as `chat` jobs). The undo guard refuses to
    /// restore files that overlap any of them.
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

    /// The user opened this job: its "not seen" badge goes away.
    pub fn mark_seen(&self, id: &str) -> Result<Job, String> {
        self.db()
            .execute(
                "UPDATE jobs SET seen_ms=?2 WHERE id=?1",
                params![id, now_ms() as i64],
            )
            .map_err(|e| format!("{ERR_JOBS}: {e}"))?;
        self.job(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no job {id}"))
    }

    /// A routine's result reaches the user only when it changed and the
    /// routine is not silenced.
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
        // A cost ceiling (3.3) and a token budget (3.7) are hard budget
        // policies on the loop: one mechanism, so crossing either pauses the
        // loop with an incident ("raise and resume"), and the audits count
        // too (they are bound to the loop's scope).
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

    pub fn loop_cancel(&self, id: &str) -> Result<LoopDef, String> {
        let mut l = self
            .loop_get(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no loop {id}"))?;
        if l.state == "running" || l.state == "paused" {
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
                    && matches!(
                        j.state.as_str(),
                        "queued" | "waiting_workspace" | "running" | "waiting_approval"
                    )
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
        // Its budget policies go with it (3.3): nothing else can cross them.
        forget_budget_policies(&omniget_core::core::llm::budget::policy::Scope::Loop(
            id.to_string(),
        ));
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
        let _power_hold = omniget_core::core::washmybelly::power::auto_hold_begin("agent loop");
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
        // No check command and an agent to ask: an independent auditor
        // decides when the work is done (84#P1), never the agent's own "DONE".
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
            if max_rounds.map(|m| l.rounds_done >= m).unwrap_or(false) {
                l.stop_reason = Some("max_rounds".into());
                // The auditor never said `complete`: not done, needs a person.
                if audited {
                    l.state = "blocked".into();
                }
                break;
            }
            // With the budget engine the token budget is a policy of the loop
            // (paused with an incident, above in `loop_create`); this count is
            // the fallback when no engine runs (3.7's `budget_limited`).
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
            // A budget incident on any scope of the loop pauses it (3.3):
            // no round starts until someone raises the ceiling.
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
            // Round 2+ on a runtime that resumes its session: the task is in
            // the conversation already, only the guide goes (83#P7).
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
            // Restarted by the host (3.1): the round in flight was stopped
            // and its result is unknown.
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
            l.rounds_done = round;
            l.active_ms += now_ms().saturating_sub(round_started);
            l.next_round_ms = l
                .schedule
                .as_deref()
                .and_then(|s| next_round_at(s, now_ms()));
            let Some(done) = done else { break };
            let round_cost = done.usage.as_ref().and_then(|u| u["cost_usd"].as_f64());
            self.note_loop_cost(&mut l, &done);
            // The round was stopped by a budget (refused, or cancelled when
            // a scope of it crossed): pause at the top of the next pass.
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
                    // A round the watchdog stopped is the host's problem:
                    // when the Loop asked for it and nothing was written in
                    // the shared folder, the round goes again (3.1).
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
                        // 84#P1: a failed turn blocks an audited Loop (a
                        // person looks); only the host's own failure above
                        // goes again, with `--resume` (3.1).
                        l.state = "blocked".into();
                        l.stop_reason = Some(format!(
                            "{}: {}",
                            omniget_core::core::llm::goal_audit::STOP_ROUND_FAILED,
                            done.error.clone().unwrap_or_default()
                        ));
                        break;
                    }
                    // Budget cut, no provider, no candidate: another round would fail the same way.
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
                        .audit_round(&l, round, reply, last_audit.as_ref())
                        .await;
                    entry.round_cost_usd = round_cost;
                    entry.round_tokens = round_tokens;
                    entry.guided = guided;
                    l.tokens_used += entry.tokens;
                    if let Some(c) = entry.cost_usd {
                        l.audit_usd += c;
                    }
                    // The audit ran bound to the loop's scope (3.3): the
                    // ledger already has it; without one, add it here.
                    if !self.loop_cost_from_ledger(&mut l) {
                        l.spent_usd += entry.cost_usd.unwrap_or(0.0);
                        l.spent_tokens += entry.tokens;
                    }
                    // Refused by a paused budget scope: no verdict to count,
                    // the top of the next pass pauses the loop.
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
        // A cancel may have landed while the last round ran.
        if let Some(cur) = self.loop_get(&id) {
            if cur.state == "cancelled" || cur.state == "paused" {
                l.state = "cancelled".into();
                l.stop_reason = Some("cancelled".into());
            }
        }
        l.finished_ms = Some(now_ms());
        self.save_loop(&l);
        // A Loop is one piece of background work: it says so once, when it
        // ends, not after every round.
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

    // ── budgets (3.3) ────────────────────────────────────────────────

    /// The account a roster agent's turns are charged to.
    fn account_of(&self, agent_id: &str) -> Option<String> {
        match self.llm.agent(agent_id)?.runtime {
            omniget_core::core::llm::agent::RuntimeKind::Cli { account, .. } => Some(account),
            _ => None,
        }
    }

    /// The budget incident that holds a job back, if one of its scopes
    /// (global, agent, account, job, loop) is paused.
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

    /// The same for a Loop's next round.
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

    /// What a round cost, added to its Loop: dollars (reported, or the
    /// price table's estimate when a dollar ceiling applies), weighted
    /// tokens, and the rounds nobody could price (PROB C5; `jobs.rs:63-65`
    /// no longer reads a missing price as "no cost").
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

    /// The loop's cost as the budget ledger has it (every turn bound to
    /// `loop:<id>`: rounds and their audits). `false` = no engine or no
    /// turn there yet; the caller adds what it knows instead.
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

    /// A hard budget crossing (`BudgetCrossed`): the scope's other running
    /// and queued jobs are cancelled (a Loop's runner pauses the Loop itself, on
    /// its next pass). Jobs of other scopes are left alone.
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
            // The turn that crossed is ending by itself: its work stands.
            .filter(|j| j.request_id.as_deref() != Some(crossing_turn))
            .filter(|j| in_budget_scope(j, &scope, self.account_of(&j.agent_id).as_deref(), &bound))
            .collect();
        for j in victims {
            // Marked first, so nothing starts it again by itself (3.1): the
            // end of its run keeps the mark on the cancelled row.
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

    /// After "raise and resume": every Loop paused by that incident runs
    /// again, unless another incident still holds it.
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

    // ── boot ─────────────────────────────────────────────────────────

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
        // Rounds of a Loop that were in flight: decided with their Loop.
        let mut rounds: HashMap<String, (Job, Option<omniget_core::core::process::StaleProcess>)> =
            HashMap::new();
        for stale in open {
            let Some(mut job) = self.job(&stale.id) else {
                continue;
            };
            // A job still at the gate (queued or waiting for its folder) was
            // never sent: the queue is rebuilt by starting it again.
            let never_sent = matches!(job.state.as_str(), "queued" | "waiting_workspace")
                && job.request_id.is_none()
                && job.started_ms.is_none();
            // Mission jobs are driven (and reconciled) by the mission driver.
            if never_sent
                && job.loop_id.is_none()
                && job.playbook_id.is_none()
                && job.kind != "chat"
                && job.kind != "mission"
            {
                // Provably never dispatched: safe to start now.
                let this = self.clone();
                tauri::async_runtime::spawn(async move {
                    this.run_job(&job.id, None).await;
                });
                continue;
            }
            // Dispatched (or part of a Loop): the result is unknown. Its old
            // process, if it outlived the app, is stopped first (3.1).
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
            // Never re-sent by itself unless it asked to be and it is safe
            // (3.1); otherwise a person resumes, marks done or discards.
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
            // Between two rounds nothing was in flight; a round that was
            // follows the rule of a job (worktree or no write, process gone).
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
                        // The Loop runs this round again; this row is done.
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

    // ── resume after a restart (3.1) ─────────────────────────────────

    /// Copies what the run registry knows about the turn in flight onto the
    /// job: the CLI's session id and the pid of its process, with the OS
    /// start token that tells it from a later process with the same pid.
    /// `true` when something changed.
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

    /// The process a job's turn had when the app died: stopped (with its
    /// tree) if it still runs, so a new run never runs beside it. Written to
    /// the job's log; `None` when nothing was recorded.
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

    /// Whether this job wants to come back by itself: its own switch, else
    /// the setting.
    fn wants_auto_resume(&self, job: &Job) -> bool {
        job.auto_resume.unwrap_or_else(|| {
            crate::storage::config::load_settings(&self.app)
                .llm
                .auto_resume
        })
    }

    /// Starts a lost job again by itself when [`auto_resume_verdict`] says
    /// so, with the note and `--resume` of its conversation; otherwise
    /// leaves it as it is. The verdict goes to the log either way. `true`
    /// when it was started again.
    fn auto_resume(
        self: &Arc<Self>,
        job: &mut Job,
        reason: FailureReason,
        process: Option<omniget_core::core::process::StaleProcess>,
    ) -> bool {
        let wanted = self.wants_auto_resume(job);
        let mut verdict = auto_resume_verdict(job, wanted, reason, process, job.attempt);
        // A scope of it is paused by a budget incident (3.3): it would only
        // fail again before its turn, so it waits for "raise and resume".
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
            // After a stall the run that failed is still unwinding (its
            // cancel token and gate slot go when it returns): start once it
            // is gone, so its cleanup never takes the new run's token.
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
        let note = resume_note(FailureReason::AppRestart, &job.prompt);
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
        // A job in review is done once a person looked at what it changed
        // (2.4); its worktree and branch stay until removed or collected.
        if job.state == "review" {
            job.state = "done".into();
            job.log.push_str("✓ reviewed by you\n");
            self.save_job_checked(&job)?;
            return Ok(job);
        }
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
        if job.worktree_path.is_some() {
            let jid = job.id.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(jobs) = try_get() {
                    if let Err(e) = jobs.remove_worktree(&jid).await {
                        tracing::warn!("[jobs] {jid}: worktree kept: {e}");
                    }
                }
            });
        }
        Ok(job)
    }

    pub fn loop_resume(self: &Arc<Self>, id: &str) -> Result<LoopDef, String> {
        let mut l = self
            .loop_get(id)
            .ok_or_else(|| format!("{ERR_JOBS}: no loop {id}"))?;
        // A blocked Loop resumes too: the next round is audited again, with
        // the streaks from zero (84#P1: "blocked is resumable").
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
        read_triggers(&self.db())
    }

    pub fn trigger(&self, id: &str) -> Option<Trigger> {
        read_trigger(&self.db(), id)
    }

    /// Read, change and write one routine's body under one lock, so two
    /// writers (a firing and a notification, say) never lose each other's
    /// counters. The schedule column is not touched.
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

    pub fn trigger_save(self: &Arc<Self>, mut t: Trigger) -> Result<Trigger, String> {
        if self.llm.agent(&t.agent_id).is_none() {
            return Err(format!("{ERR_JOBS}: no agent `{}`", t.agent_id));
        }
        let cron = match t.kind.as_str() {
            "cron" => Some(Cron::parse(t.cron.as_deref().unwrap_or(""))?),
            "webhook" => {
                t.cron = None;
                None
            }
            other => return Err(format!("{ERR_JOBS}: unknown trigger kind `{other}`")),
        };
        if parse_zone(&t.tz).is_none() {
            tracing::warn!(
                "[jobs] trigger zone `{}` is not local, UTC or ±HH:MM; this machine's zone is used",
                t.tz
            );
        }
        if t.name.trim().is_empty() {
            t.name = t.prompt.chars().take(48).collect();
        }
        {
            let db = self.db();
            let old = (!t.id.is_empty())
                .then(|| read_trigger(&db, &t.id))
                .flatten();
            if t.id.is_empty() {
                t.id = short_id("t");
            }
            // A save that keeps the line, the zone and the switch keeps the
            // schedule (and what waits to catch up); anything else starts
            // over from now: turning a routine back on does not recover the
            // time it was off.
            let same = old.as_ref().is_some_and(|o| {
                o.kind == t.kind && o.cron == t.cron && o.tz == t.tz && o.enabled == t.enabled
            });
            match &old {
                Some(o) => {
                    t.created_ms = o.created_ms;
                    t.fire_count = o.fire_count;
                    t.last_fired_ms = o.last_fired_ms;
                    t.last_digest = o.last_digest.clone();
                    t.last_notified_ms = o.last_notified_ms;
                    t.missed = o.missed;
                    t.last_missed_ms = o.last_missed_ms;
                    t.last_run_id = o.last_run_id.clone();
                    t.backlog = if same { o.backlog } else { 0 };
                }
                None => {
                    t.created_ms = now_ms();
                    t.backlog = 0;
                }
            }
            t.next_run_ms = match (&old, &cron) {
                (Some(o), _) if same => o.next_run_ms,
                (_, Some(c)) if t.enabled => c.next_after(now_ms() as i64, &t.tz),
                _ => None,
            };
            set_schedule(&db, &t)?;
        }
        let _ = self.app.emit(EVENT_TRIGGER, trigger_view(&t));
        self.clone().ensure_cron();
        Ok(t)
    }

    /// Silence (or unsilence) a routine without touching its schedule.
    pub fn trigger_mute(&self, id: &str, muted: bool) -> Result<Trigger, String> {
        self.update_trigger(id, |t| t.muted = muted)
            .ok_or_else(|| format!("{ERR_JOBS}: no trigger {id}"))
    }

    pub fn trigger_delete(&self, id: &str) -> Result<(), String> {
        self.db()
            .execute("DELETE FROM triggers WHERE id=?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
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

    /// The ticker only exists while some cron trigger is enabled. Its first
    /// pass runs at once with no grace: whatever came due while the app was
    /// closed is a missed time and follows the routine's policy (1.4).
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

    /// One pass over the cron routines: each due one is claimed with a
    /// compare-and-set on its schedule before anything starts, so no time
    /// fires twice (this pass, the next one, or after a restart).
    fn cron_tick(self: &Arc<Self>, grace_ms: i64) {
        let _pass = self.cron_pass.lock().unwrap_or_else(|e| e.into_inner());
        let now = now_ms() as i64;
        // Worktree GC rides the same ticker, at most once an hour (2.3).
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
    /// time zone. `None` when it never fires within a year (e.g.
    /// `0 0 31 2 *`).
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

    /// [`Self::next_after_at`] on epoch milliseconds, with the wall clock of
    /// the routine's zone (`tz`, see [`zone_of`]).
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

// ── cron: durable schedule (1.4) ──────────────────────────────────────

/// The zone a routine's cron line is read in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Zone {
    /// This machine's zone, looked up at every computation (daylight saving
    /// follows the OS).
    Local,
    Fixed(chrono::FixedOffset),
}

/// `local` (or empty), `UTC`, `+HH:MM`, `-HHMM`, `+HH`. `None` for anything
/// else: IANA names (`America/Sao_Paulo`) would need the tz database
/// (`chrono-tz`), which the app does not carry.
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

/// [`parse_zone`], with an unknown zone read as this machine's (logged when
/// the routine is saved).
pub fn zone_of(tz: &str) -> Zone {
    parse_zone(tz).unwrap_or(Zone::Local)
}

/// A scheduled time that is at most this late when the ticker sees it still
/// runs as itself (a tick that came late is not a "missed" run). On boot the
/// grace is 0: the app was closed, every time that passed is missed.
pub const CRON_GRACE_MS: i64 = 3 * CRON_TICK_S as i64 * 1000;
/// Counting missed times stops here (a year closed with `*/1`).
const MISSED_COUNT_MAX: u32 = 10_000;

/// What one pass of the cron ticker does with one routine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TickPlan {
    /// Jobs to start now.
    pub fire_now: u32,
    /// Catch-up runs left for the next passes (the new `Trigger.backlog`).
    pub backlog: u32,
    /// Scheduled times that passed with nothing to run them.
    pub missed_slots: u32,
    /// How many of those will still run (now or from the backlog).
    pub caught_up: u32,
    /// Runs of this pass not started because the last run is still going.
    pub skipped_active: u32,
    /// To add to `Trigger.missed`.
    pub not_run: u32,
    pub next_run_ms: Option<i64>,
}

/// The pure half of the cron: given the routine as stored, the time, how
/// late a time may be seen and still run as itself (`grace_ms`), and whether
/// its last run is still going, what to start and what to count. Every
/// branch is decided here, so the clock can be injected in tests.
pub fn plan_tick(t: &Trigger, cron: &Cron, now_ms: i64, grace_ms: i64, active: bool) -> TickPlan {
    let mut plan = TickPlan {
        backlog: t.backlog.min(CATCH_UP_CAP),
        next_run_ms: t.next_run_ms,
        ..Default::default()
    };
    // A routine the ticker never saw (just migrated, just enabled) starts
    // from now: nothing before it counts as missed.
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
        // The runs of this pass are skipped while the last one goes on; the
        // catch-up already waiting keeps waiting (and goes one at a time).
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

// Trigger rows: the JSON body, plus the schedule in a column of its own.
// The schedule is the one field two writers race on (the ticker, the boot
// pass, a webhook firing, a save), so it is never rewritten from a copy of
// the body: only `cas_schedule` (the ticker) and `set_schedule` (a save that
// changed the line) move it.

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

/// Writes the body. A new row takes its schedule from `t`; an existing row
/// keeps the schedule it has.
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

/// Writes body and schedule, whatever the schedule was (a save).
fn set_schedule(db: &Connection, t: &Trigger) -> Result<(), String> {
    let body = serde_json::to_string(t).map_err(|e| e.to_string())?;
    db.execute(
        "INSERT INTO triggers (id, body, next_run_ms) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET body=excluded.body, next_run_ms=excluded.next_run_ms",
        params![t.id, body, t.next_run_ms],
    )
    .map(|_| ())
    .map_err(|e| format!("{ERR_JOBS}: {e}"))
}

/// Compare-and-set: writes `t` (body and schedule) only if the stored
/// schedule is still `seen`. `true` = this caller owns the times it planned;
/// `false` = someone else already moved it, fire nothing.
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

/// Routines saved before the durable schedule get one computed from now
/// (the times before this version are not "missed"). Idempotent: rows that
/// have a schedule are left alone.
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

/// A job of this routine has not finished yet.
pub fn trigger_busy(db: &Connection, id: &str) -> bool {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM jobs WHERE trigger_id=?1 AND state IN ('queued','waiting_workspace','running','waiting_approval'))",
        params![id],
        |r| r.get::<_, bool>(0),
    )
    .unwrap_or(false)
}

/// One routine, one pass of the ticker: plans with [`plan_tick`] and, if
/// anything changes, takes it with [`cas_schedule`]. `Some` only for the
/// caller that won: it starts `plan.fire_now` runs; a loser (or a routine
/// with nothing due) gets `None`. `mission_active` covers a routine whose
/// last firing was a mission still running (the jobs table does not know).
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
    use chrono::TimeZone;
    let now = chrono::Local::now();
    // The stored schedule; a routine the ticker has not seen yet shows the
    // time it will get.
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

/// `{ base_url }` of the local bridge, for the webhook URL in the UI.
pub fn bridge_info(app: &AppHandle) -> Value {
    let settings = crate::storage::config::load_settings(app);
    // No bearer here: the page shows `$OMNIGET_TOKEN` and the token list
    // (U6, 2.1), never a secret in plain text.
    json!({
        "base_url": format!("http://127.0.0.1:{}", settings.bridge.port),
    })
}

// ── watchdog, deadline and asks: the pure half (1.2, 1.3) ─────────────

/// What a running job's explain reads besides its row.
#[derive(Debug, Clone, Default)]
pub struct Live {
    pub request_id: String,
    pub started_ms: u64,
    /// Last event of the turn's stream (any kind).
    pub last_event_ms: u64,
    pub deadline_ms: Option<u64>,
}

/// The `state_seq` a save writes: the stored one, plus one when the state
/// changes. A new row starts at 1.
pub fn next_state_seq(stored: Option<(&str, i64)>, new_state: &str) -> i64 {
    match stored {
        None => 1,
        Some((state, seq)) if state == new_state => seq,
        Some((_, seq)) => seq + 1,
    }
}

/// The one-word reason of a failed job, read off the error it ended with.
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

/// Why a job stopped, typed (3.1, S16). The row keeps the word
/// ([`FailureReason::as_str`]) so older rows and the UI's labels stay as
/// they are; what matters here is which of them are the host's fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureReason {
    /// The app closed (or was killed) while the job ran.
    AppRestart,
    /// The watchdog heard nothing for too long.
    Stalled,
    /// The CLI never started talking.
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

    /// The run died around the agent, not because of what it did: the app
    /// went away, or the process went silent. A deadline, a budget or an
    /// unanswered question are verdicts on the job; running it again would
    /// meet them again.
    pub fn is_infrastructure(self) -> bool {
        matches!(
            self,
            FailureReason::AppRestart | FailureReason::Stalled | FailureReason::NoStart
        )
    }
}

/// Runs the host starts by itself for one job (or one Loop between two
/// finished rounds) before it leaves the rest to a person: a crash that
/// repeats is not retried forever.
pub const AUTO_RESUME_MAX: u32 = 2;

/// Whether a tool call can change something. Only reading and searching is
/// known to be harmless; anything else, an unknown MCP tool included, counts
/// as a write, so a doubt never re-runs a job in the shared folder.
pub fn tool_may_write(name: &str) -> bool {
    const READS: &[&str] = &[
        // Claude Code / Codex
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
        // OmniGet's own tools
        "kb_search",
        "web_search",
        "web_fetch",
    ];
    let n = name.trim().to_ascii_lowercase();
    !(READS.contains(&n.as_str()) || code_tools::READ_TOOLS.contains(&n.as_str()))
}

/// Why the host does or does not start a lost job again by itself (3.1).
/// Written to the job's log either way, so "why did it (not) come back" has
/// an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeVerdict {
    Resume,
    /// Neither the job, its Loop nor the setting asked for it.
    Off,
    /// A deadline, a budget, an unanswered question: not the host's fault.
    NotInfrastructure,
    /// Chat turns, missions, playbook steps and catalog tool runs have their
    /// own owners.
    NotThisKind,
    Attempts,
    /// It started a tool that writes, in the folder the person works in.
    WroteInSharedFolder,
    /// Its old process could not be stopped: never two at once.
    ProcessAlive,
    /// A budget stopped it or holds one of its scopes (3.3): only "raise and
    /// resume" (a person) starts it again.
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

/// The rule of 3.1: again with `--resume` only after an infrastructure
/// failure, only when asked (`wanted`: the job's own switch, else its
/// Loop's, else the setting), only for a job that ran in its own worktree
/// or never started a tool that writes, never while its old process lives,
/// and at most [`AUTO_RESUME_MAX`] times.
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
    // Cancelled or refused by a budget (3.3): never the host's failure,
    // whatever reason the watchdog or the boot put on it afterwards.
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

/// What a job run again by the host is told before its task.
pub fn resume_note(reason: FailureReason, prompt: &str) -> String {
    let why = match reason {
        FailureReason::AppRestart => "The app closed in the middle of this task",
        _ => "This task stopped because the agent went silent",
    };
    format!(
        "{why} and its result is unknown. Check what is already done before changing anything, and do not repeat an action that already happened. Task: {prompt}"
    )
}

/// D4: the deadline of a background job that has no guard of its own. A chat
/// turn has a person in front of it; a mission brings its own guard.
pub fn default_deadline_ms(kind: &str, started_ms: u64, deadline_min: u32) -> Option<u64> {
    matches!(kind, "run" | "loop" | "trigger")
        .then(|| started_ms + u64::from(deadline_min) * 60_000)
}

/// How long a question of a background job waits: the explicit setting, or
/// what is left of the job's deadline (79#P4). `None` = not a job with a
/// deadline (the chat's timeout applies).
pub fn job_ask_timeout(
    now_ms: u64,
    deadline_ms: Option<u64>,
    ask_job: Option<std::time::Duration>,
) -> Option<std::time::Duration> {
    ask_job
        .or_else(|| deadline_ms.map(|d| std::time::Duration::from_millis(d.saturating_sub(now_ms))))
}

/// `GET /v1/jobs/{id}/explain`: why the job is in its state right now
/// (79#P8). `asks` are the pending questions of its turn (each with
/// `asked_ms` and, when known, `expires_ms`); `pid` only ever leaves through
/// the local bridge, never through MCP.
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
        // Holds one of the parallel job slots right now.
        "slot": running,
        // Resume after a restart (3.1): the session `--resume` continues,
        // the host's own attempts, this job's switch, whether it wrote.
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

// ── one gate, one writer per folder, the queue explained (1.5, 1.6) ──

use omniget_core::core::llm::job_gate::{GateView, Limits, WaitReason};

/// The gate's limits from the settings, clamped like the settings card
/// (a 0 would admit nobody, a 99 would start 99 `claude` at once).
pub fn gate_limits(s: &omniget_core::models::settings::LlmSettings) -> Limits {
    let (lo, hi) = omniget_core::core::llm::job_gate::MAX_PARALLEL_RANGE;
    Limits {
        max_parallel: (s.jobs_max_parallel as usize).clamp(lo, hi),
        max_per_agent: s.jobs_max_per_agent.map(|n| (n as usize).clamp(lo, hi)),
        stagger: omniget_core::core::llm::job_gate::STAGGER,
    }
}

/// What the jobs page sends; `max_per_agent: 0` = no per-agent cap.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct JobLimitsInput {
    pub max_parallel: Option<u32>,
    pub max_per_agent: Option<u32>,
}

/// Applies the input over the stored settings, clamped like [`gate_limits`].
pub fn merge_limits(
    mut s: omniget_core::models::settings::LlmSettings,
    input: &JobLimitsInput,
) -> omniget_core::models::settings::LlmSettings {
    let (lo, hi) = omniget_core::core::llm::job_gate::MAX_PARALLEL_RANGE;
    if let Some(n) = input.max_parallel {
        s.jobs_max_parallel = (n as usize).clamp(lo, hi) as u32;
    }
    if let Some(n) = input.max_per_agent {
        s.jobs_max_per_agent = (n > 0).then(|| (n as usize).clamp(lo, hi) as u32);
    }
    s
}

/// The row a waiting job shows: `waiting_workspace` when another job holds
/// its folder, `queued` otherwise; `wait_reason` is a code the UI
/// translates (`slot`, `agent_cap`, `workspace:<job id>`).
pub fn wait_state(reason: &WaitReason) -> (&'static str, String) {
    match reason {
        WaitReason::Workspace { holder, .. } => {
            ("waiting_workspace", format!("workspace:{holder}"))
        }
        other => ("queued", other.code().to_string()),
    }
}

/// One English line for a waiting job (the CLI prints it; the UI translates
/// the code instead).
fn wait_detail(reason: &WaitReason, view: &GateView, agent: &str) -> String {
    match reason {
        WaitReason::Slot => format!("all {} job slots are busy", view.limits.max_parallel),
        WaitReason::AgentCap { cap } => format!("agent {agent} is at its cap of {cap} jobs"),
        WaitReason::Workspace { holder, folder } => format!("folder busy by {holder} ({folder})"),
    }
}

/// `GET /v1/state`, `llm_jobs_state` and `llm://queue`: what runs, what
/// waits and why, and the limits (symphony's state snapshot, 83#P5).
/// `blocked` = holds a slot but waits for a person.
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

// ── job diff: the pure half (1.7) ─────────────────────────────────────

/// `diff_unavailable` code of a job that ran without a folder.
/// A job that ended well: `done`, or `review` (it wrote files a person has
/// not looked at yet, 2.4).
pub fn is_success(state: &str) -> bool {
    matches!(state, "done" | "review" | "done_unreported")
}

/// A job that ended, one way or another: nothing will move it by itself.
pub fn is_terminal(state: &str) -> bool {
    matches!(
        state,
        "done" | "review" | "done_unreported" | "failed" | "cancelled" | "interrupted"
    )
}

/// What a background job's prompt ends with (2.6, 78#P7): the agent's own
/// verdict in one line the host can read. Kept short, it is paid on every
/// job; `scripts/llm/missions_eval.py` sends the same text (B0 measures it).
pub const OUTCOME_SUFFIX: &str = "\n\nEnd your reply with one line: `OUTCOME: succeeded` or `OUTCOME: failed`, then ` — ` and up to 3 sentences on what you did.";

/// Jobs whose prompt asks for the `OUTCOME:` line: a run (from the CLI,
/// the window, the bridge or MCP) and a mission task. Routines keep their
/// text as it is (its digest decides whether to notify); Loop rounds and
/// chat turns have their own end.
pub fn asks_outcome(kind: &str) -> bool {
    matches!(kind, "run" | "mission")
}

/// The agent's `OUTCOME:` line, the last one of the text: `{status,
/// summary}`, status `succeeded|failed`. Markdown around it (`**`, `` ` ``)
/// and the dash between the two halves (`—`, `–`, `-`, `:`) are tolerated.
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

/// A `run` that ended `done` without its `OUTCOME:` line becomes
/// `done_unreported` (2.6): the process ended well, the agent did not say
/// how. `review` stays `review` (a person looks anyway), and missions keep
/// `done` (their criteria decide).
pub fn settle_outcome<'a>(kind: &str, state: &'a str, reported: bool) -> &'a str {
    if kind == "run" && state == "done" && !reported {
        "done_unreported"
    } else {
        state
    }
}

/// The budget scopes a job's turns count against besides global, agent and
/// account (3.3): the job itself and its Loop.
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

/// Whether a job's work is inside a budget scope (what a hard crossing of
/// that scope cancels). `bound` = the conversations a mission bound.
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

/// A hard budget policy (warn at 80 %) for the whole run of a job or Loop,
/// the one budget mechanism (3.3) behind `budget_usd`, `max_cost_usd` and
/// `token_budget`. No engine (tests, a failed boot) = nothing to do.
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

/// The policies of a job or Loop that is gone: nothing can cross them.
fn forget_budget_policies(scope: &omniget_core::core::llm::budget::policy::Scope) {
    let Some(engine) = omniget_core::core::llm::budget::policy::active() else {
        return;
    };
    for p in engine.policies().into_iter().filter(|p| &p.scope == scope) {
        let _ = engine.delete(&p.id, omniget_core::core::llm::audit::Actor::window());
    }
}

/// Adds a `job.start` budget (US$) to the job's guard: the job stops
/// `failed` (reason `limit`) once its known cost passes it. A provider with
/// no price never trips it (the cost stays unknown).
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

/// The state a job ends in once its diff is known: a `done` job that
/// changed files goes to `review` (2.4, 83#P4); nothing else moves.
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

/// Most files a proof lists (the totals count them all).
pub const PROOF_MAX_FILES: usize = 200;

/// The proof of a finished job (2.4, symphony P4), built by the host from
/// what it recorded, never from the agent's words: 0 tokens. `ends` = the
/// two ends of the diff (merge base and worktree tree, or the shadow trees
/// in shared mode); `host` = the machine's name.
pub fn build_proof(job: &Job, ends: Option<&(String, String)>, host: &str) -> Value {
    let short = |s: &str| s.chars().take(7).collect::<String>();
    let worktree = job.branch.is_some() && job.base_oid.is_some();
    let workspace = effective_workspace(job).unwrap_or_default();
    // Worktree mode stamps commits (merge base → HEAD of the branch);
    // shared mode the snapshot trees, marked so nobody takes them for
    // commits.
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

/// Appends a check to a proof's `checks`, its exit code read from the
/// `exit N` first line `run_check` writes.
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

/// What `GET /v1/jobs/{id}/proof` answers: the proof, or why there is none
/// (409 while the job runs, 404 for a job that ended before 2.4).
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

/// A job still at the gate or running: its folder or worktree is in use.
pub fn is_active(state: &str) -> bool {
    matches!(
        state,
        "queued" | "waiting_workspace" | "running" | "waiting_approval"
    )
}

/// How long a finished job keeps its worktree before the GC removes it
/// (its work stays on the branch).
pub const WORKTREE_TTL_MS: u64 = 7 * 24 * 3600 * 1000;
/// Least time between two GC passes of the cron ticker.
pub const WORKTREE_GC_EVERY_MS: u64 = 3600 * 1000;

/// Why the GC removes the worktree of a job (`None` = keep it): `orphan`
/// when the job row is gone, `ttl` when it ended long enough ago. An active
/// job's worktree is never touched.
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

/// The repository of a job's worktree and the worktree's root folder
/// (`worktree_path` may be a subfolder of it).
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

/// [`Jobs::remove_worktree`] without the row: for a job being deleted.
async fn remove_job_worktree(
    job: &Job,
) -> Result<omniget_core::core::vcs::job_worktree::RemoveOutcome, String> {
    remove_job_worktree_with(crate::job_workflow::store().as_deref(), job).await
}

/// [`remove_job_worktree`] with the workflow store given (tests bring their own).
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
    // The workflow's `before_remove` hook (3.10), when trusted: a failure
    // blocks the removal.
    let hook = crate::job_workflow::before_remove(store, job, &repo, &root).await?;
    let mut out = job_worktree::remove(&repo, &root, &branch, job.base_oid.as_deref())
        .await
        .map_err(|e| format!("{ERR_WORKTREE}: {e}"))?;
    out.hook_note = (!hook.is_empty()).then_some(hook);
    Ok(out)
}

/// The job log line of a removal: what was saved, what the branch became.
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

/// The job log line of a new worktree: where, from what, what came along.
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

/// `<llm dir>/jobs/<id>.patch`.
pub fn patch_path(job_id: &str) -> Option<std::path::PathBuf> {
    let name = sanitize_id(job_id);
    if name.is_empty() || name.contains("..") {
        return None;
    }
    omniget_core::core::llm::roster_store::llm_dir()
        .map(|d| d.join("jobs").join(format!("{name}.patch")))
}

/// Copies a [`omniget_core::core::llm::snapshot::job_diff`] outcome into the
/// job's columns; returns the patch text when the diff is known.
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

/// What `GET /v1/jobs/{id}/diff` answers.
#[derive(Debug, PartialEq)]
pub enum DiffBody {
    Json(Value),
    /// `format=patch`: the unified diff, `text/x-diff`.
    Patch(String),
}

/// `format=files` (default): the file list, totals and why it is
/// unavailable; `format=patch`: the patch itself, 409 when unavailable.
/// `complete` is false for a job still running (diffed against now).
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
    use omniget_core::core::llm::snapshot::{FileChange, JobDiff, ERR_DIFF_UNAVAILABLE};

    fn change(path: &str, status: &str, add: Option<u64>, del: Option<u64>) -> FileChange {
        FileChange {
            path: path.into(),
            status: status.into(),
            additions: add,
            deletions: del,
            binary: add.is_none() && del.is_none(),
        }
    }

    fn done_job() -> Job {
        Job {
            id: "j1".into(),
            kind: "run".into(),
            state: "done".into(),
            ..Default::default()
        }
    }

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

    #[test]
    fn a_diff_outcome_fills_the_columns() {
        let mut job = done_job();
        let patch = apply_diff(
            &mut job,
            Ok(JobDiff {
                files: vec![
                    change("a.txt", "M", Some(2), Some(1)),
                    change("b.pyc", "A", None, None),
                ],
                additions: 2,
                deletions: 1,
                clipped: true,
                complete: true,
                patch: "diff --git a/a.txt b/a.txt\n".into(),
                ..Default::default()
            }),
        );
        assert_eq!(patch.as_deref(), Some("diff --git a/a.txt b/a.txt\n"));
        assert_eq!(job.diff_files.as_ref().map(Vec::len), Some(2));
        assert_eq!(
            (job.additions, job.deletions, job.diff_clipped),
            (Some(2), Some(1), true)
        );
        assert_eq!(job.diff_unavailable, None);

        let none = apply_diff(
            &mut job,
            Err(format!(
                "{ERR_DIFF_UNAVAILABLE}: 1 of 2 turns of this job took no snapshot"
            )),
        );
        assert_eq!(none, None);
        assert_eq!(job.diff_files, None, "never an empty list for unknown");
        assert_eq!(
            job.diff_unavailable.as_deref(),
            Some("no_snapshot: 1 of 2 turns of this job took no snapshot")
        );
        apply_diff(
            &mut job,
            Err("no_workspace: the job ran without a folder".into()),
        );
        assert_eq!(
            job.diff_unavailable.as_deref(),
            Some("no_workspace: the job ran without a folder")
        );
        apply_diff(&mut job, Err("ERR_CODE_SNAPSHOT: git diff: boom".into()));
        assert_eq!(
            job.diff_unavailable.as_deref(),
            Some("error: ERR_CODE_SNAPSHOT: git diff: boom")
        );
    }

    #[test]
    fn the_view_says_unavailable_with_its_reason() {
        let mut job = done_job();
        job.diff_unavailable = Some("no_snapshot: no turn of this job took a snapshot".into());
        let DiffBody::Json(v) = diff_view(&job, None, None, "files", true).unwrap() else {
            panic!("files is json")
        };
        assert_eq!(v["available"], false);
        assert_eq!(v["reason"], "no_snapshot");
        assert_eq!(v["detail"], "no turn of this job took a snapshot");
        assert_eq!(v["files"], json!([]));
        let (status, msg) = diff_view(&job, None, None, "patch", true).unwrap_err();
        assert_eq!(status, 409);
        assert!(msg.starts_with(ERR_DIFF_UNAVAILABLE), "{msg}");
        // Not computed at all reads as unavailable too, never as "no changes".
        let DiffBody::Json(v) = diff_view(&done_job(), None, None, "", true).unwrap() else {
            panic!()
        };
        assert_eq!(v["available"], false);
        assert_eq!(
            diff_view(&done_job(), None, None, "zip", true)
                .unwrap_err()
                .0,
            400
        );
    }

    #[test]
    fn the_files_format_carries_totals_and_the_patch_path() {
        let mut job = done_job();
        job.diff_files = Some(vec![
            change("a.txt", "A", Some(3), Some(0)),
            change("m.bin", "M", None, None),
        ]);
        job.additions = Some(3);
        job.deletions = Some(0);
        let path = std::path::Path::new("/data/llm/jobs/j1.patch");
        let DiffBody::Json(v) = diff_view(&job, Some("p"), Some(path), "files", false).unwrap()
        else {
            panic!()
        };
        assert_eq!(v["available"], true);
        assert_eq!(v["job_id"], "j1");
        assert_eq!(v["file_count"], 2);
        assert_eq!(
            (v["additions"].as_u64(), v["deletions"].as_u64()),
            (Some(3), Some(0))
        );
        assert_eq!(v["files"][1]["binary"], true);
        assert_eq!(v["complete"], false);
        assert_eq!(v["head_oid"], Value::Null);
        assert_eq!(v["patch_path"], "/data/llm/jobs/j1.patch");
        assert_eq!(v["reason"], Value::Null);
    }

    #[test]
    fn the_patch_format_returns_the_stored_patch() {
        let mut job = done_job();
        job.diff_files = Some(vec![change("a.txt", "A", Some(1), Some(0))]);
        assert_eq!(
            diff_view(&job, Some("+x\n"), None, "patch", true).unwrap(),
            DiffBody::Patch("+x\n".into())
        );
        // Known, but the file went away: said, not faked.
        assert_eq!(
            diff_view(&job, None, None, "patch", true).unwrap_err().0,
            404
        );
        // Known and empty (a read-only job): an empty patch, 200.
        job.diff_files = Some(Vec::new());
        assert_eq!(
            diff_view(&job, Some(""), None, "patch", true).unwrap(),
            DiffBody::Patch(String::new())
        );
    }

    #[test]
    fn patches_live_next_to_the_jobs_database() {
        if let Some(p) = patch_path("j1") {
            assert!(p.ends_with("llm/jobs/j1.patch"), "{p:?}");
        }
        if let Some(p) = patch_path("../../etc/x") {
            assert!(!p.to_string_lossy().contains(".."), "{p:?}");
        }
    }
}

/// BN4 (`scripts/llm/diff_eval.py`): what `GET /v1/jobs/{id}/diff` does
/// for a job that already ended, on a 50-file diff: read the row (the file
/// list is JSON in `diff_files`), read the patch file, build both answers.
/// Everything but axum and the socket.
#[cfg(test)]
mod bn4_read {
    use super::*;
    use omniget_core::core::llm::snapshot::FileChange;
    use std::time::Instant;

    fn bench(metric: &str, value: Value) {
        if let Ok(path) = std::env::var("OMNIGET_BENCH_OUT") {
            use std::io::Write as _;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let _ = writeln!(
                    f,
                    "{}",
                    json!({"bench": "bn4", "metric": metric, "value": value, "level": "stored read (row + patch file + diff_view)"})
                );
            }
        }
    }

    #[test]
    fn bn4_the_stored_diff_answers_fast() {
        let dir = std::env::temp_dir().join(format!("omniget-bn4-read-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = Connection::open(dir.join("jobs.db")).unwrap();
        ensure_schema(&db, 0).unwrap();
        let files: Vec<FileChange> = (0..50)
            .map(|i| FileChange {
                path: format!("src/m{:02}/f{i:02}.txt", i % 6),
                status: "M".into(),
                additions: Some(1),
                deletions: Some(1),
                binary: false,
            })
            .collect();
        let patch: String = files
            .iter()
            .map(|f| {
                let body: String = (0..6).map(|l| format!(" context {l}\n")).collect();
                format!(
                    "diff --git a/{p} b/{p}\nindex 1111111..2222222 100644\n--- a/{p}\n+++ b/{p}\n@@ -5,7 +5,7 @@\n{body}-old line\n+new line\n{body}",
                    p = f.path
                )
            })
            .collect();
        db.execute(
            "INSERT INTO jobs (id, kind, agent_id, conversation_id, prompt, state, created_ms, diff_files, additions, deletions)
             VALUES ('j50', 'run', 'a', 'c', 'p', 'done', 1, ?1, 50, 50)",
            params![serde_json::to_string(&files).unwrap()],
        )
        .unwrap();
        let patch_file = dir.join("j50.patch");
        std::fs::write(&patch_file, &patch).unwrap();
        let reps = 50;
        let mut times = Vec::new();
        for _ in 0..reps {
            let t = Instant::now();
            let job = db
                .query_row(
                    &format!("SELECT {JOB_COLS} FROM jobs WHERE id='j50'"),
                    [],
                    row_to_job,
                )
                .unwrap();
            let text = std::fs::read_to_string(&patch_file).unwrap();
            let DiffBody::Json(v) =
                diff_view(&job, Some(&text), Some(&patch_file), "files", true).unwrap()
            else {
                panic!()
            };
            let bytes = serde_json::to_vec(&v).unwrap();
            let DiffBody::Patch(p) =
                diff_view(&job, Some(&text), Some(&patch_file), "patch", true).unwrap()
            else {
                panic!()
            };
            assert!(!bytes.is_empty() && p.len() == patch.len());
            assert_eq!(v["file_count"], 50);
            times.push(t.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let (p50, max) = (times[reps / 2], times[reps - 1]);
        bench("stored_read_files", json!(50));
        bench("stored_read_ms_p50", json!(p50));
        bench("stored_read_ms_max", json!(max));
        assert!(p50 < 200.0, "p50 {p50} ms");
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod queue_tests {
    use super::*;
    use omniget_core::core::llm::folder_key::FolderKey;
    use omniget_core::core::llm::job_gate::{Class, Held, Queued};
    use omniget_core::models::settings::LlmSettings;

    #[test]
    fn the_limits_are_clamped() {
        let l = gate_limits(&LlmSettings::default());
        assert_eq!((l.max_parallel, l.max_per_agent), (4, None));
        assert_eq!(l.stagger, omniget_core::core::llm::job_gate::STAGGER);
        let odd = LlmSettings {
            jobs_max_parallel: 0,
            jobs_max_per_agent: Some(99),
            ..LlmSettings::default()
        };
        let l = gate_limits(&odd);
        assert_eq!((l.max_parallel, l.max_per_agent), (1, Some(16)));
        let merged = merge_limits(
            LlmSettings::default(),
            &JobLimitsInput {
                max_parallel: Some(40),
                max_per_agent: Some(2),
            },
        );
        assert_eq!(
            (merged.jobs_max_parallel, merged.jobs_max_per_agent),
            (16, Some(2))
        );
        let merged = merge_limits(
            merged,
            &JobLimitsInput {
                max_parallel: None,
                max_per_agent: Some(0),
            },
        );
        assert_eq!(
            (merged.jobs_max_parallel, merged.jobs_max_per_agent),
            (16, None),
            "0 = no per-agent cap"
        );
    }

    #[test]
    fn a_folder_wait_is_waiting_workspace_with_the_holder() {
        let folder = FolderKey::normalize("/w/repo", false, false);
        assert_eq!(
            wait_state(&WaitReason::Workspace {
                holder: "j1".into(),
                folder
            }),
            ("waiting_workspace", "workspace:j1".to_string())
        );
        assert_eq!(
            wait_state(&WaitReason::Slot),
            ("queued", "slot".to_string())
        );
        assert_eq!(
            wait_state(&WaitReason::AgentCap { cap: 2 }),
            ("queued", "agent_cap".to_string())
        );
    }

    #[test]
    fn the_state_view_explains_every_queued_job() {
        let folder = FolderKey::normalize("/w/repo", false, false);
        let held = |id: &str, folder: Option<FolderKey>| Held {
            job_id: id.into(),
            agent_id: "claude".into(),
            class: Class::Manual,
            folder,
        };
        let view = GateView {
            held: vec![held("run1", Some(folder.clone())), held("ask1", None)],
            queued: vec![
                Queued {
                    job_id: "q1".into(),
                    agent_id: "claude".into(),
                    class: Class::Manual,
                    folder: Some(folder.clone()),
                    position: 1,
                    reason: WaitReason::Workspace {
                        holder: "run1".into(),
                        folder: folder.clone(),
                    },
                },
                Queued {
                    job_id: "q2".into(),
                    agent_id: "codex".into(),
                    class: Class::Mission,
                    folder: None,
                    position: 2,
                    reason: WaitReason::Slot,
                },
            ],
            limits: Limits {
                max_parallel: 2,
                max_per_agent: None,
                stagger: std::time::Duration::from_millis(500),
            },
            available: 0,
        };
        let job = |id: &str| -> Option<Job> {
            Some(Job {
                id: id.into(),
                kind: if id == "q2" { "mission" } else { "run" }.into(),
                agent_id: "claude".into(),
                state: match id {
                    "ask1" => "waiting_approval",
                    "run1" => "running",
                    "q1" => "waiting_workspace",
                    _ => "queued",
                }
                .into(),
                workspace: Some("/w/repo".into()),
                ..Default::default()
            })
        };
        let v = state_view(&view, &job);
        assert_eq!(v["running"].as_array().unwrap().len(), 1);
        assert_eq!(v["running"][0]["id"], "run1");
        assert_eq!(v["blocked"][0]["id"], "ask1");
        assert_eq!(v["blocked"][0]["reason"], "approval");
        let queued = v["queued"].as_array().unwrap();
        assert_eq!(queued.len(), 2);
        for q in queued {
            assert!(q["position"].as_u64().is_some(), "{q}");
            assert!(!q["reason"].as_str().unwrap_or("").is_empty(), "{q}");
            assert!(!q["detail"].as_str().unwrap_or("").is_empty(), "{q}");
        }
        assert_eq!(queued[0]["reason"], "workspace");
        assert_eq!(queued[0]["holder"], "run1");
        assert!(queued[0]["detail"].as_str().unwrap().contains("run1"));
        assert_eq!(queued[1]["reason"], "slot");
        assert_eq!(queued[1]["kind"], "mission");
        assert_eq!(v["limits"]["max_parallel"], 2);
        assert_eq!(v["limits"]["in_use"], 2);
        assert_eq!(v["limits"]["available"], 0);
        assert_eq!(v["limits"]["stagger_ms"], 500);
        assert!(v["limits"]["max_per_agent"].is_null());
    }
}

#[cfg(test)]
mod watchdog_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_state_seq_only_grows_on_a_state_change() {
        assert_eq!(next_state_seq(None, "queued"), 1);
        assert_eq!(next_state_seq(Some(("queued", 1)), "queued"), 1);
        assert_eq!(next_state_seq(Some(("queued", 1)), "running"), 2);
        assert_eq!(next_state_seq(Some(("running", 2)), "waiting_approval"), 3);
        assert_eq!(next_state_seq(Some(("waiting_approval", 3)), "running"), 4);
        // A row from before the column (seq 0) still moves forward.
        assert_eq!(next_state_seq(Some(("running", 0)), "done"), 1);
    }

    #[test]
    fn a_budget_scope_reaches_the_jobs_inside_it() {
        use omniget_core::core::llm::budget::policy::Scope;
        let job = Job {
            id: "j1".into(),
            agent_id: "worker".into(),
            conversation_id: "loop-l1".into(),
            loop_id: Some("l1".into()),
            ..Default::default()
        };
        assert_eq!(
            work_scopes(&job),
            vec![Scope::Job("j1".into()), Scope::Loop("l1".into())]
        );
        let inside =
            |s: Scope, acct: Option<&str>, bound: &[String]| in_budget_scope(&job, &s, acct, bound);
        assert!(inside(Scope::Global, None, &[]));
        assert!(inside(Scope::Agent("worker".into()), None, &[]));
        assert!(!inside(Scope::Agent("other".into()), None, &[]));
        assert!(inside(Scope::Account("max-1".into()), Some("max-1"), &[]));
        assert!(!inside(Scope::Account("max-1".into()), Some("max-2"), &[]));
        assert!(inside(Scope::Loop("l1".into()), None, &[]));
        assert!(!inside(Scope::Job("j2".into()), None, &[]));
        assert!(inside(
            Scope::Mission("m".into()),
            None,
            &["loop-l1".to_string()]
        ));
        assert!(!inside(Scope::Mission("m".into()), None, &[]));
        assert_eq!(
            failure_reason_of("ERR_LLM_BUDGET: budget paused: loop:l1 0.06 of 0.05 USD (run)"),
            Some("budget")
        );
    }

    #[test]
    fn a_stall_is_failure_reason_stalled() {
        assert_eq!(
            failure_reason_of("ERR_CLI_STALLED: the CLI printed nothing for 600 s"),
            Some("stalled")
        );
        assert_eq!(
            failure_reason_of("ERR_CLI_NO_START: nothing"),
            Some("no_start")
        );
        assert_eq!(
            failure_reason_of("ERR_TOOL_TIMEOUT: nobody answered"),
            Some("ask_expired")
        );
        assert_eq!(failure_reason_of("ERR_CLI_EXIT: exited with 1"), None);
    }

    /// D4: run, loop and trigger all get a deadline; a chat turn (a person
    /// is right there) and a mission (its own guard) do not get this one.
    #[test]
    fn every_background_kind_gets_a_deadline_and_chat_does_not() {
        for kind in ["run", "loop", "trigger"] {
            assert_eq!(
                default_deadline_ms(kind, 1_000, 60),
                Some(1_000 + 3_600_000),
                "{kind}"
            );
        }
        assert_eq!(default_deadline_ms("chat", 1_000, 60), None);
        assert_eq!(default_deadline_ms("mission", 1_000, 60), None);
    }

    /// A job's question waits as long as the job may still run (or the
    /// explicit setting), never zero: the broker floors it to 1 s and the
    /// deadline then fails the job with its own reason.
    #[test]
    fn the_job_ask_timeout_is_the_deadline_left_never_negative() {
        assert_eq!(
            job_ask_timeout(1_000, Some(61_000), None),
            Some(Duration::from_secs(60))
        );
        assert_eq!(
            job_ask_timeout(1_000, Some(61_000), Some(Duration::from_secs(300))),
            Some(Duration::from_secs(300))
        );
        assert_eq!(
            job_ask_timeout(90_000, Some(61_000), None),
            Some(Duration::ZERO)
        );
        assert_eq!(job_ask_timeout(1_000, None, None), None);
    }

    #[test]
    fn explain_reports_the_age_the_ask_and_the_deadline_left() {
        let job = Job {
            id: "j1".into(),
            kind: "run".into(),
            state: "waiting_approval".into(),
            state_seq: 3,
            request_id: Some("r1".into()),
            started_ms: Some(10_000),
            deadline_ms: Some(70_000),
            ..Default::default()
        };
        let live = Live {
            request_id: "r1".into(),
            started_ms: 10_000,
            last_event_ms: 20_000,
            deadline_ms: Some(70_000),
        };
        let asks = vec![
            json!({"id": "apr_0123456789abcdef0123456789abcdef", "request_id": "r1", "tool": "cli:Bash", "tool_call_id": "c1", "asked_ms": 25_000u64}),
        ];
        let v = explain_view(&job, Some(&live), 30_000, &asks, Some(4242));
        // 2.2: a job's own token reads its explain; the ask id (what answers
        // it) never goes there.
        assert!(!v.to_string().contains("apr_"), "{v}");
        assert_eq!(v["id"], "j1");
        assert_eq!(v["state"], "waiting_approval");
        assert_eq!(v["state_seq"], 3);
        assert_eq!(v["last_event_age_s"], 10);
        assert_eq!(v["ask_pending"], true);
        assert_eq!(v["asks"][0]["tool"], "cli:Bash");
        assert_eq!(v["asks"][0]["age_s"], 5);
        assert_eq!(v["deadline_left_s"], 40);
        assert_eq!(v["pid"], 4242);
        assert_eq!(v["slot"], true);
        // A finished job: no live info, no slot, the reason stays.
        let done = Job {
            state: "failed".into(),
            failure_reason: Some("stalled".into()),
            ..job.clone()
        };
        let v = explain_view(&done, None, 30_000, &[], None);
        assert_eq!(v["slot"], false);
        assert_eq!(v["ask_pending"], false);
        assert_eq!(v["failure_reason"], "stalled");
        assert!(v["last_event_age_s"].is_null());
        assert!(v.get("pid").is_none_or(|p| p.is_null()));
    }
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
        let next = c.next_after_at(&from).unwrap();
        assert_eq!(
            next,
            tz.with_ymd_and_hms(2026, 9, 28, 9, 30, 0).unwrap(),
            "Monday"
        );
        // The wall clock of the zone, not UTC.
        assert_eq!(next.hour(), 9);
        assert!(Cron::parse("0 0 31 2 *")
            .unwrap()
            .next_after_at(&from)
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
mod cron_tests {
    use super::*;
    use chrono::TimeZone;

    const MIN: i64 = 60_000;

    /// 2026-09-26 10:00:00 UTC, a Saturday.
    fn t0() -> i64 {
        chrono::Utc
            .with_ymd_and_hms(2026, 9, 26, 10, 0, 0)
            .unwrap()
            .timestamp_millis()
    }

    fn routine(line: &str, catch_up: CatchUp, concurrency: TriggerConcurrency) -> (Trigger, Cron) {
        let t = Trigger {
            id: "t1".into(),
            kind: "cron".into(),
            cron: Some(line.into()),
            agent_id: "a".into(),
            prompt: "p".into(),
            enabled: true,
            tz: "UTC".into(),
            catch_up,
            concurrency,
            ..Default::default()
        };
        (t, Cron::parse(line).unwrap())
    }

    /// Applies a plan the way the ticker does.
    fn apply(t: &mut Trigger, plan: &TickPlan) {
        t.next_run_ms = plan.next_run_ms;
        t.backlog = plan.backlog;
        t.missed += plan.not_run;
    }

    #[test]
    fn next_after_takes_a_timestamp_and_a_zone() {
        let every = Cron::parse("*/1 * * * *").unwrap();
        assert_eq!(every.next_after(t0(), "UTC"), Some(t0() + MIN));
        assert_eq!(
            every.next_after(t0() + 30_000, "UTC"),
            Some(t0() + MIN),
            "strictly after, on the minute"
        );
        // 09:30 on the wall clock of the zone, not UTC.
        let nine = Cron::parse("30 9 * * *").unwrap();
        let utc = nine.next_after(t0(), "UTC").unwrap();
        let plus2 = nine.next_after(t0(), "+02:00").unwrap();
        assert_eq!(utc - t0(), 23 * 60 * MIN + 30 * MIN, "09:30 UTC tomorrow");
        assert_eq!(plus2, utc - 2 * 60 * MIN, "09:30 at +02:00 is 07:30 UTC");
        assert!(nine.next_after(t0(), "local").is_some());
        assert!(Cron::parse("0 0 31 2 *")
            .unwrap()
            .next_after(t0(), "UTC")
            .is_none());
    }

    #[test]
    fn zones_parse_or_say_unknown() {
        assert_eq!(parse_zone(""), Some(Zone::Local));
        assert_eq!(parse_zone("local"), Some(Zone::Local));
        let east = |s: i32| Some(Zone::Fixed(chrono::FixedOffset::east_opt(s).unwrap()));
        assert_eq!(parse_zone("UTC"), east(0));
        assert_eq!(parse_zone("+02:00"), east(7200));
        assert_eq!(parse_zone("-0330"), east(-(3 * 3600 + 1800)));
        assert_eq!(parse_zone("+05"), east(5 * 3600));
        assert_eq!(parse_zone("America/Sao_Paulo"), None);
        assert_eq!(parse_zone("+25:00"), None);
        assert_eq!(
            zone_of("America/Sao_Paulo"),
            Zone::Local,
            "unknown reads as local"
        );
    }

    #[test]
    fn a_trigger_without_next_run_is_initialised_not_fired() {
        let (t, c) = routine("*/1 * * * *", CatchUp::AllCapped, TriggerConcurrency::Queue);
        let plan = plan_tick(&t, &c, t0() + 10_000, 0, false);
        assert_eq!(plan.fire_now, 0);
        assert_eq!(plan.missed_slots, 0);
        assert_eq!(plan.next_run_ms, Some(t0() + MIN));
    }

    /// The acceptance test of 1.4 (80#P2 "Como medir" 1): the app closes
    /// before three `*/1` times and opens after them; the boot pass (grace 0)
    /// follows the policy.
    #[test]
    fn three_missed_minutes_follow_the_policy() {
        let now = t0() + 3 * MIN + 30_000; // 10:03:30; 10:01, 10:02, 10:03 passed
        let at = |catch_up, concurrency| {
            let (mut t, c) = routine("*/1 * * * *", catch_up, concurrency);
            t.next_run_ms = Some(t0() + MIN);
            plan_tick(&t, &c, now, 0, false)
        };
        let once = at(CatchUp::Once, TriggerConcurrency::SkipIfActive);
        assert_eq!(
            (
                once.fire_now,
                once.backlog,
                once.missed_slots,
                once.caught_up
            ),
            (1, 0, 3, 1)
        );
        assert_eq!(once.not_run, 2);
        let skip = at(CatchUp::Skip, TriggerConcurrency::SkipIfActive);
        assert_eq!(
            (
                skip.fire_now,
                skip.backlog,
                skip.missed_slots,
                skip.caught_up
            ),
            (0, 0, 3, 0)
        );
        assert_eq!(skip.not_run, 3, "logged as missed=3");
        // All three run: at once when the routine may overlap itself, one at
        // a time (1 now + 2 waiting) when it may not.
        let all_q = at(CatchUp::AllCapped, TriggerConcurrency::Queue);
        assert_eq!(
            (
                all_q.fire_now,
                all_q.backlog,
                all_q.caught_up,
                all_q.not_run
            ),
            (3, 0, 3, 0)
        );
        let all_s = at(CatchUp::AllCapped, TriggerConcurrency::SkipIfActive);
        assert_eq!(
            (
                all_s.fire_now,
                all_s.backlog,
                all_s.caught_up,
                all_s.not_run
            ),
            (1, 2, 3, 0)
        );
        for p in [once, skip, all_q, all_s] {
            assert_eq!(
                p.next_run_ms,
                Some(t0() + 4 * MIN),
                "the schedule moves past now"
            );
        }
    }

    #[test]
    fn a_timely_tick_fires_once_under_every_policy() {
        for catch_up in [CatchUp::Skip, CatchUp::Once, CatchUp::AllCapped] {
            for concurrency in [TriggerConcurrency::SkipIfActive, TriggerConcurrency::Queue] {
                let (mut t, c) = routine("*/5 * * * *", catch_up, concurrency);
                t.next_run_ms = Some(t0() + 5 * MIN);
                // Not due yet: nothing.
                assert_eq!(
                    plan_tick(&t, &c, t0() + 4 * MIN, CRON_GRACE_MS, false).fire_now,
                    0
                );
                // Seen 20 s late by the ticker: it runs, nothing is missed.
                let plan = plan_tick(&t, &c, t0() + 5 * MIN + 20_000, CRON_GRACE_MS, false);
                assert_eq!(
                    (plan.fire_now, plan.missed_slots, plan.not_run),
                    (1, 0, 0),
                    "{catch_up:?} {concurrency:?}"
                );
                assert_eq!(plan.next_run_ms, Some(t0() + 10 * MIN));
            }
        }
    }

    #[test]
    fn all_capped_stops_at_25() {
        let (mut t, c) = routine("*/1 * * * *", CatchUp::AllCapped, TriggerConcurrency::Queue);
        t.next_run_ms = Some(t0() + MIN);
        let plan = plan_tick(&t, &c, t0() + 100 * MIN + 30_000, 0, false);
        assert_eq!(plan.missed_slots, 100);
        assert_eq!(plan.fire_now, CATCH_UP_CAP);
        assert_eq!(plan.not_run, 100 - CATCH_UP_CAP);
        // The backlog of a one-at-a-time routine is capped the same way.
        t.concurrency = TriggerConcurrency::SkipIfActive;
        let plan = plan_tick(&t, &c, t0() + 100 * MIN + 30_000, 0, false);
        assert_eq!(plan.fire_now + plan.backlog, CATCH_UP_CAP);
    }

    #[test]
    fn a_busy_routine_skips_and_counts_it() {
        let (mut t, c) = routine(
            "*/1 * * * *",
            CatchUp::Once,
            TriggerConcurrency::SkipIfActive,
        );
        t.next_run_ms = Some(t0() + MIN);
        let plan = plan_tick(&t, &c, t0() + MIN + 5_000, CRON_GRACE_MS, true);
        assert_eq!(
            (plan.fire_now, plan.skipped_active, plan.not_run),
            (0, 1, 1)
        );
        t.concurrency = TriggerConcurrency::Queue;
        let plan = plan_tick(&t, &c, t0() + MIN + 5_000, CRON_GRACE_MS, true);
        assert_eq!(
            (plan.fire_now, plan.skipped_active),
            (1, 0),
            "queue starts it anyway"
        );
    }

    /// 80#P2 "Como medir" 2: never two jobs of one routine at once with
    /// `skip_if_active`, whatever the catch-up policy, including a boot
    /// with a backlog. A 30-minute day of the real ticker (30 s) on an
    /// injected clock, `*/1`, every run lasting 150 s.
    #[test]
    fn skip_if_active_never_runs_two_of_one_routine() {
        for catch_up in [CatchUp::Skip, CatchUp::Once, CatchUp::AllCapped] {
            let (mut t, c) = routine("*/1 * * * *", catch_up, TriggerConcurrency::SkipIfActive);
            t.next_run_ms = Some(t0() + MIN);
            let mut running: Vec<i64> = Vec::new(); // end times
            let (mut started, mut peak) = (0u32, 0usize);
            let mut now = t0() + 5 * MIN + 10_000; // boot after 5 missed minutes
            let mut grace = 0;
            while now < t0() + 35 * MIN {
                running.retain(|end| *end > now);
                let plan = plan_tick(&t, &c, now, grace, !running.is_empty());
                apply(&mut t, &plan);
                for _ in 0..plan.fire_now {
                    running.push(now + 150_000);
                    started += 1;
                }
                peak = peak.max(running.len());
                grace = CRON_GRACE_MS;
                now += CRON_TICK_S as i64 * 1000;
            }
            assert_eq!(peak, 1, "{catch_up:?}: two runs of one routine at once");
            assert!(started >= 10, "{catch_up:?}: it still runs ({started})");
            assert_eq!(t.backlog, 0, "{catch_up:?}: the backlog drains");
        }
        // With `queue`, the same boot with `all_capped` does overlap: the
        // simulation can see it.
        let (mut t, c) = routine("*/1 * * * *", CatchUp::AllCapped, TriggerConcurrency::Queue);
        t.next_run_ms = Some(t0() + MIN);
        let plan = plan_tick(&t, &c, t0() + 5 * MIN + 10_000, 0, false);
        assert_eq!(plan.fire_now, 5);
    }

    fn db_file() -> (std::path::PathBuf, Connection) {
        let path = std::env::temp_dir().join(format!("omniget-cron-{}.db", uuid::Uuid::new_v4()));
        let db = Connection::open(&path).unwrap();
        ensure_schema(&db, t0()).unwrap();
        (path, db)
    }

    /// A routine saved by 0.9.x: a body without any of the new fields and no
    /// schedule column.
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
        // Nothing before the migration counts as missed.
        assert!(claim_due(&db, "t1", t0() + 40_000, 0, false).is_none());
        // Running it again changes nothing.
        ensure_schema(&db, t0() + 50 * MIN).unwrap();
        assert_eq!(
            read_trigger(&db, "t1").unwrap().next_run_ms,
            Some(t0() + MIN)
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn only_one_claim_wins_a_slot() {
        let (path, db) = db_file();
        let (mut t, _) = routine("*/1 * * * *", CatchUp::Once, TriggerConcurrency::Queue);
        t.next_run_ms = Some(t0() + MIN);
        set_schedule(&db, &t).unwrap();
        let now = t0() + MIN + 10_000;
        let first =
            claim_due(&db, "t1", now, CRON_GRACE_MS, false).expect("the first pass takes it");
        assert_eq!(first.1.fire_now, 1);
        assert!(
            claim_due(&db, "t1", now, CRON_GRACE_MS, false).is_none(),
            "the same slot twice"
        );
        // Another process that read the old schedule loses the CAS.
        let other = Connection::open(&path).unwrap();
        let mut stale = t.clone();
        stale.next_run_ms = Some(t0() + 2 * MIN);
        assert!(!cas_schedule(&other, &stale, Some(t0() + MIN)));
        assert_eq!(
            read_trigger(&db, "t1").unwrap().next_run_ms,
            Some(t0() + 2 * MIN)
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_restart_does_not_fire_the_same_slot_again() {
        let (path, db) = db_file();
        let (mut t, _) = routine("*/1 * * * *", CatchUp::AllCapped, TriggerConcurrency::Queue);
        t.next_run_ms = Some(t0() + MIN);
        set_schedule(&db, &t).unwrap();
        let (_, plan) = claim_due(&db, "t1", t0() + MIN + 20_000, CRON_GRACE_MS, false).unwrap();
        assert_eq!(plan.fire_now, 1);
        drop(db);
        // The app restarts 5 s later: the boot pass (grace 0) sees the
        // stored schedule, already past this slot.
        let db = Connection::open(&path).unwrap();
        ensure_schema(&db, t0() + MIN + 25_000).unwrap();
        assert!(claim_due(&db, "t1", t0() + MIN + 25_000, 0, false).is_none());
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_body_write_never_moves_the_schedule() {
        let (path, db) = db_file();
        let (mut t, _) = routine("*/1 * * * *", CatchUp::Once, TriggerConcurrency::Queue);
        t.next_run_ms = Some(t0() + MIN);
        set_schedule(&db, &t).unwrap();
        // A webhook-style firing read the row, the ticker advanced it, the
        // firing writes its counters back with the old schedule in the body.
        let mut copy = read_trigger(&db, "t1").unwrap();
        assert!(claim_due(&db, "t1", t0() + MIN + 1_000, CRON_GRACE_MS, false).is_some());
        copy.fire_count += 1;
        write_trigger_body(&db, &copy).unwrap();
        let now = read_trigger(&db, "t1").unwrap();
        assert_eq!(now.next_run_ms, Some(t0() + 2 * MIN), "the column wins");
        assert_eq!(now.fire_count, 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_busy_routine_is_seen_through_its_jobs() {
        let (path, db) = db_file();
        let (mut t, _) = routine(
            "*/1 * * * *",
            CatchUp::Once,
            TriggerConcurrency::SkipIfActive,
        );
        t.next_run_ms = Some(t0() + MIN);
        set_schedule(&db, &t).unwrap();
        db.execute(
            "INSERT INTO jobs (id, kind, agent_id, conversation_id, prompt, state, trigger_id, created_ms) VALUES ('j1','trigger','a','c','p','running','t1',1)",
            [],
        )
        .unwrap();
        assert!(trigger_busy(&db, "t1"));
        let (after, plan) = claim_due(&db, "t1", t0() + MIN + 1_000, CRON_GRACE_MS, false).unwrap();
        assert_eq!((plan.fire_now, plan.skipped_active), (0, 1));
        assert_eq!(after.missed, 1);
        assert!(after.last_missed_ms.is_some());
        let _ = std::fs::remove_file(path);
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

#[cfg(test)]
mod worktree_tests {
    use super::*;

    fn job(kind: &str, isolation: Isolation) -> Job {
        Job {
            id: "j0123456789".into(),
            kind: kind.into(),
            agent_id: "a".into(),
            workspace: Some("/w/repo".into()),
            isolation,
            state: "queued".into(),
            ..Default::default()
        }
    }

    #[test]
    fn background_runs_default_to_auto_and_missions_stay_shared() {
        assert_eq!(Isolation::default_for("run", "claude"), Isolation::Auto);
        assert_eq!(Isolation::default_for("trigger", "claude"), Isolation::Auto);
        for kind in ["mission", "loop", "chat"] {
            assert_eq!(Isolation::default_for(kind, "claude"), Isolation::Shared);
        }
        assert_eq!(
            Isolation::default_for("run", "tool:codex"),
            Isolation::Shared
        );
        assert_eq!(Isolation::parse(" Worktree "), Ok(Isolation::Worktree));
        assert_eq!(Isolation::parse("auto"), Ok(Isolation::Auto));
        assert!(Isolation::parse("sharde").is_err(), "a typo is an error");
        assert_eq!(
            serde_json::to_value(Isolation::Auto).unwrap(),
            json!("auto")
        );
    }

    #[test]
    fn the_placement_follows_the_isolation_and_the_repository() {
        let repo = Some(std::path::PathBuf::from("/w/repo"));
        let p = placement(&job("run", Isolation::Auto), repo.clone()).unwrap();
        assert!(p.if_busy && !p.force, "auto asks the gate");
        let p = placement(&job("run", Isolation::Auto), None).unwrap();
        assert!(!p.if_busy && !p.force, "auto without git = shared");
        let p = placement(&job("run", Isolation::Shared), repo.clone()).unwrap();
        assert!(!p.if_busy && !p.force);
        let p = placement(&job("run", Isolation::Worktree), repo).unwrap();
        assert!(p.force);
        let e = placement(&job("run", Isolation::Worktree), None).unwrap_err();
        assert!(e.starts_with(ERR_WORKTREE), "{e}");
        // A job that already has one (resumed) goes back to it.
        let mut resumed = job("run", Isolation::Auto);
        resumed.worktree_path = Some("/wt".into());
        assert!(placement(&resumed, None).unwrap().force);
        assert_eq!(effective_workspace(&resumed).as_deref(), Some("/wt"));
        assert_eq!(
            effective_workspace(&job("run", Isolation::Auto)).as_deref(),
            Some("/w/repo")
        );
    }

    #[test]
    fn the_gc_removes_orphans_and_old_ones_never_active_ones() {
        let now = 10 * WORKTREE_TTL_MS;
        assert_eq!(worktree_gc_verdict(None, now), Some("orphan"));
        let mut j = job("run", Isolation::Auto);
        for st in ["queued", "waiting_workspace", "running", "waiting_approval"] {
            j.state = st.into();
            j.finished_ms = Some(0);
            assert_eq!(worktree_gc_verdict(Some(&j), now), None, "{st}");
        }
        j.state = "done".into();
        j.finished_ms = Some(now - WORKTREE_TTL_MS + 1);
        assert_eq!(worktree_gc_verdict(Some(&j), now), None, "still fresh");
        j.finished_ms = Some(now - WORKTREE_TTL_MS);
        assert_eq!(worktree_gc_verdict(Some(&j), now), Some("ttl"));
    }

    #[test]
    fn the_placement_columns_round_trip() {
        let db = Connection::open_in_memory().unwrap();
        ensure_schema(&db, 0).unwrap();
        db.execute(
            "INSERT INTO jobs (id, kind, agent_id, conversation_id, prompt, state, created_ms, isolation, base_ref, worktree_path, branch, base_oid)
             VALUES ('j0', 'run', 'a', 'c', 'p', 'done', 1, 'worktree', 'main', '/wt/x', 'omniget/job-j0', 'abc')",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO jobs (id, kind, agent_id, conversation_id, prompt, state, created_ms)
             VALUES ('j1', 'run', 'a', 'c', 'p', 'done', 1)",
            [],
        )
        .unwrap();
        let read = |id: &str| {
            db.query_row(
                &format!("SELECT {JOB_COLS} FROM jobs WHERE id=?1"),
                [id],
                row_to_job,
            )
            .unwrap()
        };
        let j = read("j0");
        assert_eq!(j.isolation, Isolation::Worktree);
        assert_eq!(j.base_ref.as_deref(), Some("main"));
        assert_eq!(j.worktree_path.as_deref(), Some("/wt/x"));
        assert_eq!(j.branch.as_deref(), Some("omniget/job-j0"));
        assert_eq!(j.base_oid.as_deref(), Some("abc"));
        assert_eq!(read("j1").isolation, Isolation::Shared, "older rows");
    }

    #[test]
    fn the_notes_say_what_came_along_and_what_was_kept() {
        use omniget_core::core::vcs::job_worktree::{JobWorktree, RemoveOutcome};
        let note = worktree_note(&JobWorktree {
            path: "/wt/r-job-j1".into(),
            branch: "omniget/job-j1".into(),
            base_oid: "0123456789abcdef".into(),
            carried: vec!["a.txt".into()],
            shared: vec!["node_modules".into()],
            took_ms: 42,
            ..Default::default()
        });
        assert!(
            note.contains("omniget/job-j1 from 0123456 in 42 ms"),
            "{note}"
        );
        assert!(note.contains("1 uncommitted file(s) of yours carried in"));
        assert!(note.contains("linked node_modules"));
        let kept = removal_note(&RemoveOutcome {
            dirty: vec!["a".into(), "b".into()],
            saved: Some("fedcba987".into()),
            preserved_branch: Some("omniget/job-j1".into()),
            ..Default::default()
        });
        assert!(
            kept.contains("2 uncommitted file(s) saved as fedcba9"),
            "{kept}"
        );
        assert!(kept.contains("branch omniget/job-j1 kept"));
    }
}

#[cfg(test)]
mod proof_tests {
    use super::*;
    use omniget_core::core::llm::snapshot::FileChange;

    fn change(path: &str) -> FileChange {
        FileChange {
            path: path.into(),
            status: "M".into(),
            additions: Some(2),
            deletions: Some(1),
            binary: false,
        }
    }

    fn ended(state: &str) -> Job {
        Job {
            id: "j0123456789".into(),
            kind: "run".into(),
            state: state.into(),
            workspace: Some("/w/repo".into()),
            started_ms: Some(1_000),
            finished_ms: Some(4_500),
            diff_files: Some(vec![change("a.txt")]),
            additions: Some(2),
            deletions: Some(1),
            usage: Some(
                json!({"model": "m", "input_tokens": 100, "output_tokens": 20, "cache_read_tokens": 1000, "cost_usd": 0.01, "calls": 1}),
            ),
            ..Default::default()
        }
    }

    #[test]
    fn a_job_that_wrote_ends_in_review_and_nothing_else_moves() {
        let files = [change("a.txt")];
        assert_eq!(settle_state("done", Some(&files)), "review");
        assert_eq!(settle_state("done", Some(&[])), "done", "read-only job");
        assert_eq!(settle_state("done", None), "done", "no diff known");
        assert_eq!(settle_state("failed", Some(&files)), "failed");
        assert_eq!(settle_state("cancelled", Some(&files)), "cancelled");
        assert!(is_success("review") && is_success("done") && !is_success("failed"));
    }

    #[test]
    fn the_worktree_proof_stamps_host_folder_base_and_head() {
        let mut job = ended("review");
        job.worktree_path = Some("/data/worktrees/repo-job-j0123456789".into());
        job.branch = Some("omniget/job-j0123456789".into());
        job.base_oid = Some("1111111aaaa".into());
        job.head_oid = Some("2222222bbbb".into());
        let ends = ("1111111aaaa".to_string(), "treeeeeee".to_string());
        let p = build_proof(&job, Some(&ends), "mac");
        assert_eq!(p["generated_by"], "host");
        assert_eq!(
            p["stamp"],
            "mac:/data/worktrees/repo-job-j0123456789@1111111→2222222"
        );
        assert_eq!(p["mode"], "worktree");
        assert_eq!(p["base"], "1111111aaaa");
        assert_eq!(p["head"], "2222222bbbb");
        assert_eq!(p["file_count"], 1);
        assert_eq!(p["files"][0]["path"], "a.txt");
        assert_eq!(p["tokens"]["input"], 100);
        assert_eq!(p["cost_usd"], 0.01);
        assert_eq!(p["duration_ms"], 3_500);
        assert_eq!(p["checks"], json!([]));
    }

    #[test]
    fn the_shared_proof_stamps_the_snapshot_trees() {
        let job = ended("review");
        let ends = ("abcdef0123".to_string(), "9876543210".to_string());
        let p = build_proof(&job, Some(&ends), "pc");
        assert_eq!(p["mode"], "shared");
        assert_eq!(p["stamp"], "pc:/w/repo@tree:abcdef0→tree:9876543");
        assert_eq!(p["base"], "tree:abcdef0123");
        // Unknown ends: said, not made up.
        let p = build_proof(&job, None, "pc");
        assert_eq!(p["stamp"], "pc:/w/repo@?→?");
    }

    #[test]
    fn a_check_lands_in_the_proof_with_its_exit_code() {
        let mut p = build_proof(&ended("review"), None, "h");
        push_check(&mut p, "npm test", false, "exit 1\nFAIL x");
        let mut empty = json!({});
        push_check(&mut empty, "true", true, "exit 0\n");
        assert_eq!(p["checks"][0]["command"], "npm test");
        assert_eq!(p["checks"][0]["exit_code"], 1);
        assert_eq!(p["checks"][0]["ok"], false);
        assert_eq!(empty["checks"][0]["exit_code"], 0);
        push_check(&mut empty, "x", false, "ERR_TOOL_DENIED");
        assert_eq!(empty["checks"][1]["exit_code"], Value::Null);
    }

    #[test]
    fn the_proof_route_answers_by_state() {
        let mut job = ended("running");
        assert_eq!(proof_view(&job).unwrap_err().0, 409);
        job.state = "done".into();
        assert_eq!(proof_view(&job).unwrap_err().0, 404, "ended before 2.4");
        job.proof = Some(json!({"stamp": "h:/w@a→b"}));
        let v = proof_view(&job).unwrap();
        assert_eq!(v["proof"]["stamp"], "h:/w@a→b");
    }

    #[test]
    fn head_and_proof_round_trip_and_leave_the_list() {
        let db = Connection::open_in_memory().unwrap();
        ensure_schema(&db, 0).unwrap();
        db.execute(
            "INSERT INTO jobs (id, kind, agent_id, conversation_id, prompt, state, created_ms, head_oid, proof)
             VALUES ('j0', 'run', 'a', 'c', 'p', 'review', 1, 'abc', '{\"stamp\":\"h:/w@a→b\"}')",
            [],
        )
        .unwrap();
        let j = db
            .query_row(
                &format!("SELECT {JOB_COLS} FROM jobs WHERE id='j0'"),
                [],
                row_to_job,
            )
            .unwrap();
        assert_eq!(j.head_oid.as_deref(), Some("abc"));
        assert_eq!(j.proof.unwrap()["stamp"], "h:/w@a→b");
        let mut with_head = ended("review");
        with_head.head_oid = Some("abc".into());
        let v = match diff_view(&with_head, None, None, "files", true).unwrap() {
            DiffBody::Json(v) => v,
            other => panic!("{other:?}"),
        };
        assert_eq!(v["head_oid"], "abc");
    }
}

#[cfg(test)]
mod auto_resume_tests {
    use super::*;
    use omniget_core::core::process::StaleProcess;

    fn job(kind: &str) -> Job {
        Job {
            id: "j1".into(),
            kind: kind.into(),
            agent_id: "coder".into(),
            conversation_id: "c".into(),
            prompt: "fix it".into(),
            state: "interrupted".into(),
            ..Default::default()
        }
    }

    #[test]
    fn only_the_hosts_failures_are_infrastructure() {
        for r in [
            FailureReason::AppRestart,
            FailureReason::Stalled,
            FailureReason::NoStart,
        ] {
            assert!(r.is_infrastructure(), "{r:?}");
            assert_eq!(FailureReason::parse(r.as_str()), Some(r));
        }
        for r in [
            FailureReason::Deadline,
            FailureReason::Limit,
            FailureReason::AskExpired,
        ] {
            assert!(!r.is_infrastructure(), "{r:?}");
            assert_eq!(FailureReason::parse(r.as_str()), Some(r));
        }
        assert_eq!(FailureReason::parse("nope"), None);
    }

    #[test]
    fn reading_and_searching_are_the_only_harmless_tools() {
        for t in [
            "Read",
            "Glob",
            "Grep",
            "WebSearch",
            "fs_read",
            "fs_grep",
            "TodoWrite",
        ] {
            assert!(!tool_may_write(t), "{t}");
        }
        for t in [
            "Edit",
            "Write",
            "Bash",
            "fs_edit",
            "shell_exec",
            "mcp__x__y",
            "Task",
        ] {
            assert!(tool_may_write(t), "{t}");
        }
    }

    /// A job a budget stopped (3.3) never comes back by itself (3.1), even
    /// when the watchdog or the boot put a host reason on it afterwards; a
    /// cancelled one neither.
    #[test]
    fn a_job_stopped_by_a_budget_is_not_resumed_by_itself() {
        let mut cut = job("run");
        cut.failure_reason = Some("budget".into());
        for r in [FailureReason::AppRestart, FailureReason::Stalled] {
            assert_eq!(
                auto_resume_verdict(&cut, true, r, None, 0),
                ResumeVerdict::BudgetPaused
            );
        }
        assert_eq!(ResumeVerdict::BudgetPaused.as_str(), "budget_paused");
        let mut cancelled = job("run");
        cancelled.state = "cancelled".into();
        assert_eq!(
            auto_resume_verdict(&cancelled, true, FailureReason::Stalled, None, 0),
            ResumeVerdict::NotInfrastructure
        );
    }

    /// The rule of 3.1, one condition at a time.
    #[test]
    fn a_lost_job_comes_back_only_when_asked_safe_and_not_too_often() {
        let r = FailureReason::AppRestart;
        let clean = job("run");
        assert_eq!(
            auto_resume_verdict(&clean, false, r, None, 0),
            ResumeVerdict::Off
        );
        assert_eq!(
            auto_resume_verdict(&clean, true, r, None, 0),
            ResumeVerdict::Resume
        );
        assert_eq!(
            auto_resume_verdict(&clean, true, FailureReason::Deadline, None, 0),
            ResumeVerdict::NotInfrastructure
        );
        assert_eq!(
            auto_resume_verdict(&clean, true, FailureReason::Stalled, None, 0),
            ResumeVerdict::Resume
        );
        // Wrote in the shared folder: never again by itself…
        let mut wrote = job("run");
        wrote.side_effects = true;
        assert_eq!(
            auto_resume_verdict(&wrote, true, r, None, 0),
            ResumeVerdict::WroteInSharedFolder
        );
        // …but in its own worktree it may.
        wrote.worktree_path = Some("/wt/j1".into());
        assert_eq!(
            auto_resume_verdict(&wrote, true, r, None, 0),
            ResumeVerdict::Resume
        );
        // Never beside its old process.
        assert_eq!(
            auto_resume_verdict(&clean, true, r, Some(StaleProcess::Survived), 0),
            ResumeVerdict::ProcessAlive
        );
        assert_eq!(
            auto_resume_verdict(&clean, true, r, Some(StaleProcess::Killed), 0),
            ResumeVerdict::Resume
        );
        assert_eq!(
            auto_resume_verdict(&clean, true, r, None, AUTO_RESUME_MAX),
            ResumeVerdict::Attempts
        );
        for kind in ["chat", "mission", "playbook"] {
            assert_eq!(
                auto_resume_verdict(&job(kind), true, r, None, 0),
                ResumeVerdict::NotThisKind,
                "{kind}"
            );
        }
        let mut tool = job("run");
        tool.agent_id = "tool:codex".into();
        assert_eq!(
            auto_resume_verdict(&tool, true, r, None, 0),
            ResumeVerdict::NotThisKind
        );
        let mut round = job("loop");
        round.loop_id = Some("l1".into());
        assert_eq!(
            auto_resume_verdict(&round, true, r, None, 0),
            ResumeVerdict::Resume
        );
    }

    #[test]
    fn the_note_says_to_check_before_repeating() {
        let n = resume_note(FailureReason::AppRestart, "fix it");
        assert!(n.starts_with("The app closed in the middle of this task"));
        assert!(n.contains("do not repeat an action that already happened"));
        assert!(n.ends_with("Task: fix it"));
        assert!(resume_note(FailureReason::Stalled, "x").contains("went silent"));
    }

    #[test]
    fn the_resume_columns_round_trip_and_older_rows_read_as_before() {
        let db = Connection::open_in_memory().unwrap();
        ensure_schema(&db, 0).unwrap();
        db.execute(
            "INSERT INTO jobs (id, kind, agent_id, conversation_id, prompt, state, created_ms)
             VALUES ('old', 'run', 'a', 'c', 'p', 'done', 1)",
            [],
        )
        .unwrap();
        db.execute(
            "INSERT INTO jobs (id, kind, agent_id, conversation_id, prompt, state, created_ms, session_id, pid, pid_start, attempt, auto_resume, side_effects)
             VALUES ('new', 'run', 'a', 'c', 'p', 'running', 1, 'sess-9', 4242, 1700000000123456, 1, 1, 1)",
            [],
        )
        .unwrap();
        let read = |id: &str| {
            db.query_row(
                &format!("SELECT {JOB_COLS} FROM jobs WHERE id=?1"),
                [id],
                row_to_job,
            )
            .unwrap()
        };
        let old = read("old");
        assert_eq!(
            (
                old.session_id,
                old.pid,
                old.pid_start,
                old.attempt,
                old.auto_resume,
                old.side_effects
            ),
            (None, None, None, 0, None, false)
        );
        let new = read("new");
        assert_eq!(new.session_id.as_deref(), Some("sess-9"));
        assert_eq!(new.pid, Some(4242));
        assert_eq!(new.pid_start, Some(1_700_000_000_123_456));
        assert_eq!(new.attempt, 1);
        assert_eq!(new.auto_resume, Some(true));
        assert!(new.side_effects);
    }

    #[test]
    fn the_setting_is_off_by_default_and_a_loop_keeps_its_own_switch() {
        assert!(!omniget_core::models::settings::LlmSettings::default().auto_resume);
        let l: LoopDef =
            serde_json::from_value(json!({"agent_id": "a", "prompt": "p", "auto_resume": true}))
                .unwrap();
        assert_eq!(l.auto_resume, Some(true));
        assert!(!l.restarted);
        let old: LoopDef = serde_json::from_value(json!({"agent_id": "a", "prompt": "p"})).unwrap();
        assert_eq!(old.auto_resume, None);
    }
}
