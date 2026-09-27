//! `POST /v1/observe/<tool>`: OmniGet's observation hook (plan §5.3, F5).
//!
//! `omniget-hook-shim --observe` posts every hook event of a coding tool that
//! runs outside the app (Claude Code in a terminal, Cursor, Gemini …) here,
//! already in Claude's hook JSON, with the bearer of that tool's installation
//! (`<app_data>/agentkit/observe/tokens.json`, not the extension token).
//!
//! Each event becomes bus traffic for agent `ext:<tool>` (`ExternalAgentActive`,
//! `TurnStarted`, `ToolCalled`, `TurnEnded`). A `PermissionRequest` (or a
//! `PreToolUse` sent with `hold`) is asked through the tool broker, the same
//! `ToolAsk` → pet/UI → `llm_tool_answer` path as the app's own agents, and the
//! HTTP answer is held until the owner decides. Nobody answering within the
//! wait (or nobody listening) answers `ask`: the tool shows its own prompt.
//!
//! Also two Tauri commands for the Central: `agentkit_observe_component`
//! (creates the token and returns the hook component to plan/apply like any
//! catalog component) and `agentkit_observe_revoke`.

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

/// Body cap: tool responses can be long, the event itself is small.
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

/// What the owner sees in the ask: the command, the path, else the input head.
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
    // nobody to show the question to: the tool asks as usual
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
            // drop the question everywhere and let the tool ask
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

// ---------------------------------------------------------------- commands

/// Creates (or reuses) the observe token of `tool` and returns the observe
/// hook component, to pass to `agentkit_plan` (`componentsJson`) with
/// `targets: [tool]`. Opt-in: nothing is installed until that plan is applied.
#[tauri::command]
pub async fn agentkit_observe_component(
    tool: String,
    gate: Option<bool>,
    wait_secs: Option<u64>,
) -> Result<Value, String> {
    let env = omniget_core::core::agentkit::Env::system().map_err(|e| e.to_string())?;
    let targets = omniget_core::core::agentkit::targets::load_targets(&env);
    let target = omniget_core::core::agentkit::targets::find(&targets, &tool)
        .map_err(|e| e.to_string())?
        .clone();
    if target
        .format(omniget_core::core::agentkit::model::ComponentKind::Hook)
        .is_none()
    {
        return Err(format!(
            "AGENTKIT_UNSUPPORTED: {} has no hooks",
            target.name
        ));
    }
    hook_observe::ensure_token(&env, &target.id).map_err(|e| e.to_string())?;
    let opts = hook_observe::ObserveOptions {
        gate: gate.unwrap_or(false),
        wait_secs,
    };
    let c = hook_observe::observe_component(&env, &target, &opts);
    serde_json::to_value(c).map_err(|e| format!("AGENTKIT_JSON: {e}"))
}

/// Forgets the observe token of `tool`: an observe hook still installed there
/// stops being accepted (uninstall it with `agentkit_uninstall`).
#[tauri::command]
pub async fn agentkit_observe_revoke(tool: String) -> Result<Value, String> {
    let env = omniget_core::core::agentkit::Env::system().map_err(|e| e.to_string())?;
    let had = hook_observe::revoke_token(&env, &tool).map_err(|e| e.to_string())?;
    Ok(json!({ "revoked": had, "observed": hook_observe::observed_tools(&env) }))
}

/// Tools with an observe token.
#[tauri::command]
pub async fn agentkit_observe_tools() -> Result<Value, String> {
    let env = omniget_core::core::agentkit::Env::system().map_err(|e| e.to_string())?;
    Ok(json!(hook_observe::observed_tools(&env)))
}
