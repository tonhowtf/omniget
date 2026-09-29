//! Integrating a job's branch into the user's checkout (roadmap 3.5, S20,
//! 78#P3): a local `git merge --no-ff` or `--squash` + commit, never a PR.
//!
//! It is the first time OmniGet writes in the user's repository outside an
//! agent's folder, so the rules are strict:
//! - the checkout must be on a branch (not detached) with no tracked change
//!   and no merge, rebase, cherry-pick or revert in progress; untracked files
//!   are allowed (git itself refuses a merge that would overwrite one, before
//!   anything starts);
//! - the merge commit and the squash commit are the user's: their git
//!   identity and their hooks (`pre-merge-commit`, `pre-commit`,
//!   `commit-msg`) run, and a hook that fails is a failure with its output;
//! - on a conflict or any failure the merge is undone (`merge --abort`, then
//!   `reset --merge` if something is still left) and the checkout is checked
//!   again: the answer says `clean` only when there is no `MERGE_HEAD`, no
//!   `SQUASH_MSG` and no tracked change.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::repo::GitCtx;
use super::runner::{VcsError, VcsResult};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Strategy {
    /// `merge --no-ff`: one merge commit, the job's commits kept.
    #[default]
    Merge,
    /// `merge --squash` + one commit with the job's title.
    Squash,
}

impl Strategy {
    pub fn parse(s: &str) -> Result<Strategy, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "merge" | "no-ff" => Ok(Strategy::Merge),
            "squash" => Ok(Strategy::Squash),
            other => Err(format!("strategy must be merge or squash (got `{other}`)")),
        }
    }
}

/// What the checkout looks like before an integration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Preview {
    /// The branch the job would be merged into (`None` = detached HEAD).
    pub target_branch: Option<String>,
    pub head: String,
    /// Tracked files changed in the checkout (they block the merge).
    pub dirty: Vec<String>,
    /// `MERGE_HEAD`, `rebase-merge`… : an operation already in progress.
    pub in_progress: Option<String>,
    /// `git log --oneline HEAD..branch`: what the merge brings, newest first.
    pub commits: Vec<String>,
    /// The branch holds nothing the checkout lacks.
    pub up_to_date: bool,
}

impl Preview {
    /// Why the merge cannot start, or `None`.
    pub fn blocker(&self) -> Option<String> {
        if self.target_branch.is_none() {
            return Some("your checkout is on a detached HEAD: switch to a branch first".into());
        }
        if let Some(op) = &self.in_progress {
            return Some(format!(
                "a {op} is in progress in your checkout: finish or abort it first"
            ));
        }
        if !self.dirty.is_empty() {
            return Some(format!(
                "your checkout has uncommitted changes to tracked files ({}): commit or stash them first",
                self.dirty
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Outcome {
    /// `merged` | `up_to_date` | `conflict` | `failed`
    pub status: String,
    pub strategy: Strategy,
    pub target_branch: String,
    /// The new commit on the target branch.
    pub commit: Option<String>,
    /// Files in conflict (status `conflict`).
    pub conflicts: Vec<String>,
    /// Tail of git's output (hook messages included) when it failed.
    pub output: String,
    /// After a conflict or failure: the checkout was verified back to where
    /// it was (no `MERGE_HEAD`, no `SQUASH_MSG`, no tracked change).
    pub clean: bool,
}

fn ctx(repo: &Path) -> GitCtx {
    GitCtx::Repo {
        work_tree: repo.to_path_buf(),
    }
}

async fn text(ctx: &GitCtx, args: &[&str]) -> VcsResult<String> {
    Ok(ctx.git().args(args).run().await?.trimmed())
}

/// Tracked changes (staged or not) of the checkout, by path.
async fn tracked_dirty(ctx: &GitCtx) -> VcsResult<Vec<String>> {
    let out = ctx
        .git()
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=no"])
        .run()
        .await?;
    Ok(out
        .stdout
        .split(|b| *b == 0)
        .filter(|r| r.len() > 3)
        .map(|r| String::from_utf8_lossy(&r[3..]).into_owned())
        .collect())
}

/// The operation in progress in the checkout, by its marker in the git dir.
fn in_progress(git_dir: &Path) -> Option<String> {
    [
        ("MERGE_HEAD", "merge"),
        ("CHERRY_PICK_HEAD", "cherry-pick"),
        ("REVERT_HEAD", "revert"),
        ("rebase-merge", "rebase"),
        ("rebase-apply", "rebase"),
    ]
    .into_iter()
    .find(|(f, _)| git_dir.join(f).exists())
    .map(|(_, op)| op.to_string())
}

/// What integrating `branch` into the checkout at `repo` would do.
pub async fn preview(repo: &Path, branch: &str) -> VcsResult<Preview> {
    if branch.starts_with('-') || branch.is_empty() {
        return Err(VcsError::Invalid(format!("bad branch name `{branch}`")));
    }
    let ctx = ctx(repo);
    let git_dir = ctx.git_dir().await?;
    let target = ctx
        .git()
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .unchecked()
        .run()
        .await?;
    let head = text(&ctx, &["rev-parse", "HEAD"]).await?;
    let tip = ctx
        .git()
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch}^{{commit}}"))
        .unchecked()
        .run()
        .await?;
    if !tip.ok() {
        return Err(VcsError::NotFound(format!("no branch {branch}")));
    }
    let log = ctx
        .git()
        .args(["log", "--oneline", "--no-decorate", "-n", "50"])
        .arg(format!("HEAD..refs/heads/{branch}"))
        .run()
        .await?
        .text();
    let commits: Vec<String> = log.lines().map(str::to_string).collect();
    Ok(Preview {
        target_branch: target.ok().then(|| target.trimmed()),
        up_to_date: commits.is_empty(),
        head,
        dirty: tracked_dirty(&ctx).await?,
        in_progress: in_progress(&git_dir),
        commits,
    })
}

/// Undoes whatever a failed merge left and says whether the checkout is
/// back: no `MERGE_HEAD`, no `SQUASH_MSG`, no tracked change.
async fn restore(ctx: &GitCtx, git_dir: &Path) -> bool {
    if git_dir.join("MERGE_HEAD").exists() {
        let _ = ctx.git().args(["merge", "--abort"]).unchecked().run().await;
    }
    let dirty = tracked_dirty(ctx)
        .await
        .map(|d| !d.is_empty())
        .unwrap_or(true);
    if git_dir.join("MERGE_HEAD").exists() || dirty {
        // A squash has no MERGE_HEAD for `--abort` to use; `reset --merge`
        // puts back the tracked files the merge touched and keeps untracked
        // ones. The checkout was clean before, so nothing of the user's goes.
        let _ = ctx.git().args(["reset", "--merge"]).unchecked().run().await;
    }
    let _ = std::fs::remove_file(git_dir.join("SQUASH_MSG"));
    let _ = std::fs::remove_file(git_dir.join("MERGE_MSG"));
    !git_dir.join("MERGE_HEAD").exists() && tracked_dirty(ctx).await.is_ok_and(|d| d.is_empty())
}

async fn conflicted(ctx: &GitCtx) -> Vec<String> {
    ctx.git()
        .args(["diff", "--name-only", "--diff-filter=U", "-z"])
        .unchecked()
        .run()
        .await
        .map(|o| {
            o.stdout
                .split(|b| *b == 0)
                .filter(|p| !p.is_empty())
                .map(|p| String::from_utf8_lossy(p).into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Merges `branch` into the branch checked out at `repo`. `Err` = refused
/// before anything was touched (a precondition); `Ok` with `conflict` or
/// `failed` = tried and undone (`clean` says the checkout is back).
pub async fn integrate(
    repo: &Path,
    branch: &str,
    strategy: Strategy,
    message: &str,
) -> VcsResult<Outcome> {
    let pre = preview(repo, branch).await?;
    if let Some(why) = pre.blocker() {
        return Err(VcsError::Invalid(why));
    }
    let target = pre.target_branch.clone().unwrap_or_default();
    let mut out = Outcome {
        strategy,
        target_branch: target.clone(),
        clean: true,
        ..Default::default()
    };
    if pre.up_to_date {
        out.status = "up_to_date".into();
        return Ok(out);
    }
    let ctx = ctx(repo);
    let git_dir = ctx.git_dir().await?;
    let subject = message.lines().next().unwrap_or("").trim();
    let subject = if subject.is_empty() {
        format!("Merge {branch}")
    } else {
        subject.chars().take(200).collect()
    };
    let long = Some(Duration::from_secs(600));
    let merge = match strategy {
        Strategy::Merge => {
            ctx.git()
                .args(["merge", "--no-ff", "--no-edit", "-m", &subject])
                .arg(format!("refs/heads/{branch}"))
                .timeout(long)
                .unchecked()
                .run()
                .await?
        }
        Strategy::Squash => {
            ctx.git()
                .args(["merge", "--squash"])
                .arg(format!("refs/heads/{branch}"))
                .timeout(long)
                .unchecked()
                .run()
                .await?
        }
    };
    let mut failed = (!merge.ok()).then(|| {
        let mut t = merge.text();
        t.push_str(&merge.err_text());
        t
    });
    if failed.is_none() && strategy == Strategy::Squash {
        let commit = ctx
            .git()
            .args(["commit", "--no-edit", "-m", &subject])
            .timeout(long)
            .unchecked()
            .run()
            .await?;
        if !commit.ok() {
            let mut t = commit.text();
            t.push_str(&commit.err_text());
            failed = Some(t);
        }
    }
    match failed {
        None => {
            out.status = "merged".into();
            out.commit = Some(text(&ctx, &["rev-parse", "HEAD"]).await?);
        }
        Some(output) => {
            out.conflicts = conflicted(&ctx).await;
            out.status = if out.conflicts.is_empty() {
                "failed"
            } else {
                "conflict"
            }
            .into();
            out.output = super::runner::tail(&output, 30, 4000);
            out.clean = restore(&ctx, &git_dir).await;
        }
    }
    Ok(out)
}

/// Deletes a job branch the user chose to throw away (`branch -D`), and
/// returns the commit it pointed at, so the log can say how to get it back
/// (`git branch <name> <sha>`) until git collects it.
pub async fn delete_branch(repo: &Path, branch: &str) -> VcsResult<Option<String>> {
    if branch.starts_with('-') || branch.is_empty() {
        return Err(VcsError::Invalid(format!("bad branch name `{branch}`")));
    }
    let ctx = ctx(repo);
    let tip = ctx
        .git()
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch}^{{commit}}"))
        .unchecked()
        .run()
        .await?;
    if !tip.ok() {
        return Ok(None);
    }
    ctx.git().args(["branch", "-D", "--", branch]).run().await?;
    Ok(Some(tip.trimmed()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "u")
            .env("GIT_AUTHOR_EMAIL", "u@u")
            .env("GIT_COMMITTER_NAME", "u")
            .env("GIT_COMMITTER_EMAIL", "u@u")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A repository on `main` with `a.txt` and `b.txt`, and a local identity
    /// (the merge commit is the user's).
    fn repo() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("omniget-integrate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        git(&dir, &["config", "user.name", "User"]);
        git(&dir, &["config", "user.email", "user@example.com"]);
        std::fs::write(dir.join("a.txt"), "a1\na2\na3\n").unwrap();
        std::fs::write(dir.join("b.txt"), "b1\nb2\nb3\n").unwrap();
        git(&dir, &["add", "-A"]);
        git(&dir, &["commit", "-qm", "seed"]);
        dir
    }

    /// A job branch off `main` that rewrites `file`.
    fn job_branch(dir: &Path, name: &str, file: &str, body: &str) {
        git(dir, &["branch", name, "main"]);
        let wt = dir.join(format!("wt-{}", name.replace('/', "-")));
        git(dir, &["worktree", "add", "-q", wt.to_str().unwrap(), name]);
        std::fs::write(wt.join(file), body).unwrap();
        git(&wt, &["commit", "-qam", &format!("job edits {file}")]);
    }

    #[tokio::test]
    async fn two_jobs_on_different_files_become_two_merges() {
        let dir = repo();
        job_branch(&dir, "omniget/job-j1", "a.txt", "a1\nA2\na3\n");
        job_branch(&dir, "omniget/job-j2", "b.txt", "b1\nB2\nb3\n");
        for b in ["omniget/job-j1", "omniget/job-j2"] {
            let out = integrate(&dir, b, Strategy::Merge, "Job title")
                .await
                .unwrap();
            assert_eq!(out.status, "merged", "{out:?}");
            assert_eq!(out.target_branch, "main");
        }
        let merges = git(&dir, &["log", "--merges", "--oneline", "main"]);
        assert_eq!(merges.lines().count(), 2, "{merges}");
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "a1\nA2\na3\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("b.txt")).unwrap(),
            "b1\nB2\nb3\n"
        );
        // The merge commit is the user's, not OmniGet's.
        assert_eq!(
            git(&dir, &["log", "-1", "--format=%an <%ae>"]),
            "User <user@example.com>"
        );
        // Merged again: nothing to do.
        let again = integrate(&dir, "omniget/job-j1", Strategy::Merge, "x")
            .await
            .unwrap();
        assert_eq!(again.status, "up_to_date");
    }

    #[tokio::test]
    async fn a_conflict_is_aborted_and_the_checkout_is_left_clean() {
        for strategy in [Strategy::Merge, Strategy::Squash] {
            let dir = repo();
            job_branch(&dir, "omniget/job-j1", "a.txt", "a1\nONE\na3\n");
            job_branch(&dir, "omniget/job-j2", "a.txt", "a1\nTWO\na3\n");
            std::fs::write(dir.join("untracked.txt"), "mine").unwrap();
            let head_before = git(&dir, &["rev-parse", "HEAD"]);
            let first = integrate(&dir, "omniget/job-j1", strategy, "one")
                .await
                .unwrap();
            assert_eq!(first.status, "merged");
            let head_mid = git(&dir, &["rev-parse", "HEAD"]);
            assert_ne!(head_before, head_mid);
            let second = integrate(&dir, "omniget/job-j2", strategy, "two")
                .await
                .unwrap();
            assert_eq!(second.status, "conflict", "{strategy:?} {second:?}");
            assert_eq!(second.conflicts, vec!["a.txt".to_string()]);
            assert!(second.clean);
            let git_dir = dir.join(".git");
            assert!(!git_dir.join("MERGE_HEAD").exists());
            assert!(!git_dir.join("SQUASH_MSG").exists());
            assert_eq!(
                git(&dir, &["status", "--porcelain", "--untracked-files=no"]),
                ""
            );
            assert_eq!(git(&dir, &["rev-parse", "HEAD"]), head_mid);
            assert_eq!(
                std::fs::read_to_string(dir.join("a.txt")).unwrap(),
                "a1\nONE\na3\n"
            );
            // The user's untracked file is untouched.
            assert_eq!(
                std::fs::read_to_string(dir.join("untracked.txt")).unwrap(),
                "mine"
            );
        }
    }

    #[tokio::test]
    async fn squash_makes_one_commit_with_the_title() {
        let dir = repo();
        job_branch(&dir, "omniget/job-j1", "a.txt", "a1\nA2\na3\n");
        let out = integrate(
            &dir,
            "omniget/job-j1",
            Strategy::Squash,
            "Fix the second line\n\nbody",
        )
        .await
        .unwrap();
        assert_eq!(out.status, "merged");
        assert_eq!(
            git(&dir, &["log", "-1", "--format=%s"]),
            "Fix the second line"
        );
        assert_eq!(git(&dir, &["log", "--merges", "--oneline"]), "");
        assert_eq!(git(&dir, &["rev-list", "--count", "HEAD"]), "2");
    }

    #[tokio::test]
    async fn a_dirty_or_detached_checkout_is_refused_before_anything() {
        let dir = repo();
        job_branch(&dir, "omniget/job-j1", "a.txt", "a1\nA2\na3\n");
        std::fs::write(dir.join("b.txt"), "changed by the user\n").unwrap();
        let err = integrate(&dir, "omniget/job-j1", Strategy::Merge, "x")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("uncommitted changes"), "{err}");
        assert_eq!(
            std::fs::read_to_string(dir.join("b.txt")).unwrap(),
            "changed by the user\n"
        );
        git(&dir, &["checkout", "-q", "--", "b.txt"]);
        let head = git(&dir, &["rev-parse", "HEAD"]);
        git(&dir, &["checkout", "-q", "--detach", &head]);
        let err = integrate(&dir, "omniget/job-j1", Strategy::Merge, "x")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("detached"), "{err}");
        let err = integrate(&dir, "omniget/job-nope", Strategy::Merge, "x")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no branch"), "{err}");
    }

    #[tokio::test]
    async fn a_failing_hook_is_reported_and_undone() {
        let dir = repo();
        job_branch(&dir, "omniget/job-j1", "a.txt", "a1\nA2\na3\n");
        let hook = dir.join(".git/hooks/pre-merge-commit");
        std::fs::create_dir_all(hook.parent().unwrap()).unwrap();
        std::fs::write(&hook, "#!/bin/sh\necho blocked by policy >&2\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let head = git(&dir, &["rev-parse", "HEAD"]);
        let out = integrate(&dir, "omniget/job-j1", Strategy::Merge, "x")
            .await
            .unwrap();
        if cfg!(unix) {
            assert_eq!(out.status, "failed", "{out:?}");
            assert!(out.output.contains("blocked by policy"), "{out:?}");
            assert!(out.clean);
            assert!(!dir.join(".git/MERGE_HEAD").exists());
            assert_eq!(git(&dir, &["rev-parse", "HEAD"]), head);
        }
    }

    #[tokio::test]
    async fn a_discarded_branch_is_deleted_and_its_tip_returned() {
        let dir = repo();
        job_branch(&dir, "omniget/job-j1", "a.txt", "a1\nA2\na3\n");
        git(
            &dir,
            &[
                "worktree",
                "remove",
                "--force",
                dir.join("wt-omniget-job-j1").to_str().unwrap(),
            ],
        );
        let tip = git(&dir, &["rev-parse", "omniget/job-j1"]);
        assert_eq!(
            delete_branch(&dir, "omniget/job-j1").await.unwrap(),
            Some(tip)
        );
        assert_eq!(delete_branch(&dir, "omniget/job-j1").await.unwrap(), None);
    }
}
