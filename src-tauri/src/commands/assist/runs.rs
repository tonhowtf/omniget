//! Commands for `assist::runs`: the Activity screen (runs, their state, the
//! tools they really ran, the diff they left, pending permissions) and the
//! actions a person takes on an interrupted run. Owner: worker W4.
//!
//! Also [`install`], called once at boot (from `jobs::boot`): it turns the
//! durable registry on, sends `assist://run` to the window after every
//! committed state change, and reconciles what a previous app session left
//! in flight — marking it, never re-sending it.

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, State};

use omniget_core::core::assist::runs::{
    self, PermissionRequest, Registry, RunEvent, RunRecord, RunResolution, RunState,
};
use omniget_core::core::llm::broker::{Answer, AnswerRefused};

use crate::AppState;

/// Tauri event with the `RunUpdate` payload (briefing contract 5).
pub const EVENT_RUN: &str = "assist://run";
pub const ERR_ASSIST_RUNS: &str = "ERR_ASSIST_RUNS";

/// Boot wiring. Idempotent.
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

/// One row of the Activity list.
#[derive(Debug, Serialize)]
pub struct RunView {
    #[serde(flatten)]
    pub run: RunRecord,
    pub pending_permissions: usize,
    /// Driven by this app session right now.
    pub live: bool,
    /// What a person may do: `cancel`, `continue`, `mark_done`, `discard`.
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

/// A command the run executed, read from its tool calls.
#[derive(Debug, Serialize)]
pub struct CommandRun {
    pub command: String,
    pub ok: Option<bool>,
    pub output: String,
    /// Heuristic from the command line (`test`, `pytest`, `vitest`, …).
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
    /// Files the tools said they wrote (the diff is the proof, see
    /// `assist_run_diff`).
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

/// The real diff of the run, from the shadow snapshot of its folder:
/// the tree before its first write against the tree when it ended.
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
pub async fn assist_permissions_pending() -> Result<Vec<PermissionRequest>, String> {
    let reg = registry()?;
    let now = omniget_core::core::assist::now_ms();
    // A request past its deadline was already denied by the broker: mark it
    // expired instead of offering buttons that can only fail.
    let mut out = Vec::new();
    for p in reg.permissions(None, true) {
        if p.deadline_ms <= now {
            let _ = reg.expire_permission(&p.id);
        } else {
            out.push(p);
        }
    }
    Ok(out)
}

/// Answers one durable permission request. `answer` ∈ `once|always|deny`.
/// A late answer is refused as expired and releases nothing.
#[tauri::command]
pub async fn assist_permission_answer(
    app: AppHandle,
    state: State<'_, AppState>,
    permission_id: String,
    answer: String,
) -> Result<Value, String> {
    let a = match answer.as_str() {
        "once" => Answer::Once,
        "always" => Answer::Always,
        "deny" => Answer::Deny,
        other => return Err(format!("{ERR_ASSIST_RUNS}: unknown answer `{other}`")),
    };
    let perm = registry()?.permission(&permission_id);
    match state.llm.broker().answer_permission(&permission_id, a) {
        Ok(()) => {
            if let Some(p) = perm {
                let _ = app.emit(
                    "omni://tool-resolved",
                    json!({ "request_id": p.run_id, "tool_call_id": p.tool_call_id, "allow": !matches!(a, Answer::Deny) }),
                );
            }
            Ok(json!({ "ok": true }))
        }
        Err(AnswerRefused::Expired) => Err(format!(
            "{}: this request expired before the answer",
            runs::ERR_PERMISSION_EXPIRED
        )),
        Err(AnswerRefused::Resolved) => Err(format!(
            "{}: this request was already answered or cancelled",
            runs::ERR_PERMISSION_RESOLVED
        )),
        Err(AnswerRefused::Unknown) => Err(format!(
            "{ERR_ASSIST_RUNS}: nothing is waiting for this answer"
        )),
    }
}

/// Stops a run this app session drives: its questions resolve as
/// cancelled, its process tree stops, its reservation is released.
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
        // Not driven by this session (it would be `unknown` after a restart).
        Err(e) => Err(e),
    }
}

/// What a person decides about an interrupted or unknown run:
/// - `mark_done`: it finished (the person checked); nothing is sent;
/// - `discard`: forget it; nothing is sent;
/// - `continue`: a NEW turn in the same conversation, telling the agent the
///   previous one was interrupted so it checks before acting again.
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

/// Real capabilities of a runtime (probes the installed executable).
/// `runtime` is the roster's `RuntimeKind` JSON.
#[tauri::command]
pub async fn assist_runtime_caps(runtime: Value) -> Result<Value, String> {
    let kind: omniget_core::core::llm::agent::RuntimeKind =
        serde_json::from_value(runtime).map_err(|e| format!("{ERR_ASSIST_RUNS}: {e}"))?;
    let caps = omniget_core::core::llm::caps::probe(&kind).await;
    serde_json::to_value(caps).map_err(|e| e.to_string())
}

/// Budget pools with reservations in flight (Activity → advanced details).
#[tauri::command]
pub async fn assist_budget_in_flight(state: State<'_, AppState>) -> Result<Value, String> {
    let budget = state.llm.budget();
    let flights = budget.in_flight();
    let mut pools: Vec<String> = flights.iter().map(|f| f.pool.clone()).collect();
    pools.sort();
    pools.dedup();
    let views: Vec<Value> = pools
        .iter()
        .map(|p| json!({ "pool": p, "view": budget.pool(p) }))
        .collect();
    Ok(json!({ "in_flight": flights, "pools": views }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_commands_are_recognised() {
        assert!(looks_like_test("cargo test -p x"));
        assert!(looks_like_test("pnpm vitest run a.test.ts"));
        assert!(looks_like_test("python -m pytest"));
        assert!(!looks_like_test("ls -la"));
    }

    #[test]
    fn commands_and_files_come_from_tool_call_events() {
        let ev = |name: &str, input: Value| RunEvent {
            id: "e".into(),
            run_id: "r".into(),
            seq: 1,
            kind: "tool_call".into(),
            provider_id: None,
            payload: json!({ "name": name, "input": input, "ok": true, "output": "1 passed" }),
            ts_ms: 0,
        };
        let events = vec![
            ev("Bash", json!({ "command": "cargo test" })),
            ev("Edit", json!({ "file_path": "src/a.rs" })),
            ev("fs_write", json!({ "path": "b.txt" })),
            ev("Read", json!({ "file_path": "c.rs" })),
        ];
        let cmds = commands_of(&events);
        assert_eq!(cmds.len(), 1);
        assert!(cmds[0].is_test);
        assert_eq!(
            files_of(&events),
            vec!["src/a.rs".to_string(), "b.txt".to_string()]
        );
    }

    #[test]
    fn interrupted_runs_offer_explicit_actions_only() {
        let run = |state: RunState| RunRecord {
            id: "r".into(),
            session_id: None,
            conversation_id: "c".into(),
            bot_id: "b".into(),
            parent_run_id: None,
            state,
            resume_kind: None,
            runtime: None,
            cwd: None,
            instance_id: "x".into(),
            launch_id: None,
            pid: None,
            input_preview: String::new(),
            summary: None,
            error: None,
            resolution: None,
            tokens_in: 0,
            tokens_out: 0,
            cost_usd: None,
            events_dropped: 0,
            created_ms: 0,
            started_ms: None,
            ended_ms: None,
            updated_ms: 0,
        };
        assert_eq!(
            actions_of(&run(RunState::Unknown), false),
            vec!["continue", "mark_done", "discard"]
        );
        assert_eq!(actions_of(&run(RunState::Running), true), vec!["cancel"]);
        assert!(actions_of(&run(RunState::Running), false).is_empty());
        assert!(actions_of(&run(RunState::Completed), false).is_empty());
    }
}
