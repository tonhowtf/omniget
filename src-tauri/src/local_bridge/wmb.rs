use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::local_bridge_telegram::authorized;
use omniget_core::core::washmybelly::{approvals, plan::Source, store::Ctx, tools};

#[derive(Clone)]
pub struct WmbBridgeState {
    pub token: std::sync::Arc<String>,
    pub app: AppHandle,
    pub guard: crate::bridge_tokens::Guard,
}

impl WmbBridgeState {
    pub fn from_bridge(state: &crate::local_bridge::BridgeState) -> Self {
        Self {
            token: state.token.clone(),
            app: state.app.clone(),
            guard: crate::local_bridge_jobs::JobsBridgeState::from_bridge(state).guard(),
        }
    }
}

fn fail(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

async fn run(app: &AppHandle, tool: &str, input: Value) -> Result<Value, String> {
    if !omniget_core::core::llm::tool_table::feature_enabled("washmybelly") {
        return Err("washmybelly is disabled in settings".into());
    }
    if !tool.starts_with("wmb_") || !tools::all_names().contains(&tool) {
        return Err(format!("unknown tool: {tool}"));
    }
    if matches!(tool, "wmb_execute" | "wmb_restore") {
        let is_exec = tool == "wmb_execute";
        let target = input
            .get(if is_exec { "plan_id" } else { "receipt_id" })
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let approval = tauri::async_runtime::spawn_blocking(move || {
            let ctx = Ctx::app()?;
            if is_exec {
                approvals::request_execute(&ctx, &target, "omniget CLI")
            } else {
                approvals::request_restore(&ctx, &target, "omniget CLI")
            }
        })
        .await
        .map_err(|e| e.to_string())??;
        let _ = app.emit("washmybelly-approval", &approval);
        return Ok(json!({ "status": "pending_approval", "approval_id": approval.id }));
    }
    tools::call_as(Source::Cli, tool, input).await
}

pub fn router<S>(state: WmbBridgeState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route(
        "/v1/wmb/{tool}",
        post(
            move |Path(tool): Path<String>, headers: HeaderMap, body: Option<Json<Value>>| async move {
                if !authorized(&headers, &state.token, &state.guard) {
                    return fail(StatusCode::UNAUTHORIZED, "bad or missing bearer token");
                }
                let input = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));
                match run(&state.app, &tool, input).await {
                    Ok(v) => Json(v).into_response(),
                    Err(e) => fail(StatusCode::BAD_REQUEST, e),
                }
            },
        ),
    )
}
