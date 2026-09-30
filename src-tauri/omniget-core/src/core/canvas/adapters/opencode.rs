//! OpenCode has an HTTP server of its own (https://opencode.ai/docs/server/):
//! `POST /session/:id/prompt_async` takes a prompt without a terminal, and the
//! SSE stream `GET /event` says `session.idle` when a turn ends. Delivery and
//! state without the PTY. Spike (C2): the helpers are here; delivery still
//! goes through the PTY until a node knows its server port and session.

use serde_json::{json, Value};

pub fn prompt_async_path(session: &str) -> String {
    format!("/session/{session}/prompt_async")
}

pub fn prompt_body(text: &str) -> Value {
    json!({ "parts": [{ "type": "text", "text": text }] })
}

/// One `data:` line of the SSE stream -> the session that went idle.
pub fn idle_session(sse_data: &str) -> Option<String> {
    let v: Value = serde_json::from_str(sse_data.trim_start_matches("data:").trim()).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("session.idle") {
        return None;
    }
    v.pointer("/properties/sessionID")
        .and_then(|s| s.as_str())
        .map(str::to_string)
}
