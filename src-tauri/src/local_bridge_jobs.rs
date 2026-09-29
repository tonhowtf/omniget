//! Bridge routes of the job system, same bearer as the rest of the bridge:
//! `POST /v1/hooks/<id>` (webhook trigger; the body becomes `{{body}}`), and
//! the small API `omniget-cli agent run|loop|jobs|explain|status|diff` talks to,
//! and `GET /v1/jobs/{id}/proof` (the host's proof of a finished job, 2.4).
//! `POST /v1/agent/run-tool` runs an agent of the catalog (or an `.md` file
//! the CLI read) as the system prompt of a coding CLI (`omniget agent run
//! <agent> --tool <x>`); `POST /v1/agent/loop-catalog` starts a catalog loop
//! on the Loops engine (`omniget agent loop --catalog <id>`); `GET
//! /v1/playbooks` lists the playbook runs. The review of a job (3.5):
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
    let (c1, c2, c3) = (state.clone(), state.clone(), state.clone());
    let review = state.clone();
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
                move |headers: HeaderMap,
                      Json(b): Json<crate::commands::central::run::ToolRunRequest>| async move {
                    if !check_bearer(&headers, &c1.token) {
                        return fail(StatusCode::UNAUTHORIZED, "bad or missing bearer token");
                    }
                    ok(crate::commands::central::run::submit_tool_run(&c1.app, b).await)
                },
            ),
        )
        .route(
            "/v1/agent/loop-catalog",
            post(
                move |headers: HeaderMap,
                      Json(b): Json<crate::commands::central::run::CatalogLoopRequest>| async move {
                    if !check_bearer(&headers, &c3.token) {
                        return fail(StatusCode::UNAUTHORIZED, "bad or missing bearer token");
                    }
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
                move |Path(id): Path<String>, RawQuery(query): RawQuery, headers: HeaderMap| async move {
                    if !check_bearer(&headers, &s12.token) {
                        return fail(StatusCode::UNAUTHORIZED, "bad or missing bearer token");
                    }
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
                    match jobs::diff_view(&job, patch.as_deref(), path.as_deref(), &format, complete) {
                        Ok(jobs::DiffBody::Json(v)) => Json(v).into_response(),
                        Ok(jobs::DiffBody::Patch(text)) => (
                            [(header::CONTENT_TYPE, "text/x-diff; charset=utf-8")],
                            text,
                        )
                            .into_response(),
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
        .route(
            "/v1/loops",
            get(move |headers: HeaderMap| async move {
                guarded!(s8, headers, |j: std::sync::Arc<jobs::Jobs>| Json(json!(
                    j.loops()
                ))
                .into_response())
            }),
        )        .merge(review_routes(review))
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

/// The review routes of a job (3.5). Same bearer as the other job routes;
/// after the merge with 2.1 the writes are `jobs:operate` and the reads
/// `jobs:read`.
fn review_routes<S>(state: JobsBridgeState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    // Bearer, then the jobs instance; the body runs with both.
    macro_rules! with_jobs {
        ($state:ident, $headers:ident) => {{
            if !check_bearer(&$headers, &$state.token) {
                return fail(StatusCode::UNAUTHORIZED, "bad or missing bearer token");
            }
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
                move |Path(id): Path<String>, headers: HeaderMap, Json(b): Json<DiscardHunkBody>| async move {
                    let j = with_jobs!(r2, headers);
                    answer(j.discard_hunk(&id, &b.hunk_id, &b.patch).await)
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
                move |Path(id): Path<String>, headers: HeaderMap, body: Option<Json<IntegrateBody>>| async move {
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
            post(move |Path(id): Path<String>, headers: HeaderMap| async move {
                let j = with_jobs!(r7, headers);
                answer(j.discard_work(&id).await)
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
