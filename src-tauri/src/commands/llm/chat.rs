use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

use crate::AppState;

#[tauri::command]
pub async fn llm_tool_asks_pending(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    Ok(state.llm.pending_ask_list())
}

#[tauri::command]
pub async fn llm_tool_answer(
    app: AppHandle,
    state: State<'_, AppState>,
    request_id: String,
    tool_call_id: String,
    allow: bool,
    always: Option<bool>,
) -> Result<Value, String> {
    state
        .llm
        .answer_tool(&request_id, &tool_call_id, allow, always.unwrap_or(false))?;
    let _ = app.emit(
        "omni://tool-resolved",
        json!({ "request_id": request_id, "tool_call_id": tool_call_id, "allow": allow }),
    );
    Ok(json!({ "ok": true }))
}
