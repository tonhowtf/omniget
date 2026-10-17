use std::convert::Infallible;
use std::sync::Arc;

use axum::response::sse::{Event, Sse};
use axum::{
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use futures::stream::{Stream, StreamExt};
use omniget_core::core::llm::types::{ContentPart, Message, Role, TurnEvent};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::llm_manager::LlmManager;

pub const MODEL_PREFIX: &str = "omniget/";

#[derive(Clone)]
pub struct LlmBridgeState {
    pub token: Arc<String>,
    pub manager: Arc<LlmManager>,
}

impl LlmBridgeState {
    pub fn new(token: Arc<String>, manager: Arc<LlmManager>) -> Self {
        Self { token, manager }
    }

    pub fn from_bridge(state: &crate::local_bridge::BridgeState) -> Self {
        use tauri::Manager;
        let app_state = state.app.state::<crate::AppState>();
        crate::commands::llm::ensure_wired(&state.app);
        Self {
            token: state.token.clone(),
            manager: app_state.llm.clone(),
        }
    }
}

pub fn router<S>(state: LlmBridgeState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let models_state = state.clone();
    Router::new()
        .route(
            "/v1/models",
            get(move |headers: HeaderMap| {
                let state = models_state.clone();
                async move { models(state, headers).await }
            }),
        )
        .route(
            "/v1/chat/completions",
            post(move |headers: HeaderMap, body: axum::body::Bytes| {
                let state = state.clone();
                async move { chat_completions(state, headers, body).await }
            }),
        )
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    #[serde(default)]
    pub messages: Vec<ChatMessage>,
    #[serde(default)]
    pub stream: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default)]
    pub content: Value,
}

#[derive(Debug, Clone, Serialize)]
struct ErrorBody {
    error: ErrorDetail,
}

#[derive(Debug, Clone, Serialize)]
struct ErrorDetail {
    message: String,
    code: String,
    #[serde(rename = "type")]
    kind: &'static str,
}

fn error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorBody {
            error: ErrorDetail {
                message: message.into(),
                code: code.to_string(),
                kind: "invalid_request_error",
            },
        }),
    )
        .into_response()
}

pub fn check_bearer(headers: &HeaderMap, expected: &str) -> bool {
    let Some(raw) = headers.get("authorization").and_then(|v| v.to_str().ok()) else {
        return false;
    };
    let Some(provided) = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
    else {
        return false;
    };
    constant_time_eq::constant_time_eq(provided.trim().as_bytes(), expected.as_bytes())
}

pub fn agent_of_model(model: &str) -> Option<&str> {
    let id = model.strip_prefix(MODEL_PREFIX)?;
    (!id.is_empty()).then_some(id)
}

pub fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

pub fn to_messages(request: &ChatRequest) -> Vec<Message> {
    request
        .messages
        .iter()
        .map(|m| {
            let role = match m.role.as_str() {
                "system" | "developer" => Role::System,
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                _ => Role::User,
            };
            Message {
                role,
                parts: vec![ContentPart::Text {
                    text: content_text(&m.content),
                }],
            }
        })
        .collect()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn chunk_json(id: &str, model: &str, delta: Value, finish: Option<&str>) -> Value {
    json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": now_secs(),
        "model": model,
        "choices": [ { "index": 0, "delta": delta, "finish_reason": finish } ],
    })
}

async fn models(state: LlmBridgeState, headers: HeaderMap) -> Response {
    if !check_bearer(&headers, &state.token) {
        return error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", "invalid bearer");
    }
    if !state.manager.bridge_openai_enabled() {
        return error(
            StatusCode::FORBIDDEN,
            "ERR_LLM_BRIDGE_OFF",
            "the OpenAI-compatible bridge is off (OmniGet → LLM → Local)",
        );
    }
    let created = now_secs();
    let data: Vec<Value> = state
        .manager
        .roster()
        .into_iter()
        .map(|a| {
            json!({
                "id": format!("{MODEL_PREFIX}{}", a.id),
                "object": "model",
                "created": created,
                "owned_by": "omniget",
            })
        })
        .collect();
    (
        StatusCode::OK,
        Json(json!({ "object": "list", "data": data })),
    )
        .into_response()
}

async fn chat_completions(
    state: LlmBridgeState,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !check_bearer(&headers, &state.token) {
        return error(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", "invalid bearer");
    }
    if !state.manager.bridge_openai_enabled() {
        return error(
            StatusCode::FORBIDDEN,
            "ERR_LLM_BRIDGE_OFF",
            "the OpenAI-compatible bridge is off (OmniGet → LLM → Local)",
        );
    }
    let request: ChatRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return error(StatusCode::BAD_REQUEST, "ERR_LLM_PARSE", e.to_string()),
    };
    let Some(agent_id) = agent_of_model(&request.model).map(str::to_string) else {
        return error(
            StatusCode::NOT_FOUND,
            "ERR_LLM_MODEL",
            format!(
                "model must be {MODEL_PREFIX}<agent_id>, got {}",
                request.model
            ),
        );
    };
    let messages = to_messages(&request);
    if messages.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            "ERR_LLM_PARSE",
            "messages is empty",
        );
    }

    let turn = state.manager.turn_stateless(&agent_id, messages).await;
    let (request_id, conversation_id, _cancel, stream) = match turn {
        Ok(t) => t,
        Err(message) => {
            let status = if message.starts_with("ERR_LLM_NO_AGENT") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::BAD_GATEWAY
            };
            let code = message
                .split(':')
                .next()
                .unwrap_or("ERR_LLM_NET")
                .to_string();
            return error(status, &code, message);
        }
    };
    let completion_id = format!("chatcmpl-{request_id}");
    let model = request.model.clone();

    if request.stream {
        let sse = sse_stream(
            state.manager.clone(),
            agent_id,
            request_id,
            conversation_id,
            completion_id,
            model,
            stream,
        );
        return Sse::new(sse).into_response();
    }

    let manager = state.manager.clone();
    let mut cleanup = TurnCleanup {
        manager: manager.clone(),
        agent_id: agent_id.clone(),
        request_id,
        conversation_id,
        ended: false,
    };
    let mut stream = stream;
    let mut answer = String::new();
    let mut usage = json!({ "prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0 });
    while let Some(event) = stream.next().await {
        manager.note_event(&agent_id, &event);
        match event {
            TurnEvent::TextDelta { text } => answer.push_str(&text),
            TurnEvent::Usage { usage: u } => {
                usage = json!({
                    "prompt_tokens": u.input_tokens,
                    "completion_tokens": u.output_tokens,
                    "total_tokens": u.input_tokens + u.output_tokens,
                });
            }
            TurnEvent::Error { error: e } => {
                cleanup.ended = true;
                return error(StatusCode::BAD_GATEWAY, &e.code, e.message);
            }
            TurnEvent::Finished { .. } => break,
            _ => {}
        }
    }
    cleanup.ended = true;

    (
        StatusCode::OK,
        Json(json!({
            "id": completion_id,
            "object": "chat.completion",
            "created": now_secs(),
            "model": model,
            "choices": [ {
                "index": 0,
                "message": { "role": "assistant", "content": answer },
                "finish_reason": "stop",
            } ],
            "usage": usage,
        })),
    )
        .into_response()
}

/// Ends the turn when the non-streaming handler returns or its future is dropped
/// (client timeout/disconnect), so the turn never lingers in the manager.
/// A turn that did not end on its own is cancelled so the agent stops working.
struct TurnCleanup {
    manager: Arc<LlmManager>,
    agent_id: String,
    request_id: String,
    conversation_id: String,
    ended: bool,
}

impl Drop for TurnCleanup {
    fn drop(&mut self) {
        if !self.ended {
            let _ = self.manager.cancel(&self.request_id);
        }
        self.manager.finish_turn(&self.request_id, &self.agent_id);
        self.manager.drop_conversation(&self.conversation_id);
    }
}

struct SseState {
    manager: Arc<LlmManager>,
    agent_id: String,
    completion_id: String,
    model: String,
    stream: futures::stream::BoxStream<'static, TurnEvent>,
    request_id: String,
    conversation_id: String,
    sent_role: bool,
    done: bool,
}

impl SseState {
    fn close(&mut self) {
        self.done = true;
        self.manager.finish_turn(&self.request_id, &self.agent_id);
        self.manager.drop_conversation(&self.conversation_id);
    }
}

impl Drop for SseState {
    fn drop(&mut self) {
        // Client aborted the SSE before the turn finished.
        if !self.done {
            let _ = self.manager.cancel(&self.request_id);
            self.close();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn sse_stream(
    manager: Arc<LlmManager>,
    agent_id: String,
    request_id: String,
    conversation_id: String,
    completion_id: String,
    model: String,
    stream: futures::stream::BoxStream<'static, TurnEvent>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    let state = SseState {
        manager,
        agent_id,
        request_id,
        conversation_id,
        completion_id,
        model,
        stream,
        sent_role: false,
        done: false,
    };
    futures::stream::unfold(state, |mut s| async move {
        if s.done {
            return None;
        }
        if !s.sent_role {
            s.sent_role = true;
            let chunk = chunk_json(
                &s.completion_id,
                &s.model,
                json!({ "role": "assistant", "content": "" }),
                None,
            );
            return Some((Ok(Event::default().data(chunk.to_string())), s));
        }
        while let Some(event) = s.stream.next().await {
            s.manager.note_event(&s.agent_id, &event);
            match event {
                TurnEvent::TextDelta { text } => {
                    let chunk =
                        chunk_json(&s.completion_id, &s.model, json!({ "content": text }), None);
                    return Some((Ok(Event::default().data(chunk.to_string())), s));
                }
                TurnEvent::Error { error } => {
                    let chunk =
                        json!({ "error": { "message": error.message, "code": error.code } });
                    s.close();
                    return Some((Ok(Event::default().data(chunk.to_string())), s));
                }
                TurnEvent::Finished { .. } => break,
                _ => continue,
            }
        }
        let stop = chunk_json(&s.completion_id, &s.model, json!({}), Some("stop"));
        s.close();
        Some((Ok(Event::default().data(stop.to_string())), s))
    })
    .chain(futures::stream::once(async {
        Ok(Event::default().data("[DONE]"))
    }))
}
