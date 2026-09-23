//! Shared pieces of the hook converters (plan §4.3 "Hook", F5).
//!
//! Every non-Claude dialect runs the original hook command through
//! `omniget-hook-shim` ([`super::hook_shim`]): the shim turns the tool's stdin
//! into Claude's hook JSON, runs the command, and turns Claude's answer (exit 2,
//! `hookSpecificOutput.permissionDecision`, `decision`) back into what the tool
//! expects. So the 62 catalog hooks run unchanged everywhere.

use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde_json::Value;

use super::{place_support, rewrite_command, ConvertCtx, PlannedFile};
use crate::core::agentkit::model::{Component, HookEntry, HookSpec};
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{AgentkitError, Env, Os, Result, Scope};

/// File name of the shim binary (without `.exe`).
pub const SHIM_NAME: &str = "omniget-hook-shim";

static SHIM_OVERRIDE: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Pins the shim path written into hook commands (tests, or the host after it
/// copied the shim somewhere stable). `None` restores the lookup.
pub fn set_shim_path(p: Option<PathBuf>) {
    if let Ok(mut g) = SHIM_OVERRIDE.write() {
        *g = p;
    }
}

fn exe_name(os: Os) -> String {
    if os == Os::Windows {
        format!("{SHIM_NAME}.exe")
    } else {
        SHIM_NAME.to_string()
    }
}

/// Absolute path of the shim for hook commands: the override, else next to the
/// running executable when it is there, else `<app_data>/bin/`.
pub fn shim_path(env: &Env) -> PathBuf {
    if let Some(p) = SHIM_OVERRIDE.read().ok().and_then(|g| g.clone()) {
        return p;
    }
    let name = exe_name(env.os);
    // a sandboxed env (tests, compat) never points at the machine's binaries
    let sandboxed = env.path_var.is_none() && !env.probe_versions;
    if !sandboxed {
        if let Some(dir) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
        {
            let beside = dir.join(&name);
            if beside.is_file() {
                return beside;
            }
        }
    }
    env.app_data.join("bin").join(name)
}

/// Quotes one argument for the shell the tool runs hook commands with
/// (`sh -c` on macOS/Linux, `cmd /C` on Windows).
pub fn quote(s: &str, os: Os) -> String {
    if os == Os::Windows {
        if !s.is_empty() && !s.contains([' ', '"', '&', '|', '<', '>', '^', '%', '\t']) {
            return s.to_string();
        }
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        if !s.is_empty()
            && s.chars().all(|c| {
                c.is_ascii_alphanumeric()
                    || matches!(c, '/' | '.' | '_' | '-' | ':' | '=' | ',' | '+' | '@')
            })
        {
            return s.to_string();
        }
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// PowerShell single-quoted literal.
pub fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Arguments that go between the shim and `--` for one hook.
#[derive(Debug, Clone, Default)]
pub struct ShimCall<'a> {
    pub tool: &'a str,
    pub event: &'a str,
    pub native_event: Option<&'a str>,
    /// Canonical (Claude) matcher checked by the shim, for tools that cannot
    /// filter by tool name themselves.
    pub matcher: Option<&'a str>,
}

impl ShimCall<'_> {
    fn args(&self) -> Vec<String> {
        let mut v = vec![
            "--tool".to_string(),
            self.tool.to_string(),
            "--event".to_string(),
            self.event.to_string(),
        ];
        if let Some(n) = self.native_event {
            v.push("--native-event".into());
            v.push(n.to_string());
        }
        if let Some(m) = self.matcher.filter(|m| !m.is_empty() && *m != "*") {
            v.push("--matcher".into());
            v.push(m.to_string());
        }
        v
    }
}

/// `'<shim>' --tool x --event E -- '<original>'` for a POSIX/cmd shell.
pub fn shim_command(env: &Env, call: &ShimCall, original: &str) -> String {
    let os = env.os;
    let mut parts = vec![quote(&shim_path(env).display().to_string(), os)];
    parts.extend(call.args().iter().map(|a| quote(a, os)));
    parts.push("--".into());
    parts.push(quote(original, os));
    parts.join(" ")
}

/// The same line for a PowerShell field (Copilot `powershell`, Cascade `powershell`).
pub fn shim_command_ps(env: &Env, call: &ShimCall, original: &str) -> String {
    let mut parts = vec![format!(
        "& {}",
        ps_quote(&shim_path(env).display().to_string())
    )];
    parts.extend(call.args().iter().map(|a| ps_quote(a)));
    parts.push("'--'".into());
    parts.push(ps_quote(original));
    parts.join(" ")
}

/// Support scripts of a hook placed for a target, plus the command rewrites
/// (`.claude/hooks/x.py` → `.cursor/hooks/x.py`, absolute in global scope).
pub fn support_files(
    c: &Component,
    h: &HookSpec,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<(Vec<PlannedFile>, Vec<(String, String)>)> {
    let mut out = Vec::new();
    let mut rewrites = Vec::new();
    for sf in &h.supporting_files {
        if sf.source.starts_with("inline:") {
            continue;
        }
        let (abs, shown) = place_support(&sf.destination, target, "hook_scripts", scope, ctx)
            .ok_or_else(|| {
                AgentkitError::new(
                    "AGENTKIT_NO_PATH",
                    format!(
                        "{} has no place for hook scripts in scope {}",
                        target.name,
                        scope.as_str()
                    ),
                )
            })?;
        let bytes = c.file(&sf.source).map(|f| f.bytes.clone()).ok_or_else(|| {
            AgentkitError::new(
                "AGENTKIT_PARSE",
                format!("support file `{}` is missing", sf.source),
            )
        })?;
        let mut pf = PlannedFile::write(&target.id, c, abs, bytes, "hook script");
        pf.executable = sf.executable;
        out.push(pf);
        let from = sf.destination.trim_start_matches("./").to_string();
        if from != shown {
            rewrites.push((from, shown));
        }
    }
    Ok((out, rewrites))
}

pub fn apply_rewrites(cmd: &str, rewrites: &[(String, String)]) -> String {
    let mut out = cmd.to_string();
    for (from, to) in rewrites {
        out = rewrite_command(&out, from, to);
    }
    out
}

/// One hook entry ready for a dialect: canonical event, native events, command
/// (support paths rewritten, not yet wrapped), matcher both ways, timeout (s).
#[derive(Debug, Clone)]
pub struct Prepared {
    pub event: String,
    pub natives: Vec<String>,
    pub command: String,
    pub matcher: Option<String>,
    pub native_matcher: Option<String>,
    pub timeout: Option<f64>,
    pub entry: HookEntry,
}

/// Walks a hook's entries for a target: maps events, rewrites support paths,
/// and collects what is lost (events without equivalent, non-command handlers,
/// Claude-only handler fields).
pub fn prepare(
    h: &HookSpec,
    target: &TargetAdapter,
    rewrites: &[(String, String)],
    allow_kinds: &[&str],
) -> (Vec<Prepared>, Vec<String>, Vec<String>) {
    let mut out = Vec::new();
    let mut losses: Vec<String> = Vec::new();
    let mut commands = Vec::new();
    let lose = |l: String, losses: &mut Vec<String>| {
        if !losses.contains(&l) {
            losses.push(l);
        }
    };
    for e in &h.entries {
        let natives = target.map_event(&e.event);
        if natives.is_empty() {
            lose(
                format!("hook event {} (no equivalent in {})", e.event, target.name),
                &mut losses,
            );
            continue;
        }
        let kind = e.handler.kind.as_str();
        if kind != "command" && !allow_kinds.contains(&kind) {
            lose(format!("`{kind}` hook handler"), &mut losses);
            continue;
        }
        if e.handler.extra.contains_key("if") {
            lose(
                "the `if` permission-rule filter of a handler".into(),
                &mut losses,
            );
        }
        for k in ["async", "asyncRewake", "once", "statusMessage"] {
            if e.handler.extra.contains_key(k) {
                lose(format!("handler field `{k}`"), &mut losses);
            }
        }
        let command = e
            .handler
            .command
            .as_deref()
            .map(|c| apply_rewrites(c, rewrites))
            .unwrap_or_default();
        if !command.is_empty() {
            commands.push(command.clone());
        }
        let matcher = e.matcher.clone().filter(|m| !m.is_empty() && m != "*");
        let native_matcher = matcher.as_deref().map(|m| target.map_matcher(m));
        out.push(Prepared {
            event: e.event.clone(),
            natives,
            command,
            matcher,
            native_matcher,
            timeout: e.handler.timeout,
            entry: e.clone(),
        });
    }
    (out, losses, commands)
}

/// Number for a JSON timeout field (integer when whole).
pub fn num(v: f64) -> Value {
    if v.fract() == 0.0 {
        Value::from(v as i64)
    } else {
        Value::from(v)
    }
}

/// Note shown on every file whose commands go through the shim.
pub fn shim_note(env: &Env) -> String {
    format!(
        "hook commands run through {} (translates the tool's hook input/output to Claude's); it must stay at that path",
        shim_path(env).display()
    )
}

/// Merges `losses`/`commands`/`notes` into the first planned file.
pub fn annotate(
    files: &mut [PlannedFile],
    losses: Vec<String>,
    commands: Vec<String>,
    note: Option<String>,
) {
    if let Some(f) = files.first_mut() {
        for l in losses {
            if !f.losses.contains(&l) {
                f.losses.push(l);
            }
        }
        for c in commands {
            if !f.commands.contains(&c) {
                f.commands.push(c);
            }
        }
        if let Some(n) = note {
            if !f.notes.contains(&n) {
                f.notes.push(n);
            }
        }
    }
}

/// Claude-only env vars the catalog uses that no other tool sets.
pub fn legacy_env_loss(c: &Component) -> Option<String> {
    c.files
        .iter()
        .any(|f| {
            f.text()
                .map(|t| t.contains("CLAUDE_TOOL_FILE_PATH") || t.contains("CLAUDE_TOOL_NAME"))
                .unwrap_or(false)
        })
        .then(|| "uses $CLAUDE_TOOL_FILE_PATH/$CLAUDE_TOOL_NAME, which no tool sets (the shim gives the JSON on stdin)".to_string())
}

/// `true` when a canonical matcher could match this Claude tool name
/// (letters-only matchers are exact lists, anything else a regex).
pub fn matcher_matches(matcher: &str, tool: &str) -> bool {
    let m = matcher.trim();
    if m.is_empty() || m == "*" {
        return true;
    }
    let simple = m
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | ' ' | ',' | '|'));
    if simple {
        return m
            .split(['|', ','])
            .map(str::trim)
            .any(|p| !p.is_empty() && p == tool);
    }
    regex::Regex::new(m)
        .map(|r| r.is_match(tool))
        .unwrap_or(false)
}

/// The installed name of the unit (collision rename honoured).
pub fn unit_name(c: &Component, ctx: &ConvertCtx) -> String {
    ctx.name_for(c)
}

/// `true` when the component is OmniGet's own observation hook.
pub fn is_observe(c: &Component) -> bool {
    matches!(&c.body, crate::core::agentkit::model::ComponentBody::Hook(h) if h.tags.iter().any(|t| t == super::hook_observe::OBSERVE_TAG))
}

pub fn missing(target: &TargetAdapter, what: &str, scope: Scope) -> AgentkitError {
    AgentkitError::new(
        "AGENTKIT_NO_PATH",
        format!(
            "{} has no {what} location for scope {}",
            target.name,
            scope.as_str()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_survives_the_shell() {
        assert_eq!(quote("python3", Os::Linux), "python3");
        assert_eq!(quote("a b", Os::Linux), "'a b'");
        assert_eq!(quote("it's", Os::Linux), "'it'\\''s'");
        assert_eq!(quote("C:\\x y\\a.exe", Os::Windows), "\"C:\\x y\\a.exe\"");
    }

    #[test]
    fn matcher_semantics_follow_claude() {
        assert!(matcher_matches("Edit|Write", "Write"));
        assert!(!matcher_matches("Edit|Write", "MultiEdit"));
        assert!(matcher_matches("Edit, Write", "Edit"));
        assert!(matcher_matches("mcp__memory__.*", "mcp__memory__save"));
        assert!(matcher_matches("^Notebook", "NotebookEdit"));
        assert!(matcher_matches("*", "Bash"));
        assert!(!matcher_matches("Bash", "Read"));
    }
}
