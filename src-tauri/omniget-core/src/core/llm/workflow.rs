//! `.omniget/workflow.md`: the project's contract for delegated work (item
//! 3.10 of the `/llm` roadmap, S12 phase 2, 83#P6).
//!
//! A versioned file in the user's repository, YAML on top and a prompt
//! template below:
//!
//! ```text
//! ---
//! agent: claude-code
//! run_mode: auto_edits
//! isolation: worktree
//! deadline_min: 30
//! checks: [cargo test]
//! hooks:
//!   after_create: pnpm install --frozen-lockfile
//!   before_remove: ./scripts/save-logs.sh
//! include: [.env.local]
//! ---
//! You work on {{repo}}, branch {{branch}}.
//!
//! {{task}}
//! ```
//!
//! Every job started in that repository (the CLI, `/v1/commands`, MCP
//! `agent_run`, the window) takes these values for whatever the caller left
//! out, and its prompt goes through the template.
//!
//! - **Strict template.** `{{name}}` must be one of [`VARIABLES`]; anything
//!   else makes the whole file invalid at load time (not at the first job),
//!   and the template must use `{{task}}`, so the task can never be dropped.
//! - **Last known good.** The file is read again every time it is used (job
//!   start, the jobs page, the CLI), so an edit applies at once without a
//!   watcher. An edit that does not parse leaves the previous valid version
//!   in effect (kept in `<llm dir>/workflows.db`, so it survives a restart)
//!   and the error is shown next to it; with no previous version, a job that
//!   would use the file is refused with the error instead of silently
//!   running without it.
//! - **Trust by SHA-256.** Opening someone else's repository must never run
//!   its code (Symphony treats its hooks as "fully trusted", `SPEC.md:1762`;
//!   OmniGet does not). The hooks, the checks and a `run_mode` looser than
//!   `approval_required` apply only while the SHA-256 of the version in
//!   effect is the one the person approved (in the window or with `omniget
//!   agent workflow trust`); any change to the file needs a new approval.
//!   Untrusted, they are skipped and the job says why. The rest (agent,
//!   isolation, deadline, template…) only chooses among things the caller
//!   could have asked for anyway, so it applies without trust.
//!
//! Ideas from openai/symphony (`WORKFLOW.md`, Apache-2.0); no code copied.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Where the contract lives, relative to the repository root.
pub const FILE: &str = ".omniget/workflow.md";
pub const DB_FILE: &str = "workflows.db";
/// The file does not parse and there is no earlier valid version.
pub const ERR_INVALID: &str = "ERR_WORKFLOW_INVALID";
/// A template variable had no value when a job's prompt was rendered.
pub const ERR_TEMPLATE: &str = "template_render_error";
/// A repository hook failed (or timed out).
pub const ERR_HOOK: &str = "ERR_WORKFLOW_HOOK";
/// The trust asked for does not match the version in effect.
pub const ERR_TRUST: &str = "ERR_WORKFLOW_TRUST";
/// Longest a hook may run (the roadmap's 120 s); then its whole process
/// tree is killed and the hook counts as failed.
pub const HOOK_TIMEOUT: Duration = Duration::from_secs(120);
/// Longest a check (`checks:`) may run after a job.
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(600);
const MAX_BYTES: u64 = 64 * 1024;
const OUTPUT_TAIL: usize = 4000;

/// The template's variables, with what each one holds.
pub const VARIABLES: &[(&str, &str)] = &[
    ("task", "the task as the caller wrote it"),
    (
        "title",
        "the job's short name (the task's first line when none)",
    ),
    ("agent", "the roster id of the agent that runs it"),
    ("workspace", "the folder the job was started in"),
    ("repo", "the repository root (where this file lives)"),
    (
        "branch",
        "the branch the work starts from (the base, else the checked-out one)",
    ),
    ("base", "the base ref asked for, empty when none"),
    ("isolation", "shared, worktree or auto"),
    (
        "run_mode",
        "approval_required, auto_edits, auto or full_access",
    ),
    ("files", "the `include` paths, one per line"),
];

// ── the file ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hooks {
    /// Runs in a job's new worktree, before the agent starts. A failure
    /// fails the job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_create: Option<String>,
    /// Runs in a job's worktree before it is removed. A failure keeps the
    /// worktree (removal is blocked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_remove: Option<String>,
}

/// The YAML front matter. Unknown keys are an error (a typo must not be
/// silently ignored).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, alias = "deadline", skip_serializing_if = "Option::is_none")]
    pub deadline_min: Option<u32>,
    #[serde(default, alias = "budget", skip_serializing_if = "Option::is_none")]
    pub budget_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Commands run in the job's folder after it ends; their exit codes go
    /// into the job's proof.
    #[serde(
        default,
        alias = "verify",
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub checks: Vec<String>,
    #[serde(default)]
    pub hooks: Hooks,
    /// Repo-relative files the job needs: copied into its worktree when git
    /// ignores them (like `.worktreeinclude`) and listed in `{{files}}`.
    #[serde(
        default,
        deserialize_with = "one_or_many",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub include: Vec<String>,
}

fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Option::<OneOrMany>::deserialize(d)? {
        None => Vec::new(),
        Some(OneOrMany::One(s)) => vec![s],
        Some(OneOrMany::Many(v)) => v,
    })
}

#[derive(Debug, Clone, PartialEq)]
enum Part {
    Text(String),
    Var(String),
}

/// A parsed, valid `.omniget/workflow.md`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Workflow {
    pub config: Config,
    /// The template as written (the body under the front matter).
    pub template: String,
    /// Of the whole file, as bytes on disk.
    pub sha256: String,
    #[serde(skip)]
    parts: Vec<Part>,
}

/// A repository command the file asks OmniGet to run.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RepoCommand {
    /// `after_create | before_remove | check`.
    pub kind: &'static str,
    pub command: String,
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

fn is_agent_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

/// Front matter and body, or the whole text as the body when the file does
/// not start with a `---` line.
fn split(text: &str) -> Result<(&str, &str), String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    let Some(first) = lines.next() else {
        return Ok(("", ""));
    };
    if first.trim_end() != "---" {
        return Ok(("", text));
    }
    let start = first.len();
    let mut at = start;
    for line in lines {
        let t = line.trim_end();
        if t == "---" || t == "..." {
            return Ok((&text[start..at], &text[at + line.len()..]));
        }
        at += line.len();
    }
    Err("the front matter opened with `---` is never closed by another `---` line".into())
}

fn parse_template(body: &str) -> Result<Vec<Part>, String> {
    let known: Vec<&str> = VARIABLES.iter().map(|(n, _)| *n).collect();
    let mut parts = Vec::new();
    let mut rest = body;
    let mut consumed = 0usize;
    let line_at = |offset: usize| body[..offset].matches('\n').count() + 1;
    while let Some(open) = rest.find("{{") {
        if open > 0 {
            parts.push(Part::Text(rest[..open].to_string()));
        }
        let after = &rest[open + 2..];
        let Some(close) = after.find("}}") else {
            return Err(format!(
                "template, line {}: `{{{{` is never closed by `}}}}`",
                line_at(consumed + open)
            ));
        };
        let name = after[..close].trim();
        let ident = !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !ident {
            return Err(format!(
                "template, line {}: `{{{{{}}}}}` is not a variable (known: {})",
                line_at(consumed + open),
                &after[..close],
                known.join(", ")
            ));
        }
        if !known.contains(&name) {
            return Err(format!(
                "template, line {}: unknown variable `{name}` (known: {})",
                line_at(consumed + open),
                known.join(", ")
            ));
        }
        parts.push(Part::Var(name.to_string()));
        let step = open + 2 + close + 2;
        consumed += step;
        rest = &rest[step..];
    }
    if !rest.is_empty() {
        parts.push(Part::Text(rest.to_string()));
    }
    if !parts
        .iter()
        .any(|p| matches!(p, Part::Var(v) if v == "task"))
    {
        return Err(
            "template: it must use {{task}} (else the task would never reach the agent)".into(),
        );
    }
    Ok(parts)
}

fn check_config(c: &Config) -> Result<(), String> {
    if let Some(a) = &c.agent {
        if !is_agent_id(a) {
            return Err(format!("agent: `{a}` is not a roster id"));
        }
    }
    if let Some(m) = &c.run_mode {
        crate::core::llm::run_mode::RunMode::parse(m).map_err(|e| format!("run_mode: {e}"))?;
    }
    if let Some(i) = &c.isolation {
        if !matches!(i.as_str(), "shared" | "worktree" | "auto") {
            return Err(format!("isolation: shared, worktree or auto (got `{i}`)"));
        }
    }
    if let Some(b) = &c.base {
        if b.trim().is_empty() || b.starts_with('-') || b.chars().any(char::is_whitespace) {
            return Err(format!("base: `{b}` is not a ref"));
        }
        if c.isolation.as_deref() == Some("shared") {
            return Err("base: a base ref needs isolation worktree or auto".into());
        }
    }
    if let Some(d) = c.deadline_min {
        if !(1..=1440).contains(&d) {
            return Err("deadline_min: 1–1440 minutes".into());
        }
    }
    if let Some(b) = c.budget_usd {
        if !(b > 0.0 && b <= 1000.0) {
            return Err("budget_usd: between US$ 0 and 1000".into());
        }
    }
    if let Some(m) = &c.model {
        if m.trim().is_empty() || m.len() > 100 {
            return Err("model: 1–100 characters".into());
        }
    }
    if let Some(e) = &c.effort {
        if !matches!(e.as_str(), "low" | "medium" | "high" | "xhigh" | "max") {
            return Err("effort: low, medium, high, xhigh or max".into());
        }
    }
    for (what, cmd) in [
        ("hooks.after_create", &c.hooks.after_create),
        ("hooks.before_remove", &c.hooks.before_remove),
    ] {
        if cmd.as_deref().is_some_and(|s| s.trim().is_empty()) {
            return Err(format!("{what}: empty command"));
        }
    }
    if c.checks.iter().any(|s| s.trim().is_empty()) {
        return Err("checks: empty command".into());
    }
    for p in &c.include {
        let clean = crate::core::vcs::job_worktree::parse_worktree_include(p);
        if clean.len() != 1 {
            return Err(format!(
                "include: `{p}` must be a plain path inside the repository (no glob, `..` or absolute path)"
            ));
        }
    }
    Ok(())
}

/// Parses and validates the whole file.
pub fn parse(text: &str) -> Result<Workflow, String> {
    let (yaml, body) = split(text)?;
    let config: Config = if yaml.trim().is_empty() {
        Config::default()
    } else {
        serde_yaml::from_str(yaml).map_err(|e| format!("front matter: {e}"))?
    };
    check_config(&config)?;
    let body = body.trim();
    let template = if body.is_empty() { "{{task}}" } else { body };
    let parts = parse_template(template)?;
    Ok(Workflow {
        config,
        template: template.to_string(),
        sha256: sha256_hex(text.as_bytes()),
        parts,
    })
}

impl Workflow {
    /// Fills the template. Every variable must have a value (strict): a
    /// missing one is [`ERR_TEMPLATE`], never an empty string.
    pub fn render(&self, vars: &BTreeMap<&str, String>) -> Result<String, String> {
        let mut out = String::new();
        for p in &self.parts {
            match p {
                Part::Text(t) => out.push_str(t),
                Part::Var(v) => out.push_str(
                    vars.get(v.as_str())
                        .ok_or_else(|| format!("{ERR_TEMPLATE}: no value for `{{{{{v}}}}}`"))?,
                ),
            }
        }
        Ok(out)
    }

    /// The repository commands this version asks to run.
    pub fn commands(&self) -> Vec<RepoCommand> {
        let mut out = Vec::new();
        if let Some(c) = &self.config.hooks.after_create {
            out.push(RepoCommand {
                kind: "after_create",
                command: c.clone(),
            });
        }
        if let Some(c) = &self.config.hooks.before_remove {
            out.push(RepoCommand {
                kind: "before_remove",
                command: c.clone(),
            });
        }
        for c in &self.config.checks {
            out.push(RepoCommand {
                kind: "check",
                command: c.clone(),
            });
        }
        out
    }

    /// What waits for the person's trust: the commands, and a run mode that
    /// lets the agent act without asking.
    pub fn gated(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .commands()
            .into_iter()
            .map(|c| c.kind.to_string())
            .collect();
        out.dedup();
        if self.run_mode_needs_trust() {
            out.push("run_mode".into());
        }
        out
    }

    /// `run_mode` looser than asking first (D2: more power only by choice).
    pub fn run_mode_needs_trust(&self) -> bool {
        self.config.run_mode.as_deref().is_some_and(|m| {
            crate::core::llm::run_mode::RunMode::parse(m)
                .is_ok_and(|m| m != crate::core::llm::run_mode::RunMode::ApprovalRequired)
        })
    }
}

// ── where it is ─────────────────────────────────────────────────────────

/// The repository root of `folder` (the nearest ancestor with a `.git`), or
/// the folder itself outside git; canonical when it can be.
pub fn root_of(folder: &Path) -> PathBuf {
    let start = std::fs::canonicalize(folder).unwrap_or_else(|_| folder.to_path_buf());
    start
        .ancestors()
        .find(|a| a.join(".git").exists())
        .map(Path::to_path_buf)
        .unwrap_or(start)
}

/// The branch checked out at `root` (from `.git/HEAD`, no process), or the
/// short commit when detached.
pub fn checked_out_branch(root: &Path) -> Option<String> {
    let dot = root.join(".git");
    let git_dir = if dot.is_dir() {
        dot
    } else {
        let link = std::fs::read_to_string(&dot).ok()?;
        let dir = PathBuf::from(link.trim().strip_prefix("gitdir:")?.trim());
        if dir.is_absolute() {
            dir
        } else {
            root.join(dir)
        }
    };
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    match head.strip_prefix("ref:") {
        Some(r) => Some(
            r.trim()
                .strip_prefix("refs/heads/")
                .unwrap_or(r.trim())
                .to_string(),
        ),
        None if head.len() >= 7 => Some(head[..12.min(head.len())].to_string()),
        None => None,
    }
}

// ── state: last known good and trust ────────────────────────────────────

/// What the jobs page, the CLI and a job start see of a repository's file.
#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub root: String,
    pub path: String,
    /// `none` (no file), `active` (the file on disk is in effect) or
    /// `invalid` (it does not parse; `active` is the last good version, if any).
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The version in effect.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<Workflow>,
    /// `active` is an earlier version kept because the file is invalid now.
    pub from_last_good: bool,
    /// When `active` was last read as valid (epoch ms).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loaded_ms: Option<i64>,
    /// The approved SHA-256 is the one of `active`.
    pub trusted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trusted_sha256: Option<String>,
    /// The repository commands of `active`.
    pub commands: Vec<RepoCommand>,
    /// What of `active` waits for trust (empty = nothing to approve).
    pub gated: Vec<String>,
}

impl Status {
    /// Whether the gated parts of the version in effect may run.
    pub fn may_run_commands(&self) -> bool {
        self.active.is_some() && self.trusted
    }

    /// Why the gated parts are skipped, for a job's log.
    pub fn skipped_reason(&self) -> String {
        match (&self.active, &self.trusted_sha256) {
            (None, _) => "no valid workflow".into(),
            (Some(w), None) => format!(
                "{FILE} is not trusted (sha256 {}): approve it on the jobs page or with `omniget agent workflow trust`",
                short(&w.sha256)
            ),
            (Some(w), Some(t)) => format!(
                "{FILE} changed since it was trusted (sha256 {} → {}): approve the new version",
                short(t),
                short(&w.sha256)
            ),
        }
    }
}

pub fn short(sha: &str) -> &str {
    &sha[..12.min(sha.len())]
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub struct Store {
    db: Mutex<Connection>,
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("workflow::Store").finish()
    }
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        Self::with(Connection::open(path).map_err(|e| e.to_string())?)
    }

    /// No file: tests.
    pub fn memory() -> Self {
        Self::with(Connection::open_in_memory().expect("in-memory sqlite")).expect("schema")
    }

    fn with(db: Connection) -> Result<Self, String> {
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS workflow_good (
                root TEXT PRIMARY KEY, sha256 TEXT NOT NULL, text TEXT NOT NULL,
                loaded_ms INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS workflow_trust (
                root TEXT PRIMARY KEY, sha256 TEXT NOT NULL, trusted_ms INTEGER NOT NULL,
                by TEXT NOT NULL);",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self { db: Mutex::new(db) })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Reads the file of `folder`'s repository now and says what is in
    /// effect. A valid file becomes the new last good version; an invalid
    /// one leaves the last good version in effect.
    pub fn status(&self, folder: &Path) -> Status {
        let root = root_of(folder);
        let key = root.to_string_lossy().into_owned();
        let path = root.join(FILE);
        let db = self.conn();
        let trusted_sha256: Option<String> = db
            .query_row(
                "SELECT sha256 FROM workflow_trust WHERE root=?1",
                [&key],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten();
        let mut st = Status {
            root: key.clone(),
            path: path.to_string_lossy().into_owned(),
            state: "none",
            file_sha256: None,
            error: None,
            active: None,
            from_last_good: false,
            loaded_ms: None,
            trusted: false,
            trusted_sha256,
            commands: Vec::new(),
            gated: Vec::new(),
        };
        let read = match std::fs::metadata(&path) {
            Err(_) => None,
            Ok(m) if !m.is_file() => Some(Err(format!("{FILE} is not a file"))),
            Ok(m) if m.len() > MAX_BYTES => Some(Err(format!("{FILE} is over 64 KB"))),
            Ok(_) => Some(std::fs::read(&path).map_err(|e| e.to_string())),
        };
        let Some(read) = read else {
            // No file, no contract: an old version must not come back if a
            // broken file appears later.
            let _ = db.execute("DELETE FROM workflow_good WHERE root=?1", [&key]);
            return st;
        };
        let parsed = read.and_then(|bytes| {
            st.file_sha256 = Some(sha256_hex(&bytes));
            String::from_utf8(bytes).map_err(|_| format!("{FILE} is not UTF-8"))
        });
        let parsed = parsed.and_then(|text| parse(&text).map(|w| (w, text)));
        match parsed {
            Ok((w, text)) => {
                let now = now_ms();
                let _ = db.execute(
                    "INSERT INTO workflow_good (root, sha256, text, loaded_ms) VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(root) DO UPDATE SET sha256=excluded.sha256, text=excluded.text,
                       loaded_ms=CASE WHEN workflow_good.sha256=excluded.sha256 THEN workflow_good.loaded_ms ELSE excluded.loaded_ms END",
                    params![key, w.sha256, text, now],
                );
                st.state = "active";
                st.loaded_ms = db
                    .query_row(
                        "SELECT loaded_ms FROM workflow_good WHERE root=?1",
                        [&key],
                        |r| r.get(0),
                    )
                    .ok();
                st.active = Some(w);
            }
            Err(e) => {
                st.state = "invalid";
                st.error = Some(e);
                let good: Option<(String, i64)> = db
                    .query_row(
                        "SELECT text, loaded_ms FROM workflow_good WHERE root=?1",
                        [&key],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .optional()
                    .ok()
                    .flatten();
                if let Some((text, at)) = good {
                    if let Ok(w) = parse(&text) {
                        st.active = Some(w);
                        st.from_last_good = true;
                        st.loaded_ms = Some(at);
                    }
                }
            }
        }
        if let Some(w) = &st.active {
            st.trusted = st.trusted_sha256.as_deref() == Some(w.sha256.as_str());
            st.commands = w.commands();
            st.gated = w.gated();
        }
        st
    }

    /// Approves the version in effect in `folder`'s repository. `sha256`
    /// (full, or a prefix of at least 8) must be that version's: the person
    /// approves what they saw, never whatever the file holds by then.
    pub fn trust(&self, folder: &Path, sha256: &str, by: &str) -> Result<Status, String> {
        let st = self.status(folder);
        let Some(w) = &st.active else {
            return Err(format!(
                "{ERR_TRUST}: no valid {FILE} in {}{}",
                st.root,
                st.error
                    .as_deref()
                    .map(|e| format!(" ({e})"))
                    .unwrap_or_default()
            ));
        };
        let asked = sha256.trim().to_ascii_lowercase();
        if asked.len() < 8 || !w.sha256.starts_with(&asked) {
            return Err(format!(
                "{ERR_TRUST}: the version in effect is sha256 {}, not {}: read it again before trusting",
                short(&w.sha256),
                short(&asked)
            ));
        }
        self.conn()
            .execute(
                "INSERT INTO workflow_trust (root, sha256, trusted_ms, by) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(root) DO UPDATE SET sha256=excluded.sha256, trusted_ms=excluded.trusted_ms, by=excluded.by",
                params![st.root, w.sha256, now_ms(), by],
            )
            .map_err(|e| e.to_string())?;
        Ok(self.status(folder))
    }

    /// Withdraws the approval of `folder`'s repository.
    pub fn untrust(&self, folder: &Path) -> Result<Status, String> {
        let root = root_of(folder).to_string_lossy().into_owned();
        self.conn()
            .execute("DELETE FROM workflow_trust WHERE root=?1", [&root])
            .map_err(|e| e.to_string())?;
        Ok(self.status(folder))
    }
}

static STORE: OnceLock<Arc<Store>> = OnceLock::new();

/// Opens `<llm dir>/workflows.db` once for the app.
pub fn init(path: &Path) -> Result<Arc<Store>, String> {
    if let Some(s) = STORE.get() {
        return Ok(s.clone());
    }
    let _ = STORE.set(Arc::new(Store::open(path)?));
    Ok(STORE.get().cloned().expect("set above"))
}

/// The app's store, if the app opened one (tests bring their own).
pub fn active() -> Option<Arc<Store>> {
    STORE.get().cloned()
}

// ── running a repository command ────────────────────────────────────────

/// One hook or check that ran (or was stopped).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandRun {
    pub kind: String,
    pub command: String,
    pub ok: bool,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub timed_out: bool,
    pub ms: u64,
    /// The last lines of stdout + stderr.
    #[serde(default)]
    pub output: String,
}

impl CommandRun {
    /// One line for a job's log.
    pub fn note(&self) -> String {
        let how = if self.timed_out {
            "timed out".to_string()
        } else {
            match self.exit_code {
                Some(c) => format!("exit {c}"),
                None => "did not run".into(),
            }
        };
        format!(
            "{} {} `{}`: {how} in {} ms\n",
            if self.ok { "✓" } else { "✗" },
            self.kind,
            self.command,
            self.ms
        )
    }
}

fn tail(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    format!("…{}", &s[start..])
}

/// Runs `command` through the shell in `cwd`, in its own process tree,
/// killed whole when it outlives `timeout`.
pub async fn run_command(
    kind: &str,
    command: &str,
    cwd: &Path,
    env: &[(String, String)],
    timeout: Duration,
) -> CommandRun {
    let t0 = Instant::now();
    let failed = |output: String| CommandRun {
        kind: kind.to_string(),
        command: command.to_string(),
        ok: false,
        exit_code: None,
        timed_out: false,
        ms: t0.elapsed().as_millis() as u64,
        output,
    };
    #[cfg(windows)]
    let mut cmd = {
        let mut c = crate::core::process::command("cmd");
        c.args(["/D", "/C", command]);
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = crate::core::process::command("sh");
        c.args(["-c", command]);
        c
    };
    cmd.current_dir(cwd)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    crate::core::process::suspend_on_spawn(&mut cmd);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return failed(format!("cannot start the shell: {e}")),
    };
    let mut tree = match crate::core::process::ProcessTreeGuard::new(&child) {
        Ok(t) => t,
        Err(e) => return failed(format!("cannot watch the process: {e}")),
    };
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => {
            tree.release();
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            CommandRun {
                kind: kind.to_string(),
                command: command.to_string(),
                ok: out.status.success(),
                exit_code: out.status.code(),
                timed_out: false,
                ms: t0.elapsed().as_millis() as u64,
                output: tail(&text, OUTPUT_TAIL),
            }
        }
        Ok(Err(e)) => {
            tree.terminate();
            failed(e.to_string())
        }
        Err(_) => {
            tree.terminate();
            CommandRun {
                timed_out: true,
                output: format!("stopped after {} s", timeout.as_secs()),
                ..failed(String::new())
            }
        }
    }
}
