use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::repo::GitCtx;
use super::runner::{VcsError, VcsResult};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum Strategy {
    #[default]
    Merge,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Preview {
    pub target_branch: Option<String>,
    pub head: String,
    pub dirty: Vec<String>,
    pub in_progress: Option<String>,
    pub commits: Vec<String>,
    pub up_to_date: bool,
}

impl Preview {
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
    pub status: String,
    pub strategy: Strategy,
    pub target_branch: String,
    pub commit: Option<String>,
    pub conflicts: Vec<String>,
    pub output: String,
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

async fn restore(ctx: &GitCtx, git_dir: &Path) -> bool {
    if git_dir.join("MERGE_HEAD").exists() {
        let _ = ctx.git().args(["merge", "--abort"]).unchecked().run().await;
    }
    let dirty = tracked_dirty(ctx)
        .await
        .map(|d| !d.is_empty())
        .unwrap_or(true);
    if git_dir.join("MERGE_HEAD").exists() || dirty {
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
