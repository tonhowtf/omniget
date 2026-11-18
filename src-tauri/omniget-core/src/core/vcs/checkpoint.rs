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

async fn stage_worktree(ctx: &GitCtx) -> VcsResult<TempIndex> {
    let git_dir = ctx.git_dir().await?;
    stage_worktree_at(ctx, &git_dir).await
}

async fn stage_worktree_at(ctx: &GitCtx, git_dir: &Path) -> VcsResult<TempIndex> {
    let tmp = TempIndex {
        path: git_dir.join(format!(
            "omniget-checkpoint-index-{}",
            uuid::Uuid::new_v4().simple()
        )),
    };
    let real = git_dir.join("index");
    let mut seeded = false;
    if real.is_file() && std::fs::copy(&real, &tmp.path).is_ok() {
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

pub async fn live_tree(ctx: &GitCtx) -> VcsResult<String> {
    let idx = stage_worktree(ctx).await?;
    write_tree(ctx, &idx).await
}

pub async fn live_tree_at(ctx: &GitCtx, git_dir: &Path) -> VcsResult<String> {
    let idx = stage_worktree_at(ctx, git_dir).await?;
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

pub async fn changed_since(path: &Path, thread: &str, turn: u32) -> VcsResult<Vec<String>> {
    let ctx = context_for(path).await?;
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
    Ok(changed
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect())
}

pub async fn delete_before(path: &Path, thread: &str, keep: usize) -> VcsResult<u32> {
    let ctx = context_for(path).await?;
    let all = list_in(&ctx, thread).await?;
    let mut n = 0;
    for c in all.iter().rev().skip(keep) {
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
    pub files: Vec<String>,
    pub dropped_later: u32,
}

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

#[cfg(test)]
mod turn_checkpoint_tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) -> bool {
        std::process::Command::new("git")
            .current_dir(dir)
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    fn repo() -> Option<tempfile::TempDir> {
        let dir = tempfile::tempdir().ok()?;
        let ok = git(dir.path(), &["init", "-q"])
            && git(dir.path(), &["config", "user.email", "t@example.com"])
            && git(dir.path(), &["config", "user.name", "t"]);
        std::fs::write(dir.path().join("a.txt"), "one\n").ok()?;
        (ok && git(dir.path(), &["add", "."]) && git(dir.path(), &["commit", "-qm", "init"]))
            .then_some(dir)
    }

    #[tokio::test]
    async fn a_turn_checkpoint_lists_what_changed_and_brings_it_back() {
        let Some(dir) = repo() else { return };
        let path = dir.path();
        capture(path, "canvas:n1", 1).await.unwrap();
        std::fs::write(path.join("a.txt"), "two\n").unwrap();
        std::fs::write(path.join("b.txt"), "new\n").unwrap();
        let mut changed = changed_since(path, "canvas:n1", 1).await.unwrap();
        changed.sort();
        assert_eq!(changed, vec!["a.txt", "b.txt"]);
        assert_eq!(
            std::fs::read_to_string(path.join("a.txt")).unwrap(),
            "two\n"
        );
        let done = restore(path, "canvas:n1", 1, true, true).await.unwrap();
        assert_eq!(done.files.len(), 2);
        assert_eq!(
            std::fs::read_to_string(path.join("a.txt")).unwrap(),
            "one\n"
        );
        assert!(!path.join("b.txt").exists());
    }

    #[tokio::test]
    async fn restoring_without_confirmation_is_refused_in_a_plain_repository() {
        let Some(dir) = repo() else { return };
        capture(dir.path(), "canvas:n1", 1).await.unwrap();
        assert!(restore(dir.path(), "canvas:n1", 1, false, false)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn only_the_newest_checkpoints_are_kept() {
        let Some(dir) = repo() else { return };
        for turn in 1..=4 {
            capture(dir.path(), "canvas:n1", turn).await.unwrap();
        }
        assert_eq!(delete_before(dir.path(), "canvas:n1", 2).await.unwrap(), 2);
        let ctx = context_for(dir.path()).await.unwrap();
        let left: Vec<u32> = list_in(&ctx, "canvas:n1")
            .await
            .unwrap()
            .iter()
            .map(|c| c.turn)
            .collect();
        assert_eq!(left, vec![3, 4]);
    }
}
