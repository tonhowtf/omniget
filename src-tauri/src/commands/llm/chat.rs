use std::time::Instant;

use futures::StreamExt;
use omniget_core::core::llm::types::ModelRef;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

use super::{emit_turn, ensure_wired, spawn_telemetry_ticker, DeltaCoalescer, DELTA_HZ};
use crate::AppState;

#[tauri::command]
pub async fn llm_conversation_list(state: State<'_, AppState>) -> Result<Value, String> {
    serde_json::to_value(state.llm.conversations()).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn llm_conversation_get(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Value, String> {
    let messages = state.llm.conversation(&conversation_id);
    Ok(json!({
        "conversation_id": conversation_id,
        "messages": serde_json::to_value(messages).map_err(|e| e.to_string())?,
    }))
}

#[tauri::command]
pub async fn llm_turn_start(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    agent_id: String,
    input: String,
) -> Result<Value, String> {
    ensure_wired(&app);
    let manager = state.llm.clone();
    let (request_id, _cancel, stream) = manager
        .turn_stream(&conversation_id, &agent_id, &input)
        .await?;
    forward_turn(
        app,
        manager,
        agent_id,
        conversation_id,
        input,
        request_id.clone(),
        stream,
        None,
    );
    Ok(json!({ "request_id": request_id }))
}

#[derive(Debug, Clone, Default)]
pub struct TurnSummary {
    pub text: String,
    pub error: Option<String>,
    pub cancelled: bool,
    pub tokens: Option<i64>,
}

pub type TurnDone = Box<dyn FnOnce(TurnSummary) + Send + 'static>;

#[allow(clippy::too_many_arguments)]
pub(crate) fn forward_turn(
    app: AppHandle,
    manager: std::sync::Arc<crate::llm_manager::LlmManager>,
    agent_id: String,
    conversation_id: String,
    input: String,
    request_id: String,
    mut stream: futures::stream::BoxStream<'static, omniget_core::core::llm::types::TurnEvent>,
    on_done: Option<TurnDone>,
) {
    spawn_telemetry_ticker(app.clone());
    let jobs = crate::jobs::get(&app).ok();
    let job_id = jobs
        .as_ref()
        .map(|j| j.chat_started(&agent_id, &conversation_id, &input, &request_id));
    tauri::async_runtime::spawn(async move {
        let mut coalescer = DeltaCoalescer::new(DELTA_HZ);
        let mut said = String::new();
        let mut failed: Option<String> = None;
        let mut cancelled = false;
        let mut usage = Vec::new();
        let mut receipts = Vec::new();
        while let Some(event) = stream.next().await {
            manager.note_event(&agent_id, &event);
            match &event {
                omniget_core::core::llm::types::TurnEvent::TextDelta { text } => {
                    said.push_str(text)
                }
                r @ omniget_core::core::llm::types::TurnEvent::PruneReceipt { .. } => {
                    receipts.push(r.clone())
                }
                omniget_core::core::llm::types::TurnEvent::Usage { usage: u } => {
                    usage.push(u.clone())
                }
                omniget_core::core::llm::types::TurnEvent::Error { error } => {
                    failed = Some(format!("{}: {}", error.code, error.message))
                }
                omniget_core::core::llm::types::TurnEvent::Finished { reason } => {
                    cancelled = matches!(
                        reason,
                        omniget_core::core::llm::types::FinishReason::Cancelled
                    )
                }
                _ => {}
            }
            for out in coalescer.push(event, Instant::now()) {
                emit_turn(&app, &request_id, &out);
            }
        }
        if let Some(rest) = coalescer.flush() {
            emit_turn(&app, &request_id, &rest);
        }
        manager.finish_turn(&request_id, &agent_id);
        let tokens = (!usage.is_empty()).then(|| {
            usage
                .iter()
                .map(|u| (u.input_tokens as i64) + (u.output_tokens as i64))
                .sum::<i64>()
        });
        if let (Some(jobs), Some(job_id)) = (jobs, job_id) {
            jobs.chat_finished(&job_id, &said, failed.clone(), cancelled, &usage, &receipts);
        }
        if let Some(done) = on_done {
            done(TurnSummary {
                text: said,
                error: failed,
                cancelled,
                tokens,
            });
        }
    });
}

#[tauri::command]
pub async fn llm_turn_cancel(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<Value, String> {
    state.llm.cancel(&request_id)?;
    Ok(json!({ "ok": true }))
}

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

#[tauri::command]
pub async fn llm_switch_model(
    state: State<'_, AppState>,
    conversation_id: String,
    model_ref: Value,
) -> Result<Value, String> {
    let model: ModelRef =
        serde_json::from_value(model_ref).map_err(|e| format!("ERR_LLM_MODEL: {e}"))?;
    state.llm.switch_model(&conversation_id, model.clone())?;
    Ok(json!({
        "conversation_id": conversation_id,
        "model": serde_json::to_value(model).map_err(|e| e.to_string())?,
    }))
}

#[tauri::command]
pub async fn llm_workspace_set(
    path: Option<String>,
    conversation_id: Option<String>,
) -> Result<serde_json::Value, String> {
    use omniget_core::core::llm::code_tools;
    let path = path.map(std::path::PathBuf::from);
    let set = match conversation_id.as_deref().filter(|c| !c.is_empty()) {
        Some(conv) => {
            omniget_core::core::assist::groups::set_context(
                &crate::llm_manager::sanitize_id(conv),
                path,
            )?
            .1
        }
        None => code_tools::set_workspace(path)?,
    };
    Ok(workspace_json(set, conversation_id.as_deref()))
}

#[tauri::command]
pub async fn llm_workspace_get(
    conversation_id: Option<String>,
) -> Result<serde_json::Value, String> {
    use omniget_core::core::llm::code_tools;
    let path = match conversation_id.as_deref().filter(|c| !c.is_empty()) {
        Some(conv) => code_tools::workspace_of(&crate::llm_manager::sanitize_id(conv)),
        None => code_tools::workspace(),
    };
    Ok(workspace_json(path, conversation_id.as_deref()))
}

fn workspace_json(
    path: Option<std::path::PathBuf>,
    conversation_id: Option<&str>,
) -> serde_json::Value {
    use omniget_core::core::llm::{code_tools, snapshot};
    let undo_depth = match (&path, conversation_id) {
        (Some(p), Some(c)) => snapshot::depth(p, &crate::llm_manager::sanitize_id(c)),
        _ => 0,
    };
    let undo_error = path.as_deref().and_then(snapshot::problem);
    serde_json::json!({
        "context": if path.is_some() { "project" } else { "projectless" },
        "undo_error": undo_error,
        "path": path.as_ref().map(|p| p.to_string_lossy().to_string()),
        "name": path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()),
        "sandbox": code_tools::sandbox_kind(),
        "plan": code_tools::current_plan(),
        "undo_depth": undo_depth,
    })
}

#[tauri::command]
pub async fn llm_turn_undo(
    state: State<'_, AppState>,
    conversation_id: String,
    conversation_only: Option<bool>,
) -> Result<serde_json::Value, String> {
    use omniget_core::core::llm::types::ContentPart;
    use omniget_core::core::llm::{code_tools, coordinator, snapshot};
    let conv = crate::llm_manager::sanitize_id(&conversation_id);
    let ws = code_tools::workspace_of(&conv)
        .ok_or_else(|| format!("{}: no workspace", code_tools::ERR_NO_WORKSPACE))?;
    if let Some(problem) = snapshot::problem(&ws)
        .filter(|_| !snapshot::git_available() && conversation_only != Some(true))
    {
        return Err(problem);
    }
    let undone = if conversation_only.unwrap_or(false) {
        snapshot::forget_last(&ws, &conv).await?
    } else {
        let live = crate::jobs::try_get()
            .map(|j| j.live_folders(&conversation_id))
            .unwrap_or_default();
        snapshot::undo(&ws, &conv, &live).await?
    };
    let removed = match (undone.messages_before, state.llm.coordinator().dir()) {
        (Some(keep), Some(dir)) => {
            coordinator::truncate_conversation(dir, &conv, keep).map_err(|e| e.to_string())?
        }
        _ => Vec::new(),
    };
    let shell: Vec<String> = removed
        .iter()
        .flat_map(|m| m.parts.iter())
        .filter_map(|p| match p {
            ContentPart::ToolUse { name, input, .. } if name == "shell_exec" => input
                .get("command")
                .and_then(|c| c.as_str())
                .map(str::to_string),
            _ => None,
        })
        .collect();
    let user_messages_kept = state
        .llm
        .coordinator()
        .history(&conv)
        .map(|h| {
            h.iter()
                .filter(|m| m.role == omniget_core::core::llm::types::Role::User)
                .count()
        })
        .unwrap_or(0);
    Ok(serde_json::json!({
        "user_messages_kept": user_messages_kept,
        "files": undone.files,
        "messages_removed": removed.len(),
        "conversation_restored": undone.messages_before.is_some(),
        "shell_commands": shell,
        "undo_depth": snapshot::depth(&ws, &conv),
    }))
}

#[tauri::command]
pub async fn llm_permission_rules_get(agent_id: String) -> Result<serde_json::Value, String> {
    serde_json::to_value(omniget_core::core::llm::perm::rules_of(&agent_id))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn llm_permission_rules_set(
    agent_id: String,
    rules: Vec<omniget_core::core::llm::perm::Rule>,
) -> Result<serde_json::Value, String> {
    omniget_core::core::llm::perm::set_rules(&agent_id, rules);
    Ok(serde_json::json!({ "ok": true }))
}

#[tauri::command]
pub async fn llm_conversation_run_mode(
    conversation_id: String,
) -> Result<serde_json::Value, String> {
    use omniget_core::core::llm::run_mode;
    let conv = crate::llm_manager::sanitize_id(&conversation_id);
    let chosen = run_mode::chosen(&conv);
    Ok(serde_json::json!({
        "run_mode": chosen.unwrap_or(run_mode::RunMode::AutoEdits),
        "chosen": chosen.is_some(),
        "modes": run_mode::ALL,
    }))
}

#[tauri::command]
pub async fn llm_conversation_run_mode_set(
    conversation_id: String,
    run_mode: Option<String>,
) -> Result<serde_json::Value, String> {
    use omniget_core::core::llm::run_mode::{self, RunMode};
    let conv = crate::llm_manager::sanitize_id(&conversation_id);
    let mode = run_mode
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(RunMode::parse)
        .transpose()?;
    run_mode::choose(&conv, mode);
    Ok(serde_json::json!({ "run_mode": mode.unwrap_or(RunMode::AutoEdits) }))
}
