use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::agent::GrantMode;
use super::perm::{self, Action, Rule, Source};
use super::run_mode::RunMode;

pub const POST_MATCHER: &str = "Write|Edit|MultiEdit|NotebookEdit";
pub const HOOK_TIMEOUT_S: u64 = 10;
pub const OWN_TOOLS_PREFIX: &str = "mcp__omniget__";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Policy {
    pub agent: String,
    #[serde(default)]
    pub rules_file: Option<PathBuf>,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub run_mode: Option<RunMode>,
    #[serde(default)]
    pub workspace: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct TurnHooks {
    pub dir: PathBuf,
    pub settings: PathBuf,
}

impl TurnHooks {
    pub fn cleanup(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn resolve_shim() -> Option<PathBuf> {
    use crate::core::agentkit::convert::hook_common::{pinned_shim, SHIM_NAME, SHIM_SIDECAR};
    let exe = |n: &str| {
        if cfg!(windows) {
            format!("{n}.exe")
        } else {
            n.to_string()
        }
    };
    let dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let beside = [SHIM_SIDECAR, SHIM_NAME]
        .into_iter()
        .filter_map(|n| dir.as_ref().map(|d| d.join(exe(n))));
    let in_data = crate::core::paths::app_data_dir().map(|d| d.join("bin").join(exe(SHIM_NAME)));
    first_usable(pinned_shim().into_iter().chain(beside).chain(in_data))
}

fn first_usable(candidates: impl IntoIterator<Item = PathBuf>) -> Option<PathBuf> {
    use crate::core::agentkit::convert::hook_common::usable_binary;
    candidates.into_iter().find(|p| usable_binary(p))
}

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

pub fn default_root() -> PathBuf {
    std::env::temp_dir().join("omniget-turns")
}

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

pub fn deny_reason(agent: &str, native: &str, input: &Value) -> String {
    let rules = perm::rules_of(agent);
    let verdict = perm::gate(&rules, None, perm::cli_grant(native), native, input);
    if matches!(verdict.mode, GrantMode::Deny) {
        reason_of(&rules, native, input, verdict)
    } else {
        "[OmniGet] Denied: the run mode or the grants of this turn do not allow this.".into()
    }
}

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
        _ => Outcome::silent(),
    }
}

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
