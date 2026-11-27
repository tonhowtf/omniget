use std::time::Duration;

use axum::extract::Path;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use omniget_core::core::agentkit::convert::{hook_observe, hook_shim};
use omniget_core::core::llm::broker::Answer;
use omniget_core::core::omni::bus::BusEvent;

const BODY_LIMIT: usize = 4 * 1024 * 1024;

pub fn router<S>(app: AppHandle) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new().route(
        "/v1/observe/{tool}",
        post(
            move |Path(tool): Path<String>, headers: HeaderMap, body: axum::body::Bytes| {
                let app = app.clone();
                async move { observe(app, tool, headers, body).await }
            },
        )
        .layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT)),
    )
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get("authorization")?.to_str().ok()?;
    raw.strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .map(|s| s.trim().to_string())
}

fn fail(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({ "error": msg }))).into_response()
}

fn str_at<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}

fn preview(tool_name: &str, input: &Value) -> String {
    for k in ["command", "file_path", "url", "pattern", "prompt"] {
        if let Some(s) = input.get(k).and_then(|x| x.as_str()) {
            return s.chars().take(400).collect();
        }
    }
    let s = input.to_string();
    if s == "{}" || s == "null" {
        tool_name.to_string()
    } else {
        s.chars().take(400).collect()
    }
}

async fn observe(
    app: AppHandle,
    tool: String,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let tool: String = tool
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(40)
        .collect();
    let Some(data) = omniget_core::core::paths::app_data_dir() else {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, "no app data dir");
    };
    let Some(token) = bearer(&headers) else {
        return fail(StatusCode::UNAUTHORIZED, "bad or missing bearer token");
    };
    if !hook_observe::check_token(&data, &tool, &token) {
        return fail(StatusCode::UNAUTHORIZED, "bad or missing bearer token");
    }
    let v: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return fail(StatusCode::BAD_REQUEST, &format!("bad JSON: {e}")),
    };
    let event = str_at(&v, "hook_event_name").to_string();
    let tool_name = str_at(&v, "tool_name").to_string();
    let session = str_at(&v, "session_id").to_string();
    let cwd = str_at(&v, "cwd").to_string();
    let hold = v
        .get("omniget")
        .and_then(|o| o.get("hold"))
        .and_then(|h| h.as_bool())
        .unwrap_or(false);
    let wait_ms = v
        .get("omniget")
        .and_then(|o| o.get("wait_ms"))
        .and_then(|w| w.as_u64())
        .unwrap_or(hook_shim::DEFAULT_WAIT_SECS * 1000)
        .min(hook_shim::MAX_WAIT_SECS * 1000);
    let agent = format!("ext:{tool}");

    let state = app.state::<crate::AppState>();
    let bus = state.llm.bus();
    bus.emit(BusEvent::ExternalAgentActive {
        cli: tool.clone(),
        project: cwd.clone(),
    });
    match event.as_str() {
        "UserPromptSubmit" => bus.emit(BusEvent::TurnStarted {
            agent: agent.clone(),
            conversation: session.clone(),
        }),
        "PostToolUse" | "PostToolUseFailure" => bus.emit(BusEvent::ToolCalled {
            agent: agent.clone(),
            tool: tool_name.clone(),
            ok: event == "PostToolUse"
                && v.get("tool_response")
                    .and_then(|r| r.get("success"))
                    .and_then(|s| s.as_bool())
                    != Some(false),
            ms: 0,
        }),
        "Stop" | "SessionEnd" => bus.emit(BusEvent::TurnEnded {
            agent: agent.clone(),
            usage: Default::default(),
        }),
        _ => {}
    }
    let asks = event == "PermissionRequest" || (hold && event == "PreToolUse");
    if !asks {
        return Json(json!({ "ok": true })).into_response();
    }
    if bus.receiver_count() == 0 || wait_ms == 0 {
        return Json(json!({ "decision": "ask" })).into_response();
    }
    let broker = state.llm.broker();
    let call_id = format!("{agent}:{}", uuid::Uuid::new_v4().simple());
    let request_id = if session.is_empty() {
        agent.clone()
    } else {
        format!("{agent}:{session}")
    };
    let label = if tool_name.is_empty() {
        tool.clone()
    } else {
        format!("{tool_name} ({tool})")
    };
    let prev = preview(&tool_name, v.get("tool_input").unwrap_or(&Value::Null));
    let answer = tokio::time::timeout(
        Duration::from_millis(wait_ms),
        broker.ask_user(&agent, &request_id, &call_id, &label, &prev),
    )
    .await;
    let decision = match answer {
        Ok(Answer::Deny) => json!({ "decision": "deny", "reason": "Denied in OmniGet" }),
        Ok(Answer::Once) | Ok(Answer::Always) => json!({ "decision": "allow" }),
        Err(_) => {
            broker.answer_with(&call_id, Answer::Deny);
            let _ = app.emit(
                "omni://tool-resolved",
                json!({ "request_id": request_id, "tool_call_id": call_id, "allow": false, "expired": true }),
            );
            json!({ "decision": "ask" })
        }
    };
    Json(decision).into_response()
}
