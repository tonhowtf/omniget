//! Per-turn checkpoints as hidden refs
//! `refs/omniget/checkpoints/<b64url(thread)>/turn/<N>`, one commit each,
//! built through a temporary `GIT_INDEX_FILE`: the user's index, branch,
//! stash and HEAD never move. Untracked files are captured, ignored ones are
//! not (`add -A` honors `.gitignore`). Turn 0 is the baseline before the
//! first turn.
//!
//! A folder with no git gets the same thing in a shadow git dir under the app
//! data dir, the way `llm::snapshot` does its undo (same exclude list, never a
//! `.git` inside the user's folder).

use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::diff::{self, DiffOptions, DiffResult};
use super::repo::{self, GitCtx};
use super::runner::{VcsError, VcsResult};

pub const REF_ROOT: &str = "refs/omniget/checkpoints";
const AUTHOR_NAME: &str = "OmniGet";
const AUTHOR_EMAIL: &str = "omniget@users.noreply.github.com";

pub fn thread_key(thread: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(thread.as_bytes())
}

pub fn thread_prefix(thread: &str) -> String {
    format!("{REF_ROOT}/{}/turn/", thread_key(thread))
}

pub fn ref_name(thread: &str, turn: u32) -> String {
    format!("{}{turn}", thread_prefix(thread))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Checkpoint {
    pub thread: String,
    pub turn: u32,
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub commit: String,
    pub tree: String,
    pub at: i64,
    /// Taken in a shadow git dir (the folder has no git).
    pub shadow: bool,
}

fn shadow_root() -> VcsResult<PathBuf> {
    crate::core::paths::app_data_dir()
        .map(|d| d.join("vcs-shadow"))
        .ok_or_else(|| VcsError::Io("no app data dir".into()))
}

fn shadow_dir_for(work_tree: &Path) -> VcsResult<PathBuf> {
    let canon = std::fs::canonicalize(work_tree).unwrap_or_else(|_| work_tree.to_path_buf());
    let mut h = Sha256::new();
    h.update(canon.to_string_lossy().as_bytes());
    let hex: String = h
        .finalize()
        .iter()
        .take(10)
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok(shadow_root()?.join(hex))
}

/// The context checkpoints of `path` live in: its repository, or a shadow
/// git dir when there is none (created on first use).
pub async fn context_for(path: &Path) -> VcsResult<GitCtx> {
    match repo::discover(path).await {
        Ok(p) => Ok(GitCtx::Repo { work_tree: p.root }),
        Err(VcsError::NotRepo { .. }) => {
            if !path.is_dir() {
                return Err(VcsError::NotFound(format!(
                    "{} is not a folder",
                    path.display()
                )));
            }
            let work_tree = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            let git_dir = shadow_dir_for(&work_tree)?;
            let ctx = GitCtx::Shadow {
                git_dir: git_dir.clone(),
                work_tree,
            };
            if !git_dir.join("HEAD").exists() {
                std::fs::create_dir_all(&git_dir)?;
                ctx.git().args(["init", "-q"]).run().await?;
                let info = git_dir.join("info");
                std::fs::create_dir_all(&info)?;
                std::fs::write(
                    info.join("exclude"),
                    "node_modules/\ntarget/\ndist/\nbuild/\n.svelte-kit/\n.venv/\n__pycache__/\n*.omniget-tmp\n.DS_Store\n",
                )?;
            }
            Ok(ctx)
        }
        Err(e) => Err(e),
    }
}

/// A temporary index file that is removed (with its lock) on drop.
struct TempIndex {
    path: PathBuf,
}

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let mut lock = self.path.clone().into_os_string();
        lock.push(".lock");
        let _ = std::fs::remove_file(PathBuf::from(lock));
    }
}

async fn has_head(ctx: &GitCtx) -> VcsResult<bool> {
    Ok(ctx
        .git()
        .args(["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
        .unchecked()
        .run()
        .await?
        .ok())
}

/// Builds a temp index that mirrors the work tree right now (tracked +
/// untracked, minus ignored). The real index is only read, never written.
async fn stage_worktree(ctx: &GitCtx) -> VcsResult<TempIndex> {
    let git_dir = ctx.git_dir().await?;
    let tmp = TempIndex {
        path: git_dir.join(format!(
            "omniget-checkpoint-index-{}",
            uuid::Uuid::new_v4().simple()
        )),
    };
    let real = git_dir.join("index");
    let mut seeded = false;
    if real.is_file() {
        // Copying keeps the stat cache (fast `add -A` on big trees). The copy's
        // mtime goes 1 s before the original's so git's racy-clean check still
        // re-reads any file touched in the same second as the real index.
        if std::fs::copy(&real, &tmp.path).is_ok() {
            if let Ok(meta) = std::fs::metadata(&real) {
                if let Ok(m) = meta.modified() {
                    let back = m.checked_sub(Duration::from_secs(1)).unwrap_or(m);
                    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&tmp.path) {
                        let _ = f.set_modified(back);
                    }
                }
            }
            seeded = true;
        }
    }
    if !seeded && has_head(ctx).await? {
        ctx.git()
            .env("GIT_INDEX_FILE", &tmp.path)
            .args(["read-tree", "HEAD"])
            .run()
            .await?;
    }
    let add = |extra: &'static [&'static str]| {
        ctx.git()
            .env("GIT_INDEX_FILE", &tmp.path)
            .args([
                "-c",
                "core.fsync=objects,reference",
                "-c",
                "core.fsyncMethod=fsync",
                "-c",
                "core.autocrlf=false",
            ])
            .arg("add")
            .args(extra)
            .args(["-A", "--", "."])
            .timeout(Some(Duration::from_secs(300)))
    };
    if let Err(e) = add(&[]).run().await {
        // Nested repos without a commit and unreadable files: keep going with
        // what can be staged instead of losing the whole checkpoint.
        // `--ignore-errors` still exits 1 after skipping; 128 is a real failure.
        let retry = add(&["--ignore-errors"]).unchecked().run().await?;
        if retry.code == Some(128) || retry.code.is_none() {
            return Err(e);
        }
    }
    Ok(tmp)
}

async fn write_tree(ctx: &GitCtx, idx: &TempIndex) -> VcsResult<String> {
    Ok(ctx
        .git()
        .env("GIT_INDEX_FILE", &idx.path)
        .arg("write-tree")
        .run()
        .await?
        .trimmed())
}

/// Tree id of the work tree as it is now, without writing any ref.
pub async fn live_tree(ctx: &GitCtx) -> VcsResult<String> {
    let idx = stage_worktree(ctx).await?;
    write_tree(ctx, &idx).await
}

async fn commit_tree(ctx: &GitCtx, tree: &str, message: &str) -> VcsResult<String> {
    Ok(ctx
        .git()
        .env("GIT_AUTHOR_NAME", AUTHOR_NAME)
        .env("GIT_AUTHOR_EMAIL", AUTHOR_EMAIL)
        .env("GIT_COMMITTER_NAME", AUTHOR_NAME)
        .env("GIT_COMMITTER_EMAIL", AUTHOR_EMAIL)
        .args([
            "-c",
            "core.fsync=objects,reference",
            "-c",
            "core.fsyncMethod=fsync",
        ])
        .args(["commit-tree", "--no-gpg-sign", tree, "-m", message])
        .run()
        .await?
        .trimmed())
}

/// Captures turn `turn` of `thread` in the checkout at `path`. Capturing the
/// same turn again moves the ref (the last capture wins).
pub async fn capture(path: &Path, thread: &str, turn: u32) -> VcsResult<Checkpoint> {
    validate_thread(thread)?;
    let ctx = context_for(path).await?;
    capture_in(&ctx, thread, turn).await
}

pub async fn capture_in(ctx: &GitCtx, thread: &str, turn: u32) -> VcsResult<Checkpoint> {
    let tree = live_tree(ctx).await?;
    let name = ref_name(thread, turn);
    let commit = commit_tree(ctx, &tree, &format!("omniget checkpoint ref={name}")).await?;
    ctx.git()
        .args([
            "-c",
            "core.fsync=objects,reference",
            "-c",
            "core.fsyncMethod=fsync",
        ])
        .args(["update-ref", "-m", "omniget checkpoint", &name, &commit])
        .run()
        .await?;
    Ok(Checkpoint {
        thread: thread.into(),
        turn,
        ref_name: name,
        commit,
        tree,
        at: chrono::Utc::now().timestamp(),
        shadow: ctx.is_shadow(),
    })
}

fn validate_thread(thread: &str) -> VcsResult<()> {
    if thread.is_empty() || thread.len() > 256 {
        return Err(VcsError::Invalid("thread id must be 1..256 bytes".into()));
    }
    Ok(())
}

pub async fn list(path: &Path, thread: &str) -> VcsResult<Vec<Checkpoint>> {
    let ctx = context_for(path).await?;
    list_in(&ctx, thread).await
}

pub async fn list_in(ctx: &GitCtx, thread: &str) -> VcsResult<Vec<Checkpoint>> {
    let prefix = thread_prefix(thread);
    let out = ctx
        .git()
        .args([
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(tree)%00%(creatordate:unix)",
            prefix.trim_end_matches('/'),
        ])
        .run()
        .await?;
    let mut v: Vec<Checkpoint> = out
        .text()
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\0').collect();
            let turn: u32 = f.first()?.strip_prefix(&prefix)?.parse().ok()?;
            Some(Checkpoint {
                thread: thread.into(),
                turn,
                ref_name: f[0].into(),
                commit: f.get(1)?.to_string(),
                tree: f.get(2).unwrap_or(&"").to_string(),
                at: f.get(3).and_then(|s| s.parse().ok()).unwrap_or(0),
                shadow: ctx.is_shadow(),
            })
        })
        .collect();
    v.sort_by_key(|c| c.turn);
    Ok(v)
}

async fn resolve(ctx: &GitCtx, thread: &str, turn: u32) -> VcsResult<String> {
    let name = ref_name(thread, turn);
    let out = ctx
        .git()
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{name}^{{commit}}"),
        ])
        .unchecked()
        .run()
        .await?;
    if !out.ok() {
        return Err(VcsError::NotFound(format!(
            "no checkpoint for turn {turn} of this thread"
        )));
    }
    Ok(out.trimmed())
}

/// Diff of one turn: checkpoint `turn - 1` → `turn`.
pub async fn diff_turn(
    path: &Path,
    thread: &str,
    turn: u32,
    opts: &DiffOptions,
) -> VcsResult<DiffResult> {
    if turn == 0 {
        return Err(VcsError::Invalid(
            "turn 0 is the baseline; the first turn is 1".into(),
        ));
    }
    diff_range(path, thread, turn - 1, Some(turn), opts).await
}

/// Diff between two checkpoints; `to = None` diffs against the live work tree.
pub async fn diff_range(
    path: &Path,
    thread: &str,
    from: u32,
    to: Option<u32>,
    opts: &DiffOptions,
) -> VcsResult<DiffResult> {
    let ctx = context_for(path).await?;
    let a = resolve(&ctx, thread, from).await?;
    let b = match to {
        Some(t) => resolve(&ctx, thread, t).await?,
        None => live_tree(&ctx).await?,
    };
    let mut r = diff::diff_trees(&ctx, &a, &b, opts).await?;
    r.from = format!("turn {from}");
    r.to = to
        .map(|t| format!("turn {t}"))
        .unwrap_or_else(|| "working tree".into());
    Ok(r)
}

/// Whole-thread diff: the first checkpoint (baseline) → `to` (or the live tree).
pub async fn diff_full(
    path: &Path,
    thread: &str,
    to: Option<u32>,
    opts: &DiffOptions,
) -> VcsResult<DiffResult> {
    let ctx = context_for(path).await?;
    let first = list_in(&ctx, thread)
        .await?
        .first()
        .map(|c| c.turn)
        .ok_or_else(|| VcsError::NotFound("this thread has no checkpoints".into()))?;
    diff_range(path, thread, first, to, opts).await
}

/// Deletes the refs of turns after `turn` (after a revert). Returns how many.
pub async fn delete_after(path: &Path, thread: &str, turn: u32) -> VcsResult<u32> {
    let ctx = context_for(path).await?;
    let mut n = 0;
    for c in list_in(&ctx, thread).await? {
        if c.turn > turn {
            ctx.git()
                .args(["update-ref", "-d", &c.ref_name])
                .run()
                .await?;
            n += 1;
        }
    }
    Ok(n)
}

/// Deletes every checkpoint of the thread.
pub async fn delete_all(path: &Path, thread: &str) -> VcsResult<u32> {
    let ctx = context_for(path).await?;
    let mut n = 0;
    for c in list_in(&ctx, thread).await? {
        ctx.git()
            .args(["update-ref", "-d", &c.ref_name])
            .run()
            .await?;
        n += 1;
    }
    Ok(n)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RestoreResult {
    pub turn: u32,
    /// Files whose content changed back (added, rewritten or removed).
    pub files: Vec<String>,
    pub dropped_later: u32,
}

/// Puts the work tree back to checkpoint `turn`. Allowed on its own only in
/// an OmniGet worktree; anywhere else (the user's own checkout, a shadow
/// folder) it needs `confirm`. Works through a temp index, so the real index
/// and HEAD stay as they are; ignored files are not touched.
pub async fn restore(
    path: &Path,
    thread: &str,
    turn: u32,
    confirm: bool,
    drop_later: bool,
) -> VcsResult<RestoreResult> {
    let ctx = context_for(path).await?;
    let isolated = match &ctx {
        GitCtx::Repo { work_tree } => super::worktree::is_omniget_worktree(work_tree),
        GitCtx::Shadow { .. } => false,
    };
    if !isolated && !confirm {
        return Err(VcsError::Unsafe(
            "restoring files outside an OmniGet worktree needs explicit confirmation".into(),
        ));
    }
    let target = resolve(&ctx, thread, turn).await?;
    let idx = stage_worktree(&ctx).await?;
    let now = write_tree(&ctx, &idx).await?;
    let changed = ctx
        .git()
        .args([
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            &now,
            &target,
            "--",
        ])
        .run()
        .await?;
    let files: Vec<String> = changed
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    if !files.is_empty() {
        ctx.git()
            .env("GIT_INDEX_FILE", &idx.path)
            .args([
                "-c",
                "core.autocrlf=false",
                "read-tree",
                "--reset",
                "-u",
                &target,
            ])
            .timeout(Some(Duration::from_secs(300)))
            .run()
            .await?;
        // Folders emptied by the restore go too.
        let root = ctx.work_tree().to_path_buf();
        for f in &files {
            let mut dir = root.join(f);
            while dir.pop() && dir != root && dir.starts_with(&root) {
                if std::fs::remove_dir(&dir).is_err() {
                    break;
                }
            }
        }
    }
    drop(idx);
    let dropped_later = if drop_later {
        delete_after(path, thread, turn).await?
    } else {
        0
    };
    Ok(RestoreResult {
        turn,
        files,
        dropped_later,
    })
}
