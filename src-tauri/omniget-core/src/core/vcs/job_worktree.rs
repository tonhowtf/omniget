use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::checkpoint;
use super::repo::{self, GitCtx};
use super::runner::{VcsError, VcsResult};
use super::worktree::{self, BRANCH_PREFIX};

pub const JOB_THREAD_PREFIX: &str = "job-";
pub const INCLUDE_FILE: &str = ".worktreeinclude";
const INCLUDE_MAX_BYTES: u64 = 256 * 1024;
const INCLUDE_MAX_ENTRIES: usize = 1000;
pub const SHARED_DIRS: &[&str] = &["node_modules"];
pub const TRASH_DIR: &str = ".trash";
const AUTHOR_NAME: &str = "OmniGet";
const AUTHOR_EMAIL: &str = "omniget@users.noreply.github.com";

pub fn thread_of(job_id: &str) -> String {
    format!("{JOB_THREAD_PREFIX}{job_id}")
}

pub fn branch_of(job_id: &str) -> String {
    format!("{BRANCH_PREFIX}{JOB_THREAD_PREFIX}{job_id}")
}

pub fn path_of(repo_root: &Path, job_id: &str) -> VcsResult<PathBuf> {
    worktree::worktree_path_for(repo_root, &thread_of(job_id))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct JobWorktree {
    pub path: PathBuf,
    pub branch: String,
    pub base_oid: String,
    pub start_oid: String,
    pub carried: Vec<String>,
    pub shared: Vec<String>,
    pub copied: Vec<String>,
    pub took_ms: u64,
    pub reused: bool,
}

pub async fn repo_root(path: &Path) -> Option<PathBuf> {
    let paths = repo::discover(path).await.ok()?;
    let ctx = GitCtx::Repo {
        work_tree: paths.root.clone(),
    };
    let head = ctx
        .git()
        .args(["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
        .unchecked()
        .run()
        .await
        .ok()?;
    head.ok().then_some(paths.root)
}

async fn rev(ctx: &GitCtx, what: &str) -> VcsResult<String> {
    Ok(ctx
        .git()
        .args(["rev-parse", "--verify", what])
        .run()
        .await?
        .trimmed())
}

async fn commit_on(ctx: &GitCtx, tree: &str, parent: &str, message: &str) -> VcsResult<String> {
    Ok(ctx
        .git()
        .env("GIT_AUTHOR_NAME", AUTHOR_NAME)
        .env("GIT_AUTHOR_EMAIL", AUTHOR_EMAIL)
        .env("GIT_COMMITTER_NAME", AUTHOR_NAME)
        .env("GIT_COMMITTER_EMAIL", AUTHOR_EMAIL)
        .args([
            "commit-tree",
            "--no-gpg-sign",
            tree,
            "-p",
            parent,
            "-m",
            message,
        ])
        .run()
        .await?
        .trimmed())
}

async fn names_between(ctx: &GitCtx, from: &str, to: &str) -> VcsResult<Vec<String>> {
    let out = ctx
        .git()
        .args(["diff", "--no-renames", "--name-only", "-z", from, to, "--"])
        .run()
        .await?;
    Ok(out
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect())
}

pub async fn carry_uncommitted(
    ctx: &GitCtx,
    git_dir: &Path,
    job_id: &str,
) -> VcsResult<(String, String, Vec<String>)> {
    let both = ctx
        .git()
        .args(["rev-parse", "HEAD^{commit}", "HEAD^{tree}"])
        .run()
        .await?
        .text();
    let mut lines = both.lines().map(str::trim);
    let (Some(head), Some(head_tree)) = (lines.next(), lines.next()) else {
        return Err(VcsError::Invalid(
            "the repository has no HEAD commit".into(),
        ));
    };
    let head = head.to_string();
    let tree = checkpoint::live_tree_at(ctx, git_dir).await?;
    if tree == head_tree {
        return Ok((head.clone(), head, Vec::new()));
    }
    let carry = commit_on(
        ctx,
        &tree,
        &head,
        &format!("omniget: uncommitted changes carried into job {job_id}"),
    )
    .await?;
    let files = names_between(ctx, &head, &carry).await?;
    Ok((carry, head, files))
}

pub async fn create(repo: &Path, job_id: &str, base: Option<&str>) -> VcsResult<JobWorktree> {
    let t0 = Instant::now();
    let paths = repo::discover(repo).await?;
    let ctx = GitCtx::Repo {
        work_tree: paths.root.clone(),
    };
    let path = path_of(&paths.root, job_id)?;
    if path.join(".git").is_file() {
        let wctx = GitCtx::Repo {
            work_tree: path.clone(),
        };
        let branch = worktree::current_branch(&wctx)
            .await?
            .unwrap_or_else(|| branch_of(job_id));
        let head = rev(&wctx, "HEAD^{commit}").await?;
        return Ok(JobWorktree {
            path,
            branch,
            base_oid: head.clone(),
            start_oid: head,
            took_ms: t0.elapsed().as_millis() as u64,
            reused: true,
            ..Default::default()
        });
    }
    let (base_oid, start_oid, carried) = match base.map(str::trim).filter(|b| !b.is_empty()) {
        Some(b) if b.starts_with('-') => return Err(VcsError::Invalid(format!("bad base {b}"))),
        Some(b) => {
            let oid = rev(&ctx, &format!("{b}^{{commit}}"))
                .await
                .map_err(|_| VcsError::NotFound(format!("base {b} is not a commit")))?;
            (oid.clone(), oid, Vec::new())
        }
        None => carry_uncommitted(&ctx, &paths.git_dir, job_id).await?,
    };
    let req = worktree::WorktreeRequest {
        repo: paths.root.clone(),
        thread: thread_of(job_id),
        base: Some(base_oid.clone()),
        branch: Some(branch_of(job_id)),
        start_from_origin: false,
        submodules: None,
        run_setup: false,
    };
    let info = worktree::create(&req, Arc::new(AtomicBool::new(false)), |_| {}).await?;
    let (shared, copied) = share_ignored(&paths.root, &info.path).await;
    Ok(JobWorktree {
        path: info.path,
        branch: info.branch,
        base_oid,
        start_oid,
        carried,
        shared,
        copied,
        took_ms: t0.elapsed().as_millis() as u64,
        reused: info.reused,
    })
}

pub async fn ensure(repo: &Path, path: &Path, branch: &str) -> VcsResult<bool> {
    if path.join(".git").is_file() {
        return Ok(false);
    }
    let paths = repo::discover(repo).await?;
    let ctx = GitCtx::Repo {
        work_tree: paths.root.clone(),
    };
    if path.exists() {
        return Err(VcsError::Unsafe(format!(
            "{} exists and is not a worktree",
            path.display()
        )));
    }
    if !worktree::branch_exists(&ctx, branch).await? {
        return Err(VcsError::NotFound(format!(
            "the job's branch {branch} is gone; nothing to recreate the worktree from"
        )));
    }
    let _ = ctx
        .git()
        .args(["worktree", "prune"])
        .unchecked()
        .run()
        .await;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    ctx.git()
        .args(["-c", "core.longpaths=true", "worktree", "add"])
        .arg(path)
        .arg(branch)
        .timeout(Some(Duration::from_secs(600)))
        .run()
        .await?;
    share_ignored(&paths.root, path).await;
    Ok(true)
}

pub fn parse_worktree_include(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let norm = line.replace('\\', "/");
        let norm = norm
            .strip_prefix("./")
            .unwrap_or(&norm)
            .trim_end_matches('/');
        if norm.is_empty()
            || norm.starts_with('!')
            || norm.contains('*')
            || norm.contains('?')
            || norm.starts_with('/')
            || norm.contains(':')
        {
            continue;
        }
        let segs: Vec<&str> = norm.split('/').collect();
        if segs.iter().any(|s| s.is_empty() || *s == ".." || *s == ".") || segs[0] == ".git" {
            continue;
        }
        if !out.iter().any(|e| e == norm) {
            out.push(norm.to_string());
        }
        if out.len() >= INCLUDE_MAX_ENTRIES {
            break;
        }
    }
    out
}

async fn is_ignored(ctx: &GitCtx, rel: &str) -> bool {
    ctx.git()
        .args(["check-ignore", "-q", "--", rel])
        .unchecked()
        .run()
        .await
        .map(|o| o.ok())
        .unwrap_or(false)
}

async fn visible(wt: &Path, rel: &str) -> bool {
    let ctx = GitCtx::Repo {
        work_tree: wt.to_path_buf(),
    };
    ctx.git()
        .args(["status", "--porcelain", "--untracked-files=all", "--", rel])
        .unchecked()
        .run()
        .await
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(true)
}

#[cfg(unix)]
fn link_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dst)
}

#[cfg(windows)]
fn link_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    let out = std::process::Command::new("cmd")
        .args(["/D", "/C", "mklink", "/J"])
        .arg(dst)
        .arg(src)
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        return Ok(());
    }
    if meta.is_dir() {
        std::fs::create_dir_all(dst)?;
        for e in std::fs::read_dir(src)?.flatten() {
            copy_tree(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(p) = dst.parent() {
            std::fs::create_dir_all(p)?;
        }
        std::fs::copy(src, dst).map(|_| ())
    }
}

pub async fn share_ignored(main_root: &Path, wt: &Path) -> (Vec<String>, Vec<String>) {
    let main = GitCtx::Repo {
        work_tree: main_root.to_path_buf(),
    };
    let mut linked = Vec::new();
    for d in SHARED_DIRS {
        let (src, dst) = (main_root.join(d), wt.join(d));
        if !src.is_dir() || dst.symlink_metadata().is_ok() || !is_ignored(&main, d).await {
            continue;
        }
        if link_dir(&src, &dst).is_err() {
            continue;
        }
        if visible(wt, d).await {
            let _ = std::fs::remove_file(&dst).or_else(|_| std::fs::remove_dir(&dst));
            continue;
        }
        linked.push(d.to_string());
    }
    let include = main_root.join(INCLUDE_FILE);
    let text = match std::fs::symlink_metadata(&include) {
        Ok(m) if m.is_file() && m.len() <= INCLUDE_MAX_BYTES => {
            std::fs::read_to_string(&include).unwrap_or_default()
        }
        _ => String::new(),
    };
    let copied = copy_included(main_root, wt, &parse_worktree_include(&text)).await;
    (linked, copied)
}

pub async fn copy_included(main_root: &Path, wt: &Path, paths: &[String]) -> Vec<String> {
    let main = GitCtx::Repo {
        work_tree: main_root.to_path_buf(),
    };
    let mut copied = Vec::new();
    for rel in paths {
        let (src, dst) = (main_root.join(rel), wt.join(rel));
        if src.symlink_metadata().is_err()
            || dst.symlink_metadata().is_ok()
            || !is_ignored(&main, rel).await
        {
            continue;
        }
        if copy_tree(&src, &dst).is_ok() {
            copied.push(rel.clone());
        }
    }
    copied
}

pub async fn dirty_files(wt: &Path) -> VcsResult<Vec<String>> {
    let ctx = GitCtx::Repo {
        work_tree: wt.to_path_buf(),
    };
    let out = ctx
        .git()
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=all"])
        .run()
        .await?;
    let parts: Vec<&[u8]> = out.stdout.split(|b| *b == 0).collect();
    let mut files = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        let e = parts[i];
        if e.len() > 3 {
            files.push(String::from_utf8_lossy(&e[3..]).into_owned());
            if e[0] == b'R' || e[0] == b'C' {
                i += 1;
            }
        }
        i += 1;
    }
    Ok(files)
}

pub async fn save_work(wt: &Path, message: &str) -> VcsResult<Option<String>> {
    let ctx = GitCtx::Repo {
        work_tree: wt.to_path_buf(),
    };
    let branch = worktree::current_branch(&ctx)
        .await?
        .ok_or_else(|| VcsError::Invalid("the worktree is on a detached HEAD".into()))?;
    let all = ctx
        .git()
        .args([
            "rev-parse",
            "--absolute-git-dir",
            "HEAD^{commit}",
            "HEAD^{tree}",
        ])
        .run()
        .await?
        .text();
    let mut lines = all.lines().map(str::trim);
    let (Some(dir), Some(head), Some(head_tree)) = (lines.next(), lines.next(), lines.next())
    else {
        return Err(VcsError::Invalid("the worktree has no HEAD commit".into()));
    };
    let tree = checkpoint::live_tree_at(&ctx, Path::new(dir)).await?;
    if tree == head_tree {
        return Ok(None);
    }
    let commit = commit_on(&ctx, &tree, head, message).await?;
    ctx.git()
        .args(["update-ref", &format!("refs/heads/{branch}"), &commit, head])
        .run()
        .await?;
    ctx.git().args(["reset", "-q"]).run().await?;
    Ok(Some(commit))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RemoveOutcome {
    pub dirty: Vec<String>,
    pub saved: Option<String>,
    pub preserved_branch: Option<String>,
    pub branch_deleted: Option<String>,
    pub trashed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_note: Option<String>,
}

fn trash_root() -> VcsResult<PathBuf> {
    Ok(worktree::worktrees_root()?.join(TRASH_DIR))
}

pub async fn remove(
    repo: &Path,
    path: &Path,
    branch: &str,
    base_oid: Option<&str>,
) -> VcsResult<RemoveOutcome> {
    let mut out = RemoveOutcome::default();
    let paths = repo::discover(repo).await?;
    let main = GitCtx::Repo {
        work_tree: paths.root.clone(),
    };
    if path.exists() {
        if !worktree::is_omniget_worktree(path) {
            return Err(VcsError::Unsafe(format!(
                "{} is not an OmniGet worktree",
                path.display()
            )));
        }
        out.dirty = dirty_files(path).await?;
        if !out.dirty.is_empty() {
            out.saved =
                save_work(path, "omniget: work left in the job's worktree at removal").await?;
        }
        let trash = trash_root()?;
        std::fs::create_dir_all(&trash)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "worktree".into());
        let bin = trash.join(format!(
            "{name}-{}",
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        ));
        out.trashed = std::fs::rename(path, &bin).is_ok();
        if out.trashed {
            std::thread::spawn(move || {
                let _ = std::fs::remove_dir_all(&bin);
            });
        } else {
            main.git()
                .args(["worktree", "remove", "--force"])
                .arg(path)
                .timeout(Some(Duration::from_secs(300)))
                .run()
                .await?;
        }
    }
    let _ = main
        .git()
        .args(["worktree", "prune"])
        .unchecked()
        .run()
        .await;
    let tip = main
        .git()
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .unchecked()
        .run()
        .await?;
    if tip.ok() {
        let tip = tip.trimmed();
        let flag = if base_oid == Some(tip.as_str()) {
            "-D"
        } else {
            "-d"
        };
        let del = main
            .git()
            .args(["branch", flag, "--", branch])
            .unchecked()
            .run()
            .await?;
        if del.ok() {
            out.branch_deleted = Some(branch.to_string());
        } else {
            out.preserved_branch = Some(branch.to_string());
        }
    }
    Ok(out)
}

fn is_job_id(id: &str) -> bool {
    id.len() == 11 && id.starts_with('j') && id[1..].chars().all(|c| c.is_ascii_hexdigit())
}

pub async fn branch_at(wt: &Path) -> Option<String> {
    let ctx = GitCtx::Repo {
        work_tree: wt.to_path_buf(),
    };
    worktree::current_branch(&ctx).await.ok().flatten()
}

pub fn list_job_worktrees() -> Vec<(String, PathBuf)> {
    let Ok(root) = worktree::worktrees_root() else {
        return Vec::new();
    };
    let marker = format!("-{JOB_THREAD_PREFIX}");
    let Ok(rd) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    rd.flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.rsplit_once(&marker)?.1.to_string();
            let p = e.path();
            (is_job_id(&id) && p.join(".git").is_file()).then_some((id, p))
        })
        .collect()
}

pub async fn main_repo_of(wt: &Path) -> Option<PathBuf> {
    let paths = repo::discover(wt).await.ok()?;
    match worktree::main_ctx(&paths.common_dir) {
        GitCtx::Repo { work_tree } => Some(work_tree),
        GitCtx::Shadow { .. } => None,
    }
}

pub fn host_label() -> String {
    sysinfo::System::host_name()
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "localhost".into())
}

pub fn empty_trash() -> usize {
    let Ok(t) = trash_root() else { return 0 };
    let Ok(rd) = std::fs::read_dir(&t) else {
        return 0;
    };
    rd.flatten()
        .filter(|e| std::fs::remove_dir_all(e.path()).is_ok())
        .count()
}
