use omniget_core::core::threads::external::{self, ExternalImport};
use omniget_core::core::threads::git::{CommitMessage, RevertOutcome};
use omniget_core::core::threads::model::{CommandEnvelope, DomainEvent};
use omniget_core::core::threads::store::{self, CheckpointRow, UsageRow};
use omniget_core::core::threads::{DispatchResult, EventsPage, Snapshot, TurnsPage};
use omniget_core::core::vcs::actions::{
    CommitResult, PrCreateResult, PrInfo, PrRequest, PushResult,
};
use omniget_core::core::vcs::diff::{DiffOptions, DiffResult};
use omniget_core::core::vcs::repo::RepoStatus;
use serde_json::Value;
use tauri::AppHandle;

use crate::pty::{self, OpenRequest, SessionInfo};
use crate::threads_host::limits::InstanceLimitsView;
use crate::threads_host::{self, InstanceView};

pub const FIRST_PAGE: u32 = 10;

#[tauri::command]
pub async fn threads_snapshot(app: AppHandle) -> Result<Snapshot, String> {
    threads_host::get(&app)?.engine.snapshot().await
}

#[tauri::command]
pub async fn threads_events_after(
    app: AppHandle,
    after_sequence: i64,
    limit: Option<i64>,
) -> Result<EventsPage, String> {
    threads_host::get(&app)?
        .engine
        .events_after(after_sequence, limit.unwrap_or(1000))
        .await
}

#[tauri::command]
pub async fn threads_dispatch(
    app: AppHandle,
    command_json: Value,
) -> Result<DispatchResult, String> {
    let value = match command_json {
        Value::String(s) => {
            serde_json::from_str(&s).map_err(|e| format!("ERR_THREADS_INVALID: {e}"))?
        }
        v => v,
    };
    let env: CommandEnvelope =
        serde_json::from_value(value).map_err(|e| format!("ERR_THREADS_INVALID: {e}"))?;
    if matches!(
        env.command,
        omniget_core::core::threads::Command::RuntimeAppend { .. }
            | omniget_core::core::threads::Command::HistoryImport { .. }
            | omniget_core::core::threads::Command::HostRecord { .. }
    ) {
        return Err("ERR_THREADS_INVALID: internal command".into());
    }
    threads_host::get(&app)?.engine.dispatch(env).await
}

#[tauri::command]
pub async fn threads_turns_page(
    app: AppHandle,
    thread_id: String,
    before_turn: Option<u32>,
    limit: Option<u32>,
) -> Result<TurnsPage, String> {
    threads_host::get(&app)?
        .engine
        .turns_page(thread_id, before_turn, limit.unwrap_or(FIRST_PAGE))
        .await
}

#[tauri::command]
pub async fn threads_drivers(app: AppHandle) -> Result<Vec<InstanceView>, String> {
    Ok(threads_host::get(&app)?.instances())
}

#[tauri::command]
pub async fn threads_checkpoints(
    app: AppHandle,
    thread_id: String,
) -> Result<Vec<CheckpointRow>, String> {
    let host = threads_host::get(&app)?;
    let engine = host.engine.clone();
    tauri::async_runtime::spawn_blocking(move || engine.read(|c| store::checkpoints(c, &thread_id)))
        .await
        .map_err(|e| format!("ERR_THREADS_HOST: {e}"))?
}

#[tauri::command]
pub async fn threads_diff(
    app: AppHandle,
    thread_id: String,
    turn: Option<u32>,
    scope: Option<String>,
    options: Option<DiffOptions>,
) -> Result<DiffResult, String> {
    let turn = if scope.as_deref() == Some("total") {
        None
    } else {
        turn
    };
    threads_host::get(&app)?
        .git
        .diff(&thread_id, turn, &options.unwrap_or_default())
        .await
}

#[tauri::command]
pub async fn threads_revert_to_turn(
    app: AppHandle,
    thread_id: String,
    turn: Option<u32>,
    turn_count: Option<u32>,
    restore_files: Option<bool>,
) -> Result<RevertOutcome, String> {
    let turn = turn
        .or(turn_count)
        .ok_or_else(|| "ERR_THREADS_INVALID: missing turn".to_string())?;
    threads_host::get(&app)?
        .git
        .revert_to_turn(&thread_id, turn, restore_files.unwrap_or(false))
        .await
}

#[tauri::command]
pub async fn threads_worktree_cancel(app: AppHandle, thread_id: String) -> Result<bool, String> {
    Ok(threads_host::get(&app)?.git.cancel_setup(&thread_id))
}

#[tauri::command]
pub async fn threads_git_status(app: AppHandle, thread_id: String) -> Result<RepoStatus, String> {
    threads_host::get(&app)?.git.status(&thread_id).await
}

#[tauri::command]
pub async fn threads_git_commit_message(
    app: AppHandle,
    thread_id: String,
) -> Result<CommitMessage, String> {
    threads_host::get(&app)?
        .git
        .commit_message(&thread_id)
        .await
}

#[tauri::command]
pub async fn threads_git_commit(
    app: AppHandle,
    thread_id: String,
    message: Option<String>,
) -> Result<CommitResult, String> {
    threads_host::get(&app)?
        .git
        .commit(&thread_id, message)
        .await
}

#[tauri::command]
pub async fn threads_git_push(app: AppHandle, thread_id: String) -> Result<PushResult, String> {
    threads_host::get(&app)?.git.push(&thread_id).await
}

#[tauri::command]
pub async fn threads_git_pr_create(
    app: AppHandle,
    thread_id: String,
    title: Option<String>,
    body: Option<String>,
    base: Option<String>,
    draft: Option<bool>,
) -> Result<PrCreateResult, String> {
    threads_host::get(&app)?
        .git
        .open_pr(&thread_id, title, body, base, draft.unwrap_or(true))
        .await
}

#[tauri::command]
pub async fn threads_git_pr(
    app: AppHandle,
    thread_id: String,
    request: PrRequest,
) -> Result<PrCreateResult, String> {
    threads_host::get(&app)?
        .git
        .open_pr(
            &thread_id,
            Some(request.title),
            Some(request.body).filter(|b| !b.trim().is_empty()),
            request.base,
            request.draft,
        )
        .await
}

#[tauri::command]
pub async fn threads_files_search(
    app: AppHandle,
    thread_id: String,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<String>, String> {
    threads_host::get(&app)?
        .git
        .files(
            &thread_id,
            query.as_deref().unwrap_or(""),
            limit.unwrap_or(50),
        )
        .await
}

#[tauri::command]
pub async fn threads_git_pr_refresh(
    app: AppHandle,
    thread_id: String,
) -> Result<Option<PrInfo>, String> {
    threads_host::get(&app)?.git.refresh_pr(&thread_id).await
}

fn terminal_id_for(thread_id: &str, n: usize) -> String {
    let safe: String = thread_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(100)
        .collect();
    format!("thr-{safe}-{n}")
}

#[tauri::command]
pub async fn threads_terminal_open(
    app: AppHandle,
    thread_id: String,
    terminal_id: Option<String>,
    new_tab: Option<bool>,
    cols: Option<u16>,
    rows: Option<u16>,
) -> Result<SessionInfo, String> {
    let host = threads_host::get(&app)?;
    let row = host
        .engine
        .read(|c| store::thread_row(c, &thread_id))?
        .ok_or_else(|| format!("ERR_THREADS_NOT_FOUND: no thread {thread_id}"))?;
    let cwd = host.git.cwd(&thread_id)?;
    let id = match terminal_id.filter(|t| !t.trim().is_empty()) {
        Some(t) => t,
        None if !new_tab.unwrap_or(false) && !row.terminals.is_empty() => row.terminals[0].clone(),
        None => {
            let mut n = row.terminals.len() + 1;
            while row.terminals.contains(&terminal_id_for(&thread_id, n)) {
                n += 1;
            }
            terminal_id_for(&thread_id, n)
        }
    };
    let m = pty::manager();
    m.bind(&app);
    let req = OpenRequest {
        id: Some(id.clone()),
        cwd: Some(cwd.to_string_lossy().into_owned()),
        cols,
        rows,
        title: Some(row.title.clone()),
        ..Default::default()
    };
    let info = tauri::async_runtime::spawn_blocking(move || m.open(req))
        .await
        .map_err(|e| format!("PTY_OPEN: {e}"))??;
    if !row.terminals.contains(&info.id) {
        host.git
            .record(DomainEvent::TerminalAttached {
                thread_id: thread_id.clone(),
                terminal_id: info.id.clone(),
                cwd: Some(cwd.to_string_lossy().into_owned()),
                attached_at: omniget_core::core::llm::drivers::now_iso(),
            })
            .await?;
    }
    Ok(info)
}

#[tauri::command]
pub async fn threads_terminal_close(
    app: AppHandle,
    thread_id: String,
    terminal_id: String,
) -> Result<(), String> {
    let host = threads_host::get(&app)?;
    let _ = pty::manager().close(&terminal_id, true);
    host.git
        .record(DomainEvent::TerminalClosed {
            thread_id,
            terminal_id,
        })
        .await
}

#[tauri::command]
pub async fn threads_usage(app: AppHandle, thread_id: String) -> Result<Vec<UsageRow>, String> {
    let host = threads_host::get(&app)?;
    let engine = host.engine.clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine.read(|c| store::thread_usage(c, &thread_id))
    })
    .await
    .map_err(|e| format!("ERR_THREADS_HOST: {e}"))?
}

#[tauri::command]
pub async fn threads_limits(app: AppHandle) -> Result<Vec<InstanceLimitsView>, String> {
    let host = threads_host::get(&app)?;
    Ok(threads_host::limits::view(&host, &app))
}

#[tauri::command]
pub async fn threads_import_external(
    app: AppHandle,
    tool: String,
    session_id: String,
) -> Result<ExternalImport, String> {
    let host = threads_host::get(&app)?;
    external::import_external(&host.engine, &tool, &session_id).await
}

#[tauri::command]
pub async fn threads_resume_external(
    app: AppHandle,
    thread_id: String,
    instance_id: Option<String>,
    driver: Option<String>,
) -> Result<String, String> {
    let host = threads_host::get(&app)?;
    external::resume_external(&host.engine, &thread_id, instance_id, driver).await
}
