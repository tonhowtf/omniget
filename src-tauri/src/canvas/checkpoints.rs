use std::path::{Path, PathBuf};

use omniget_core::core::vcs::checkpoint::{self, RestoreResult};
use omniget_core::core::vcs::repo;
use serde_json::{json, Value};

const KEPT: usize = 20;
const MAX_LISTED_FILES: usize = 200;

pub fn thread_of(node: &str) -> String {
    format!("canvas:{node}")
}

fn undo_thread_of(node: &str) -> String {
    format!("canvas-undo:{node}")
}

async fn repo_root(cwd: &Path) -> Option<PathBuf> {
    let root = repo::discover(cwd).await.ok().map(|p| p.root)?;
    let home = dirs::home_dir();
    (home.as_deref() != Some(root.as_path())).then_some(root)
}

pub async fn capture(node: &str, cwd: &Path) -> Result<Option<u32>, String> {
    let Some(root) = repo_root(cwd).await else {
        return Ok(None);
    };
    let thread = thread_of(node);
    let ctx = checkpoint::context_for(&root)
        .await
        .map_err(|e| e.to_string())?;
    let last = checkpoint::list_in(&ctx, &thread)
        .await
        .map_err(|e| e.to_string())?
        .last()
        .map(|c| c.turn)
        .unwrap_or(0);
    let turn = last + 1;
    checkpoint::capture_in(&ctx, &thread, turn)
        .await
        .map_err(|e| e.to_string())?;
    let _ = checkpoint::delete_before(&root, &thread, KEPT).await;
    Ok(Some(turn))
}

pub async fn list(node: &str, cwd: &Path) -> Result<Value, String> {
    let Some(root) = repo_root(cwd).await else {
        return Ok(
            json!({ "repository": false, "items": [], "empty_reason": "this terminal's folder is not a git repository, so no checkpoint is taken" }),
        );
    };
    let ctx = checkpoint::context_for(&root)
        .await
        .map_err(|e| e.to_string())?;
    let mut items: Vec<Value> = checkpoint::list_in(&ctx, &thread_of(node))
        .await
        .map_err(|e| e.to_string())?
        .iter()
        .map(|c| json!({ "turn": c.turn, "atMs": c.at * 1000 }))
        .collect();
    items.reverse();
    let mut out = json!({ "repository": true, "root": root, "items": items });
    if out["items"].as_array().is_some_and(Vec::is_empty) {
        out["empty_reason"] =
            json!("no turn started in this terminal since checkpoints were turned on");
    }
    Ok(out)
}

pub async fn changes(node: &str, cwd: &Path, turn: u32) -> Result<Value, String> {
    let root = repo_root(cwd).await.ok_or("CHECKPOINT_NOT_REPO")?;
    let files = checkpoint::changed_since(&root, &thread_of(node), turn)
        .await
        .map_err(|e| e.to_string())?;
    let total = files.len();
    let listed: Vec<&String> = files.iter().take(MAX_LISTED_FILES).collect();
    Ok(
        json!({ "turn": turn, "total": total, "files": listed, "truncated": total > MAX_LISTED_FILES }),
    )
}

pub async fn restore(node: &str, cwd: &Path, turn: u32) -> Result<RestoreResult, String> {
    let root = repo_root(cwd).await.ok_or("CHECKPOINT_NOT_REPO")?;
    checkpoint::capture(&root, &undo_thread_of(node), 1)
        .await
        .map_err(|e| format!("CHECKPOINT_UNDO_FAILED: nothing was changed: {e}"))?;
    checkpoint::restore(&root, &thread_of(node), turn, true, false)
        .await
        .map_err(|e| e.to_string())
}

pub async fn undo_restore(node: &str, cwd: &Path) -> Result<RestoreResult, String> {
    let root = repo_root(cwd).await.ok_or("CHECKPOINT_NOT_REPO")?;
    let undo = undo_thread_of(node);
    let done = checkpoint::restore(&root, &undo, 1, true, false)
        .await
        .map_err(|e| e.to_string())?;
    let _ = checkpoint::delete_all(&root, &undo).await;
    Ok(done)
}
