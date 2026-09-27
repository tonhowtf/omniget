//! Git actions for the split button: commit (with a suggested message built
//! from the diff, no model here), push with upstream, and pull requests via
//! `gh` (GitHub) or `glab` (GitLab). Other hosts answer "unavailable".

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::diff::{self, DiffOptions};
use super::repo::{self, GitCtx};
use super::runner::{which, Invocation, VcsError, VcsResult};

// ---- commit message suggestion ----

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChangeSummary {
    pub path: String,
    /// added | deleted | modified | renamed | binary
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitSuggestion {
    pub subject: String,
    pub body: String,
    pub files: Vec<ChangeSummary>,
}

fn is_doc(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    l.ends_with(".md")
        || l.ends_with(".mdx")
        || l.ends_with(".txt")
        || l.ends_with(".rst")
        || l.starts_with("docs/")
}

fn is_test(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    l.contains("/tests/")
        || l.starts_with("tests/")
        || l.contains("/test/")
        || l.contains(".test.")
        || l.contains(".spec.")
        || l.contains("_test.")
        || l.ends_with("_tests.rs")
}

fn is_config(p: &str) -> bool {
    let name = p.rsplit('/').next().unwrap_or(p).to_ascii_lowercase();
    matches!(
        name.as_str(),
        "package.json"
            | "cargo.toml"
            | "cargo.lock"
            | "pnpm-lock.yaml"
            | "package-lock.json"
            | "yarn.lock"
            | "tsconfig.json"
            | ".gitignore"
            | "go.mod"
            | "go.sum"
            | "pyproject.toml"
            | "requirements.txt"
    ) || name.starts_with('.')
        && (name.ends_with("rc")
            || name.ends_with(".json")
            || name.ends_with(".yml")
            || name.ends_with(".yaml"))
}

fn common_dir(paths: &[&str]) -> String {
    let mut parts: Option<Vec<&str>> = None;
    for p in paths {
        let dirs: Vec<&str> = p.split('/').collect();
        let dirs = &dirs[..dirs.len().saturating_sub(1)];
        parts = Some(match parts {
            None => dirs.to_vec(),
            Some(prev) => prev
                .iter()
                .zip(dirs.iter())
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| *a)
                .collect(),
        });
    }
    parts.unwrap_or_default().join("/")
}

fn file_name(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// A commit message from the list of changed files: a type prefix when the
/// change is all docs/tests/config, a verb from the statuses, the target
/// (file, common folder or count) and a body listing the files.
pub fn suggest_commit_message(changes: &[ChangeSummary]) -> CommitSuggestion {
    if changes.is_empty() {
        return CommitSuggestion {
            subject: "Update files".into(),
            body: String::new(),
            files: vec![],
        };
    }
    let paths: Vec<&str> = changes.iter().map(|c| c.path.as_str()).collect();
    let all = |f: fn(&str) -> bool| paths.iter().all(|p| f(p));
    let prefix = if all(is_doc) {
        "docs: "
    } else if all(is_test) {
        "test: "
    } else if all(is_config) {
        "chore: "
    } else {
        ""
    };
    let all_status = |s: &str| changes.iter().all(|c| c.status == s);
    let verb = if all_status("added") {
        "Add"
    } else if all_status("deleted") {
        "Remove"
    } else if all_status("renamed") {
        "Rename"
    } else {
        "Update"
    };
    let target = if changes.len() == 1 {
        let c = &changes[0];
        if c.path.len() <= 50 {
            c.path.clone()
        } else {
            file_name(&c.path).to_string()
        }
    } else {
        let dir = common_dir(&paths);
        if dir.is_empty() {
            let mut names: Vec<&str> = paths.iter().map(|p| file_name(p)).collect();
            names.dedup();
            if names.len() <= 3 && names.iter().map(|n| n.len() + 2).sum::<usize>() <= 50 {
                names.join(", ")
            } else {
                format!("{} files", changes.len())
            }
        } else {
            format!("{dir} ({} files)", changes.len())
        }
    };
    // A conventional prefix takes a lowercase verb; a bare subject starts upper.
    let mut subject = if prefix.is_empty() {
        format!("{verb} {target}")
    } else {
        format!("{prefix}{} {target}", verb.to_lowercase())
    };
    if subject.chars().count() > 72 {
        subject = subject.chars().take(71).collect::<String>() + "…";
    }
    let mut body = String::new();
    if changes.len() > 1 {
        for c in changes.iter().take(20) {
            let mark = match c.status.as_str() {
                "added" => "A",
                "deleted" => "D",
                "renamed" => "R",
                _ => "M",
            };
            body.push_str(&format!(
                "- {mark} {} (+{} -{})\n",
                c.path, c.additions, c.deletions
            ));
        }
        if changes.len() > 20 {
            body.push_str(&format!("- … and {} more\n", changes.len() - 20));
        }
    }
    CommitSuggestion {
        subject,
        body: body.trim_end().to_string(),
        files: changes.to_vec(),
    }
}

async fn ctx_of(path: &Path) -> VcsResult<GitCtx> {
    Ok(GitCtx::Repo {
        work_tree: repo::discover(path).await?.root,
    })
}

async fn head_or_empty(ctx: &GitCtx) -> VcsResult<String> {
    let out = ctx
        .git()
        .args(["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
        .unchecked()
        .run()
        .await?;
    if out.ok() {
        return Ok(out.trimmed());
    }
    // Unborn branch: diff against the empty tree.
    Ok(ctx
        .git()
        .args(["hash-object", "-t", "tree", "--stdin"])
        .stdin(Vec::new())
        .run()
        .await?
        .trimmed())
}

/// Every change in the work tree against HEAD (staged, unstaged and
/// untracked), without touching the index.
pub async fn working_changes(path: &Path) -> VcsResult<Vec<ChangeSummary>> {
    let ctx = ctx_of(path).await?;
    let head = head_or_empty(&ctx).await?;
    let tree = super::checkpoint::live_tree(&ctx).await?;
    let d = diff::diff_trees(
        &ctx,
        &head,
        &tree,
        &DiffOptions {
            stat_only: false,
            max_bytes: 256 * 1024,
            ..Default::default()
        },
    )
    .await?;
    Ok(d.files
        .into_iter()
        .map(|f| ChangeSummary {
            path: f.path,
            status: f.status,
            additions: f.additions,
            deletions: f.deletions,
        })
        .collect())
}

pub async fn suggest(path: &Path) -> VcsResult<CommitSuggestion> {
    Ok(suggest_commit_message(&working_changes(path).await?))
}

// ---- commit ----

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitResult {
    /// created | skipped_no_changes
    pub status: String,
    pub sha: Option<String>,
    pub subject: String,
    pub branch: Option<String>,
}

/// Stages everything (or just `files`) and commits. The message's first line
/// is the subject; the rest after a blank line is the body. Empty message →
/// the suggestion.
pub async fn commit(
    path: &Path,
    message: Option<&str>,
    files: Option<&[String]>,
) -> VcsResult<CommitResult> {
    let ctx = ctx_of(path).await?;
    let (subject, body) = match message.map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) => {
            if m.len() > 10_000 {
                return Err(VcsError::Invalid(
                    "commit message longer than 10000 chars".into(),
                ));
            }
            let mut it = m.splitn(2, '\n');
            (
                it.next().unwrap_or("").trim().to_string(),
                it.next().unwrap_or("").trim().to_string(),
            )
        }
        None => {
            let s = suggest(path).await?;
            (s.subject, s.body)
        }
    };
    if subject.is_empty() {
        return Err(VcsError::Invalid("empty commit subject".into()));
    }
    let files: Vec<String> = files.map(|f| f.to_vec()).unwrap_or_default();
    if files.iter().any(|f| f.starts_with('-')) {
        return Err(VcsError::Invalid("file paths cannot start with '-'".into()));
    }
    let mut add = ctx.git().args(["--literal-pathspecs", "add", "-A", "--"]);
    if files.is_empty() {
        add = add.arg(".");
    } else {
        add = add.args(&files);
    }
    add.timeout(Some(Duration::from_secs(300))).run().await?;
    let mut staged = ctx
        .git()
        .args(["--literal-pathspecs", "diff", "--cached", "--quiet", "--"]);
    if !files.is_empty() {
        staged = staged.args(&files);
    }
    let staged = staged.unchecked().run().await?;
    let branch = {
        let o = ctx
            .git()
            .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
            .unchecked()
            .run()
            .await?;
        o.ok().then(|| o.trimmed())
    };
    if staged.code == Some(0) {
        return Ok(CommitResult {
            status: "skipped_no_changes".into(),
            sha: None,
            subject,
            branch,
        });
    }
    let mut c = ctx
        .git()
        .args(["--literal-pathspecs", "commit", "-m", &subject]);
    if !body.is_empty() {
        c = c.args(["-m", &body]);
    }
    if !files.is_empty() {
        c = c.arg("--only").arg("--").args(&files);
    }
    c.timeout(Some(Duration::from_secs(600))).run().await?;
    let sha = ctx.git().args(["rev-parse", "HEAD"]).run().await?.trimmed();
    Ok(CommitResult {
        status: "created".into(),
        sha: Some(sha),
        subject,
        branch,
    })
}

// ---- push ----

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PushResult {
    /// pushed | skipped_up_to_date
    pub status: String,
    pub remote: String,
    pub branch: String,
    pub upstream: String,
    pub set_upstream: bool,
}

async fn push_remote(ctx: &GitCtx, branch: &str) -> VcsResult<String> {
    for key in [
        format!("branch.{branch}.pushRemote"),
        "remote.pushDefault".into(),
        format!("branch.{branch}.remote"),
    ] {
        if let Some(v) = repo::config_get(ctx, &key).await {
            if v != "." {
                return Ok(v);
            }
        }
    }
    repo::primary_remote(ctx)
        .await?
        .map(|r| r.name)
        .ok_or_else(|| VcsError::Unavailable("the repository has no remote to push to".into()))
}

/// Pushes the current branch. With no upstream, or an upstream of another
/// name (OmniGet worktrees start that way), it publishes the branch under its
/// own name with `-u` instead of pushing onto someone else's branch.
pub async fn push(path: &Path) -> VcsResult<PushResult> {
    let ctx = ctx_of(path).await?;
    let branch = {
        let o = ctx
            .git()
            .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
            .unchecked()
            .run()
            .await?;
        if !o.ok() {
            return Err(VcsError::Invalid(
                "detached HEAD: create a branch before pushing".into(),
            ));
        }
        o.trimmed()
    };
    let remote = push_remote(&ctx, &branch).await?;
    let up = ctx
        .git()
        .args(["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"])
        .unchecked()
        .run()
        .await?;
    let upstream = up.ok().then(|| up.trimmed());
    let merge = repo::config_get(&ctx, &format!("branch.{branch}.merge")).await;
    let same_name = merge
        .as_deref()
        .map(|m| m.trim_start_matches("refs/heads/") == branch)
        .unwrap_or(false);
    let timeout = Some(Duration::from_secs(900));
    if upstream.is_none() || !same_name {
        if let (Some(u), None) = (
            &upstream,
            repo::config_get(&ctx, &format!("branch.{branch}.gh-merge-base")).await,
        ) {
            let base = u.split_once('/').map(|(_, b)| b).unwrap_or(u);
            let _ = ctx
                .git()
                .args(["config", &format!("branch.{branch}.gh-merge-base"), base])
                .run()
                .await;
        }
        ctx.git()
            .args(["push", "-u", &remote, &format!("HEAD:refs/heads/{branch}")])
            .timeout(timeout)
            .run()
            .await?;
        return Ok(PushResult {
            status: "pushed".into(),
            upstream: format!("{remote}/{branch}"),
            remote,
            branch,
            set_upstream: true,
        });
    }
    let upstream = upstream.unwrap_or_default();
    let ahead = ctx
        .git()
        .args(["rev-list", "--count", "@{u}..HEAD"])
        .unchecked()
        .run()
        .await?
        .trimmed()
        .parse::<u32>()
        .unwrap_or(1);
    if ahead == 0 {
        return Ok(PushResult {
            status: "skipped_up_to_date".into(),
            remote,
            branch,
            upstream,
            set_upstream: false,
        });
    }
    ctx.git()
        .args(["push", &remote, &format!("HEAD:refs/heads/{branch}")])
        .timeout(timeout)
        .run()
        .await?;
    Ok(PushResult {
        status: "pushed".into(),
        remote,
        branch,
        upstream,
        set_upstream: false,
    })
}

// ---- hosts and pull requests ----

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostInfo {
    /// github | gitlab | bitbucket | azure | forgejo | unknown | none
    pub kind: String,
    pub host: Option<String>,
    pub cli: Option<String>,
    pub cli_path: Option<PathBuf>,
    pub authenticated: bool,
    pub available: bool,
    /// no-remote | provider-unsupported | cli-missing | cli-unauthenticated
    pub reason: Option<String>,
    /// "PR" or "MR".
    pub term: String,
}

fn url_host(url: &str) -> Option<String> {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let authority = rest.split('/').next()?;
    let authority = authority.rsplit('@').next()?;
    let host = authority.split(':').next()?;
    (!host.is_empty()).then(|| host.to_string())
}

/// Which code host the primary remote points at, and whether its CLI is
/// installed and logged in. Only presence and `auth status` are checked; no
/// token is read.
pub async fn host(path: &Path) -> VcsResult<HostInfo> {
    let ctx = ctx_of(path).await?;
    let Some(remote) = repo::primary_remote(&ctx).await? else {
        return Ok(HostInfo {
            kind: "none".into(),
            host: None,
            cli: None,
            cli_path: None,
            authenticated: false,
            available: false,
            reason: Some("no-remote".into()),
            term: "PR".into(),
        });
    };
    let kind = remote.host_kind.clone();
    let hostname = url_host(&remote.url);
    let (cli, term) = match kind.as_str() {
        "github" => (Some("gh"), "PR"),
        "gitlab" => (Some("glab"), "MR"),
        _ => (None, "PR"),
    };
    let mut info = HostInfo {
        kind: kind.clone(),
        host: hostname.clone(),
        cli: cli.map(str::to_string),
        cli_path: None,
        authenticated: false,
        available: false,
        reason: None,
        term: term.into(),
    };
    let Some(cli) = cli else {
        info.reason = Some("provider-unsupported".into());
        return Ok(info);
    };
    let Some(p) = which(cli) else {
        info.reason = Some("cli-missing".into());
        return Ok(info);
    };
    info.cli_path = Some(p);
    let mut inv = Invocation::new(cli)
        .cwd(ctx.work_tree())
        .args(["auth", "status"])
        .timeout(Some(Duration::from_secs(20)))
        .unchecked();
    if let Some(h) = &hostname {
        inv = inv.args(["--hostname", h]);
    }
    let ok = inv.run().await.map(|o| o.ok()).unwrap_or(false);
    info.authenticated = ok;
    info.available = ok;
    if !ok {
        info.reason = Some("cli-unauthenticated".into());
    }
    Ok(info)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrInfo {
    pub number: u64,
    pub title: String,
    pub url: String,
    /// open | closed | merged
    pub state: String,
    pub draft: bool,
    pub base: String,
    pub head: String,
    pub updated_at: Option<String>,
    pub review_decision: Option<String>,
    pub host: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PrRequest {
    pub title: String,
    #[serde(default)]
    pub body: String,
    /// Base branch; default the recorded `gh-merge-base`, then the host default.
    #[serde(default)]
    pub base: Option<String>,
    #[serde(default)]
    pub draft: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrCreateResult {
    /// created | opened_existing
    pub status: String,
    pub pr: PrInfo,
}

/// Maps `gh pr view --json` output.
pub fn parse_gh_pr(v: &serde_json::Value) -> Option<PrInfo> {
    let state = v.get("state")?.as_str()?.to_ascii_lowercase();
    Some(PrInfo {
        number: v.get("number")?.as_u64()?,
        title: v.get("title").and_then(|x| x.as_str()).unwrap_or("").into(),
        url: v.get("url").and_then(|x| x.as_str()).unwrap_or("").into(),
        state: if v.get("mergedAt").map(|m| !m.is_null()).unwrap_or(false) {
            "merged".into()
        } else {
            state
        },
        draft: v.get("isDraft").and_then(|x| x.as_bool()).unwrap_or(false),
        base: v
            .get("baseRefName")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .into(),
        head: v
            .get("headRefName")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .into(),
        updated_at: v
            .get("updatedAt")
            .and_then(|x| x.as_str())
            .map(str::to_string),
        review_decision: v
            .get("reviewDecision")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        host: "github".into(),
    })
}

/// Maps `glab mr view --output json` output.
pub fn parse_glab_mr(v: &serde_json::Value) -> Option<PrInfo> {
    let state = match v.get("state")?.as_str()? {
        "opened" => "open",
        "merged" => "merged",
        _ => "closed",
    };
    Some(PrInfo {
        number: v.get("iid")?.as_u64()?,
        title: v.get("title").and_then(|x| x.as_str()).unwrap_or("").into(),
        url: v
            .get("web_url")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .into(),
        state: state.into(),
        draft: v
            .get("draft")
            .or_else(|| v.get("work_in_progress"))
            .and_then(|x| x.as_bool())
            .unwrap_or(false),
        base: v
            .get("target_branch")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .into(),
        head: v
            .get("source_branch")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .into(),
        updated_at: v
            .get("updated_at")
            .and_then(|x| x.as_str())
            .map(str::to_string),
        review_decision: None,
        host: "gitlab".into(),
    })
}

const GH_FIELDS: &str =
    "number,title,url,state,isDraft,baseRefName,headRefName,updatedAt,mergedAt,reviewDecision";

async fn require_host(path: &Path) -> VcsResult<(GitCtx, HostInfo)> {
    let ctx = ctx_of(path).await?;
    let h = host(path).await?;
    if !h.available {
        let why = h.reason.clone().unwrap_or_else(|| "unavailable".into());
        let msg = match why.as_str() {
            "no-remote" => "the repository has no remote".to_string(),
            "provider-unsupported" => format!("pull requests on {} are not supported yet", h.kind),
            "cli-missing" => format!(
                "install {} to open {}s",
                h.cli.clone().unwrap_or_default(),
                h.term
            ),
            "cli-unauthenticated" => {
                format!(
                    "log in with `{} auth login` to open {}s",
                    h.cli.clone().unwrap_or_default(),
                    h.term
                )
            }
            _ => why.clone(),
        };
        return Err(VcsError::Unavailable(format!("{why}: {msg}")));
    }
    Ok((ctx, h))
}

/// State of the PR/MR for `reference` (number, URL or branch; default the
/// current branch). `None` when there is none.
pub async fn pr_view(path: &Path, reference: Option<&str>) -> VcsResult<Option<PrInfo>> {
    let (ctx, h) = require_host(path).await?;
    if let Some(r) = reference {
        if r.starts_with('-') {
            return Err(VcsError::Invalid(format!("bad reference {r}")));
        }
    }
    let branch = {
        let o = ctx
            .git()
            .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
            .unchecked()
            .run()
            .await?;
        o.ok().then(|| o.trimmed())
    };
    let target = reference.map(str::to_string).or(branch);
    let t = Some(Duration::from_secs(30));
    match h.kind.as_str() {
        "github" => {
            let mut inv = Invocation::new("gh")
                .cwd(ctx.work_tree())
                .args(["pr", "view"]);
            if let Some(r) = &target {
                inv = inv.arg(r);
            }
            let out = inv
                .args(["--json", GH_FIELDS])
                .timeout(t)
                .unchecked()
                .run()
                .await?;
            if !out.ok() {
                let err = out.err_text().to_ascii_lowercase();
                if err.contains("no pull requests found")
                    || err.contains("no open pull requests")
                    || err.contains("could not resolve")
                {
                    return Ok(None);
                }
                return Err(VcsError::Exit {
                    program: "gh".into(),
                    op: "pr view".into(),
                    code: out.code,
                    stderr: super::runner::tail(&out.err_text(), 3, 400),
                });
            }
            let v: serde_json::Value =
                serde_json::from_slice(&out.stdout).map_err(|e| VcsError::Io(e.to_string()))?;
            Ok(parse_gh_pr(&v))
        }
        "gitlab" => {
            let mut inv = Invocation::new("glab")
                .cwd(ctx.work_tree())
                .args(["mr", "view"]);
            if let Some(r) = &target {
                inv = inv.arg(r);
            }
            let out = inv
                .args(["--output", "json"])
                .timeout(t)
                .unchecked()
                .run()
                .await?;
            if !out.ok() {
                let err = out.err_text().to_ascii_lowercase();
                if err.contains("no open merge request") || err.contains("not found") {
                    return Ok(None);
                }
                return Err(VcsError::Exit {
                    program: "glab".into(),
                    op: "mr view".into(),
                    code: out.code,
                    stderr: super::runner::tail(&out.err_text(), 3, 400),
                });
            }
            let v: serde_json::Value =
                serde_json::from_slice(&out.stdout).map_err(|e| VcsError::Io(e.to_string()))?;
            Ok(parse_glab_mr(&v))
        }
        k => Err(VcsError::Unavailable(format!("provider-unsupported: {k}"))),
    }
}

/// Opens a PR/MR for the current branch (which must be pushed). An open one
/// for the same branch is returned instead of a duplicate.
pub async fn pr_create(path: &Path, req: &PrRequest) -> VcsResult<PrCreateResult> {
    let (ctx, h) = require_host(path).await?;
    let title = req.title.trim();
    if title.is_empty() {
        return Err(VcsError::Invalid("empty title".into()));
    }
    let branch = {
        let o = ctx
            .git()
            .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
            .unchecked()
            .run()
            .await?;
        if !o.ok() {
            return Err(VcsError::Invalid(
                "detached HEAD: no branch to open a pull request from".into(),
            ));
        }
        o.trimmed()
    };
    let up = ctx
        .git()
        .args(["rev-parse", "--abbrev-ref", "@{u}"])
        .unchecked()
        .run()
        .await?;
    if !up.ok() {
        return Err(VcsError::Invalid(
            "push the branch before opening a pull request".into(),
        ));
    }
    if let Some(existing) = pr_view(path, Some(&branch)).await? {
        if existing.state == "open" {
            return Ok(PrCreateResult {
                status: "opened_existing".into(),
                pr: existing,
            });
        }
    }
    let base = match req.base.clone().filter(|b| !b.trim().is_empty()) {
        Some(b) => Some(b),
        None => repo::config_get(&ctx, &format!("branch.{branch}.gh-merge-base")).await,
    };
    if let Some(b) = &base {
        if b.starts_with('-') {
            return Err(VcsError::Invalid(format!("bad base {b}")));
        }
    }
    let body_file = std::env::temp_dir().join(format!(
        "omniget-pr-body-{}.md",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::write(&body_file, req.body.as_bytes())?;
    struct Rm(PathBuf);
    impl Drop for Rm {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _guard = Rm(body_file.clone());
    let t = Some(Duration::from_secs(120));
    let out = match h.kind.as_str() {
        "github" => {
            let mut inv = Invocation::new("gh")
                .cwd(ctx.work_tree())
                .args([
                    "pr",
                    "create",
                    "--head",
                    &branch,
                    "--title",
                    title,
                    "--body-file",
                ])
                .arg(&body_file);
            if let Some(b) = &base {
                inv = inv.args(["--base", b]);
            }
            if req.draft {
                inv = inv.arg("--draft");
            }
            inv.timeout(t).run().await?
        }
        "gitlab" => {
            let mut inv = Invocation::new("glab").cwd(ctx.work_tree()).args([
                "mr",
                "create",
                "--source-branch",
                &branch,
                "--title",
                title,
                "--description",
                &req.body,
                "--yes",
            ]);
            if let Some(b) = &base {
                inv = inv.args(["--target-branch", b]);
            }
            if req.draft {
                inv = inv.arg("--draft");
            }
            inv.timeout(t).run().await?
        }
        k => return Err(VcsError::Unavailable(format!("provider-unsupported: {k}"))),
    };
    let url = out
        .text()
        .split_whitespace()
        .rev()
        .find(|w| w.starts_with("https://") || w.starts_with("http://"))
        .map(str::to_string);
    let pr = pr_view(path, url.as_deref().or(Some(&branch)))
        .await?
        .ok_or_else(|| {
            VcsError::NotFound("the pull request was created but could not be read back".into())
        })?;
    Ok(PrCreateResult {
        status: "created".into(),
        pr,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ch(p: &str, s: &str) -> ChangeSummary {
        ChangeSummary {
            path: p.into(),
            status: s.into(),
            additions: 1,
            deletions: 0,
        }
    }

    #[test]
    fn suggestions() {
        assert_eq!(
            suggest_commit_message(&[ch("src/a.rs", "modified")]).subject,
            "Update src/a.rs"
        );
        let s =
            suggest_commit_message(&[ch("src/core/a.rs", "added"), ch("src/core/b.rs", "added")]);
        assert_eq!(s.subject, "Add src/core (2 files)");
        assert!(s.body.contains("- A src/core/a.rs (+1 -0)"));
        assert_eq!(
            suggest_commit_message(&[ch("docs/x.md", "modified"), ch("README.md", "modified")])
                .subject,
            "docs: update x.md, README.md"
        );
        assert_eq!(
            suggest_commit_message(&[ch("tests/a.rs", "deleted")]).subject,
            "test: remove tests/a.rs"
        );
    }

    #[test]
    fn pr_json() {
        let v = serde_json::json!({"number":7,"title":"T","url":"u","state":"OPEN","isDraft":true,"baseRefName":"main","headRefName":"f","mergedAt":null});
        let p = parse_gh_pr(&v).unwrap();
        assert_eq!((p.number, p.state.as_str(), p.draft), (7, "open", true));
        let m = serde_json::json!({"iid":3,"state":"merged","web_url":"w","source_branch":"f","target_branch":"main"});
        assert_eq!(parse_glab_mr(&m).unwrap().state, "merged");
    }
}
