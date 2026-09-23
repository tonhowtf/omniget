//! Bridge routes of the job system, same bearer as the rest of the bridge:
//! `POST /v1/hooks/<id>` (webhook trigger; the body becomes `{{body}}`), and
//! the small API `omniget-cli agent run|loop|jobs` talks to. `POST
//! /v1/agent/run-tool` runs an agent of the catalog (or an `.md` file the CLI
//! read) as the system prompt of a coding CLI (`omniget agent run <agent>
//! --tool <x>`); `POST /v1/agent/loop-catalog` starts a catalog loop on the
//! Loops engine (`omniget agent loop --catalog <id>`); `GET /v1/playbooks`
//! lists the playbook runs.

use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::jobs::{self, LoopDef};
use crate::local_bridge_llm::check_bearer;

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
}

fn fail(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
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
}

pub fn router<S>(state: JobsBridgeState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    macro_rules! guarded {
        ($state:ident, $headers:ident, $body:expr) => {{
            if !check_bearer(&$headers, &$state.token) {
                return fail(StatusCode::UNAUTHORIZED, "bad or missing bearer token");
            }
            let jobs = match jobs::get(&$state.app) {
                Ok(j) => j,
                Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e),
            };
            #[allow(clippy::redundant_closure_call)]
            ($body)(jobs)
        }};
    }
    let (s10, s11, s12) = (state.clone(), state.clone(), state.clone());
    let (s1, s2, s3, s4, s5, s6, s7, s8, s9) = (
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
                move |headers: HeaderMap,
                      Json(b): Json<crate::commands::central::run::ToolRunRequest>| async move {
                    if !check_bearer(&headers, &s10.token) {
                        return fail(StatusCode::UNAUTHORIZED, "bad or missing bearer token");
                    }
                    ok(crate::commands::central::run::submit_tool_run(&s10.app, b).await)
                },
            ),
        )
        .route(
            "/v1/agent/loop-catalog",
            post(
                move |headers: HeaderMap,
                      Json(b): Json<crate::commands::central::run::CatalogLoopRequest>| async move {
                    if !check_bearer(&headers, &s12.token) {
                        return fail(StatusCode::UNAUTHORIZED, "bad or missing bearer token");
                    }
                    ok(crate::commands::central::run::start_catalog_loop(&s12.app, b).await)
                },
            ),
        )
        .route(
            "/v1/playbooks",
            get(move |headers: HeaderMap| async move {
                guarded!(s11, headers, |j: std::sync::Arc<jobs::Jobs>| Json(json!(
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
                    guarded!(s2, headers, |j: std::sync::Arc<jobs::Jobs>| ok(j.submit(
                        "run",
                        &b.agent_id,
                        &b.prompt,
                        b.workspace.clone(),
                        None,
                        None
                    )))
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
        .route(
            "/v1/loops",
            get(move |headers: HeaderMap| async move {
                guarded!(s8, headers, |j: std::sync::Arc<jobs::Jobs>| Json(json!(
                    j.loops()
                ))
                .into_response())
            }),
        )
}
