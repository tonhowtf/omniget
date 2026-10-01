use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

use omniget_core::core::assist::runs::{
    self, PermissionRequest, Registry, RunEvent, RunRecord, RunResolution, RunState,
};

use crate::AppState;

pub const EVENT_RUN: &str = "assist://run";
pub const ERR_ASSIST_RUNS: &str = "ERR_ASSIST_RUNS";

pub fn install(app: &AppHandle) {
    runs::enable();
    let emitter = app.clone();
    runs::set_sink(std::sync::Arc::new(move |update| {
        let _ = emitter.emit(EVENT_RUN, update);
    }));
    match runs::active().map(|r| r.reconcile()) {
        Some(Ok(out)) => {
            if !out.unknown.is_empty() || !out.interrupted.is_empty() {
                tracing::info!(
                    "[runs] after restart: {} unknown, {} interrupted (waiting for a person)",
                    out.unknown.len(),
                    out.interrupted.len()
                );
            }
        }
        Some(Err(e)) => tracing::warn!("[runs] reconcile: {e}"),
        None => tracing::warn!("[runs] no assistant database; runs are not recorded"),
    }
}

fn registry() -> Result<Registry, String> {
    runs::active().ok_or_else(|| format!("{ERR_ASSIST_RUNS}: the run record is not available"))
}

#[derive(Debug, Serialize)]
pub struct RunView {
    #[serde(flatten)]
    pub run: RunRecord,
    pub pending_permissions: usize,
    pub live: bool,
    pub actions: Vec<&'static str>,
    pub tool_calls: usize,
}

fn actions_of(run: &RunRecord, live: bool) -> Vec<&'static str> {
    match run.state {
        RunState::Interrupted | RunState::Unknown if run.resolution.is_none() => {
            vec!["continue", "mark_done", "discard"]
        }
        s if s.is_live() && live => vec!["cancel"],
        _ => Vec::new(),
    }
}

fn view(reg: &Registry, run: RunRecord) -> RunView {
    let pending = reg.permissions(Some(&run.id), true).len();
    let live = run.instance_id == reg.instance() && run.state.is_live();
    let tool_calls = reg
        .events(&run.id)
        .iter()
        .filter(|e| e.kind == "tool_call")
        .count();
    RunView {
        actions: actions_of(&run, live),
        pending_permissions: pending,
        live,
        tool_calls,
        run,
    }
}

#[tauri::command]
pub async fn assist_runs_list(
    conversation_id: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<RunView>, String> {
    let reg = registry()?;
    Ok(reg
        .list_runs(conversation_id.as_deref(), limit.unwrap_or(100))
        .into_iter()
        .map(|r| view(&reg, r))
        .collect())
}

#[derive(Debug, Serialize)]
pub struct CommandRun {
    pub command: String,
    pub ok: Option<bool>,
    pub output: String,
    pub is_test: bool,
}

const SHELL_TOOLS: &[&str] = &["shell_exec", "Bash", "bash", "execute", "terminal"];
const WRITE_TOOLS: &[&str] = &[
    "fs_write",
    "fs_edit",
    "fs_apply_patch",
    "Write",
    "Edit",
    "MultiEdit",
    "NotebookEdit",
];

pub fn looks_like_test(cmd: &str) -> bool {
    let c = cmd.to_ascii_lowercase();
    [
        "test",
        "pytest",
        "vitest",
        "jest",
        "spec",
        "cargo check",
        "tsc",
    ]
    .iter()
    .any(|w| c.contains(w))
}

fn commands_of(events: &[RunEvent]) -> Vec<CommandRun> {
    events
        .iter()
        .filter(|e| e.kind == "tool_call")
        .filter(|e| {
            let name = e.payload["name"].as_str().unwrap_or("");
            SHELL_TOOLS.contains(&name) || e.payload["input"]["command"].is_string()
        })
        .filter_map(|e| {
            let command = e.payload["input"]["command"].as_str()?.to_string();
            Some(CommandRun {
                is_test: looks_like_test(&command),
                ok: e.payload["ok"].as_bool(),
                output: e.payload["output"].as_str().unwrap_or("").to_string(),
                command,
            })
        })
        .collect()
}

fn files_of(events: &[RunEvent]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for e in events.iter().filter(|e| e.kind == "tool_call") {
        let name = e.payload["name"].as_str().unwrap_or("");
        if !WRITE_TOOLS.contains(&name) {
            continue;
        }
        let input = &e.payload["input"];
        if let Some(p) = input["path"].as_str().or(input["file_path"].as_str()) {
            if !out.iter().any(|x| x == p) {
                out.push(p.to_string());
            }
        }
    }
    out
}

#[derive(Debug, Serialize)]
pub struct RunDetail {
    #[serde(flatten)]
    pub view: RunView,
    pub events: Vec<RunEvent>,
    pub permissions: Vec<PermissionRequest>,
    pub commands: Vec<CommandRun>,
    pub files_touched: Vec<String>,
    pub session: Option<runs::RuntimeSession>,
}

#[tauri::command]
pub async fn assist_run_get(run_id: String) -> Result<RunDetail, String> {
    let reg = registry()?;
    let run = reg
        .run(&run_id)
        .ok_or_else(|| format!("{ERR_ASSIST_RUNS}: no run {run_id}"))?;
    let events = reg.events(&run_id);
    let session = reg.session(&run.conversation_id, &run.bot_id);
    Ok(RunDetail {
        commands: commands_of(&events),
        files_touched: files_of(&events),
        permissions: reg.permissions(Some(&run_id), false),
        session,
        view: view(&reg, run),
        events,
    })
}

#[tauri::command]
pub async fn assist_run_diff(run_id: String) -> Result<Value, String> {
    let reg = registry()?;
    let run = reg
        .run(&run_id)
        .ok_or_else(|| format!("{ERR_ASSIST_RUNS}: no run {run_id}"))?;
    let workspace = run
        .cwd
        .clone()
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_dir())
        .or_else(|| omniget_core::core::llm::code_tools::workspace_of(&run.conversation_id));
    let Some(ws) = workspace else {
        return Ok(json!({ "available": false, "reason": "no_workspace" }));
    };
    match omniget_core::core::llm::snapshot::turn_diff(&ws, &run.conversation_id, &run_id).await {
        Ok(Some(diff)) => Ok(json!({ "available": true, "workspace": ws, "diff": diff })),
        Ok(None) => Ok(json!({ "available": false, "reason": "no_changes", "workspace": ws })),
        Err(e) => Ok(json!({ "available": false, "reason": "error", "error": e })),
    }
}

#[tauri::command]
pub async fn assist_run_cancel(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Value, String> {
    let reg = registry()?;
    let run = reg
        .run(&run_id)
        .ok_or_else(|| format!("{ERR_ASSIST_RUNS}: no run {run_id}"))?;
    if !run.state.is_live() {
        return Err(format!(
            "{}: the run is {}",
            runs::ERR_RUNS_STATE,
            run.state.as_str()
        ));
    }
    state.llm.broker().cancel_request(&run_id);
    match state.llm.cancel(&run_id) {
        Ok(()) => Ok(json!({ "ok": true })),
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn assist_run_resolve(
    app: AppHandle,
    run_id: String,
    action: String,
) -> Result<Value, String> {
    let reg = registry()?;
    let run = reg
        .run(&run_id)
        .ok_or_else(|| format!("{ERR_ASSIST_RUNS}: no run {run_id}"))?;
    match action.as_str() {
        "mark_done" => Ok(json!(reg.resolve_run(&run_id, RunResolution::MarkedDone)?)),
        "discard" => Ok(json!(reg.resolve_run(&run_id, RunResolution::Discarded)?)),
        "continue" => {
            if !matches!(run.state, RunState::Interrupted | RunState::Unknown) {
                return Err(format!(
                    "{}: only an interrupted run can be continued",
                    runs::ERR_RUNS_STATE
                ));
            }
            let note = match run.state {
                RunState::Unknown => "The app closed while your previous turn was running; its result is unknown. Before doing anything, check what was already done (files, commands) and do not repeat an action that already happened. Then continue the task:",
                _ => "The app closed before your previous turn started. Continue the task:",
            };
            let prompt = format!("{note}\n\n{}", run.input_preview);
            let jobs = crate::jobs::get(&app)?;
            let job = jobs.submit(
                "run",
                &run.bot_id,
                &prompt,
                None,
                Some(run.conversation_id.clone()),
                None,
            )?;
            let resolved = reg.resolve_run(
                &run_id,
                RunResolution::ContinuedBy(format!("job:{}", job.id)),
            )?;
            Ok(json!({ "run": resolved, "job": job }))
        }
        other => Err(format!("{ERR_ASSIST_RUNS}: unknown action `{other}`")),
    }
}
