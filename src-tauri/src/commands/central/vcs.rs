//! Comandos `vcs_*` da Central: status, branches e log; worktree por thread
//! (com progresso em `central://vcs/worktree-setup`); checkpoints por turno,
//! diffs e restauração; commit, push e PR. Erros no formato `"CODE: msg"`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use omniget_core::core::vcs::{
    actions::{
        self, ChangeSummary, CommitResult, CommitSuggestion, HostInfo, PrCreateResult, PrInfo,
        PrRequest, PushResult,
    },
    checkpoint::{self, Checkpoint, RestoreResult},
    diff::{DiffOptions, DiffResult},
    repo::{self, BranchInfo, LogEntry, RepoStatus},
    worktree::{self, ProjectConfig, RemoveResult, WorktreeEntry, WorktreeInfo, WorktreeRequest},
};
use tauri::{AppHandle, Emitter};

pub const WORKTREE_SETUP_EVENT: &str = "central://vcs/worktree-setup";

fn cancels() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static C: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn path_of(p: &str) -> Result<PathBuf, String> {
    let p = p.trim();
    if p.is_empty() {
        return Err("ERR_VCS_INVALID: empty path".into());
    }
    let pb = PathBuf::from(p);
    if !pb.is_absolute() {
        return Err(format!("ERR_VCS_INVALID: {p} is not an absolute path"));
    }
    Ok(pb)
}

// ---- repository ----

#[tauri::command]
pub async fn vcs_status(path: String) -> Result<RepoStatus, String> {
    Ok(repo::status(&path_of(&path)?).await?)
}

#[tauri::command]
pub async fn vcs_branches(path: String) -> Result<Vec<BranchInfo>, String> {
    Ok(repo::branches(&path_of(&path)?).await?)
}

#[tauri::command]
pub async fn vcs_log(
    path: String,
    n: Option<u32>,
    rev: Option<String>,
) -> Result<Vec<LogEntry>, String> {
    Ok(repo::log(&path_of(&path)?, n.unwrap_or(50), rev.as_deref()).await?)
}

#[tauri::command]
pub async fn vcs_project_config(path: String) -> Result<ProjectConfig, String> {
    let p = path_of(&path)?;
    let root = repo::discover(&p).await.map(|r| r.root).unwrap_or(p);
    Ok(worktree::read_project_config(&root))
}

// ---- worktrees ----

/// Creates (or reuses) the thread's worktree. Every stage change is emitted
/// on `central://vcs/worktree-setup` as a `SetupSnapshot`.
#[tauri::command]
pub async fn vcs_worktree_create(
    app: AppHandle,
    request: WorktreeRequest,
) -> Result<WorktreeInfo, String> {
    let flag = Arc::new(AtomicBool::new(false));
    {
        let mut map = cancels().lock().unwrap_or_else(|e| e.into_inner());
        if map.contains_key(&request.thread) {
            return Err("ERR_VCS_INVALID: this thread's worktree is already being set up".into());
        }
        map.insert(request.thread.clone(), flag.clone());
    }
    let thread = request.thread.clone();
    let out = worktree::create(&request, flag, |snap| {
        let _ = app.emit(WORKTREE_SETUP_EVENT, snap);
    })
    .await;
    cancels()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&thread);
    Ok(out?)
}

/// Asks a running setup to stop; it removes what it made. `false` when no
/// setup is running for the thread.
#[tauri::command]
pub async fn vcs_worktree_cancel(thread: String) -> Result<bool, String> {
    let map = cancels().lock().unwrap_or_else(|e| e.into_inner());
    Ok(match map.get(&thread) {
        Some(f) => {
            f.store(true, Ordering::SeqCst);
            true
        }
        None => false,
    })
}

#[tauri::command]
pub async fn vcs_worktree_rename_branch(path: String, name: String) -> Result<String, String> {
    Ok(worktree::rename_branch(&path_of(&path)?, &name).await?)
}

#[tauri::command]
pub async fn vcs_worktree_remove(
    path: String,
    force: Option<bool>,
    delete_branch: Option<bool>,
    thread: Option<String>,
) -> Result<RemoveResult, String> {
    Ok(worktree::remove(
        &path_of(&path)?,
        force.unwrap_or(false),
        delete_branch.unwrap_or(false),
        thread.as_deref(),
    )
    .await?)
}

#[tauri::command]
pub async fn vcs_worktree_list(path: String) -> Result<Vec<WorktreeEntry>, String> {
    Ok(worktree::list(&path_of(&path)?).await?)
}

#[tauri::command]
pub async fn vcs_worktree_cleanup(path: String) -> Result<Vec<PathBuf>, String> {
    Ok(worktree::cleanup(&path_of(&path)?).await?)
}

// ---- checkpoints and diffs ----

#[tauri::command]
pub async fn vcs_checkpoint_capture(
    path: String,
    thread: String,
    turn: u32,
) -> Result<Checkpoint, String> {
    Ok(checkpoint::capture(&path_of(&path)?, &thread, turn).await?)
}

#[tauri::command]
pub async fn vcs_checkpoint_list(path: String, thread: String) -> Result<Vec<Checkpoint>, String> {
    Ok(checkpoint::list(&path_of(&path)?, &thread).await?)
}

/// Deletes the checkpoints after `after_turn`, or all of them when omitted.
#[tauri::command]
pub async fn vcs_checkpoint_delete(
    path: String,
    thread: String,
    after_turn: Option<u32>,
) -> Result<u32, String> {
    let p = path_of(&path)?;
    Ok(match after_turn {
        Some(t) => checkpoint::delete_after(&p, &thread, t).await?,
        None => checkpoint::delete_all(&p, &thread).await?,
    })
}

#[tauri::command]
pub async fn vcs_diff_turn(
    path: String,
    thread: String,
    turn: u32,
    options: Option<DiffOptions>,
) -> Result<DiffResult, String> {
    Ok(checkpoint::diff_turn(
        &path_of(&path)?,
        &thread,
        turn,
        &options.unwrap_or_default(),
    )
    .await?)
}

/// Baseline → `to_turn`, or → the live work tree when `to_turn` is omitted.
#[tauri::command]
pub async fn vcs_diff_full(
    path: String,
    thread: String,
    to_turn: Option<u32>,
    options: Option<DiffOptions>,
) -> Result<DiffResult, String> {
    Ok(checkpoint::diff_full(
        &path_of(&path)?,
        &thread,
        to_turn,
        &options.unwrap_or_default(),
    )
    .await?)
}

#[tauri::command]
pub async fn vcs_diff_range(
    path: String,
    thread: String,
    from_turn: u32,
    to_turn: Option<u32>,
    options: Option<DiffOptions>,
) -> Result<DiffResult, String> {
    Ok(checkpoint::diff_range(
        &path_of(&path)?,
        &thread,
        from_turn,
        to_turn,
        &options.unwrap_or_default(),
    )
    .await?)
}

/// Puts the files back to checkpoint `turn`. Outside an OmniGet worktree it
/// fails with `ERR_VCS_UNSAFE` unless `confirm` is true.
#[tauri::command]
pub async fn vcs_restore_turn(
    path: String,
    thread: String,
    turn: u32,
    confirm: Option<bool>,
    drop_later: Option<bool>,
) -> Result<RestoreResult, String> {
    Ok(checkpoint::restore(
        &path_of(&path)?,
        &thread,
        turn,
        confirm.unwrap_or(false),
        drop_later.unwrap_or(false),
    )
    .await?)
}

// ---- actions ----

#[tauri::command]
pub async fn vcs_working_changes(path: String) -> Result<Vec<ChangeSummary>, String> {
    Ok(actions::working_changes(&path_of(&path)?).await?)
}

#[tauri::command]
pub async fn vcs_commit_suggest(path: String) -> Result<CommitSuggestion, String> {
    Ok(actions::suggest(&path_of(&path)?).await?)
}

#[tauri::command]
pub async fn vcs_commit(
    path: String,
    message: Option<String>,
    files: Option<Vec<String>>,
) -> Result<CommitResult, String> {
    Ok(actions::commit(&path_of(&path)?, message.as_deref(), files.as_deref()).await?)
}

#[tauri::command]
pub async fn vcs_push(path: String) -> Result<PushResult, String> {
    Ok(actions::push(&path_of(&path)?).await?)
}

#[tauri::command]
pub async fn vcs_host(path: String) -> Result<HostInfo, String> {
    Ok(actions::host(&path_of(&path)?).await?)
}

#[tauri::command]
pub async fn vcs_pr_create(path: String, request: PrRequest) -> Result<PrCreateResult, String> {
    Ok(actions::pr_create(&path_of(&path)?, &request).await?)
}

#[tauri::command]
pub async fn vcs_pr_view(
    path: String,
    reference: Option<String>,
) -> Result<Option<PrInfo>, String> {
    Ok(actions::pr_view(&path_of(&path)?, reference.as_deref()).await?)
}
