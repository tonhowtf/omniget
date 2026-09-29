//! Claude Code hooks as OmniGet's permission gate (3.4, S19, `85#P3`).
//!
//! Before 3.4 the CLI had its own permission story: `acceptEdits` let every
//! `Write`/`Edit` through without ever calling the prompt tool, so a rule
//! saved in OmniGet ("deny `fs_write` `**/.env`") never reached a Claude
//! turn (PROBLEMAS S4). Now each Claude turn gets a `--settings` file with a
//! `PreToolUse` and a `PostToolUse` hook that run `omniget-hook-shim hook
//! claude` (the same entry as `omniget hook claude`), and the hook decides
//! with [`perm::gate`] over the same rules, run mode and grants as the native
//! broker.
//!
//! The contract, taken from omnigent's `native_policy_hook.py` (the JSON
//! shape is Claude Code's public hook contract):
//! - **allow prints nothing**: the CLI's own permission logic (its mode, the
//!   prompt tool, the user's settings) stays in charge; the prompt tool then
//!   asks the same gate ([`crate::core::assist::projection`]);
//! - **deny** prints `hookSpecificOutput.permissionDecision = "deny"` with the
//!   reason, which reaches the model;
//! - **ask** is printed only when a stored rule says "ask" (in `acceptEdits`
//!   the CLI would otherwise let the edit through). SP2 verified that an
//!   `ask` from the hook goes to the prompt tool even in `acceptEdits`;
//! - `PreToolUse` **fails closed** (exit 2, which Claude treats as a block,
//!   also for a panic), `PostToolUse` **fails open** (exit 0, no output);
//! - `mcp__omniget__*` calls are skipped: they already pass through the
//!   broker via the projection, and judging them twice would ask twice.
//!
//! No bridge round trip: the hook reads the turn's `policy.json` (agent, run
//! mode, workspace, the rules file) and the rules file itself on every call,
//! so a rule saved mid-turn counts from the next call and the hook keeps
//! working when the app is busy (p50 is a process start plus two small
//! reads, measured by BN8).
//!
//! Everything lives in a private per-turn folder (0700, files 0600) that the
//! runtime deletes when the turn ends.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::agent::GrantMode;
use super::perm::{self, Action, Rule, Source};
use super::run_mode::RunMode;

/// The tools the `PostToolUse` hook watches (the file writers).
pub const POST_MATCHER: &str = "Write|Edit|MultiEdit|NotebookEdit";
/// Seconds Claude waits for one hook run before giving up on it.
pub const HOOK_TIMEOUT_S: u64 = 10;
/// The projection's tools: judged by the broker, never by the hook.
pub const OWN_TOOLS_PREFIX: &str = "mcp__omniget__";

/// What the hook needs to decide, written once per turn.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    /// The agent the rules are stored under (`perm::rules_of`).
    pub agent: String,
    /// `permission-rules.json`, read on every call. `None` = only [`Self::rules`].
    #[serde(default)]
    pub rules_file: Option<PathBuf>,
    /// Extra rules after the file's (the benches; the app leaves it empty).
    #[serde(default)]
    pub rules: Vec<Rule>,
    /// The turn's run mode (after the account's ceiling). `None` = nobody
    /// chose: grants stay as they are, like the native harness.
    #[serde(default)]
    pub run_mode: Option<RunMode>,
    /// The folder the turn works in (paths become relative to it).
    #[serde(default)]
    pub workspace: Option<PathBuf>,
}

/// The per-turn folder and the `--settings` file in it.
#[derive(Debug, Clone)]
pub struct TurnHooks {
    pub dir: PathBuf,
    pub settings: PathBuf,
}

impl TurnHooks {
    /// Deletes the turn's folder (settings, policy). Idempotent.
    pub fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The hook binary: the host's pinned copy (`<app_data>/bin/`), else next to
/// the running executable, else `<app_data>/bin/`. An empty file (the
/// sidecar placeholder of a dev build) does not count: a missing binary
/// would make the shell fail with 127, which Claude reads as "go ahead".
pub fn resolve_shim() -> Option<PathBuf> {
    use crate::core::agentkit::convert::hook_common::{pinned_shim, SHIM_NAME};
    let name = if cfg!(windows) {
        format!("{SHIM_NAME}.exe")
    } else {
        SHIM_NAME.to_string()
    };
    let beside = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join(&name)));
    let in_data = crate::core::paths::app_data_dir().map(|d| d.join("bin").join(&name));
    first_usable([pinned_shim(), beside, in_data].into_iter().flatten())
}

/// The first candidate that is a non-empty file.
fn first_usable(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    candidates.into_iter().find(|p| {
        std::fs::metadata(p)
            .map(|m| m.is_file() && m.len() > 0)
            .unwrap_or(false)
    })
}

/// The hook command line Claude runs (through its shell).
pub fn command(shim: &Path, event: &str, dir: &Path) -> String {
    use crate::core::agentkit::convert::hook_common::quote;
    use crate::core::agentkit::Os;
    let os = Os::current();
    format!(
        "{} hook claude --event {event} --turn {}",
        quote(&shim.to_string_lossy(), os),
        quote(&dir.to_string_lossy(), os)
    )
}

/// The `--settings` body: `PreToolUse` on every tool, `PostToolUse` on the
/// file writers. No status hooks: the stream already carries the turn's
/// status, and each hook is one more process per call.
pub fn settings_json(shim: &Path, dir: &Path) -> Value {
    let hook = |event: &str| json!({ "type": "command", "command": command(shim, event, dir), "timeout": HOOK_TIMEOUT_S });
    json!({
        "hooks": {
            "PreToolUse": [{ "matcher": "*", "hooks": [hook("PreToolUse")] }],
            "PostToolUse": [{ "matcher": POST_MATCHER, "hooks": [hook("PostToolUse")] }],
        }
    })
}

fn write_private(path: &Path, body: &[u8]) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        f.write_all(body)
            .map_err(|e| format!("{}: {e}", path.display()))
    }
    #[cfg(not(unix))]
    std::fs::write(path, body).map_err(|e| format!("{}: {e}", path.display()))
}

/// Writes the turn's private folder under `root` (0700) with `policy.json`
/// and `settings.json` (0600). The caller passes `settings` to `--settings`
/// and calls [`TurnHooks::cleanup`] when the turn ends.
pub fn prepare(root: &Path, shim: &Path, policy: &Policy) -> Result<TurnHooks, String> {
    std::fs::create_dir_all(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let dir = root.join(format!("turn-{}", uuid::Uuid::new_v4().simple()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&dir)
            .map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let hooks = TurnHooks {
        settings: dir.join("settings.json"),
        dir: dir.clone(),
    };
    let body = serde_json::to_vec(policy).map_err(|e| e.to_string());
    let out = body
        .and_then(|b| write_private(&dir.join("policy.json"), &b))
        .and_then(|_| {
            write_private(
                &hooks.settings,
                settings_json(shim, &dir).to_string().as_bytes(),
            )
        });
    if let Err(e) = out {
        hooks.cleanup();
        return Err(e);
    }
    Ok(hooks)
}

/// Where the app puts the turn folders.
pub fn default_root() -> PathBuf {
    std::env::temp_dir().join("omniget-turns")
}

// ── the hook itself ─────────────────────────────────────────────────────

/// What one hook run prints and exits with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Outcome {
    fn silent() -> Self {
        Self {
            code: 0,
            stdout: String::new(),
            stderr: String::new(),
        }
    }
}

/// The verdict of one call as the hook sees it (also what BN8 compares).
#[derive(Debug, Clone, PartialEq)]
pub struct HookVerdict {
    pub native_tool: String,
    pub mode: GrantMode,
    pub source: Source,
    pub reason: Option<String>,
}

fn load_policy(dir: &Path) -> Result<Policy, String> {
    let path = dir.join("policy.json");
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))
}

fn rules_of_policy(policy: &Policy) -> Result<Vec<Rule>, String> {
    let mut rules = match &policy.rules_file {
        Some(file) => perm::rules_in_file(file, &policy.agent)?,
        None => Vec::new(),
    };
    rules.extend(policy.rules.iter().cloned());
    Ok(rules)
}

fn describe(rule: &Rule) -> String {
    let action = match rule.action {
        Action::Allow => "allow",
        Action::Ask => "ask",
        Action::Deny => "deny",
    };
    format!("{action} {} {}", rule.tool, rule.pattern)
}

fn reason_of(rules: &[Rule], native: &str, input: &Value, verdict: perm::Verdict) -> String {
    match verdict.source {
        Source::Rule => {
            let action = match verdict.mode {
                GrantMode::Deny => Action::Deny,
                GrantMode::Ask => Action::Ask,
                GrantMode::Auto => Action::Allow,
            };
            match perm::rule_for(rules, native, input, action) {
                Some(rule) => format!(
                    "[OmniGet] {} by the permission rule `{}`.",
                    if action == Action::Deny {
                        "Denied"
                    } else {
                        "Needs approval"
                    },
                    describe(&rule)
                ),
                None => "[OmniGet] Denied by a permission rule.".into(),
            }
        }
        Source::RunMode => "[OmniGet] Denied: the turn's run mode does not allow this.".into(),
        _ => format!("[OmniGet] Denied: `{native}` is not granted to this agent."),
    }
}

/// The reason next to a denial of the prompt tool (the app's side).
pub fn deny_reason(agent: &str, native: &str, input: &Value) -> String {
    let rules = perm::rules_of(agent);
    let verdict = perm::gate(&rules, None, perm::cli_grant(native), native, input);
    if matches!(verdict.mode, GrantMode::Deny) {
        reason_of(&rules, native, input, verdict)
    } else {
        "[OmniGet] Denied: the run mode or the grants of this turn do not allow this.".into()
    }
}

/// The gate for one Claude tool call under `policy`. `Ok(None)` = not ours
/// to judge (`mcp__omniget__*`).
pub fn evaluate(policy: &Policy, payload: &Value) -> Result<Option<HookVerdict>, String> {
    let tool = payload
        .get("tool_name")
        .and_then(Value::as_str)
        .ok_or("the hook input has no tool_name")?;
    if tool.starts_with(OWN_TOOLS_PREFIX) {
        return Ok(None);
    }
    let input = payload.get("tool_input").cloned().unwrap_or(json!({}));
    let rules = rules_of_policy(policy)?;
    let (native, native_input) = perm::from_claude(tool, &input, policy.workspace.as_deref());
    let verdict = perm::gate(
        &rules,
        policy.run_mode,
        perm::cli_grant(&native),
        &native,
        &native_input,
    );
    let reason = (!matches!(verdict.mode, GrantMode::Auto))
        .then(|| reason_of(&rules, &native, &native_input, verdict));
    Ok(Some(HookVerdict {
        native_tool: native,
        mode: verdict.mode,
        source: verdict.source,
        reason,
    }))
}

fn pre_output(v: &HookVerdict) -> Option<Value> {
    let decision = match (v.mode, v.source) {
        (GrantMode::Deny, _) => "deny",
        // Only a stored "ask" forces a question: the grant's and the run
        // mode's asks are the CLI's own to raise (its mode already does),
        // and printing them would override the user's own allow list.
        (GrantMode::Ask, Source::Rule) => "ask",
        _ => return None,
    };
    Some(json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": decision,
            "permissionDecisionReason": v.reason.clone().unwrap_or_default(),
        }
    }))
}

fn pre(dir: &Path, stdin: &[u8]) -> Result<Outcome, String> {
    let payload: Value =
        serde_json::from_slice(stdin).map_err(|e| format!("the hook input is not JSON: {e}"))?;
    let policy = load_policy(dir)?;
    let Some(verdict) = evaluate(&policy, &payload)? else {
        return Ok(Outcome::silent());
    };
    Ok(match pre_output(&verdict) {
        Some(out) => Outcome {
            code: 0,
            stdout: out.to_string(),
            stderr: String::new(),
        },
        None => Outcome::silent(),
    })
}

/// After a file tool ran: a rule that denies what just happened (saved
/// while the call ran) is told to the model; nothing is undone here.
fn post(dir: &Path, stdin: &[u8]) -> Option<Outcome> {
    let payload: Value = serde_json::from_slice(stdin).ok()?;
    let policy = load_policy(dir).ok()?;
    let verdict = evaluate(&policy, &payload).ok()??;
    if !matches!(verdict.mode, GrantMode::Deny) {
        return None;
    }
    let out = json!({
        "hookSpecificOutput": {
            "hookEventName": "PostToolUse",
            "additionalContext": format!(
                "{} This change was made before the rule applied; tell the user.",
                verdict.reason.unwrap_or_default()
            ),
        }
    });
    Some(Outcome {
        code: 0,
        stdout: out.to_string(),
        stderr: String::new(),
    })
}

/// `PermissionRequest` (Claude asks before a call its mode does not let
/// through): the gate's answer, in that event's shape. The app does not
/// install it (its prompt tool asks the same [`perm::gate`]); BN8's fake CLI
/// uses it as that prompt tool. `Ask` prints nothing: a person decides.
fn permission_request(dir: &Path, stdin: &[u8]) -> Option<Outcome> {
    let payload: Value = serde_json::from_slice(stdin).ok()?;
    let policy = load_policy(dir).ok()?;
    let verdict = evaluate(&policy, &payload).ok()??;
    let decision = match verdict.mode {
        GrantMode::Auto => json!({ "behavior": "allow" }),
        GrantMode::Deny => {
            json!({ "behavior": "deny", "message": verdict.reason.unwrap_or_default() })
        }
        GrantMode::Ask => return None,
    };
    let out = json!({
        "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision }
    });
    Some(Outcome {
        code: 0,
        stdout: out.to_string(),
        stderr: String::new(),
    })
}

/// One hook run: `event` is `PreToolUse` or `PostToolUse`, `dir` the turn's
/// folder, `stdin` Claude's hook JSON. Never panics outward: a panic in
/// `PreToolUse` blocks the call like any other failure.
pub fn run(event: &str, dir: &Path, stdin: &[u8]) -> Outcome {
    let fail_closed = |why: String| Outcome {
        code: 2,
        stdout: String::new(),
        stderr: format!("[OmniGet] The permission hook failed, so the call is blocked: {why}"),
    };
    match event {
        "PreToolUse" => match std::panic::catch_unwind(|| pre(dir, stdin)) {
            Ok(Ok(out)) => out,
            Ok(Err(why)) => fail_closed(why),
            Err(_) => fail_closed("internal error".into()),
        },
        "PostToolUse" => std::panic::catch_unwind(|| post(dir, stdin))
            .ok()
            .flatten()
            .unwrap_or_else(Outcome::silent),
        "PermissionRequest" => std::panic::catch_unwind(|| permission_request(dir, stdin))
            .ok()
            .flatten()
            .unwrap_or_else(Outcome::silent),
        // An event this version does not know: never block.
        _ => Outcome::silent(),
    }
}

/// `omniget hook claude --event <E> --turn <dir>` (args after `claude`).
/// Returns the exit code; prints what Claude reads.
pub fn main_with_args(args: &[String]) -> i32 {
    let mut event = None;
    let mut dir = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--event" => {
                event = args.get(i + 1).cloned();
                i += 1;
            }
            "--turn" => {
                dir = args.get(i + 1).map(PathBuf::from);
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    let mut stdin = Vec::new();
    let _ = std::io::Read::read_to_end(&mut std::io::stdin(), &mut stdin);
    let event = event.unwrap_or_default();
    let out = match dir {
        Some(dir) => run(&event, &dir, &stdin),
        None if event == "PreToolUse" => Outcome {
            code: 2,
            stdout: String::new(),
            stderr: "[OmniGet] The permission hook has no --turn folder, so the call is blocked."
                .into(),
        },
        None => Outcome::silent(),
    };
    if !out.stdout.is_empty() {
        let mut o = std::io::stdout();
        let _ = o.write_all(out.stdout.as_bytes());
        let _ = o.flush();
    }
    if !out.stderr.is_empty() {
        let _ = std::io::stderr().write_all(out.stderr.as_bytes());
    }
    out.code
}

#[cfg(test)]
#[path = "claude_hooks_tests.rs"]
mod tests;
