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
    decision: Option<String>,
    allow: Option<bool>,
    always: Option<bool>,
) -> Result<Value, String> {
    use omniget_core::core::llm::broker::{resolve_decision, Answer, Decision};
    let decision = resolve_decision(decision.as_deref(), allow, always.unwrap_or(false))?;
    let answered = state.llm.answer_tool_with(&tool_call_id, decision);
    if decision == Decision::Cancel {
        let stopped = state.llm.cancel(&request_id);
        if answered.is_err() {
            stopped?;
        }
    } else {
        answered?;
    }
    let allow = !matches!(decision.answer(), Answer::Deny);
    let _ = app.emit(
        "omni://tool-resolved",
        json!({ "request_id": request_id, "tool_call_id": tool_call_id, "allow": allow, "decision": decision.as_str() }),
    );
    Ok(json!({ "ok": true }))
}
