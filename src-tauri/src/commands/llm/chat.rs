//! `llm_*` commands: conversations and turns. The turn runs on a background
//! task; the UI hears it through `llm://turn`. Owned by f2-llm-commands.

use std::time::Instant;

use futures::StreamExt;
use omniget_core::core::llm::types::ModelRef;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

use super::{emit_turn, ensure_wired, spawn_telemetry_ticker, DeltaCoalescer, DELTA_HZ};
use crate::AppState;

/// Starts a turn and returns the `request_id` the Coordinator minted for it —
/// the same one that comes back in `TurnEvent::Started`, in every
/// `llm://tool-ask` of the turn and in `llm_tool_answer`. Every event arrives
/// on `llm://turn` as `{ request_id, event }`, with `TextDelta` coalesced to
/// 30 Hz.
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
    let (request_id, _cancel, mut stream) = manager
        .turn_stream(&conversation_id, &agent_id, &input)
        .await?;

    spawn_telemetry_ticker(app.clone());

    let id = request_id.clone();
    let agent = agent_id.clone();
    // Mirrored as a job so `/llm/jobs` lists everything the agents did.
    let jobs = crate::jobs::get(&app).ok();
    let job_id = jobs
        .as_ref()
        .map(|j| j.chat_started(&agent_id, &conversation_id, &input, &request_id));
    // The Coordinator persists the conversation itself; this task only
    // re-emits what it streams.
    tauri::async_runtime::spawn(async move {
        let mut coalescer = DeltaCoalescer::new(DELTA_HZ);
        // Every event of the turn is forwarded as it comes, `Started`
        // included: its `request_id` is the one returned above and the one
        // `llm://tool-ask` carries, so the front can filter by equality.
        let mut said = String::new();
        let mut failed: Option<String> = None;
        let mut cancelled = false;
        let mut usage = Vec::new();
        let mut receipts = Vec::new();
        while let Some(event) = stream.next().await {
            manager.note_event(&agent, &event);
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
                emit_turn(&app, &id, &out);
            }
        }
        if let Some(rest) = coalescer.flush() {
            emit_turn(&app, &id, &rest);
        }
        manager.finish_turn(&id, &agent);
        if let (Some(jobs), Some(job_id)) = (jobs, job_id) {
            jobs.chat_finished(&job_id, &said, failed, cancelled, &usage, &receipts);
        }
    });

    Ok(json!({ "request_id": request_id }))
}

#[tauri::command]
pub async fn llm_turn_cancel(
    state: State<'_, AppState>,
    request_id: String,
) -> Result<Value, String> {
    state.llm.cancel(&request_id)?;
    Ok(json!({ "ok": true }))
}

/// Answers a `GrantMode::Ask` question raised by the tool broker.
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
    // Whoever else was showing the question (the pet, the /llm tab) drops it.
    let _ = app.emit(
        "omni://tool-resolved",
        json!({ "request_id": request_id, "tool_call_id": tool_call_id, "allow": allow }),
    );
    Ok(json!({ "ok": true }))
}

/// Switches the model mid-conversation. The next turn uses it.
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

/// The folder the coding tools are confined to. With a `conversation_id` the
/// folder belongs to that conversation; without one it is the process-wide
/// fallback (what the embedded MCP server and new conversations use). `null`
/// detaches it.
#[tauri::command]
pub async fn llm_workspace_set(
    path: Option<String>,
    conversation_id: Option<String>,
) -> Result<serde_json::Value, String> {
    use omniget_core::core::llm::code_tools;
    let path = path.map(std::path::PathBuf::from);
    let set = match conversation_id.as_deref().filter(|c| !c.is_empty()) {
        Some(conv) => {
            // The last folder picked is also the default of the next conversation.
            if path.is_some() {
                let _ = code_tools::set_workspace(path.clone());
            }
            code_tools::set_conversation_workspace(&crate::llm_manager::sanitize_id(conv), path)?
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
        "undo_error": undo_error,
        "path": path.as_ref().map(|p| p.to_string_lossy().to_string()),
        "name": path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()),
        "sandbox": code_tools::sandbox_kind(),
        "plan": code_tools::current_plan(),
        "undo_depth": undo_depth,
    })
}

/// Puts the workspace back to how it was before the last turn of this
/// conversation that wrote files.
#[tauri::command]
pub async fn llm_turn_undo(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<serde_json::Value, String> {
    use omniget_core::core::llm::types::ContentPart;
    use omniget_core::core::llm::{code_tools, coordinator, snapshot};
    let conv = crate::llm_manager::sanitize_id(&conversation_id);
    let ws = code_tools::workspace_of(&conv)
        .ok_or_else(|| format!("{}: no workspace", code_tools::ERR_NO_WORKSPACE))?;
    if let Some(problem) = snapshot::problem(&ws).filter(|_| !snapshot::git_available()) {
        return Err(problem);
    }
    let undone = snapshot::undo(&ws, &conv).await?;
    // The bubbles of that turn go with the files. Nothing is run again: the
    // records are cut from the conversation, the tools are not replayed.
    let removed = match (undone.messages_before, state.llm.coordinator().dir()) {
        (Some(keep), Some(dir)) => {
            coordinator::truncate_conversation(dir, &conv, keep).map_err(|e| e.to_string())?
        }
        _ => Vec::new(),
    };
    // A shell command may have written outside the folder (no sandbox on
    // Windows and Linux, temp dirs everywhere). That is not in the snapshot,
    // and the UI says so instead of pretending the turn never happened.
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
