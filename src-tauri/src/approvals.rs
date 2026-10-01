use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use omniget_core::core::assist::runs::{self, Actor, PermissionRequest, PermissionState, Registry};
use omniget_core::core::llm::broker::{AnswerRefused, Decision, ToolBroker};
use serde_json::{json, Value};

pub const ERR_NO_ASK: &str = "ERR_NO_ASK";
pub const ERR_ASK_STALE: &str = "ERR_ASK_STALE";
pub const ERR_ASK_DECISION: &str = "ERR_ASK_DECISION";

const LIST_MAX: usize = 200;

pub type JobOf = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;
pub type RunsOfJob = Arc<dyn Fn(&str) -> Vec<String> + Send + Sync>;

#[derive(Clone)]
pub struct Approvals {
    pub broker: Arc<ToolBroker>,
    pub registry: Registry,
    pub job_of: JobOf,
    pub runs_of_job: RunsOfJob,
    pub on_cancel: Arc<dyn Fn(&str) + Send + Sync>,
}

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
            match crate::jobs::try_get().and_then(|j| j.job_of_request(run).map(|id| (j, id))) {
                Some((jobs, id)) => {
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

pub fn api_state(s: PermissionState) -> &'static str {
    match s {
        PermissionState::Pending => "pending",
        PermissionState::Approved | PermissionState::Denied => "resolved",
        PermissionState::Cancelled => "cancelled",
        PermissionState::Stale => "stale",
        PermissionState::Expired => "expired",
    }
}

fn decision_of(word: &str) -> &'static str {
    match word {
        "once" => "accept",
        "session" => "accept_session",
        "always" => "accept_always",
        "deny" => "decline",
        _ => "cancel",
    }
}

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
        "deadline_ms": (p.deadline_ms < i64::MAX / 2).then_some(p.deadline_ms),
        "resolved_ms": p.resolved_ms,
        "channel": p.channel,
        "decided_by": p.decided_by,
        "note": p.note,
        "elicitation": p.payload,
    })
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filter {
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

pub fn http_list(a: &Approvals, query: Option<&str>) -> Response {
    match list(a, &parse_query(query)) {
        Ok(rows) => Json(json!(rows)).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response(),
    }
}

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

#[tauri::command]
pub async fn llm_approvals_list(
    app: tauri::AppHandle,
    state: Option<String>,
    job_id: Option<String>,
) -> Result<Vec<Value>, String> {
    list(&from_app(&app)?, &Filter { state, job_id })
}

#[tauri::command]
pub async fn llm_approval_respond(
    app: tauri::AppHandle,
    id: String,
    decision: String,
) -> Result<Value, String> {
    respond(&from_app(&app)?, &id, &decision, &Actor::window()).map_err(|r| r.text())
}
