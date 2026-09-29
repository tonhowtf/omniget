//! Worktrees of background jobs (roadmap 2.3, S10), built on the Central's
//! worktree per thread ([`super::worktree`]): the same root, the same guard
//! ([`super::worktree::is_omniget_worktree`]) and the same staged `create`,
//! with the thread id `job-<id>` and the branch `omniget/job-<id>`. What a
//! job needs on top of a thread:
//!
//! - **The user's uncommitted work comes along** (81#P5, "nothing is lost"):
//!   the work tree is staged into a *private* index (`GIT_INDEX_FILE`, see
//!   [`super::checkpoint::live_tree`]) and committed on top of `HEAD` without
//!   moving anything of the user's; the worktree starts from that commit, so
//!   the job sees the code as it is on disk and its own diff (`base..tip`)
//!   holds only what the job did.
//! - **Ignored dependencies**: `node_modules` is linked (symlink, junction on
//!   Windows) and the literal paths of `.worktreeinclude` are copied, each
//!   only when it exists in the main checkout *and* is gitignored; `target/`
//!   is never shared (Cargo's lock would serialise the builds).
//! - **Removal that never loses work**: a preflight lists what is not
//!   committed, that work is saved on the job's branch first (a failure keeps
//!   the worktree), the folder goes to a trash folder, `git worktree prune`
//!   drops the admin entry, and the branch goes with `branch -d`: one git
//!   considers unmerged is preserved and reported.
//! - **Recreation** of a worktree that vanished, from its branch, when the
//!   job is resumed (t3code P6), and the listing the orphan GC walks.
//!
//! The include/share rules and the removal order are adapted from
//! stablyai/orca (MIT): `worktree-include-file.ts`,
//! `worktree-shared-directories.ts`, `worktree-removal-preflight.ts`,
//! `worktree-removal.ts`, `worktree-branch-removal.ts`.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::checkpoint;
use super::repo::{self, GitCtx};
use super::runner::{VcsError, VcsResult};
use super::worktree::{self, BRANCH_PREFIX};

/// Thread id (and folder suffix) of a job's worktree: `job-<id>`.
pub const JOB_THREAD_PREFIX: &str = "job-";
/// Repo-root list of gitignored paths copied into each new worktree
/// (cross-tool convention).
pub const INCLUDE_FILE: &str = ".worktreeinclude";
const INCLUDE_MAX_BYTES: u64 = 256 * 1024;
const INCLUDE_MAX_ENTRIES: usize = 1000;
/// Ignored folders linked, never copied: one install serves every worktree.
pub const SHARED_DIRS: &[&str] = &["node_modules"];
/// Folder under the worktrees root where removed worktrees wait to be
/// deleted, so a removal returns at once.
pub const TRASH_DIR: &str = ".trash";
const AUTHOR_NAME: &str = "OmniGet";
const AUTHOR_EMAIL: &str = "omniget@users.noreply.github.com";

pub fn thread_of(job_id: &str) -> String {
    format!("{JOB_THREAD_PREFIX}{job_id}")
}

pub fn branch_of(job_id: &str) -> String {
    format!("{BRANCH_PREFIX}{JOB_THREAD_PREFIX}{job_id}")
}

/// Where the worktree of job `job_id` of the repository rooted at
/// `repo_root` lives (whether it exists or not).
pub fn path_of(repo_root: &Path, job_id: &str) -> VcsResult<PathBuf> {
    worktree::worktree_path_for(repo_root, &thread_of(job_id))
}

/// A job's worktree as it was made (or found).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct JobWorktree {
    pub path: PathBuf,
    pub branch: String,
    /// The commit the worktree started from: the base the caller named, or
    /// the user's `HEAD`, or the commit carrying their uncommitted work.
    pub base_oid: String,
    /// The user's `HEAD` when the job started (equal to `base_oid` unless
    /// work was carried).
    pub start_oid: String,
    /// Files whose uncommitted state was carried in.
    pub carried: Vec<String>,
    /// Ignored folders linked from the main checkout.
    pub shared: Vec<String>,
    /// `.worktreeinclude` paths copied.
    pub copied: Vec<String>,
    pub took_ms: u64,
    /// The worktree already existed (a resumed job) or was recreated.
    pub reused: bool,
}

/// The root of the git repository at `path` when it has at least one commit
/// (a worktree needs one to start from), else `None`.
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

/// `commit-tree` with one parent, as OmniGet (no hooks, no signing, no ref
/// moved).
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

/// The user's checkout as a commit on top of `HEAD`: `(base, head, files)`.
/// Nothing uncommitted → `base == head`, no commit made. Only a private
/// index is written; the user's index, branch, stash and files never move.
/// `git_dir` is the checkout's absolute git dir (saves a spawn).
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

/// Creates the worktree of job `job_id` from the repository at `repo`:
/// from `base` when given (then nothing uncommitted is carried: it belongs
/// to another commit), else from the user's checkout as it is on disk.
/// An existing worktree of the job is reused as is.
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

/// Makes sure the job's worktree is at `path`: nothing to do when it is,
/// else it is added again from `branch` (the job's work lives there). `Ok(true)`
/// when it was recreated.
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

// ---- ignored dependencies ----

/// `.worktreeinclude` lines as repo-relative literal paths: blank lines and
/// `#` comments skipped, `\` → `/`, `./` and trailing `/` dropped, deduped.
/// Globs, negations and anything that could leave the repository
/// (absolute, `..`, `.git`) are skipped.
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

/// Status lines of `rel` in `wt` (empty = git does not see it).
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

/// A junction: no Developer Mode needed, unlike a directory symlink.
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
        // Never followed: a link out of the repository stays out of the copy.
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

/// Links [`SHARED_DIRS`] and copies the `.worktreeinclude` paths, each only
/// when it exists in the main checkout, is gitignored there and is not in
/// the worktree yet. Returns `(linked, copied)`. Never fails the creation.
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
        // A `node_modules/` pattern (trailing slash) matches directories
        // only, and a link is not one to git: then the link would show as a
        // new file in the job's diff. Drop it rather than pollute the diff.
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

/// Copies each repo-relative path of `paths` from the main checkout into
/// the worktree when it exists there, git ignores it and the worktree does
/// not have it yet (the rule of `.worktreeinclude`; also the `include:` of
/// `.omniget/workflow.md`, 3.10). Returns what was copied.
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

// ---- saving and removing ----

/// Paths `git status` reports in the worktree (tracked changes, untracked
/// files; ignored ones are not work).
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

/// Commits whatever is not committed in the worktree onto its branch, as
/// OmniGet, through a private index (no hooks run, nothing signed), then
/// points the worktree's own index at the new commit. `Ok(None)` when there
/// was nothing to save.
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
    // The files are the commit already; only the index is behind.
    ctx.git().args(["reset", "-q"]).run().await?;
    Ok(Some(commit))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RemoveOutcome {
    /// Files the preflight found not committed.
    pub dirty: Vec<String>,
    /// The commit that saved them on the branch before the removal.
    pub saved: Option<String>,
    /// The branch was kept because it holds work not merged anywhere
    /// (`branch -d` refused): the job's result, for the user to take.
    pub preserved_branch: Option<String>,
    pub branch_deleted: Option<String>,
    /// Moved to the trash folder (deleted in the background).
    pub trashed: bool,
    /// What the repository's `before_remove` hook did (or why it was
    /// skipped), set by the job side (3.10); a log line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_note: Option<String>,
}

fn trash_root() -> VcsResult<PathBuf> {
    Ok(worktree::worktrees_root()?.join(TRASH_DIR))
}

/// Removes job worktree `path` of the repository at `repo` and settles its
/// `branch`, never losing work: uncommitted files are saved on the branch
/// first (a failure there keeps the worktree), and a branch with work git
/// considers unmerged is preserved. `base_oid` is where the job started: a
/// branch still at it has nothing of the job and is deleted outright. A
/// folder that is not an OmniGet worktree is refused.
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
            // Deleting a big tree takes seconds; the worktree is already gone
            // for git after the prune below.
            std::thread::spawn(move || {
                let _ = std::fs::remove_dir_all(&bin);
            });
        } else {
            // A file held open (Windows) can block the rename: let git try.
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

/// A job id as `jobs.rs` makes them (`j` + 10 hex): a thread whose name
/// merely contains `-job-` is never taken for a job's worktree.
fn is_job_id(id: &str) -> bool {
    id.len() == 11 && id.starts_with('j') && id[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// The branch checked out in `wt`, if any.
pub async fn branch_at(wt: &Path) -> Option<String> {
    let ctx = GitCtx::Repo {
        work_tree: wt.to_path_buf(),
    };
    worktree::current_branch(&ctx).await.ok().flatten()
}

/// Job worktrees under the root: `(job id, path)`.
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

/// The repository a job worktree belongs to (its main checkout).
pub async fn main_repo_of(wt: &Path) -> Option<PathBuf> {
    let paths = repo::discover(wt).await.ok()?;
    match worktree::main_ctx(&paths.common_dir) {
        GitCtx::Repo { work_tree } => Some(work_tree),
        GitCtx::Shadow { .. } => None,
    }
}

/// The machine's name, for the host's stamp on a job's proof (2.4).
pub fn host_label() -> String {
    sysinfo::System::host_name()
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "localhost".into())
}

/// Deletes what a crash left in the trash folder.
pub fn empty_trash() -> usize {
    let Ok(t) = trash_root() else { return 0 };
    let Ok(rd) = std::fs::read_dir(&t) else {
        return 0;
    };
    rd.flatten()
        .filter(|e| std::fs::remove_dir_all(e.path()).is_ok())
        .count()
}

#[cfg(test)]
mod tests;
