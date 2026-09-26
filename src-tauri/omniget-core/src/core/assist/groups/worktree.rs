//! Separate git worktrees for parallel code work, only when asked for.
//!
//! The default for two members in one folder is serialisation (the write
//! lock in `code_tools`). A worktree is created only by an explicit request;
//! the app marks it as its own (a row here plus a marker file in the
//! worktree's private git dir, never in the working tree) and cleanup
//! refuses any folder that does not carry both. `git worktree remove` without
//! `--force` also refuses a worktree with uncommitted changes.

use std::path::{Path, PathBuf};
use std::process::Command;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::super::db::AssistDb;
use super::super::now_ms;
use super::{ERR_GROUP, ERR_GROUP_NOT_OWNED};

pub const MARKER: &str = "omniget-owner";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Worktree {
    pub id: String,
    pub repo: PathBuf,
    pub path: PathBuf,
    pub branch: String,
    pub conversation: Option<String>,
    pub created_ms: i64,
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|e| format!("{ERR_GROUP}: git: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{ERR_GROUP}: git {}: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn admin_dir(worktree: &Path) -> Result<PathBuf, String> {
    let raw = git(worktree, &["rev-parse", "--git-dir"])?;
    let p = PathBuf::from(raw);
    Ok(if p.is_absolute() { p } else { worktree.join(p) })
}

/// Creates `<base>/<id>` as a new worktree of `repo` on branch
/// `omniget/<id>`, and marks it as the app's.
pub fn create(
    db: &AssistDb,
    repo: &Path,
    base: &Path,
    conversation: Option<&str>,
) -> Result<Worktree, String> {
    let top = PathBuf::from(git(repo, &["rev-parse", "--show-toplevel"])?);
    let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
    std::fs::create_dir_all(base).map_err(|e| format!("{ERR_GROUP}: {e}"))?;
    let path = base.join(&id);
    let branch = format!("omniget/{id}");
    git(
        &top,
        &[
            "worktree",
            "add",
            "-b",
            &branch,
            &path.to_string_lossy(),
            "HEAD",
        ],
    )?;
    let path = path
        .canonicalize()
        .map_err(|e| format!("{ERR_GROUP}: {e}"))?;
    std::fs::write(admin_dir(&path)?.join(MARKER), &id).map_err(|e| format!("{ERR_GROUP}: {e}"))?;
    let wt = Worktree {
        id,
        repo: top,
        path,
        branch,
        conversation: conversation.map(str::to_string),
        created_ms: now_ms(),
    };
    db.with(|c| {
        c.execute(
            "INSERT INTO groups_worktrees(id, repo, path, branch, conversation, created_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                wt.id,
                wt.repo.to_string_lossy(),
                wt.path.to_string_lossy(),
                wt.branch,
                wt.conversation,
                wt.created_ms
            ],
        )
    })?;
    Ok(wt)
}

pub fn list(db: &AssistDb) -> Result<Vec<Worktree>, String> {
    db.with(|c| {
        let mut st = c.prepare(
            "SELECT id, repo, path, branch, conversation, created_ms FROM groups_worktrees WHERE removed_ms IS NULL ORDER BY created_ms",
        )?;
        let rows = st.query_map([], |r| {
            Ok(Worktree {
                id: r.get(0)?,
                repo: PathBuf::from(r.get::<_, String>(1)?),
                path: PathBuf::from(r.get::<_, String>(2)?),
                branch: r.get(3)?,
                conversation: r.get(4)?,
                created_ms: r.get(5)?,
            })
        })?;
        rows.collect()
    })
}

/// Removes a worktree the app created. Anything else — a folder without
/// the row, without the marker, or with a marker of another id — is refused
/// and left untouched.
pub fn remove(db: &AssistDb, path: &Path) -> Result<(), String> {
    let refuse = || {
        format!(
            "{ERR_GROUP_NOT_OWNED}: {} was not created by OmniGet; nothing was removed",
            path.display()
        )
    };
    let real = path.canonicalize().map_err(|_| refuse())?;
    let row: Option<(String, String)> = db.with(|c| {
        c.query_row(
            "SELECT id, repo FROM groups_worktrees WHERE path = ?1 AND removed_ms IS NULL",
            [real.to_string_lossy()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
    })?;
    let Some((id, repo)) = row else {
        return Err(refuse());
    };
    let marker = admin_dir(&real)
        .ok()
        .and_then(|d| std::fs::read_to_string(d.join(MARKER)).ok());
    if marker.as_deref().map(str::trim) != Some(id.as_str()) {
        return Err(refuse());
    }
    git(
        Path::new(&repo),
        &["worktree", "remove", &real.to_string_lossy()],
    )?;
    db.with(|c| {
        c.execute(
            "UPDATE groups_worktrees SET removed_ms = ?2 WHERE id = ?1",
            params![id, now_ms()],
        )
    })?;
    Ok(())
}
