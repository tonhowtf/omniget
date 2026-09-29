//! Bridge routes of the job system (the agent routes). Their bearer is a
//! scoped token (`bridge_tokens`, 2.1), checked by one `route_layer` that
//! maps each path to `jobs:read` or `jobs:operate`; the extension bearer
//! still passes for one version with a `Deprecation` header. Routes:
//! `POST /v1/hooks/<id>` (webhook trigger; the body becomes `{{body}}`), and
//! the small API `omniget-cli agent run|loop|jobs|explain|status|diff` talks to,
//! and `GET /v1/jobs/{id}/proof` (the host's proof of a finished job, 2.4).
//! `POST /v1/agent/run-tool` runs an agent of the catalog (or an `.md` file
//! the CLI read) as the system prompt of a coding CLI (`omniget agent run
//! <agent> --tool <x>`); `POST /v1/agent/loop-catalog` starts a catalog loop
//! on the Loops engine (`omniget agent loop --catalog <id>`); `GET
//! /v1/playbooks` lists the playbook runs; `GET /v1/approvals` and `POST
//! /v1/approvals/{id}` read and answer the durable asks (2.2). `POST
//! /v1/commands` and `GET /v1/jobs/{id}/wait` are the asynchronous,
//! idempotent command surface (2.5, `job_commands`). The review of a job (3.5):
//! `GET /v1/jobs/{id}/hunks`, `POST /v1/jobs/{id}/hunks/discard`,
//! `GET|POST /v1/jobs/{id}/notes`, `DELETE /v1/jobs/{id}/notes/{note}`,
//! `POST /v1/jobs/{id}/notes/send`, `GET|POST /v1/jobs/{id}/integrate`,
//! `POST /v1/jobs/{id}/discard` and `POST /v1/jobs/{id}/walkthrough`
//! (`omniget agent integrate|discard`).

use axum::extract::{Path, RawQuery};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::jobs::{self, LoopDef};

/// Who called (the guard put it there), for the audit line of a route that
/// writes its own (2.8).
type Caller = Option<axum::Extension<crate::mcp::policy::Principal>>;

fn caller(p: &Caller) -> omniget_core::core::llm::audit::Actor {
    crate::llm_audit::actor_of(p.as_ref().map(|e| &e.0), "http")
}

#[derive(Clone)]
pub struct JobsBridgeState {
    pub token: std::sync::Arc<String>,
    pub app: AppHandle,
}

impl JobsBridgeState {
    pub fn from_bridge(state: &crate::local_bridge::BridgeState) -> Self {
        Self {
            token: state.token.clone(),
            app: state.app.clone(),
        }
    }

    /// The guard of every route below: tokens are read from `jobs.db`.
    pub(crate) fn guard(&self) -> crate::bridge_tokens::Guard {
        let app = self.app.clone();
        crate::bridge_tokens::Guard {
            extension: self.token.clone(),
            legacy_allowed: crate::bridge_tokens::legacy_flag(),
            legacy_log: crate::bridge_tokens::legacy_log(),
            lookup: std::sync::Arc::new(move |t: &str| match jobs::get(&app) {
                Ok(j) => j.token_lookup(t),
                Err(_) => crate::bridge_tokens::Lookup::Unknown,
            }),
            now: std::sync::Arc::new(crate::llm_manager::now_ms),
        }
    }
}

fn fail(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// `format=` of a query string (`files` when absent).
fn format_of(query: Option<&str>) -> String {
    query
        .unwrap_or("")
        .split('&')
        .find_map(|pair| pair.strip_prefix("format="))
        .unwrap_or("files")
        .to_string()
}

fn ok<T: serde::Serialize>(v: Result<T, String>) -> Response {
    match v {
        Ok(v) => Json(json!(v)).into_response(),
        Err(e) => fail(StatusCode::BAD_REQUEST, e),
    }
}

#[derive(Deserialize)]
struct RunBody {
    agent_id: String,
    prompt: String,
    #[serde(default)]
    workspace: Option<String>,
    /// This job's ceiling in minutes (1–1440); absent = the setting.
    #[serde(default)]
    deadline_min: Option<u32>,
    /// `shared | worktree | auto` (2.3); absent = `auto`.
    #[serde(default)]
    isolation: Option<String>,
    /// The ref a worktree starts from; absent = the checkout as it is on
    /// disk, uncommitted work included.
    #[serde(default)]
    base: Option<String>,
    /// `approval_required | auto_edits | auto | full_access` (2.7); absent
    /// = `approval_required`: a job started from outside asks first (D2).
    #[serde(default)]
    run_mode: Option<String>,
}

pub fn router<S>(state: JobsBridgeState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    macro_rules! guarded {
        ($state:ident, $headers:ident, $body:expr) => {{
            // The bearer was checked by the route layer (`bridge_tokens::guard`).
            let _ = &$headers;
            let jobs = match jobs::get(&$state.app) {
                Ok(j) => j,
                Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e),
            };
            #[allow(clippy::redundant_closure_call)]
            ($body)(jobs)
        }};
    }
    let guard = state.guard();
    let (c1, c2, c3) = (state.clone(), state.clone(), state.clone());
    let (a1, a2) = (state.clone(), state.clone());
    let review = state.clone();
    let flow = state.clone();
    let s13 = state.clone();
    let (k1, k2) = (state.clone(), state.clone());
    let (s1, s2, s3, s4, s5, s6, s7, s8, s9, s10, s11, s12) = (
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state,
    );
    Router::new()
        .route(
            "/v1/agent/run-tool",
            post(
                move |Json(b): Json<crate::commands::central::run::ToolRunRequest>| async move {
                    ok(crate::commands::central::run::submit_tool_run(&c1.app, b).await)
                },
            ),
        )
        .route(
            "/v1/agent/loop-catalog",
            post(
                move |Json(b): Json<crate::commands::central::run::CatalogLoopRequest>| async move {
                    ok(crate::commands::central::run::start_catalog_loop(&c3.app, b).await)
                },
            ),
        )
        .route(
            "/v1/playbooks",
            get(move |headers: HeaderMap| async move {
                guarded!(c2, headers, |j: std::sync::Arc<jobs::Jobs>| Json(json!(
                    j.playbooks()
                ))
                .into_response())
            }),
        )
        .route(
            "/v1/hooks/{id}",
            post(
                move |Path(id): Path<String>, headers: HeaderMap, body: String| async move {
                    guarded!(s1, headers, |j: std::sync::Arc<jobs::Jobs>| ok(
                        j.fire(&id, Some(&body))
                    ))
                },
            ),
        )
        // Kept for one version as a shortcut of `job.start` on `/v1/commands`
        // (2.5): same checks, a fresh command id, the whole job back as
        // before, and a `Deprecation` header pointing at the new route.
        .route(
            "/v1/agent/run",
            post(
                move |headers: HeaderMap,
                      principal: Option<axum::Extension<crate::mcp::policy::Principal>>,
                      Json(b): Json<RunBody>| async move {
                    let cmds = match crate::job_commands::from_app(&s2.app) {
                        Ok(c) => c,
                        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e),
                    };
                    let Some(p) = principal else {
                        return fail(StatusCode::UNAUTHORIZED, "no principal");
                    };
                    let c = crate::job_commands::http_caller(&p.0, &headers, false);
                    let body = json!({
                        "command_id": format!("run-{}", uuid::Uuid::new_v4().simple()),
                        "type": "job.start",
                        "agent_id": b.agent_id,
                        "prompt": b.prompt,
                        "workspace": b.workspace,
                        "deadline_min": b.deadline_min,
                        "isolation": b.isolation,
                        "base": b.base,
                        "run_mode": b.run_mode,
                    });
                    match cmds.execute(&c, &body) {
                        Ok((r, _)) => {
                            let job = r["job_id"].as_str().and_then(|id| cmds.engine.job(id));
                            let mut res = Json(json!(job)).into_response();
                            res.headers_mut().insert(
                                "deprecation",
                                axum::http::HeaderValue::from_static("true"),
                            );
                            res.headers_mut().insert(
                                header::LINK,
                                axum::http::HeaderValue::from_static("</v1/commands>; rel=\"successor-version\""),
                            );
                            res
                        }
                        Err(e) => fail(
                            StatusCode::from_u16(e.status).unwrap_or(StatusCode::BAD_REQUEST),
                            e.text(),
                        ),
                    }
                },
            ),
        )
        // The command surface (2.5): `job.start`, `job.interrupt`,
        // `approval.respond`, idempotent by `command_id`. The route asks
        // for any valid token; the command itself checks its scope.
        .route(
            "/v1/commands",
            post(
                move |headers: HeaderMap,
                      principal: Option<axum::Extension<crate::mcp::policy::Principal>>,
                      Json(b): Json<Value>| async move {
                    match crate::job_commands::from_app(&k1.app) {
                        Ok(c) => crate::job_commands::http_command(
                            &c,
                            principal.as_ref().map(|p| &p.0),
                            &headers,
                            &b,
                        ),
                        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e),
                    }
                },
            ),
        )
        // Long poll on the job's state (2.5): wakes on the change, matches
        // only a state newer than `after_seq`, 408 on timeout.
        .route(
            "/v1/jobs/{id}/wait",
            get(
                move |Path(id): Path<String>,
                      RawQuery(query): RawQuery,
                      headers: HeaderMap,
                      principal: Option<axum::Extension<crate::mcp::policy::Principal>>| async move {
                    match crate::job_commands::from_app(&k2.app) {
                        Ok(c) => {
                            crate::job_commands::http_wait(
                                &c,
                                principal.as_ref().map(|p| &p.0),
                                &headers,
                                &id,
                                query.as_deref(),
                            )
                            .await
                        }
                        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e),
                    }
                },
            ),
        )
        .route(
            "/v1/agent/loop",
            post(
                move |headers: HeaderMap, who: Caller, Json(mut def): Json<LoopDef>| async move {
                    // A Loop started from outside asks first unless told (D2).
                    def.run_mode.get_or_insert(jobs::RunMode::ApprovalRequired);
                    guarded!(s3, headers, |j: std::sync::Arc<jobs::Jobs>| ok(j
                        .loop_create(def)
                        .inspect(|l| omniget_core::core::llm::audit::log(
                            caller(&who),
                            "loop.created",
                            omniget_core::core::llm::audit::Entity::loop_(&l.id),
                            json!({ "agent_id": l.agent_id, "run_mode": l.run_mode, "max_rounds": l.max_rounds }),
                        ))))
                },
            ),
        )
        .route(
            "/v1/agents",
            get(move |headers: HeaderMap| async move {
                guarded!(s4, headers, |_j: std::sync::Arc<jobs::Jobs>| {
                    use tauri::Manager;
                    let roster = s4.app.state::<crate::AppState>().llm.roster();
                    let list: Vec<Value> = roster
                        .into_iter()
                        .map(|a| json!({ "id": a.id, "name": a.name }))
                        .collect();
                    Json(json!(list)).into_response()
                })
            }),
        )
        .route(
            "/v1/agents/cli",
            post(move |headers: HeaderMap, who: Caller, Json(b): Json<Value>| async move {
                guarded!(s9, headers, |_j: std::sync::Arc<jobs::Jobs>| {
                    use omniget_core::core::llm::agent::{
                        AgentDef, AgentRole, ModelPolicy, RuntimeKind,
                    };
                    use omniget_core::core::llm::types::{ModelRef, ProviderId};
                    use tauri::Manager;
                    let text = |k: &str, d: &str| b[k].as_str().unwrap_or(d).to_string();
                    let agent = AgentDef {
                        id: text("id", "claude-code"),
                        name: text("name", "Claude Code"),
                        role: AgentRole::Worker,
                        system_prompt: String::new(),
                        model: ModelPolicy::Fixed {
                            model: ModelRef {
                                provider: ProviderId::new(text("cli", "claude")),
                                model: text("model", "default"),
                            },
                        },
                        tools: Vec::new(),
                        skills: Vec::new(),
                        budget: Default::default(),
                        runtime: RuntimeKind::Cli {
                            cli: text("cli", "claude"),
                            account: text("account", ""),
                        },
                        skin: None,
                    };
                    let (id, cli, account) = (text("id", "claude-code"), text("cli", "claude"), text("account", ""));
                    ok(s9
                        .app
                        .state::<crate::AppState>()
                        .llm
                        .roster_create(agent)
                        .inspect(|_| omniget_core::core::llm::audit::log(
                            caller(&who),
                            "agent.created",
                            omniget_core::core::llm::audit::Entity::agent(&id),
                            json!({ "cli": cli, "account": account }),
                        ))
                        .map(|r| r.len()))
                })
            }),
        )
        .route(
            "/v1/jobs",
            get(move |headers: HeaderMap| async move {
                guarded!(s5, headers, |j: std::sync::Arc<jobs::Jobs>| Json(json!(
                    j.list(100)
                ))
                .into_response())
            }),
        )
        .route(
            "/v1/jobs/{id}",
            get(
                move |Path(id): Path<String>, RawQuery(query): RawQuery| async move {
                    match crate::job_commands::from_app(&s6.app) {
                        Ok(c) => crate::job_commands::http_job(&c, &id, query.as_deref()),
                        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e),
                    }
                },
            ),
        )
        // Why the job is in its state (79#P8). Local bridge only: it carries
        // the pid, which never leaves through MCP.
        .route(
            "/v1/jobs/{id}/explain",
            get(
                move |Path(id): Path<String>, headers: HeaderMap| async move {
                    guarded!(
                        s10,
                        headers,
                        |j: std::sync::Arc<jobs::Jobs>| match j.explain(&id) {
                            Some(v) => Json(v).into_response(),
                            None => fail(StatusCode::NOT_FOUND, format!("no job {id}")),
                        }
                    )
                },
            ),
        )
        // What runs, what waits (position and reason) and the limits (1.6,
        // symphony's state snapshot). `omniget agent status` reads it.
        .route(
            "/v1/state",
            get(move |headers: HeaderMap| async move {
                guarded!(s11, headers, |j: std::sync::Arc<jobs::Jobs>| Json(
                    j.state_view()
                )
                .into_response())
            }),
        )
        // What the job changed (1.7, 78#P2): `format=files` (default) is the
        // list and totals as JSON, `format=patch` the unified diff. A job
        // with no snapshot says so (409 for the patch), never an empty diff.
        .route(
            "/v1/jobs/{id}/diff",
            get(
                move |Path(id): Path<String>, RawQuery(query): RawQuery| async move {
                    let j = match jobs::get(&s12.app) {
                        Ok(j) => j,
                        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e),
                    };
                    let (job, patch, complete) = match j.diff(&id).await {
                        Ok(v) => v,
                        Err(e) => return fail(StatusCode::NOT_FOUND, e),
                    };
                    let path = jobs::patch_path(&job.id);
                    let format = format_of(query.as_deref());
                    match jobs::diff_view(
                        &job,
                        patch.as_deref(),
                        path.as_deref(),
                        &format,
                        complete,
                    ) {
                        Ok(jobs::DiffBody::Json(v)) => Json(v).into_response(),
                        Ok(jobs::DiffBody::Patch(text)) => {
                            ([(header::CONTENT_TYPE, "text/x-diff; charset=utf-8")], text)
                                .into_response()
                        }
                        Err((code, e)) => fail(
                            StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_REQUEST),
                            e,
                        ),
                    }
                },
            ),
        )
        // The host's proof of what the job did (2.4, 83#P4): stamp, files,
        // checks with exit codes, tokens, cost and time. 409 while it runs.
        .route(
            "/v1/jobs/{id}/proof",
            get(
                move |Path(id): Path<String>, headers: HeaderMap| async move {
                    guarded!(
                        s13,
                        headers,
                        |j: std::sync::Arc<jobs::Jobs>| match j.job(&id) {
                            None => fail(StatusCode::NOT_FOUND, format!("no job {id}")),
                            Some(job) => match jobs::proof_view(&job) {
                                Ok(v) => Json(v).into_response(),
                                Err((code, e)) => fail(
                                    StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_REQUEST),
                                    e,
                                ),
                            },
                        }
                    )
                },
            ),
        )
        .route(
            "/v1/jobs/{id}/cancel",
            post(
                move |Path(id): Path<String>, headers: HeaderMap, who: Caller| async move {
                    guarded!(s7, headers, |j: std::sync::Arc<jobs::Jobs>| ok(
                        j.cancel_by(&id, caller(&who))
                    ))
                },
            ),
        )
        // Item 2.2: the one table of asks, answerable from outside the window
        // (`omniget agent approvals|approve`). Listing needs `jobs:read`,
        // answering `approvals:respond` (the guard maps both).
        .route(
            "/v1/approvals",
            get(move |RawQuery(query): RawQuery| async move {
                match crate::approvals::from_app(&a1.app) {
                    Ok(a) => crate::approvals::http_list(&a, query.as_deref()),
                    Err(e) => fail(StatusCode::SERVICE_UNAVAILABLE, e),
                }
            }),
        )
        .route(
            "/v1/approvals/{id}",
            post(
                move |Path(id): Path<String>,
                      principal: Option<axum::Extension<crate::mcp::policy::Principal>>,
                      Json(body): Json<Value>| async move {
                    match crate::approvals::from_app(&a2.app) {
                        Ok(a) => crate::approvals::http_respond(
                            &a,
                            &id,
                            &body,
                            principal.as_ref().map(|p| &p.0),
                        ),
                        Err(e) => fail(StatusCode::SERVICE_UNAVAILABLE, e),
                    }
                },
            ),
        )
        // The audit log (2.8): `omniget audit --since 1d`. Scope `audit:read`.
        .route(
            "/v1/audit",
            get(move |RawQuery(query): RawQuery| async move {
                crate::llm_audit::http_list(query.as_deref())
            }),
        )
        .route(
            "/v1/loops",
            get(move |headers: HeaderMap| async move {
                guarded!(s8, headers, |j: std::sync::Arc<jobs::Jobs>| Json(json!(
                    j.loops()
                ))
                .into_response())
            }),
        )
        // Merged before the layer so the review routes get their scope too.
        .merge(review_routes(review))
        .merge(budget_routes())
        .merge(workflow_routes(flow))
        .route_layer(axum::middleware::from_fn_with_state(
            guard,
            crate::bridge_tokens::guard,
        ))
}

#[derive(Deserialize)]
struct TrustBody {
    workspace: String,
    #[serde(default)]
    sha256: String,
    #[serde(default)]
    revoke: bool,
}

/// `.omniget/workflow.md` (3.10): `GET /v1/workflow?workspace=<dir>` (a
/// read), `POST /v1/workflow/trust` (`workflow:trust`, which neither a job's
/// token nor the extension key ever has: trusting runs the repository's
/// commands) and `GET /v1/jobs/{id}/report` (the delegating agent's
/// one-call report, a read of that job). `omniget agent workflow|report`.
fn workflow_routes<S>(state: JobsBridgeState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route(
            "/v1/workflow",
            get(|RawQuery(query): RawQuery| async move {
                let ws = crate::job_commands::query_param(query.as_deref(), "workspace")
                    .map(|w| {
                        urlencoding::decode(w)
                            .map(|c| c.into_owned())
                            .unwrap_or_else(|_| w.to_string())
                    })
                    .unwrap_or_default();
                ok(crate::job_workflow::status_of(&ws))
            }),
        )
        .route(
            "/v1/workflow/trust",
            post(|who: Caller, Json(b): Json<TrustBody>| async move {
                ok(crate::job_workflow::trust(
                    &b.workspace,
                    &b.sha256,
                    b.revoke,
                    caller(&who),
                ))
            }),
        )
        .route(
            "/v1/jobs/{id}/report",
            get(move |Path(id): Path<String>| async move {
                match jobs::get(&state.app) {
                    Ok(j) => match j.job(&id) {
                        Some(job) => Json(crate::job_workflow::report(&job, true)).into_response(),
                        None => fail(StatusCode::NOT_FOUND, format!("no job {id}")),
                    },
                    Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, e),
                }
            }),
        )
}

/// HTTP status of a review error, by its code.
fn review_status(e: &str) -> StatusCode {
    use omniget_core::core::llm::review as r;
    if e.starts_with(r::ERR_HUNK_STALE)
        || e.starts_with(r::ERR_HUNK_PARTIAL)
        || e.starts_with(r::ERR_REVIEW_BUSY)
        || e.starts_with(jobs::ERR_WORKTREE)
    {
        StatusCode::CONFLICT
    } else if e.starts_with(r::ERR_WALKTHROUGH_TOO_LARGE) {
        StatusCode::PAYLOAD_TOO_LARGE
    } else if e.starts_with(r::ERR_WALKTHROUGH_EMPTY) {
        StatusCode::UNPROCESSABLE_ENTITY
    } else if e.contains(": no job ") {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::BAD_REQUEST
    }
}

/// Budgets (3.3): `omniget agent budget list|set|raise`. Reading needs
/// `jobs:read`; setting and raising need `budget:write` (the guard of
/// `router`, merged before its `route_layer`).
fn budget_routes<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route(
            "/v1/budget",
            get(|| async { crate::llm_budget::http_state() }),
        )
        .route(
            "/v1/budget/policies",
            post(
                |who: Caller, Json(b): Json<crate::llm_budget::PolicyBody>| async move {
                    crate::llm_budget::http_save(b, caller(&who))
                },
            ),
        )
        .route(
            "/v1/budget/incidents/{id}",
            post(
                |Path(id): Path<String>,
                 who: Caller,
                 Json(b): Json<crate::llm_budget::ResolveBody>| async move {
                    crate::llm_budget::http_resolve(&id, b, caller(&who))
                },
            ),
        )
}

fn answer(v: Result<Value, String>) -> Response {
    match v {
        Ok(v) => Json(v).into_response(),
        Err(e) => fail(review_status(&e), e),
    }
}

#[derive(Deserialize)]
struct DiscardHunkBody {
    hunk_id: String,
    patch: String,
}

#[derive(Deserialize)]
struct NoteBody {
    path: String,
    #[serde(default)]
    start_line: Option<u32>,
    #[serde(default)]
    end_line: Option<u32>,
    body: String,
}

#[derive(Deserialize, Default)]
struct IntegrateBody {
    #[serde(default)]
    strategy: Option<String>,
}

#[derive(Deserialize, Default)]
struct WalkthroughBody {
    #[serde(default)]
    refresh: bool,
}

/// The review routes of a job (3.5). The scoped token is checked by the
/// `route_layer` of `router` (reads `jobs:read`, writes `jobs:operate`).
fn review_routes<S>(state: JobsBridgeState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    // The bearer was checked by the route layer (`bridge_tokens::guard`).
    macro_rules! with_jobs {
        ($state:ident, $headers:ident) => {{
            let _ = &$headers;
            match jobs::get(&$state.app) {
                Ok(j) => j,
                Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e),
            }
        }};
    }
    let (r3b, r6b) = (state.clone(), state.clone());
    let (r1, r2, r3, r4, r5, r6, r7, r8) = (
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state,
    );
    Router::new()
        .route(
            "/v1/jobs/{id}/hunks",
            get(move |Path(id): Path<String>, headers: HeaderMap| async move {
                let j = with_jobs!(r1, headers);
                answer(j.review_hunks(&id).await)
            }),
        )
        .route(
            "/v1/jobs/{id}/hunks/discard",
            post(
                move |Path(id): Path<String>, headers: HeaderMap, who: Caller, Json(b): Json<DiscardHunkBody>| async move {
                    let j = with_jobs!(r2, headers);
                    answer(j.discard_hunk(&id, &b.hunk_id, &b.patch).await.inspect(|_| {
                        crate::llm_audit::job_reviewed(caller(&who), "job.hunk_discarded", &id, json!({ "hunk_id": b.hunk_id }))
                    }))
                },
            ),
        )
        .route(
            "/v1/jobs/{id}/notes",
            get(move |Path(id): Path<String>, headers: HeaderMap| async move {
                let j = with_jobs!(r3, headers);
                if j.job(&id).is_none() {
                    return fail(StatusCode::NOT_FOUND, format!("no job {id}"));
                }
                Json(json!(j.notes(&id))).into_response()
            })
            .post({
                let r = r3b;
                move |Path(id): Path<String>, headers: HeaderMap, Json(b): Json<NoteBody>| async move {
                    let j = with_jobs!(r, headers);
                    answer(
                        j.note_add(&id, &b.path, b.start_line, b.end_line, &b.body)
                            .map(|n| json!(n)),
                    )
                }
            }),
        )
        .route(
            "/v1/jobs/{id}/notes/{note}",
            axum::routing::delete(
                move |Path((id, note)): Path<(String, String)>, headers: HeaderMap| async move {
                    let j = with_jobs!(r4, headers);
                    answer(j.note_delete(&id, &note).map(|_| json!({ "ok": true })))
                },
            ),
        )
        .route(
            "/v1/jobs/{id}/notes/send",
            post(move |Path(id): Path<String>, headers: HeaderMap| async move {
                let j = with_jobs!(r5, headers);
                answer(j.notes_send(&id).await.map(|job| json!(job)))
            }),
        )
        .route(
            "/v1/jobs/{id}/integrate",
            get(move |Path(id): Path<String>, headers: HeaderMap| async move {
                let j = with_jobs!(r6, headers);
                answer(j.integrate_preview(&id).await)
            })
            .post({
                let r = r6b;
                move |Path(id): Path<String>, headers: HeaderMap, who: Caller, body: Option<Json<IntegrateBody>>| async move {
                    let j = with_jobs!(r, headers);
                    let b = body.map(|Json(b)| b).unwrap_or_default();
                    let strategy = match omniget_core::core::vcs::integrate::Strategy::parse(
                        b.strategy.as_deref().unwrap_or(""),
                    ) {
                        Ok(s) => s,
                        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
                    };
                    match j.integrate(&id, strategy).await {
                        Ok((outcome, job)) => {
                            crate::llm_audit::job_reviewed(
                                caller(&who),
                                "job.integrated",
                                &id,
                                json!({ "strategy": strategy, "status": outcome.status }),
                            );
                            let ok = matches!(outcome.status.as_str(), "merged" | "up_to_date");
                            let body = json!({ "outcome": outcome, "job": job });
                            if ok {
                                Json(body).into_response()
                            } else {
                                // A conflict or a hook that failed: undone, 409 with the outcome.
                                let mut body = body;
                                body["error"] = json!(format!(
                                    "integration {}: {}",
                                    outcome.status,
                                    if outcome.conflicts.is_empty() { outcome.output.clone() } else { outcome.conflicts.join(", ") }
                                ));
                                (StatusCode::CONFLICT, Json(body)).into_response()
                            }
                        }
                        Err(e) => fail(review_status(&e), e),
                    }
                }
            }),
        )
        .route(
            "/v1/jobs/{id}/discard",
            post(move |Path(id): Path<String>, headers: HeaderMap, who: Caller| async move {
                let j = with_jobs!(r7, headers);
                answer(j.discard_work(&id).await.inspect(|_| {
                    crate::llm_audit::job_reviewed(caller(&who), "job.discarded", &id, json!({}))
                }))
            }),
        )
        .route(
            "/v1/jobs/{id}/walkthrough",
            post(
                move |Path(id): Path<String>, headers: HeaderMap, body: Option<Json<WalkthroughBody>>| async move {
                    let j = with_jobs!(r8, headers);
                    let refresh = body.map(|Json(b)| b.refresh).unwrap_or(false);
                    answer(j.walkthrough(&id, refresh).await.map(|w| json!(w)))
                },
            ),
        )
}

#[cfg(test)]
mod tests {
    use super::format_of;

    /// The review paths sit next to `/v1/jobs/{id}/cancel`, `/diff`… and
    /// `notes/send` next to `notes/{note}`: axum panics on a conflicting
    /// path when the router is built (at boot), so building it is the test.
    #[test]
    fn the_review_paths_route_without_conflict() {
        use axum::routing::{delete, get, post};
        let _app: axum::Router = axum::Router::new()
            .route("/v1/jobs/{id}", get(|| async { "job" }))
            .route("/v1/jobs/{id}/cancel", post(|| async { "cancel" }))
            .route("/v1/jobs/{id}/diff", get(|| async { "diff" }))
            .route("/v1/jobs/{id}/proof", get(|| async { "proof" }))
            .merge(
                axum::Router::new()
                    .route("/v1/jobs/{id}/hunks", get(|| async { "" }))
                    .route("/v1/jobs/{id}/hunks/discard", post(|| async { "" }))
                    .route(
                        "/v1/jobs/{id}/notes",
                        get(|| async { "" }).post(|| async { "" }),
                    )
                    .route("/v1/jobs/{id}/notes/{note}", delete(|| async { "" }))
                    .route("/v1/jobs/{id}/notes/send", post(|| async { "" }))
                    .route(
                        "/v1/jobs/{id}/integrate",
                        get(|| async { "" }).post(|| async { "" }),
                    )
                    .route("/v1/jobs/{id}/discard", post(|| async { "" }))
                    .route("/v1/jobs/{id}/walkthrough", post(|| async { "" })),
            );
    }

    #[test]
    fn the_diff_format_comes_from_the_query() {
        assert_eq!(format_of(None), "files");
        assert_eq!(format_of(Some("format=patch")), "patch");
        assert_eq!(format_of(Some("x=1&format=files")), "files");
        assert_eq!(format_of(Some("x=1")), "files");
    }
}
