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
//! /v1/approvals/{id}` read and answer the durable asks (2.2).

use axum::extract::{Path, RawQuery};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::jobs::{self, LoopDef};

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
    fn guard(&self) -> crate::bridge_tokens::Guard {
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
    let s13 = state.clone();
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
        .route(
            "/v1/agent/run",
            post(
                move |headers: HeaderMap, Json(b): Json<RunBody>| async move {
                    guarded!(s2, headers, |j: std::sync::Arc<jobs::Jobs>| ok(b
                        .isolation
                        .as_deref()
                        .map(jobs::Isolation::parse)
                        .transpose()
                        .and_then(|isolation| j.submit_placed(
                            "run",
                            &b.agent_id,
                            &b.prompt,
                            b.workspace.clone(),
                            b.deadline_min,
                            isolation,
                            b.base.clone(),
                        ))))
                },
            ),
        )
        .route(
            "/v1/agent/loop",
            post(
                move |headers: HeaderMap, Json(def): Json<LoopDef>| async move {
                    guarded!(s3, headers, |j: std::sync::Arc<jobs::Jobs>| ok(
                        j.loop_create(def)
                    ))
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
            post(move |headers: HeaderMap, Json(b): Json<Value>| async move {
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
                    ok(s9
                        .app
                        .state::<crate::AppState>()
                        .llm
                        .roster_create(agent)
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
                move |Path(id): Path<String>, headers: HeaderMap| async move {
                    guarded!(
                        s6,
                        headers,
                        |j: std::sync::Arc<jobs::Jobs>| match j.job(&id) {
                            Some(job) => Json(json!(job)).into_response(),
                            None => fail(StatusCode::NOT_FOUND, format!("no job {id}")),
                        }
                    )
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
                move |Path(id): Path<String>, headers: HeaderMap| async move {
                    guarded!(s7, headers, |j: std::sync::Arc<jobs::Jobs>| ok(
                        j.cancel(&id)
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
        .route(
            "/v1/loops",
            get(move |headers: HeaderMap| async move {
                guarded!(s8, headers, |j: std::sync::Arc<jobs::Jobs>| Json(json!(
                    j.loops()
                ))
                .into_response())
            }),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            guard,
            crate::bridge_tokens::guard,
        ))
}

#[cfg(test)]
mod tests {
    use super::format_of;

    #[test]
    fn the_diff_format_comes_from_the_query() {
        assert_eq!(format_of(None), "files");
        assert_eq!(format_of(Some("format=patch")), "patch");
        assert_eq!(format_of(Some("x=1&format=files")), "files");
        assert_eq!(format_of(Some("x=1")), "files");
    }
}
