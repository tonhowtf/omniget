//! `llm_*` commands: conversations and turns. The turn runs on a background
//! task; the UI hears it through `llm://turn`. Owned by f2-llm-commands.

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
pub async fn llm_conversation_delete(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Value, String> {
    state.llm.conversation_delete(&conversation_id)?;
    Ok(json!({ "ok": true }))
}

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

/// How a forwarded turn ended, for callers that record it elsewhere (rooms).
#[derive(Debug, Clone, Default)]
pub struct TurnSummary {
    pub text: String,
    pub error: Option<String>,
    pub cancelled: bool,
    /// Input + output tokens reported by the provider; `None` when it
    /// reported nothing (unknown is not zero).
    pub tokens: Option<i64>,
}

pub type TurnDone = Box<dyn FnOnce(TurnSummary) + Send + 'static>;

/// Re-emits one running turn on `llm://turn` (text coalesced to 30 Hz),
/// mirrors it as a job and calls `on_done` with how it ended. Shared by the
/// direct chat and by the members of a group room.
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
        // included: its `request_id` is the one returned to the caller and
        // the one `llm://tool-ask` carries, so the front filters by equality.
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

/// The folder the coding tools are confined to. With a `conversation_id`
/// the folder belongs to that conversation only (it becomes a Project
/// conversation; `null` makes it personal again) and nothing else changes:
/// not another conversation, not the process-wide folder (O18/O19). Without
/// one it is the process-wide folder, which only callers outside a turn use
/// (the embedded MCP server, the `omniget` CLI).
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
        // `projectless` (personal) or `project`: what the chat header shows.
        "context": if path.is_some() { "project" } else { "projectless" },
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

#[cfg(test)]
mod workspace_command_tests {
    use omniget_core::core::llm::code_tools;

    /// O18/O19: picking a folder in one conversation changes that
    /// conversation only — never the process-wide folder, never another chat.
    #[tokio::test]
    async fn picking_a_folder_in_a_conversation_does_not_set_the_global_one() {
        let dir = std::env::temp_dir().join(format!("omniget-ws-cmd-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let before = code_tools::workspace();
        let out = super::llm_workspace_set(
            Some(dir.to_string_lossy().to_string()),
            Some("ws-cmd-project".into()),
        )
        .await
        .unwrap();
        assert_eq!(out["context"], "project");
        assert_eq!(
            code_tools::workspace(),
            before,
            "the global folder is untouched"
        );
        let other = super::llm_workspace_get(Some("ws-cmd-personal".into()))
            .await
            .unwrap();
        assert_eq!(other["context"], "projectless");
        assert!(other["path"].is_null());
    }
}
