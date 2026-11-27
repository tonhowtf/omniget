use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use futures::future::BoxFuture;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::approvals::Approvals;

pub type DiffAnswer = Result<(Job, Option<String>, bool), String>;
use crate::jobs::{self, Job};

pub const ERR_COMMAND_CONFLICT: &str = "ERR_COMMAND_CONFLICT";
pub const ERR_COMMAND_INVALID: &str = "ERR_COMMAND_INVALID";
pub const ERR_COMMAND_FORBIDDEN: &str = "ERR_COMMAND_FORBIDDEN";
pub const ERR_DELEGATION_DEPTH: &str = "ERR_DELEGATION_DEPTH";
pub const ERR_WAIT_TIMEOUT: &str = "ERR_WAIT_TIMEOUT";
pub const ERR_WAIT_UNREACHABLE: &str = "ERR_WAIT_UNREACHABLE";
pub const ERR_NO_JOB: &str = "ERR_NO_JOB";

pub const WAIT_MAX_MS: u64 = 120_000;
pub const WAIT_DEFAULT_MS: u64 = 30_000;
pub const MCP_WAIT_MAX_MS: u64 = 25_000;

pub const TERMINAL: &[&str] = &[
    "done",
    "review",
    "done_unreported",
    "failed",
    "cancelled",
    "interrupted",
];

const STATES: &[&str] = &[
    "queued",
    "waiting_workspace",
    "running",
    "waiting_approval",
    "done",
    "review",
    "done_unreported",
    "failed",
    "cancelled",
    "interrupted",
];

pub fn ensure_schema(db: &Connection) -> Result<(), String> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS command_receipts (
            principal TEXT NOT NULL,
            command_id TEXT NOT NULL,
            type TEXT NOT NULL,
            fingerprint TEXT NOT NULL,
            job_id TEXT,
            result TEXT NOT NULL,
            created_ms INTEGER NOT NULL,
            PRIMARY KEY (principal, command_id));
         CREATE INDEX IF NOT EXISTS command_receipts_job ON command_receipts(job_id);",
    )
    .map_err(|e| e.to_string())
}

pub trait Engine: Send + Sync {
    fn start(&self, spec: &StartSpec, owner: &str) -> Result<Job, String>;
    fn job(&self, id: &str) -> Option<Job>;
    fn cancel(&self, id: &str, actor: omniget_core::core::llm::audit::Actor)
        -> Result<Job, String>;
    fn subscribe(&self) -> tokio::sync::watch::Receiver<u64>;
    fn recent(&self, limit: u32) -> Vec<Job>;
    fn receipts(&self) -> MutexGuard<'_, Connection>;
    fn diff<'a>(&'a self, id: &'a str) -> BoxFuture<'a, DiffAnswer>;
}

pub struct AppEngine(pub Arc<jobs::Jobs>);

impl Engine for AppEngine {
    fn start(&self, spec: &StartSpec, owner: &str) -> Result<Job, String> {
        self.0.submit_command(spec, owner)
    }
    fn job(&self, id: &str) -> Option<Job> {
        self.0.job(id)
    }
    fn cancel(
        &self,
        id: &str,
        actor: omniget_core::core::llm::audit::Actor,
    ) -> Result<Job, String> {
        self.0.cancel_by(id, actor)
    }
    fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.0.subscribe()
    }
    fn recent(&self, limit: u32) -> Vec<Job> {
        self.0.list(limit)
    }
    fn receipts(&self) -> MutexGuard<'_, Connection> {
        self.0.db()
    }
    fn diff<'a>(&'a self, id: &'a str) -> BoxFuture<'a, DiffAnswer> {
        Box::pin(self.0.diff(id))
    }
}

#[derive(Clone)]
pub struct Commands {
    pub engine: Arc<dyn Engine>,
    pub approvals: Option<Approvals>,
    pub workflows: Option<Arc<omniget_core::core::llm::workflow::Store>>,
}

pub fn from_app(app: &tauri::AppHandle) -> Result<Commands, String> {
    Ok(Commands {
        engine: Arc::new(AppEngine(jobs::get(app)?)),
        approvals: crate::approvals::from_app(app).ok(),
        workflows: crate::job_workflow::store(),
    })
}

#[derive(Debug, Clone)]
pub struct Caller {
    pub owner: String,
    pub channel: &'static str,
    pub can_operate: bool,
    pub can_respond: bool,
    pub owned_only: bool,
    pub from_job: Option<String>,
    pub may_accept_always: bool,
}

impl Caller {
    pub fn actor(&self) -> omniget_core::core::llm::audit::Actor {
        omniget_core::core::llm::audit::Actor::via(self.channel, &self.owner)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CmdError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
    pub job: Option<Value>,
}

impl CmdError {
    pub fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            job: None,
        }
    }
    pub fn text(&self) -> String {
        format!("{}: {}", self.code, self.message)
    }
    pub fn body(&self) -> Value {
        let mut v = json!({ "error": self.text(), "code": self.code });
        if let Some(j) = &self.job {
            v["job"] = j.clone();
        }
        v
    }
}

fn invalid(m: impl Into<String>) -> CmdError {
    CmdError::new(400, ERR_COMMAND_INVALID, m)
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartSpec {
    #[serde(default)]
    pub command_id: String,
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub prompt: String,
    #[serde(default)]
    pub task: Option<String>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub purpose: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub run_mode: Option<String>,
    #[serde(default)]
    pub isolation: Option<String>,
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub budget_usd: Option<f64>,
    #[serde(default)]
    pub deadline_min: Option<u32>,
    #[serde(default)]
    pub resume_of: Option<String>,
    #[serde(default)]
    pub auto_resume: Option<bool>,
    #[serde(default)]
    pub workflow: Option<bool>,
    #[serde(skip)]
    pub applied_workflow: Option<Value>,
}

fn short(v: &Option<String>, max: usize, what: &str) -> Result<(), CmdError> {
    match v {
        Some(s) if s.trim().is_empty() || s.chars().count() > max => {
            Err(invalid(format!("`{what}` must be 1–{max} characters")))
        }
        _ => Ok(()),
    }
}

fn is_id(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

pub fn parse_start(mut body: Value) -> Result<StartSpec, CmdError> {
    if let Some(map) = body.as_object_mut() {
        map.remove("type");
        if let Some(b) = map.remove("budget") {
            let usd = b.as_f64().or_else(|| b["usd"].as_f64());
            match usd {
                Some(u) => {
                    map.insert("budget_usd".into(), json!(u));
                }
                None if b.is_null() => {}
                None => return Err(invalid("`budget` is a number of US$ or {usd}")),
            }
        }
    }
    let mut s: StartSpec = serde_json::from_value(body).map_err(|e| invalid(e.to_string()))?;
    match (s.prompt.trim().is_empty(), s.task.take()) {
        (true, Some(t)) if !t.trim().is_empty() => s.prompt = t,
        (false, None) => {}
        (false, Some(_)) => return Err(invalid("give `prompt` or `task`, not both")),
        _ => return Err(invalid("`prompt` (or `task`) is required")),
    }
    if s.prompt.len() > 64 * 1024 {
        return Err(invalid("`prompt` is over 64 KB"));
    }
    if !s.agent_id.is_empty() && !is_id(&s.agent_id, 128) {
        return Err(invalid("`agent_id` is a roster id"));
    }
    short(&s.title, 200, "title")?;
    short(&s.purpose, 1000, "purpose")?;
    short(&s.model, 100, "model")?;
    if let Some(e) = &s.effort {
        if !matches!(e.as_str(), "low" | "medium" | "high" | "xhigh" | "max") {
            return Err(invalid("`effort` is low|medium|high|xhigh|max"));
        }
    }
    if let Some(m) = &s.run_mode {
        jobs::RunMode::parse(m).map_err(invalid)?;
    }
    if let Some(i) = &s.isolation {
        jobs::Isolation::parse(i).map_err(invalid)?;
    }
    if let Some(b) = s.budget_usd {
        if !(b > 0.0 && b <= 1000.0) {
            return Err(invalid("`budget` is between US$ 0 and 1000"));
        }
    }
    if let Some(d) = s.deadline_min {
        if !(1..=1440).contains(&d) {
            return Err(invalid("`deadline_min` is 1–1440"));
        }
    }
    if let Some(r) = &s.resume_of {
        if !is_id(r, 64) {
            return Err(invalid("`resume_of` is a job id"));
        }
    }
    if let Some(w) = &s.workspace {
        let p = std::path::Path::new(w);
        if !p.is_absolute() || !p.is_dir() {
            return Err(invalid("`workspace` is an absolute path to a folder"));
        }
    }
    Ok(s)
}

fn canonical(v: &Value) -> String {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .into_iter()
                .map(|k| format!("{}:{}", json!(k), canonical(&m[k])))
                .collect();
            format!("{{{}}}", inner.join(","))
        }
        Value::Array(a) => format!(
            "[{}]",
            a.iter().map(canonical).collect::<Vec<_>>().join(",")
        ),
        other => other.to_string(),
    }
}

pub fn fingerprint(body: &Value) -> String {
    let mut b = body.clone();
    if let Some(m) = b.as_object_mut() {
        m.remove("command_id");
    }
    format!("{:x}", Sha256::digest(canonical(&b).as_bytes()))
}

fn read_receipt(
    db: &Connection,
    owner: &str,
    id: &str,
) -> Result<Option<(String, Value)>, CmdError> {
    db.query_row(
        "SELECT fingerprint, result FROM command_receipts WHERE principal=?1 AND command_id=?2",
        params![owner, id],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )
    .optional()
    .map_err(|e| CmdError::new(500, "ERR_RECEIPTS", e.to_string()))
    .map(|row| row.map(|(f, r)| (f, serde_json::from_str(&r).unwrap_or(Value::Null))))
}

fn serial() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

pub fn job_view(job: &Job) -> Value {
    json!({
        "id": job.id,
        "state": job.state,
        "state_seq": job.state_seq,
        "kind": job.kind,
        "agent_id": job.agent_id,
        "title": job.title,
        "run_mode": job.run_mode,
        "created_ms": job.created_ms,
        "started_ms": job.started_ms,
        "finished_ms": job.finished_ms,
        "wait_reason": job.wait_reason,
        "failure_reason": job.failure_reason,
        "error": job.error,
        "outcome": job.outcome,
        "branch": job.branch,
        "head_oid": job.head_oid,
        "files_changed": job.diff_files.as_ref().map(|f| f.len()),
        "additions": job.additions,
        "deletions": job.deletions,
    })
}

impl Commands {
    pub fn execute(&self, c: &Caller, body: &Value) -> Result<(Value, bool), CmdError> {
        let command_id = body["command_id"].as_str().unwrap_or("").to_string();
        if !is_id(&command_id, 128) {
            return Err(invalid(
                "`command_id` is required: 1–128 of A-Z a-z 0-9 - _ . :",
            ));
        }
        let kind = body["type"].as_str().unwrap_or("").to_string();
        let print = fingerprint(body);
        let _one = serial();
        if let Some((f, result)) = read_receipt(&self.engine.receipts(), &c.owner, &command_id)? {
            if f != print {
                return Err(CmdError::new(
                    409,
                    ERR_COMMAND_CONFLICT,
                    format!("command {command_id} was already used for another command"),
                ));
            }
            return Ok((result, true));
        }
        let mut rest = body.clone();
        if let Some(m) = rest.as_object_mut() {
            m.remove("command_id");
        }
        let (job_id, outcome) = match kind.as_str() {
            "job.start" => {
                if !c.can_operate {
                    return Err(CmdError::new(
                        403,
                        ERR_COMMAND_FORBIDDEN,
                        "job.start needs jobs:operate",
                    ));
                }
                if let Some(from) = &c.from_job {
                    return Err(CmdError::new(
                        403,
                        ERR_DELEGATION_DEPTH,
                        format!("{from} is itself a job: a job cannot start another job (delegation depth is 1); do the work yourself"),
                    ));
                }
                let mut spec = parse_start(rest)?;
                spec.command_id = command_id.clone();
                if spec.agent_id.is_empty() {
                    if let Some(earlier) =
                        spec.resume_of.as_deref().and_then(|r| self.engine.job(r))
                    {
                        spec.agent_id = earlier.agent_id;
                    }
                }
                crate::job_workflow::apply(self.workflows.as_deref(), &mut spec)?;
                crate::job_workflow::default_agent(&mut spec);
                let job = self
                    .engine
                    .start(&spec, &c.owner)
                    .map_err(|e| CmdError::new(400, ERR_COMMAND_INVALID, e))?;
                crate::llm_audit::job_created(c.actor(), &job);
                (Some(job.id.clone()), json!({ "job": job_view(&job) }))
            }
            "job.interrupt" => {
                if !c.can_operate {
                    return Err(CmdError::new(
                        403,
                        ERR_COMMAND_FORBIDDEN,
                        "job.interrupt needs jobs:operate",
                    ));
                }
                let id = rest["job_id"].as_str().unwrap_or("").to_string();
                self.readable(c, &id)?;
                let job = self
                    .engine
                    .cancel(&id, c.actor())
                    .map_err(|e| CmdError::new(404, ERR_NO_JOB, e))?;
                (Some(job.id.clone()), json!({ "job": job_view(&job) }))
            }
            "approval.respond" => {
                if !c.can_respond {
                    return Err(CmdError::new(
                        403,
                        ERR_COMMAND_FORBIDDEN,
                        "approval.respond needs approvals:respond",
                    ));
                }
                let door = self.approvals.as_ref().ok_or_else(|| {
                    CmdError::new(503, "ERR_NO_ASK", "the run record is not available")
                })?;
                let id = rest["approval_id"].as_str().unwrap_or("");
                let decision = rest["decision"].as_str().unwrap_or("");
                if decision == "accept_always" && !c.may_accept_always {
                    return Err(invalid("accept_always stays a desktop decision"));
                }
                let job = door
                    .registry
                    .permission(id)
                    .and_then(|q| (door.job_of)(&q.run_id));
                if c.owned_only {
                    let mine = job
                        .as_deref()
                        .and_then(|j| self.engine.job(j))
                        .is_some_and(|j| j.owner.as_deref() == Some(c.owner.as_str()));
                    if !mine {
                        return Err(CmdError::new(
                            403,
                            "APPROVAL_NOT_AUTHORIZED",
                            "not an ask of your jobs",
                        ));
                    }
                }
                let actor = omniget_core::core::assist::runs::Actor::new(c.channel, &c.owner);
                let v = crate::approvals::respond(door, id, decision, &actor)
                    .map_err(|r| CmdError::new(r.status.as_u16(), r.code, r.message))?;
                (job, json!({ "approval": v }))
            }
            other => {
                return Err(invalid(format!(
                    "unknown command type `{other}` (job.start|job.interrupt|approval.respond)"
                )))
            }
        };
        let mut receipt = json!({
            "command_id": command_id,
            "type": kind,
            "status": "accepted",
            "job_id": job_id,
        });
        if let (Some(r), Some(o)) = (receipt.as_object_mut(), outcome.as_object()) {
            r.extend(o.clone());
        }
        let stored = self.engine.receipts().execute(
            "INSERT INTO command_receipts (principal, command_id, type, fingerprint, job_id, result, created_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![c.owner, command_id, kind, print, job_id, receipt.to_string(), crate::llm_manager::now_ms() as i64],
        );
        if let Err(e) = stored {
            tracing::warn!("[commands] receipt of {command_id}: {e}");
        }
        Ok((receipt, false))
    }

    pub fn readable(&self, c: &Caller, id: &str) -> Result<Job, CmdError> {
        let not_found = || CmdError::new(404, ERR_NO_JOB, format!("no job {id}"));
        let job = self.engine.job(id).ok_or_else(not_found)?;
        if c.owned_only && job.owner.as_deref() != Some(c.owner.as_str()) {
            return Err(not_found());
        }
        Ok(job)
    }

    pub async fn wait(
        &self,
        c: &Caller,
        id: &str,
        until: &[String],
        after_seq: i64,
        timeout: Duration,
    ) -> Result<Job, CmdError> {
        for s in until {
            if !STATES.contains(&s.as_str()) {
                return Err(invalid(format!("unknown state `{s}` in until")));
            }
        }
        let mut changes = self.engine.subscribe();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let job = self.readable(c, id)?;
            match wait_verdict(&job, until, after_seq) {
                Verdict::Match => return Ok(job),
                Verdict::Never => {
                    let mut e = CmdError::new(
                        409,
                        ERR_WAIT_UNREACHABLE,
                        format!(
                            "job {id} ended {} and will not reach {}",
                            job.state,
                            until.join("|")
                        ),
                    );
                    e.job = Some(job_view(&job));
                    return Err(e);
                }
                Verdict::Wait => {}
            }
            let woke = tokio::time::timeout_at(deadline, changes.changed()).await;
            if !matches!(woke, Ok(Ok(()))) {
                let job = self.readable(c, id)?;
                if wait_verdict(&job, until, after_seq) == Verdict::Match {
                    return Ok(job);
                }
                let mut e = CmdError::new(
                    408,
                    ERR_WAIT_TIMEOUT,
                    format!(
                        "job {id} is still {} after {} ms",
                        job.state,
                        timeout.as_millis()
                    ),
                );
                e.job = Some(job_view(&job));
                return Err(e);
            }
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Verdict {
    Match,
    Wait,
    Never,
}

pub fn wait_verdict(job: &Job, until: &[String], after_seq: i64) -> Verdict {
    let fresh = job.state_seq > after_seq;
    let wanted = until.is_empty() || until.iter().any(|s| s == &job.state);
    if fresh && wanted {
        Verdict::Match
    } else if jobs::is_terminal(&job.state) && !until.is_empty() && !wanted {
        Verdict::Never
    } else {
        Verdict::Wait
    }
}

pub fn parse_until(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for s in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if s == "terminal" {
            out.extend(TERMINAL.iter().map(|t| t.to_string()));
        } else {
            out.push(s.to_string());
        }
    }
    out
}

pub fn query_param<'a>(query: Option<&'a str>, key: &str) -> Option<&'a str> {
    query
        .unwrap_or("")
        .split('&')
        .find_map(|pair| pair.strip_prefix(key).and_then(|r| r.strip_prefix('=')))
}

use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

pub const JOB_HEADER: &str = "x-omniget-job";

pub fn http_caller(p: &crate::mcp::policy::Principal, headers: &HeaderMap, cli: bool) -> Caller {
    let has = |s: &str| p.scopes.iter().any(|v| v == s);
    let header_job = headers
        .get(JOB_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(|v| format!("job {v}"));
    Caller {
        owner: p.id.clone(),
        channel: if cli { "cli" } else { "http" },
        can_operate: has(crate::bridge_tokens::SCOPE_JOBS_OPERATE),
        can_respond: has(crate::bridge_tokens::SCOPE_APPROVALS_RESPOND),
        owned_only: false,
        from_job: p
            .id
            .strip_prefix("job:")
            .map(|j| format!("job {j}"))
            .or(header_job),
        may_accept_always: true,
    }
}

fn error_response(e: CmdError) -> Response {
    (
        StatusCode::from_u16(e.status).unwrap_or(StatusCode::BAD_REQUEST),
        axum::Json(e.body()),
    )
        .into_response()
}

fn no_principal() -> Response {
    error_response(CmdError::new(401, ERR_COMMAND_FORBIDDEN, "no principal"))
}

pub fn http_command(
    cmds: &Commands,
    p: Option<&crate::mcp::policy::Principal>,
    headers: &HeaderMap,
    body: &Value,
) -> Response {
    let Some(p) = p else { return no_principal() };
    let c = http_caller(p, headers, body["channel"].as_str() == Some("cli"));
    let mut body = body.clone();
    if let Some(m) = body.as_object_mut() {
        m.remove("channel");
    }
    match cmds.execute(&c, &body) {
        Ok((receipt, replayed)) => {
            let mut res = axum::Json(receipt).into_response();
            if replayed {
                res.headers_mut().insert(
                    "idempotent-replayed",
                    axum::http::HeaderValue::from_static("true"),
                );
            }
            res
        }
        Err(e) => error_response(e),
    }
}

pub fn http_job(cmds: &Commands, id: &str, query: Option<&str>) -> Response {
    match cmds.engine.job(id) {
        None => error_response(CmdError::new(404, ERR_NO_JOB, format!("no job {id}"))),
        Some(job) => {
            let mut v = json!(job);
            if let Some(after) = query_param(query, "after_seq").and_then(|s| s.parse::<i64>().ok())
            {
                v["changed"] = json!(job.state_seq > after);
            }
            axum::Json(v).into_response()
        }
    }
}

pub async fn http_wait(
    cmds: &Commands,
    p: Option<&crate::mcp::policy::Principal>,
    headers: &HeaderMap,
    id: &str,
    query: Option<&str>,
) -> Response {
    let Some(p) = p else { return no_principal() };
    let c = http_caller(p, headers, false);
    let until = parse_until(query_param(query, "until").unwrap_or(""));
    let after = query_param(query, "after_seq")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let timeout = query_param(query, "timeout_ms")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(WAIT_DEFAULT_MS)
        .clamp(1, WAIT_MAX_MS);
    match cmds
        .wait(&c, id, &until, after, Duration::from_millis(timeout))
        .await
    {
        Ok(job) => axum::Json(json!(job)).into_response(),
        Err(e) => error_response(e),
    }
}

pub fn mcp_caller(p: &crate::mcp::policy::Principal) -> Caller {
    let session = p.id.starts_with(crate::mcp::SESSION_PREFIX);
    Caller {
        owner: format!("mcp:{}", p.id),
        channel: "mcp",
        can_operate: crate::mcp::policy::allowed(p, "agent_run"),
        can_respond: crate::mcp::policy::allowed(p, "approval_respond") && !session,
        owned_only: true,
        from_job: session.then(|| "an agent session".to_string()),
        may_accept_always: false,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct AgentRunArgs {
    command_id: String,
    #[serde(default)]
    agent_id: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    task: Option<String>,
    #[serde(default)]
    workspace: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    purpose: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    effort: Option<String>,
    #[serde(default)]
    run_mode: Option<String>,
    #[serde(default)]
    isolation: Option<String>,
    #[serde(default)]
    base: Option<String>,
    #[serde(default)]
    budget_usd: Option<f64>,
    #[serde(default)]
    deadline_min: Option<u32>,
    #[serde(default)]
    resume_of: Option<String>,
    #[serde(default)]
    auto_resume: Option<bool>,
    workflow: Option<bool>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct JobArgs {
    job_id: String,
    #[serde(default)]
    after_seq: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct WaitArgs {
    job_id: String,
    #[serde(default)]
    until: Vec<String>,
    #[serde(default)]
    after_seq: Option<i64>,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CancelArgs {
    job_id: String,
    command_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

fn mcp_args<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|_| "INVALID_ARGUMENTS".to_string())
}

fn mcp_err(e: CmdError) -> String {
    match e.code {
        ERR_NO_JOB => "JOB_NOT_AUTHORIZED".into(),
        _ => e.text(),
    }
}

pub fn mcp_job(cmds: &Commands, job: &Job, after_seq: Option<i64>) -> Value {
    let mut v = job_view(job);
    let clip = |t: &Option<String>, n: usize| {
        t.as_deref().map(|s| {
            crate::core::flight_recorder::redact(s)
                .chars()
                .take(n)
                .collect::<String>()
        })
    };
    v["result"] = json!(clip(&job.result, 4_000));
    if let Some(m) = v.as_object_mut() {
        m.remove("error");
    }
    v["jobError"] = json!(clip(&job.error, 1_000));
    if let Some(after) = after_seq {
        v["changed"] = json!(job.state_seq > after);
    }
    let pending: Vec<Value> = cmds
        .approvals
        .as_ref()
        .filter(|_| job.state == "waiting_approval")
        .and_then(|a| {
            crate::approvals::list(
                a,
                &crate::approvals::Filter {
                    state: None,
                    job_id: Some(job.id.clone()),
                },
            )
            .ok()
        })
        .unwrap_or_default()
        .into_iter()
        .take(10)
        .map(|q| {
            json!({
                "approvalId": q["id"], "tool": q["tool"],
                "preview": q["preview"].as_str().map(|p| crate::core::flight_recorder::redact(p).chars().take(400).collect::<String>()),
            })
        })
        .collect();
    v["pendingApprovals"] = json!(pending);
    v
}

pub fn handles(name: &str) -> bool {
    matches!(
        name,
        "agent_run" | "job_get" | "job_wait" | "job_cancel" | "jobs_state" | "job_report"
    )
}

pub async fn mcp_call(
    cmds: &Commands,
    p: &crate::mcp::policy::Principal,
    name: &str,
    args: Value,
) -> Result<Value, String> {
    let c = mcp_caller(p);
    match name {
        "agent_run" => {
            let a: AgentRunArgs = mcp_args(args)?;
            let body = json!({
                "command_id": a.command_id, "type": "job.start", "agent_id": a.agent_id.unwrap_or_default(),
                "prompt": a.prompt.unwrap_or_default(), "task": a.task, "workspace": a.workspace,
                "title": a.title, "purpose": a.purpose, "model": a.model, "effort": a.effort,
                "run_mode": a.run_mode, "isolation": a.isolation, "base": a.base,
                "budget_usd": a.budget_usd, "deadline_min": a.deadline_min, "resume_of": a.resume_of,
                "auto_resume": a.auto_resume,
                "workflow": a.workflow,
            });
            let (r, replayed) = cmds.execute(&c, &body).map_err(mcp_err)?;
            Ok(json!({
                "commandId": r["command_id"], "jobId": r["job_id"], "state": r["job"]["state"],
                "stateSeq": r["job"]["state_seq"], "replayed": replayed,
                "next": "job_wait with afterSeq=stateSeq and until=[\"waiting_approval\",\"done\",\"review\",\"done_unreported\",\"failed\",\"cancelled\"]",
            }))
        }
        "job_get" => {
            let a: JobArgs = mcp_args(args)?;
            let job = cmds.readable(&c, &a.job_id).map_err(mcp_err)?;
            Ok(mcp_job(cmds, &job, a.after_seq))
        }
        "job_wait" => {
            let a: WaitArgs = mcp_args(args)?;
            let timeout = a.timeout_ms.unwrap_or(20_000).clamp(1, MCP_WAIT_MAX_MS);
            let until: Vec<String> = a.until.iter().flat_map(|u| parse_until(u)).collect();
            let job = cmds
                .wait(
                    &c,
                    &a.job_id,
                    &until,
                    a.after_seq.unwrap_or(0),
                    Duration::from_millis(timeout),
                )
                .await
                .map_err(mcp_err)?;
            Ok(mcp_job(cmds, &job, a.after_seq))
        }
        "job_report" => {
            let a: JobArgs = mcp_args(args)?;
            let job = cmds.readable(&c, &a.job_id).map_err(mcp_err)?;
            let mut v = crate::job_workflow::report(&job, false);
            if let Some(after) = a.after_seq {
                v["changed"] = json!(job.state_seq > after);
            }
            Ok(v)
        }
        "job_cancel" => {
            let a: CancelArgs = mcp_args(args)?;
            let body =
                json!({"command_id": a.command_id, "type": "job.interrupt", "job_id": a.job_id});
            let (r, replayed) = cmds.execute(&c, &body).map_err(mcp_err)?;
            Ok(json!({"jobId": r["job_id"], "state": r["job"]["state"], "replayed": replayed}))
        }
        "jobs_state" => {
            let _: NoArgs = mcp_args(args)?;
            let mine: Vec<Value> = cmds
                .engine
                .recent(200)
                .into_iter()
                .filter(|j| j.owner.as_deref() == Some(c.owner.as_str()))
                .take(25)
                .map(|j| json!({"jobId": j.id, "state": j.state, "stateSeq": j.state_seq, "title": j.title, "createdMs": j.created_ms}))
                .collect();
            let active = mine
                .iter()
                .filter(|j| jobs::is_active(j["state"].as_str().unwrap_or("")))
                .count();
            Ok(json!({"jobs": mine, "active": active}))
        }
        "approval_respond" => {
            let id = args["approvalId"].as_str().unwrap_or("").to_string();
            let decision = args["decision"].as_str().unwrap_or("").to_string();
            if !matches!(
                decision.as_str(),
                "accept" | "accept_session" | "decline" | "cancel"
            ) {
                return Err("INVALID_ARGUMENTS".into());
            }
            if !c.can_respond {
                return Err("APPROVAL_NOT_AUTHORIZED".into());
            }
            let door = cmds.approvals.as_ref().ok_or("ORCHESTRATION_UNAVAILABLE")?;
            let job = door
                .registry
                .permission(&id)
                .and_then(|q| (door.job_of)(&q.run_id))
                .and_then(|j| cmds.engine.job(&j))
                .filter(|j| j.owner.as_deref() == Some(c.owner.as_str()))
                .ok_or("APPROVAL_NOT_AUTHORIZED")?;
            let actor = omniget_core::core::assist::runs::Actor::new("mcp", &c.owner);
            let v = crate::approvals::respond(door, &id, &decision, &actor)
                .map_err(|r| r.code.to_string())?;
            Ok(
                json!({"approvalId": v["id"], "state": v["state"], "decision": v["decision"], "jobId": job.id}),
            )
        }
        "job_diff" => {
            let job_id = args["jobId"].as_str().unwrap_or("").to_string();
            let with_patch = match args["format"].as_str().unwrap_or("files") {
                "files" => false,
                "patch" => true,
                _ => return Err("INVALID_ARGUMENTS".into()),
            };
            cmds.readable(&c, &job_id).map_err(mcp_err)?;
            let (job, patch, complete) = cmds
                .engine
                .diff(&job_id)
                .await
                .map_err(|_| "JOB_NOT_AUTHORIZED".to_string())?;
            Ok(crate::mcp::orchestration::job_diff_reply(
                &job,
                patch.as_deref(),
                complete,
                with_patch,
            ))
        }
        _ => Err("UNKNOWN_TOOL".into()),
    }
}
