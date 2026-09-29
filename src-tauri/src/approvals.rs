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
                    let _ = jobs.cancel(&id);
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

#[cfg(test)]
mod tests {
    use super::*;
    use omniget_core::core::assist::db::AssistDb;
    use omniget_core::core::assist::runs::{NewRun, RunState};
    use omniget_core::core::llm::agent::{GrantMode, ToolGrant, ToolSource};
    use omniget_core::core::llm::error::LlmError;
    use omniget_core::core::llm::types::ToolSpec;
    use omniget_core::core::omni::bus::Bus;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    /// Records when each tool really ran.
    struct Clock(Arc<Mutex<Vec<(String, Instant)>>>);

    #[async_trait::async_trait]
    impl omniget_core::core::llm::broker::ToolExecutor for Clock {
        async fn execute(&self, name: &str, _input: Value) -> Result<String, LlmError> {
            self.0
                .lock()
                .unwrap()
                .push((name.to_string(), Instant::now()));
            Ok(format!("ran {name}"))
        }
    }

    type Ran = Arc<Mutex<Vec<(String, Instant)>>>;

    fn setup() -> (Approvals, Ran, Arc<Mutex<Vec<String>>>) {
        let ran: Ran = Arc::new(Mutex::new(Vec::new()));
        let broker = Arc::new(
            ToolBroker::new(
                vec![ToolSpec {
                    name: "shell_exec".into(),
                    description: String::new(),
                    input_schema: json!({"type": "object"}),
                }],
                Arc::new(Clock(ran.clone())),
                Arc::new(Bus::new()),
            )
            .with_ask_timeout(Duration::from_secs(60)),
        );
        let registry = Registry::with_instance(
            Arc::new(AssistDb::open_in_memory().unwrap()),
            "boot-approvals",
        );
        let cancelled = Arc::new(Mutex::new(Vec::new()));
        let c = cancelled.clone();
        let a = Approvals {
            broker,
            registry,
            job_of: Arc::new(|run| run.strip_prefix("req-").map(|j| format!("job-{j}"))),
            runs_of_job: Arc::new(|job| {
                job.strip_prefix("job-")
                    .map(|j| vec![format!("req-{j}")])
                    .unwrap_or_default()
            }),
            on_cancel: Arc::new(move |run| c.lock().unwrap().push(run.to_string())),
        };
        (a, ran, cancelled)
    }

    fn grants() -> Vec<ToolGrant> {
        vec![ToolGrant {
            source: ToolSource::Internal {
                name: "shell_exec".into(),
            },
            mode: GrantMode::Ask,
        }]
    }

    fn ask(
        a: &Approvals,
        run: &str,
        call: &str,
    ) -> tokio::task::JoinHandle<Result<omniget_core::core::llm::broker::ToolOutcome, LlmError>>
    {
        let _ = a.registry.open_run(NewRun {
            id: run.into(),
            conversation_id: "c".into(),
            bot_id: "worker".into(),
            ..Default::default()
        });
        let _ = a.registry.transition(run, RunState::Running, None);
        let (b, r, run, call) = (
            a.broker.clone(),
            a.registry.clone(),
            run.to_string(),
            call.to_string(),
        );
        tokio::spawn(async move {
            runs::scope(
                Some(r),
                b.call(
                    "worker",
                    &grants(),
                    &run,
                    &call,
                    "shell_exec",
                    json!({"command": "npm test"}),
                ),
            )
            .await
        })
    }

    async fn pending_id(a: &Approvals, call: &str) -> String {
        for _ in 0..2_000 {
            if let Some(p) = a.registry.pending_of_call(call) {
                return p.id;
            }
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        panic!("no ask for {call}");
    }

    #[test]
    fn the_query_picks_state_and_job() {
        assert_eq!(parse_query(None), Filter::default());
        assert_eq!(
            parse_query(Some("state=stale&job_id=j1&x=")),
            Filter {
                state: Some("stale".into()),
                job_id: Some("j1".into())
            }
        );
    }

    /// Acceptance: the second answer is `ERR_NO_ASK`, whoever sends it; the
    /// list filters by job; the view carries the elicitation payload and who
    /// answered.
    #[tokio::test]
    async fn one_answer_applies_and_the_second_is_err_no_ask() {
        let (a, ran, _) = setup();
        let task = ask(&a, "req-1", "call-1");
        let id = pending_id(&a, "call-1").await;
        let listed = list(
            &a,
            &Filter {
                state: None,
                job_id: Some("job-1".into()),
            },
        )
        .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["id"], id.as_str());
        assert_eq!(listed[0]["job_id"], "job-1");
        assert_eq!(listed[0]["elicitation"]["method"], "elicitation/create");
        assert!(list(
            &a,
            &Filter {
                state: None,
                job_id: Some("job-2".into())
            }
        )
        .unwrap()
        .is_empty());

        let v = respond(&a, &id, "accept", &Actor::new("cli", "token:t1")).unwrap();
        assert_eq!(v["state"], "resolved");
        assert_eq!(v["decision"], "accept");
        assert_eq!(v["channel"], "cli");
        assert_eq!(v["decided_by"], "token:t1");
        assert!(task.await.unwrap().unwrap().ok);
        assert_eq!(ran.lock().unwrap().len(), 1);

        for (d, who) in [
            ("accept", Actor::window()),
            ("decline", Actor::new("mcp", "mcp:x")),
        ] {
            let e = respond(&a, &id, d, &who).unwrap_err();
            assert_eq!(e.code, ERR_NO_ASK, "{e:?}");
            assert_eq!(e.status, StatusCode::CONFLICT);
        }
        assert_eq!(
            respond(&a, "apr_nope", "accept", &Actor::window())
                .unwrap_err()
                .code,
            ERR_NO_ASK
        );
        assert_eq!(
            respond(&a, &id, "maybe", &Actor::window())
                .unwrap_err()
                .code,
            ERR_ASK_DECISION
        );
        assert_eq!(ran.lock().unwrap().len(), 1, "nothing ran twice");
    }

    /// `cancel` refuses the call and stops the turn that asked.
    #[tokio::test]
    async fn cancel_refuses_and_stops_the_turn() {
        let (a, ran, cancelled) = setup();
        let task = ask(&a, "req-9", "call-9");
        let id = pending_id(&a, "call-9").await;
        let v = respond(&a, &id, "cancel", &Actor::window()).unwrap();
        assert_eq!(v["state"], "cancelled");
        assert!(task.await.unwrap().is_err());
        assert_eq!(*cancelled.lock().unwrap(), vec!["req-9".to_string()]);
        assert!(ran.lock().unwrap().is_empty());
    }

    // ── BN3 wave 2 over HTTP (scripts/llm/approvals_eval.py) ───────────

    /// `omniget agent approve` → the tool runs: the production guard and the
    /// production handlers (`http_list`, `http_respond`) on 127.0.0.1, a
    /// real broker and run record, a CLI-scoped token from a real token
    /// table; 20 asks answered through HTTP exactly as the CLI sends them.
    /// Measured: from sending the answer to the tool running.
    #[tokio::test]
    async fn bn3_approve_over_http_runs_the_tool_fast() {
        use crate::bridge_tokens::{self as bt, Guard, Kind, LegacyLog, Lookup};
        use axum::extract::{Path, RawQuery};
        use axum::routing::{get, post};
        use std::sync::atomic::AtomicBool;

        let (a, ran, _) = setup();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        bt::ensure_schema(&conn).unwrap();
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        let cli = bt::mint(&conn, Kind::Cli, "cli", &s(bt::SCOPES), None, None, 1_000)
            .unwrap()
            .token;
        let reader = bt::mint(
            &conn,
            Kind::User,
            "reader",
            &s(&[bt::SCOPE_JOBS_READ]),
            None,
            None,
            1_000,
        )
        .unwrap()
        .token;
        let db = Arc::new(Mutex::new(conn));
        let guard = Guard {
            extension: Arc::new("ext-key-0123456789abcdefghijklmnopqrstuvw".into()),
            legacy_allowed: Arc::new(AtomicBool::new(true)),
            legacy_log: Arc::new(LegacyLog::default()),
            lookup: {
                let db = db.clone();
                Arc::new(move |t: &str| -> Lookup { bt::lookup(&db.lock().unwrap(), t, 2_000) })
            },
            now: Arc::new(|| 2_000),
        };
        let (a1, a2) = (a.clone(), a.clone());
        let app = axum::Router::new()
            .route(
                "/v1/approvals",
                get(move |RawQuery(q): RawQuery| async move { http_list(&a1, q.as_deref()) }),
            )
            .route(
                "/v1/approvals/{id}",
                post(
                    move |Path(id): Path<String>,
                          p: Option<axum::Extension<crate::mcp::policy::Principal>>,
                          Json(b): Json<Value>| async move {
                        http_respond(&a2, &id, &b, p.as_ref().map(|e| &e.0))
                    },
                ),
            )
            .route_layer(axum::middleware::from_fn_with_state(guard, bt::guard));
        let base = crate::bridge_tokens::bn5::serve(app).await;
        let http = reqwest::Client::new();

        // Scopes: a reader lists but never answers; the extension key never
        // answers, even during the grace period.
        let r = http
            .get(format!("{base}/v1/approvals"))
            .bearer_auth(&reader)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), 200);

        let mut ms = Vec::new();
        for i in 0..20 {
            let (run, call) = (format!("req-{i}"), format!("call-{i}"));
            let task = ask(&a, &run, &call);
            let _ = pending_id(&a, &call).await;
            // What `omniget agent approvals --job` reads, then `approve <id>`.
            let rows: Value = http
                .get(format!("{base}/v1/approvals?state=pending&job_id=job-{i}"))
                .bearer_auth(&cli)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            let id = rows[0]["id"].as_str().unwrap().to_string();
            if i == 0 {
                for (tok, want) in [
                    (reader.as_str(), 403),
                    ("ext-key-0123456789abcdefghijklmnopqrstuvw", 403),
                ] {
                    let r = http
                        .post(format!("{base}/v1/approvals/{id}"))
                        .bearer_auth(tok)
                        .json(&json!({"decision": "accept"}))
                        .send()
                        .await
                        .unwrap();
                    assert_eq!(r.status().as_u16(), want);
                }
            }
            let t0 = Instant::now();
            let r = http
                .post(format!("{base}/v1/approvals/{id}"))
                .bearer_auth(&cli)
                .json(&json!({"decision": "accept", "channel": "cli"}))
                .send()
                .await
                .unwrap();
            assert_eq!(r.status().as_u16(), 200);
            task.await.unwrap().unwrap();
            let at = ran.lock().unwrap().last().map(|(_, t)| *t).unwrap();
            ms.push(at.saturating_duration_since(t0).as_secs_f64() * 1000.0);
            if i == 0 {
                let again = http
                    .post(format!("{base}/v1/approvals/{id}"))
                    .bearer_auth(&cli)
                    .json(&json!({"decision": "accept"}))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(again.status().as_u16(), 409);
                let body: Value = again.json().await.unwrap();
                assert_eq!(body["code"], ERR_NO_ASK);
                crate::bridge_tokens::bn5::emit(json!({
                    "bench": "bn3", "metric": "http_second_answer_code", "value": body["code"], "level": "http"
                }));
                let row = a.registry.permission(&id).unwrap();
                assert_eq!(row.channel.as_deref(), Some("cli"));
                assert!(row.decided_by.as_deref().unwrap().starts_with("token:"));
            }
        }
        ms.sort_by(|x, y| x.partial_cmp(y).unwrap());
        let p50 = ms[ms.len() / 2];
        let max = *ms.last().unwrap();
        for (metric, value) in [
            ("approve_http_to_tool_ms_p50", json!(p50)),
            ("approve_http_to_tool_ms_max", json!(max)),
            ("approve_http_asks", json!(ms.len())),
        ] {
            crate::bridge_tokens::bn5::emit(
                json!({ "bench": "bn3", "metric": metric, "value": value, "level": "http" }),
            );
        }
        assert!(p50 < 1_000.0, "p50 {p50} ms");
        assert!(a.registry.permissions(None, true).is_empty());
    }
}
