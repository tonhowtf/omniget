use std::path::PathBuf;

use omniget_core::core::vcs::{
    actions::{
        self, CommitResult, CommitSuggestion, HostInfo, PrCreateResult, PrInfo, PrRequest,
        PushResult,
    },
    repo::{self, LogEntry, RepoStatus},
};

pub const WORKTREE_SETUP_EVENT: &str = "central://vcs/worktree-setup";

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

#[tauri::command]
pub async fn vcs_status(path: String) -> Result<RepoStatus, String> {
    Ok(repo::status(&path_of(&path)?).await?)
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
