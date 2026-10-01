//! Durable approvals answerable from anywhere (item 2.2 of the `/llm`
//! roadmap, S09).
//!
//! Every permission question of the broker is a row of `runs_permissions`
//! (the one table of asks: the in-memory list of the window is gone, A6).
//! This module is the one door every channel goes through to read and answer
//! them — the window (`llm_approval_respond`), the local bridge
//! (`GET /v1/approvals`, `POST /v1/approvals/{id}`), the CLI
//! (`omniget agent approvals|approve`, over the bridge), MCP
//! (`approval_respond`) and a paired phone (`/remote/api/approvals`) — so
//! the rules are the same whoever answers:
//!
//! - exactly one answer applies; any later one is `ERR_NO_ASK`;
//! - a question whose waiter died with another boot is `stale` ("restart the
//!   turn"): no answer releases it, `cancel` closes it;
//! - `cancel` refuses the call and stops the turn that asked;
//! - who answered and through what is recorded (`channel`, `decided_by`).
//!
//! The ask id (`apr_` + 128 random bits) is never put in anything the agent
//! reads: not the tool result, not the bus of its turn, not `explain`.
//! Answering needs `approvals:respond`, which a job's own token never has.

use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use omniget_core::core::assist::runs::{self, Actor, PermissionRequest, PermissionState, Registry};
use omniget_core::core::llm::broker::{AnswerRefused, Decision, ToolBroker};
use serde_json::{json, Value};

/// No question is waiting under this id any more (answered, expired,
/// cancelled) or it never existed.
pub const ERR_NO_ASK: &str = "ERR_NO_ASK";
/// The process that asked is gone: restart the turn.
pub const ERR_ASK_STALE: &str = "ERR_ASK_STALE";
/// The decision is not one of `accept|accept_session|accept_always|decline|cancel`.
pub const ERR_ASK_DECISION: &str = "ERR_ASK_DECISION";

/// Rows a list returns at most.
const LIST_MAX: usize = 200;

/// Run (request) id → the job whose turn it is.
pub type JobOf = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
/// Job id → the run (request) ids of its turns.
pub type RunsOfJob = Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>;

/// What the approvals door needs: the broker that holds the waiters, the run
/// record, and how runs map to jobs. The app builds it from its state
/// ([`from_app`]); the tests build it by hand.
#[derive(Clone)]
pub struct Approvals {
    pub broker: Arc<ToolBroker>,
    pub registry: Registry,
    /// The job whose turn is this run (request) id, if any.
    pub job_of: JobOf,
    /// The run (request) ids of one job's turns.
    pub runs_of_job: RunsOfJob,
    /// After a `cancel` decision: stop the turn (or job) that asked.
    pub on_cancel: Arc<dyn Fn(&str) + Send + Sync>,
}

/// The app's door. `Err` when the run record is not available (no assistant
/// database): nothing can be listed or answered durably then.
pub fn from_app(app: &tauri::AppHandle) -> Result<Approvals, String> {
    use tauri::Manager;
    let llm = app.state::<crate::AppState>().llm.clone();
    let registry =
        runs::active().ok_or_else(|| format!("{ERR_NO_ASK}: the run record is not available"))?;
    let cancel_llm = llm.clone();
    Ok(Approvals {
        broker: llm.broker(),
        registry,
        job_of: Arc::new(|run| crate::jobs::try_get().and_then(|j| j.job_of_request(run))),
        runs_of_job: Arc::new(|job| {
            crate::jobs::try_get()
                .and_then(|j| j.job(job))
                .and_then(|j| j.request_id)
                .into_iter()
                .collect()
        }),
        on_cancel: Arc::new(move |run| {
            // A job stops as a job (its row says cancelled); a chat turn stops
            // as a turn.
            match crate::jobs::try_get().and_then(|j| j.job_of_request(run).map(|id| (j, id))) {
                Some((jobs, id)) => {
                    // The answer's own line says who; this one says why (2.8).
                    let _ =
                        jobs.cancel_by(&id, omniget_core::core::llm::audit::Actor::system("ask"));
                }
                None => {
                    let _ = cancel_llm.cancel(run);
                }
            }
        }),
    })
}

/// The state the API speaks: an answered ask is `resolved`, with its
/// `decision`.
pub fn api_state(s: PermissionState) -> &'static str {
    match s {
        PermissionState::Pending => "pending",
        PermissionState::Approved | PermissionState::Denied => "resolved",
        PermissionState::Cancelled => "cancelled",
        PermissionState::Stale => "stale",
        PermissionState::Expired => "expired",
    }
}

/// The decision of a stored answer word (`runs_permissions.answer`).
fn decision_of(word: &str) -> &'static str {
    match word {
        "once" => "accept",
        "session" => "accept_session",
        "always" => "accept_always",
        "deny" => "decline",
        _ => "cancel",
    }
}

/// One approval as every channel shows it.
pub fn view(a: &Approvals, p: &PermissionRequest) -> Value {
    json!({
        "id": p.id,
        "state": api_state(p.state),
        "decision": p.answer.as_deref().map(decision_of),
        "job_id": (a.job_of)(&p.run_id),
        "run_id": p.run_id,
        "agent": p.agent_id,
        "tool": p.action,
        "preview": p.preview,
        "asked_ms": p.created_ms,
        // A thread of the Central waits with no deadline.
        "deadline_ms": (p.deadline_ms < i64::MAX / 2).then_some(p.deadline_ms),
        "resolved_ms": p.resolved_ms,
        "channel": p.channel,
        "decided_by": p.decided_by,
        "note": p.note,
        "elicitation": p.payload,
    })
}

/// `?state=&job_id=&limit=` of `GET /v1/approvals`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filter {
    /// `pending` (default) | `resolved` | `cancelled` | `stale` | `expired` | `all`.
    pub state: Option<String>,
    pub job_id: Option<String>,
}

pub fn parse_query(query: Option<&str>) -> Filter {
    let mut f = Filter::default();
    for pair in query.unwrap_or("").split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = v.trim();
        if v.is_empty() {
            continue;
        }
        match k {
            "state" => f.state = Some(v.to_string()),
            "job_id" => f.job_id = Some(v.to_string()),
            _ => {}
        }
    }
    f
}

/// The approvals in one API state (newest first).
pub fn list(a: &Approvals, f: &Filter) -> Result<Vec<Value>, String> {
    let states: &[PermissionState] = match f.state.as_deref().unwrap_or("pending") {
        "pending" => &[PermissionState::Pending],
        "resolved" => &[PermissionState::Approved, PermissionState::Denied],
        "cancelled" => &[PermissionState::Cancelled],
        "stale" => &[PermissionState::Stale],
        "expired" => &[PermissionState::Expired],
        "all" => &[
            PermissionState::Pending,
            PermissionState::Stale,
            PermissionState::Approved,
            PermissionState::Denied,
            PermissionState::Cancelled,
            PermissionState::Expired,
        ],
        other => {
            return Err(format!(
                "{ERR_ASK_DECISION}: unknown state `{other}` (pending|resolved|cancelled|stale|expired|all)"
            ))
        }
    };
    let runs = f.job_id.as_deref().map(|j| (a.runs_of_job)(j));
    let mut rows: Vec<PermissionRequest> = states
        .iter()
        .flat_map(|s| a.registry.permissions_in(*s, LIST_MAX))
        .filter(|p| runs.as_ref().is_none_or(|r| r.contains(&p.run_id)))
        .collect();
    rows.sort_by_key(|p| std::cmp::Reverse(p.created_ms));
    rows.truncate(LIST_MAX);
    Ok(rows.iter().map(|p| view(a, p)).collect())
}

/// Why an answer did not apply, as the API reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct Refused {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

impl Refused {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    pub fn text(&self) -> String {
        format!("{}: {}", self.code, self.message)
    }

    pub fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({ "error": self.text(), "code": self.code })),
        )
            .into_response()
    }
}

/// Answers the approval `id`. The one function behind every channel.
pub fn respond(a: &Approvals, id: &str, decision: &str, actor: &Actor) -> Result<Value, Refused> {
    let decision = Decision::parse(decision).ok_or_else(|| {
        Refused::new(
            StatusCode::BAD_REQUEST,
            ERR_ASK_DECISION,
            format!(
                "unknown decision `{decision}` ({})",
                Decision::ALL.join("|")
            ),
        )
    })?;
    let before = a.registry.permission(id).ok_or_else(|| {
        Refused::new(
            StatusCode::NOT_FOUND,
            ERR_NO_ASK,
            format!("no approval {id}"),
        )
    })?;
    match a.broker.respond_in(Some(&a.registry), id, decision, actor) {
        Ok(()) => {}
        Err(AnswerRefused::Stale) => {
            return Err(Refused::new(
                StatusCode::CONFLICT,
                ERR_ASK_STALE,
                before.note.unwrap_or_else(|| runs::STALE_NOTE.to_string()),
            ))
        }
        Err(_) => {
            let now = a.registry.permission(id).unwrap_or(before);
            return Err(Refused::new(
                StatusCode::CONFLICT,
                ERR_NO_ASK,
                format!(
                    "approval {id} is {} and takes no answer",
                    api_state(now.state)
                ),
            ));
        }
    }
    if decision == Decision::Cancel && before.state == PermissionState::Pending {
        (a.on_cancel)(&before.run_id);
    }
    let after = a.registry.permission(id).unwrap_or(before);
    Ok(view(a, &after))
}

/// `GET /v1/approvals?state=pending&job_id=`.
pub fn http_list(a: &Approvals, query: Option<&str>) -> Response {
    match list(a, &parse_query(query)) {
        Ok(rows) => Json(json!(rows)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

/// `POST /v1/approvals/{id}` with `{decision, channel?}`. `decided_by` is the
/// token that authenticated the request (never something the body says);
/// `channel` is `cli` when the CLI says so, `http` otherwise.
pub fn http_respond(
    a: &Approvals,
    id: &str,
    body: &Value,
    principal: Option<&crate::mcp::policy::Principal>,
) -> Response {
    let decision = body["decision"].as_str().unwrap_or("");
    let channel = match body["channel"].as_str() {
        Some("cli") => "cli",
        _ => "http",
    };
    let who = principal.map(|p| p.id.as_str()).unwrap_or("unknown");
    match respond(a, id, decision, &Actor::new(channel, who)) {
        Ok(v) => Json(v).into_response(),
        Err(r) => r.into_response(),
    }
}

// ── Tauri: the window ──────────────────────────────────────────────────

/// The approvals of one state (default `pending`), optionally of one job.
#[tauri::command]
pub async fn llm_approvals_list(
    app: tauri::AppHandle,
    state: Option<String>,
    job_id: Option<String>,
) -> Result<Vec<Value>, String> {
    list(&from_app(&app)?, &Filter { state, job_id })
}

/// The window's answer: the same door as the bridge, the CLI and MCP.
#[tauri::command]
pub async fn llm_approval_respond(
    app: tauri::AppHandle,
    id: String,
    decision: String,
) -> Result<Value, String> {
    respond(&from_app(&app)?, &id, &decision, &Actor::window()).map_err(|r| r.text())
}
