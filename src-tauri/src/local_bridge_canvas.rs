use axum::extract::{Path, Query};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::canvas::cords::{host, ApiErr};
use crate::mcp::policy::Principal;

const BODY_LIMIT: usize = 4 * 1024 * 1024;

fn node_of(p: &Principal) -> Option<String> {
    p.id.strip_prefix("canvas:")
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

fn status(code: u16) -> StatusCode {
    StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

fn fail(e: ApiErr) -> Response {
    (status(e.status), Json(e.body())).into_response()
}

fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        Json(json!({ "error": "not a canvas node token" })),
    )
        .into_response()
}

fn reply(r: Result<Value, ApiErr>) -> Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
struct WaitQuery {
    timeout_ms: Option<u64>,
}

#[derive(Deserialize)]
struct ScreenQuery {
    name: String,
    lines: Option<usize>,
}

#[derive(Deserialize)]
struct ReplyBody {
    ask_id: Option<String>,
    text: String,
    file: Option<String>,
}

#[derive(Deserialize)]
struct RpcBody {
    method: String,
    #[serde(default)]
    args: Value,
}

pub fn router<S>(app: AppHandle, guard: crate::bridge_tokens::Guard) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    host().bind(&app);
    Router::new()
        .route(
            "/v1/canvas/me",
            get(|Extension(p): Extension<Principal>| async move {
                match node_of(&p) {
                    Some(n) => reply(host().me(&n)),
                    None => forbidden(),
                }
            }),
        )
        .route(
            "/v1/canvas/peers",
            get(|Extension(p): Extension<Principal>| async move {
                match node_of(&p) {
                    Some(n) => reply(host().peers(&n)),
                    None => forbidden(),
                }
            }),
        )
        .route(
            "/v1/canvas/asks",
            post(
                |Extension(p): Extension<Principal>, Json(body): Json<Value>| async move {
                    let Some(n) = node_of(&p) else {
                        return forbidden();
                    };
                    match host().submit(&n, body).await {
                        Ok(v) => (StatusCode::ACCEPTED, Json(v)).into_response(),
                        Err(e) => fail(e),
                    }
                },
            ),
        )
        .route(
            "/v1/canvas/asks/{id}/wait",
            get(
                |Extension(p): Extension<Principal>,
                 Path(id): Path<String>,
                 Query(q): Query<WaitQuery>| async move {
                    let Some(n) = node_of(&p) else {
                        return forbidden();
                    };
                    let (code, v) = host().wait(&n, &id, q.timeout_ms.unwrap_or(100_000)).await;
                    (status(code), Json(v)).into_response()
                },
            ),
        )
        .route(
            "/v1/canvas/reply",
            post(
                |Extension(p): Extension<Principal>, Json(b): Json<ReplyBody>| async move {
                    let Some(n) = node_of(&p) else {
                        return forbidden();
                    };
                    reply(host().reply(&n, b.ask_id.as_deref(), &b.text, b.file.as_deref()))
                },
            ),
        )
        .route(
            "/v1/canvas/screen",
            get(
                |Extension(p): Extension<Principal>, Query(q): Query<ScreenQuery>| async move {
                    let Some(n) = node_of(&p) else {
                        return forbidden();
                    };
                    reply(host().screen(&n, &q.name, q.lines.unwrap_or(60)))
                },
            ),
        )
        .route(
            "/v1/canvas/hook",
            post(
                |Extension(p): Extension<Principal>, Json(body): Json<Value>| async move {
                    let Some(n) = node_of(&p) else {
                        return forbidden();
                    };
                    Json(host().hook(&n, body).await).into_response()
                },
            )
            .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT)),
        )
        .route(
            "/v1/canvas/rpc",
            post(
                |Extension(p): Extension<Principal>, Json(b): Json<RpcBody>| async move {
                    let Some(n) = node_of(&p) else {
                        return forbidden();
                    };
                    let Some((ws, floor)) = crate::canvas::doc::host().store().locate_node(&n)
                    else {
                        return fail(ApiErr {
                            status: 404,
                            code: "not_found",
                            message: format!("node {n} is not on any canvas"),
                        });
                    };
                    let ctx = crate::canvas::rpc::NodeCtx {
                        node_id: n,
                        workspace_id: ws,
                        floor_id: floor,
                        scopes: p.scopes.clone(),
                    };
                    match crate::canvas::rpc::dispatch(&ctx, &b.method, b.args).await {
                        Ok(v) => Json(json!({ "result": v })).into_response(),
                        Err(e) => {
                            let code = if e.starts_with("unknown canvas method") {
                                StatusCode::NOT_FOUND
                            } else {
                                StatusCode::BAD_REQUEST
                            };
                            (code, Json(json!({ "error": e }))).into_response()
                        }
                    }
                },
            ),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            guard,
            crate::bridge_tokens::guard,
        ))
}
