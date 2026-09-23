//! One git worktree per thread under `<app_data>/worktrees/<repo>-<id>`, on
//! a temporary branch `omniget/<hex>` that can be renamed after the first
//! message. Setup runs in stages (fetch, checkout, submodules, setup script)
//! and reports each one; a cancel removes what was made.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::repo::{self, GitCtx};
use super::runner::{tail, Invocation, VcsError, VcsResult};

pub const BRANCH_PREFIX: &str = "omniget/";

pub fn worktrees_root() -> VcsResult<PathBuf> {
    crate::core::paths::app_data_dir()
        .map(|d| d.join("worktrees"))
        .ok_or_else(|| VcsError::Io("no app data dir".into()))
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// `path` is (inside) a linked worktree OmniGet made: under the worktrees
/// root, and its top has a `.git` *file*.
pub fn is_omniget_worktree(path: &Path) -> bool {
    let Ok(root) = worktrees_root() else {
        return false;
    };
    let root = canon(&root);
    let p = canon(path);
    let Ok(rel) = p.strip_prefix(&root) else {
        return false;
    };
    let Some(first) = rel.components().next() else {
        return false;
    };
    root.join(first.as_os_str()).join(".git").is_file()
}

/// Safe folder/branch fragment: `[a-z0-9-]`, at most `max` chars.
pub fn slug(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
        if out.len() >= max {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

/// A branch name git accepts, from free text: lowercase, `[a-z0-9/_-]`,
/// no `..`, no leading/trailing `/` or `.`, at most 64 chars.
pub fn sanitize_branch_name(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars().flat_map(|c| c.to_lowercase()) {
        let c = if c.is_ascii_alphanumeric() || c == '/' || c == '_' || c == '-' {
            c
        } else {
            '-'
        };
        if (c == '-' || c == '/') && out.ends_with(c) {
            continue;
        }
        out.push(c);
        if out.len() >= 64 {
            break;
        }
    }
    let out = out.replace("/-", "/").replace("-/", "/");
    let out = out
        .trim_matches(|c| c == '/' || c == '-' || c == '.')
        .to_string();
    let out = out.trim_end_matches(".lock").to_string();
    if out.is_empty() {
        "update".into()
    } else {
        out
    }
}

pub fn temp_branch_name() -> String {
    let hex = uuid::Uuid::new_v4().simple().to_string();
    format!("{BRANCH_PREFIX}{}", &hex[..8])
}

pub fn is_temp_branch(name: &str) -> bool {
    name.strip_prefix(BRANCH_PREFIX)
        .map(|h| h.len() == 8 && h.chars().all(|c| c.is_ascii_hexdigit()))
        .unwrap_or(false)
}

/// `<worktrees>/<repo>-<thread slug>`.
pub fn worktree_path_for(repo_root: &Path, thread: &str) -> VcsResult<PathBuf> {
    let name = repo_root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    let mut id = slug(thread, 24);
    if id.is_empty() {
        id = super::checkpoint::thread_key(thread)
            .chars()
            .take(16)
            .collect();
    }
    let mut repo = slug(&name, 40);
    if repo.is_empty() {
        repo = "repo".into();
    }
    Ok(worktrees_root()?.join(format!("{repo}-{id}")))
}

// ---- project config (.omniget/project.json, t3.json) ----

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectScript {
    pub name: String,
    pub command: String,
    pub run_on_worktree_create: bool,
    /// Runs without blocking the agent (reported, not awaited by callers that care).
    pub run_async: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectConfig {
    /// Which file it came from (`.omniget/project.json` or `t3.json`).
    pub source: Option<String>,
    /// recursive | top-level | none
    pub worktree_submodules: Option<String>,
    /// local | worktree
    pub default_thread_env_mode: Option<String>,
    pub scripts: Vec<ProjectScript>,
}

/// Parses either file (same shape). Never fails: bad JSON is an empty config.
pub fn parse_project_config(text: &str) -> ProjectConfig {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else {
        return ProjectConfig::default();
    };
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_string);
    let scripts = v
        .get("scripts")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .take(50)
                .filter_map(|e| {
                    let command = e.get("command")?.as_str()?.trim().to_string();
                    if command.is_empty() {
                        return None;
                    }
                    let b = |k: &str, snake: &str| {
                        e.get(k)
                            .or_else(|| e.get(snake))
                            .and_then(|x| x.as_bool())
                            .unwrap_or(false)
                    };
                    Some(ProjectScript {
                        name: e
                            .get("name")
                            .and_then(|x| x.as_str())
                            .unwrap_or("setup")
                            .to_string(),
                        command,
                        run_on_worktree_create: b("runOnWorktreeCreate", "run_on_worktree_create"),
                        run_async: b("async", "async"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    ProjectConfig {
        source: None,
        worktree_submodules: s("worktreeSubmodules").or_else(|| s("worktree_submodules")),
        default_thread_env_mode: s("defaultThreadEnvMode").or_else(|| s("default_thread_env_mode")),
        scripts,
    }
}

pub fn read_project_config(dir: &Path) -> ProjectConfig {
    let candidates = [
        dir.join(".omniget").join("project.json"),
        dir.join("t3.json"),
    ];
    for p in candidates {
        if let Ok(text) = std::fs::read_to_string(&p) {
            let mut c = parse_project_config(&text);
            c.source = p
                .strip_prefix(dir)
                .ok()
                .map(|r| r.to_string_lossy().into_owned());
            return c;
        }
    }
    ProjectConfig::default()
}

// ---- setup with progress ----

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetupStage {
    /// fetch | checkout | submodules | setup-script
    pub id: String,
    /// pending | running | done | skipped | warning | failed
    pub state: String,
    pub detail: Option<String>,
    pub percent: Option<u8>,
    pub tail: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SetupSnapshot {
    pub thread: String,
    /// running | done | failed | cancelled
    pub phase: String,
    pub path: Option<PathBuf>,
    pub branch: Option<String>,
    pub base: Option<String>,
    pub stages: Vec<SetupStage>,
    pub error: Option<String>,
}

impl SetupSnapshot {
    fn new(thread: &str) -> Self {
        let stage = |id: &str| SetupStage {
            id: id.into(),
            state: "pending".into(),
            detail: None,
            percent: None,
            tail: vec![],
        };
        Self {
            thread: thread.into(),
            phase: "running".into(),
            path: None,
            branch: None,
            base: None,
            stages: vec![
                stage("fetch"),
                stage("checkout"),
                stage("submodules"),
                stage("setup-script"),
            ],
            error: None,
        }
    }
    fn stage(&mut self, id: &str) -> &mut SetupStage {
        self.stages
            .iter_mut()
            .find(|s| s.id == id)
            .expect("known stage")
    }
    fn set(&mut self, id: &str, state: &str, detail: Option<String>) {
        let s = self.stage(id);
        s.state = state.into();
        if detail.is_some() {
            s.detail = detail;
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct WorktreeRequest {
    /// Any path inside the project's repository.
    pub repo: PathBuf,
    pub thread: String,
    /// Base branch or commit; default the repository's current branch.
    #[serde(default)]
    pub base: Option<String>,
    /// Branch to create; default `omniget/<hex>`.
    #[serde(default)]
    pub branch: Option<String>,
    /// Fetch `origin/<base>` first and start from it.
    #[serde(default)]
    pub start_from_origin: bool,
    /// recursive | top-level | none (default: project config, then recursive)
    #[serde(default)]
    pub submodules: Option<String>,
    /// Run `runOnWorktreeCreate` scripts from the project config.
    #[serde(default)]
    pub run_setup: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub thread: String,
    pub path: PathBuf,
    pub branch: String,
    pub base: String,
    pub base_commit: String,
    pub repo_root: PathBuf,
    pub reused: bool,
}

/// Parses `Updating files:  78% (2104/2700)` style progress.
pub fn parse_percent(line: &str) -> Option<u8> {
    let idx = line.find('%')?;
    let digits: String = line[..idx]
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    digits.parse::<u32>().ok().map(|n| n.min(100) as u8)
}

async fn current_branch(ctx: &GitCtx) -> VcsResult<Option<String>> {
    let out = ctx
        .git()
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .unchecked()
        .run()
        .await?;
    Ok(out.ok().then(|| out.trimmed()))
}

async fn branch_exists(ctx: &GitCtx, name: &str) -> VcsResult<bool> {
    Ok(ctx
        .git()
        .args([
            "show-ref",
            "--verify",
            "--quiet",
            &format!("refs/heads/{name}"),
        ])
        .unchecked()
        .run()
        .await?
        .ok())
}

fn main_ctx(common_dir: &Path) -> GitCtx {
    // Non-bare repo: the common dir is `<main>/.git`.
    let root = if common_dir.file_name().map(|n| n == ".git").unwrap_or(false) {
        common_dir
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| common_dir.to_path_buf())
    } else {
        common_dir.to_path_buf()
    };
    GitCtx::Repo { work_tree: root }
}

/// Creates (or reuses) the thread's worktree, reporting every stage through
/// `on_progress`. `cancel` is checked between stages; a cancel removes the
/// half-made worktree and its branch.
pub async fn create(
    req: &WorktreeRequest,
    cancel: Arc<AtomicBool>,
    on_progress: impl Fn(&SetupSnapshot) + Send + Sync,
) -> VcsResult<WorktreeInfo> {
    let mut snap = SetupSnapshot::new(&req.thread);
    let result = create_inner(req, &cancel, &mut snap, &on_progress).await;
    match &result {
        Ok(_) => snap.phase = "done".into(),
        Err(VcsError::Cancelled) => snap.phase = "cancelled".into(),
        Err(e) => {
            snap.phase = "failed".into();
            snap.error = Some(e.to_string());
            for s in snap.stages.iter_mut() {
                if s.state == "running" {
                    s.state = "failed".into();
                }
            }
        }
    }
    on_progress(&snap);
    result
}

async fn create_inner(
    req: &WorktreeRequest,
    cancel: &AtomicBool,
    snap: &mut SetupSnapshot,
    emit: &(impl Fn(&SetupSnapshot) + Send + Sync),
) -> VcsResult<WorktreeInfo> {
    if req.thread.trim().is_empty() {
        return Err(VcsError::Invalid("thread id is empty".into()));
    }
    let paths = repo::discover(&req.repo).await?;
    let ctx = GitCtx::Repo {
        work_tree: paths.root.clone(),
    };
    let path = worktree_path_for(&paths.root, &req.thread)?;
    emit(snap);

    // An existing worktree for this thread is reused as is.
    if path.join(".git").is_file() {
        let wctx = GitCtx::Repo {
            work_tree: path.clone(),
        };
        let branch = current_branch(&wctx).await?.unwrap_or_default();
        let base = repo::config_get(&wctx, &format!("branch.{branch}.gh-merge-base"))
            .await
            .unwrap_or_default();
        let head = wctx
            .git()
            .args(["rev-parse", "HEAD"])
            .run()
            .await?
            .trimmed();
        for s in ["fetch", "checkout", "submodules", "setup-script"] {
            snap.set(s, "skipped", Some("worktree already exists".into()));
        }
        snap.path = Some(path.clone());
        snap.branch = Some(branch.clone());
        return Ok(WorktreeInfo {
            thread: req.thread.clone(),
            path,
            branch,
            base,
            base_commit: head,
            repo_root: paths.root,
            reused: true,
        });
    }
    if path.exists() {
        return Err(VcsError::Unsafe(format!(
            "{} exists and is not a worktree",
            path.display()
        )));
    }

    let has_commit = ctx
        .git()
        .args(["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
        .unchecked()
        .run()
        .await?
        .ok();
    let base = match &req.base {
        Some(b) if !b.trim().is_empty() => b.trim().to_string(),
        _ => current_branch(&ctx).await?.unwrap_or_else(|| "HEAD".into()),
    };
    if base.starts_with('-') {
        return Err(VcsError::Invalid(format!("bad base {base}")));
    }
    if !has_commit && req.base.is_none() {
        return Err(VcsError::Invalid(
            "the repository has no commit to start a worktree from".into(),
        ));
    }
    snap.base = Some(base.clone());

    // fetch
    let mut start = base.clone();
    if req.start_from_origin && base != "HEAD" {
        match repo::primary_remote(&ctx).await? {
            Some(remote) => {
                snap.set("fetch", "running", Some(format!("{}/{base}", remote.name)));
                emit(snap);
                let fetched = ctx
                    .git()
                    .args(["fetch", "--quiet", "--no-tags", &remote.name, &base])
                    .timeout(Some(Duration::from_secs(120)))
                    .unchecked()
                    .run()
                    .await?;
                let remote_ref = format!("refs/remotes/{}/{base}", remote.name);
                let sha = ctx
                    .git()
                    .args([
                        "rev-parse",
                        "--verify",
                        "--quiet",
                        &format!("{remote_ref}^{{commit}}"),
                    ])
                    .unchecked()
                    .run()
                    .await?;
                if fetched.ok() && sha.ok() {
                    let s = sha.trimmed();
                    snap.set(
                        "fetch",
                        "done",
                        Some(format!(
                            "{}/{base} at {}",
                            remote.name,
                            &s[..s.len().min(7)]
                        )),
                    );
                    start = s;
                } else {
                    snap.set(
                        "fetch",
                        "warning",
                        Some(format!(
                            "{}/{base} not found, using local branch",
                            remote.name
                        )),
                    );
                }
            }
            None => snap.set("fetch", "skipped", Some("no remote".into())),
        }
    } else {
        snap.set("fetch", "skipped", None);
    }
    emit(snap);
    if cancel.load(Ordering::SeqCst) {
        return Err(VcsError::Cancelled);
    }

    let base_commit = ctx
        .git()
        .args(["rev-parse", "--verify", &format!("{start}^{{commit}}")])
        .run()
        .await
        .map_err(|_| VcsError::NotFound(format!("base {start} is not a commit")))?
        .trimmed();

    // checkout
    let mut branch = match &req.branch {
        Some(b) if !b.trim().is_empty() => sanitize_branch_name(b),
        _ => temp_branch_name(),
    };
    let mut n = 2;
    let stem = branch.clone();
    while branch_exists(&ctx, &branch).await? {
        branch = format!("{stem}-{n}");
        n += 1;
    }
    snap.branch = Some(branch.clone());
    snap.path = Some(path.clone());
    snap.set("checkout", "running", Some(branch.clone()));
    emit(snap);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let add = {
        let progress = std::sync::Mutex::new((None::<u8>, std::time::Instant::now()));
        let snap_cell = std::sync::Mutex::new(snap.clone());
        let r = ctx
            .git()
            .env("GIT_PROGRESS_DELAY", "0")
            .args(["worktree", "add", "-b", &branch])
            .arg(&path)
            .arg(&base_commit)
            .timeout(Some(Duration::from_secs(600)))
            .run_streaming(|line| {
                if let Some(pct) = parse_percent(line) {
                    let mut p = progress.lock().unwrap_or_else(|e| e.into_inner());
                    if p.0 != Some(pct)
                        && (p.1.elapsed() > Duration::from_millis(100) || pct == 100)
                    {
                        *p = (Some(pct), std::time::Instant::now());
                        let mut s = snap_cell.lock().unwrap_or_else(|e| e.into_inner());
                        s.stage("checkout").percent = Some(pct);
                        emit(&s);
                    }
                }
            })
            .await;
        r
    };
    if let Err(e) = add {
        if path.exists() && !path.join(".git").exists() {
            let _ = std::fs::remove_dir_all(&path);
        }
        let _ = ctx
            .git()
            .args(["worktree", "prune"])
            .unchecked()
            .run()
            .await;
        return Err(e);
    }
    snap.stage("checkout").percent = Some(100);
    snap.set(
        "checkout",
        "done",
        Some(format!(
            "{branch} from {}",
            &base_commit[..base_commit.len().min(7)]
        )),
    );
    emit(snap);
    let wctx = GitCtx::Repo {
        work_tree: path.clone(),
    };
    if base != "HEAD" && !base.chars().all(|c| c.is_ascii_hexdigit()) {
        let _ = wctx
            .git()
            .args(["config", &format!("branch.{branch}.gh-merge-base"), &base])
            .run()
            .await;
    }
    let _ = wctx
        .git()
        .args([
            "config",
            &format!("branch.{branch}.omniget-thread"),
            &req.thread,
        ])
        .run()
        .await;

    let rollback = |path: PathBuf, branch: String, ctx: GitCtx| async move {
        let _ = ctx
            .git()
            .args(["worktree", "remove", "--force"])
            .arg(&path)
            .unchecked()
            .run()
            .await;
        let _ = ctx
            .git()
            .args(["branch", "-D", &branch])
            .unchecked()
            .run()
            .await;
    };
    if cancel.load(Ordering::SeqCst) {
        rollback(path.clone(), branch.clone(), ctx.clone()).await;
        return Err(VcsError::Cancelled);
    }

    // submodules
    let config = read_project_config(&path);
    if path.join(".gitmodules").is_file() {
        let mode = req
            .submodules
            .clone()
            .or_else(|| config.worktree_submodules.clone())
            .unwrap_or_else(|| "recursive".into());
        if mode == "none" {
            snap.set("submodules", "skipped", Some("disabled".into()));
        } else {
            snap.set("submodules", "running", None);
            emit(snap);
            let mut inv = wctx.git().args(["submodule", "update", "--init"]);
            if mode != "top-level" {
                inv = inv.arg("--recursive");
            }
            let last = std::sync::Mutex::new(String::new());
            let r = inv
                .timeout(Some(Duration::from_secs(900)))
                .unchecked()
                .run_streaming(|line| {
                    if let Some(rest) = line.strip_prefix("Submodule path '") {
                        *last.lock().unwrap_or_else(|e| e.into_inner()) =
                            rest.split('\'').next().unwrap_or("").to_string();
                    }
                })
                .await;
            match r {
                Ok(o) if o.ok() => {
                    let l = last.into_inner().unwrap_or_else(|e| e.into_inner());
                    snap.set("submodules", "done", (!l.is_empty()).then_some(l));
                }
                Ok(o) => snap.set("submodules", "warning", Some(tail(&o.err_text(), 2, 300))),
                Err(e) => snap.set("submodules", "warning", Some(e.to_string())),
            }
        }
    } else {
        snap.set("submodules", "skipped", Some("no submodules".into()));
    }
    emit(snap);
    if cancel.load(Ordering::SeqCst) {
        rollback(path.clone(), branch.clone(), ctx.clone()).await;
        return Err(VcsError::Cancelled);
    }

    // setup script
    let scripts: Vec<&ProjectScript> = config
        .scripts
        .iter()
        .filter(|s| s.run_on_worktree_create)
        .collect();
    if !req.run_setup || scripts.is_empty() {
        snap.set(
            "setup-script",
            "skipped",
            Some(if scripts.is_empty() {
                "no setup script".into()
            } else {
                "not requested".into()
            }),
        );
    } else {
        let mut failed = None;
        for s in scripts {
            snap.set("setup-script", "running", Some(s.name.clone()));
            emit(snap);
            let lines = std::sync::Mutex::new(Vec::<String>::new());
            let r = shell(&s.command)
                .cwd(&path)
                .env("OMNIGET_PROJECT_ROOT", &paths.root)
                .env("OMNIGET_WORKTREE_PATH", &path)
                .env("T3CODE_PROJECT_ROOT", &paths.root)
                .env("T3CODE_WORKTREE_PATH", &path)
                .env("FORCE_COLOR", "0")
                .timeout(Some(Duration::from_secs(1800)))
                .unchecked()
                .run_streaming(|line| {
                    let clean: String = strip_ansi(line).chars().take(400).collect();
                    let mut l = lines.lock().unwrap_or_else(|e| e.into_inner());
                    l.push(clean);
                    if l.len() > 4 {
                        l.remove(0);
                    }
                })
                .await;
            snap.stage("setup-script").tail = lines.into_inner().unwrap_or_default();
            match r {
                Ok(o) if o.ok() => {}
                Ok(o) => {
                    failed = Some(format!(
                        "{} exited with {}",
                        s.name,
                        o.code
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| "signal".into())
                    ));
                    break;
                }
                Err(e) => {
                    failed = Some(e.to_string());
                    break;
                }
            }
            if cancel.load(Ordering::SeqCst) {
                rollback(path.clone(), branch.clone(), ctx.clone()).await;
                return Err(VcsError::Cancelled);
            }
        }
        match failed {
            None => snap.set("setup-script", "done", None),
            Some(m) => snap.set("setup-script", "failed", Some(m)),
        }
    }
    emit(snap);

    Ok(WorktreeInfo {
        thread: req.thread.clone(),
        path,
        branch,
        base,
        base_commit,
        repo_root: paths.root,
        reused: false,
    })
}

/// A command line run through the user's shell, stdout folded into stderr so
/// the streaming reader sees every line.
fn shell(command: &str) -> Invocation {
    let inv = if cfg!(windows) {
        Invocation::new("cmd")
            .args(["/D", "/S", "/C"])
            .arg(format!("({command}) 1>&2"))
    } else {
        let sh = std::env::var("SHELL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "sh".into());
        Invocation::new(&sh)
            .arg("-c")
            .arg(format!("exec 1>&2\n{command}"))
    };
    // The user's own locale for their script, not the C locale git calls use.
    let mut inv = inv.env_remove("GIT_EDITOR").env_remove("PAGER");
    for k in ["LC_ALL", "LANG"] {
        inv = match std::env::var_os(k) {
            Some(v) => inv.env(k, v),
            None => inv.env_remove(k),
        };
    }
    inv
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                for d in it.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Renames the worktree's branch (`git branch -m`), keeping it unique.
/// Returns the final name.
pub async fn rename_branch(worktree: &Path, new_name: &str) -> VcsResult<String> {
    let ctx = GitCtx::Repo {
        work_tree: repo::discover(worktree).await?.root,
    };
    let old = current_branch(&ctx)
        .await?
        .ok_or_else(|| VcsError::Invalid("the worktree is on a detached HEAD".into()))?;
    let stem = sanitize_branch_name(new_name);
    if stem == old {
        return Ok(old);
    }
    let mut name = stem.clone();
    let mut n = 2;
    while branch_exists(&ctx, &name).await? {
        name = format!("{stem}-{n}");
        n += 1;
    }
    ctx.git()
        .args(["branch", "-m", "--", &old, &name])
        .run()
        .await?;
    Ok(name)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemoveResult {
    pub path: PathBuf,
    pub removed: bool,
    pub branch_deleted: Option<String>,
    pub checkpoints_deleted: u32,
}

/// Removes an OmniGet worktree (never a main checkout or anything outside the
/// worktrees root). `delete_branch` drops its branch too: `-D` for a temporary
/// `omniget/<hex>` branch, the safe `-d` for a renamed one.
pub async fn remove(
    path: &Path,
    force: bool,
    delete_branch: bool,
    thread: Option<&str>,
) -> VcsResult<RemoveResult> {
    if !is_omniget_worktree(path) {
        return Err(VcsError::Unsafe(format!(
            "{} is not an OmniGet worktree",
            path.display()
        )));
    }
    let paths = repo::discover(path).await?;
    let wctx = GitCtx::Repo {
        work_tree: paths.root.clone(),
    };
    let branch = current_branch(&wctx).await?;
    let main = main_ctx(&paths.common_dir);
    let mut checkpoints_deleted = 0;
    if let Some(t) = thread {
        checkpoints_deleted = super::checkpoint::delete_all(&paths.root, t)
            .await
            .unwrap_or(0);
    }
    let mut last = None;
    for attempt in 0..5 {
        let mut inv = main.git().args(["worktree", "remove"]);
        if force {
            inv = inv.arg("--force");
        }
        match inv
            .arg(&paths.root)
            .timeout(Some(Duration::from_secs(300)))
            .run()
            .await
        {
            Ok(_) => {
                last = None;
                break;
            }
            Err(e) => {
                if !paths.root.exists() {
                    last = None;
                    break;
                }
                // A dirty tree without `force` is a user decision, not a race.
                if !force {
                    return Err(e);
                }
                last = Some(e);
                if attempt < 4 {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
    }
    if let Some(e) = last {
        return Err(e);
    }
    let _ = main
        .git()
        .args(["worktree", "prune"])
        .unchecked()
        .run()
        .await;
    let mut branch_deleted = None;
    if delete_branch {
        if let Some(b) = branch {
            let flag = if is_temp_branch(&b) || force {
                "-D"
            } else {
                "-d"
            };
            if main
                .git()
                .args(["branch", flag, "--", &b])
                .unchecked()
                .run()
                .await?
                .ok()
            {
                branch_deleted = Some(b);
            }
        }
    }
    Ok(RemoveResult {
        path: paths.root,
        removed: true,
        branch_deleted,
        checkpoints_deleted,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
    pub thread: Option<String>,
    pub omniget: bool,
    pub prunable: bool,
    pub main: bool,
}

/// Worktrees of the repository that contains `repo_path`.
pub async fn list(repo_path: &Path) -> VcsResult<Vec<WorktreeEntry>> {
    let paths = repo::discover(repo_path).await?;
    let ctx = GitCtx::Repo {
        work_tree: paths.root,
    };
    let out = ctx
        .git()
        .args(["worktree", "list", "--porcelain", "-z"])
        .run()
        .await?;
    let mut list = Vec::new();
    let mut cur: Option<WorktreeEntry> = None;
    for rec in out.stdout.split(|b| *b == 0) {
        let rec = String::from_utf8_lossy(rec);
        if rec.is_empty() {
            if let Some(e) = cur.take() {
                list.push(e);
            }
            continue;
        }
        if let Some(p) = rec.strip_prefix("worktree ") {
            if let Some(e) = cur.take() {
                list.push(e);
            }
            let path = PathBuf::from(p);
            cur = Some(WorktreeEntry {
                omniget: is_omniget_worktree(&path),
                main: list.is_empty(),
                path,
                head: None,
                branch: None,
                thread: None,
                prunable: false,
            });
        } else if let Some(e) = cur.as_mut() {
            if let Some(h) = rec.strip_prefix("HEAD ") {
                e.head = Some(h.to_string());
            } else if let Some(b) = rec.strip_prefix("branch ") {
                e.branch = Some(b.trim_start_matches("refs/heads/").to_string());
            } else if rec.starts_with("prunable") {
                e.prunable = true;
            }
        }
    }
    if let Some(e) = cur.take() {
        list.push(e);
    }
    for e in list.iter_mut() {
        if let Some(b) = &e.branch {
            e.thread = repo::config_get(&ctx, &format!("branch.{b}.omniget-thread")).await;
        }
    }
    Ok(list)
}

/// `git worktree prune` for the repository, plus removal of folders under the
/// worktrees root that belong to this repository but git no longer knows
/// (a crash between `mkdir` and `worktree add`). Returns the folders removed.
pub async fn cleanup(repo_path: &Path) -> VcsResult<Vec<PathBuf>> {
    let paths = repo::discover(repo_path).await?;
    let ctx = GitCtx::Repo {
        work_tree: paths.root.clone(),
    };
    ctx.git().args(["worktree", "prune"]).run().await?;
    let known: Vec<PathBuf> = list(&paths.root)
        .await?
        .into_iter()
        .map(|e| canon(&e.path))
        .collect();
    let root = worktrees_root()?;
    let prefix = format!(
        "{}-",
        slug(
            &paths
                .root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            40
        )
    );
    let mut removed = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with(&prefix) || known.contains(&canon(&p)) {
                continue;
            }
            // Only folders git left behind: no `.git` file at all, and empty
            // (a leftover `mkdir`). Anything with content stays for the user.
            let empty = std::fs::read_dir(&p)
                .map(|mut r| r.next().is_none())
                .unwrap_or(false);
            if p.is_dir() && !p.join(".git").exists() && empty && std::fs::remove_dir(&p).is_ok() {
                removed.push(p);
            }
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branch_names() {
        assert_eq!(
            sanitize_branch_name("Fix: the Login bug!"),
            "fix-the-login-bug"
        );
        assert_eq!(sanitize_branch_name("feature//x..y"), "feature/x-y");
        assert_eq!(sanitize_branch_name("  "), "update");
        assert!(is_temp_branch(&temp_branch_name()));
        assert!(!is_temp_branch("omniget/fix-login"));
    }

    #[test]
    fn percent_and_config() {
        assert_eq!(parse_percent("Updating files:  78% (2104/2700)"), Some(78));
        assert_eq!(parse_percent("no progress"), None);
        let c = parse_project_config(
            r#"{"worktreeSubmodules":"none","scripts":[{"name":"deps","command":"pnpm i","runOnWorktreeCreate":true},{"name":"x","command":""}]}"#,
        );
        assert_eq!(c.worktree_submodules.as_deref(), Some("none"));
        assert_eq!(c.scripts.len(), 1);
        assert!(c.scripts[0].run_on_worktree_create);
        assert_eq!(parse_project_config("not json"), ProjectConfig::default());
    }
}
