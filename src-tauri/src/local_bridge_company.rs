use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use omniget_core::core::company::{self as co, rpc};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::bridge_tokens::SCOPE_JOBS_OPERATE;

#[derive(Deserialize)]
struct Body {
    method: String,
    #[serde(default)]
    args: Value,
}

pub fn router<S>(guard: crate::bridge_tokens::Guard) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route(
            "/v1/company/rpc",
            post(
                move |principal: Option<axum::Extension<crate::mcp::policy::Principal>>,
                      Json(body): Json<Body>| async move {
                let Some(axum::Extension(p)) = principal else {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(json!({"error": "no principal"})),
                    )
                        .into_response();
                };
                let operate = p.scopes.iter().any(|s| s == SCOPE_JOBS_OPERATE);
                if !operate && !rpc::is_read_only(&body.method) {
                    return (
                        StatusCode::FORBIDDEN,
                        Json(json!({"error": format!(
                            "this token lacks the {SCOPE_JOBS_OPERATE} scope (`{}` changes the company)",
                            body.method
                        )})),
                    )
                        .into_response();
                }
                let out = tauri::async_runtime::spawn_blocking(move || {
                    let st = co::store()?;
                    let args = if body.args.is_null() {
                        json!({})
                    } else {
                        body.args
                    };
                    let r = rpc::dispatch_external(&st, &body.method, &args, "bridge", operate);
                    if r.is_ok() && !rpc::is_read_only(&body.method) {
                        if let Some(e) = crate::company::get() {
                            e.poke();
                        }
                    }
                    r
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r);
                respond(out)
            },
            ),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            guard,
            crate::bridge_tokens::guard,
        ))
}

fn respond(r: Result<Value, String>) -> Response {
    match r {
        Ok(v) => Json(v).into_response(),
        Err(e) => {
            let status = if e.starts_with(co::ERR_COMPANY_NOT_FOUND) {
                StatusCode::NOT_FOUND
            } else if e.starts_with(co::ERR_COMPANY_FORBIDDEN) {
                StatusCode::FORBIDDEN
            } else if e.starts_with(co::ERR_COMPANY_CONFLICT) {
                StatusCode::CONFLICT
            } else {
                StatusCode::BAD_REQUEST
            };
            (status, Json(json!({"error": e}))).into_response()
        }
    }
}
