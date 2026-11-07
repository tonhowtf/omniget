use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde_json::Value;

use super::{place_support, rewrite_command, ConvertCtx, PlannedFile, SupportPlan};
use crate::core::agentkit::model::{Component, HookEntry, HookSpec};
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{AgentkitError, Env, Os, Result, Scope};

pub const SHIM_NAME: &str = "omniget-hook-shim";

pub const SHIM_SIDECAR: &str = "omniget-hook";

pub fn usable_binary(p: &Path) -> bool {
    use std::io::Read;
    let Ok(meta) = std::fs::metadata(p) else {
        return false;
    };
    if !meta.is_file() || meta.len() < 4 {
        return false;
    }
    let mut head = [0u8; 4];
    let Ok(mut f) = std::fs::File::open(p) else {
        return false;
    };
    if f.read_exact(&mut head).is_err() {
        return false;
    }
    matches!(
        head,
        [0x7f, b'E', b'L', b'F']
            | [0xcf, 0xfa, 0xed, 0xfe]
            | [0xce, 0xfa, 0xed, 0xfe]
            | [0xfe, 0xed, 0xfa, 0xcf]
            | [0xfe, 0xed, 0xfa, 0xce]
            | [0xca, 0xfe, 0xba, 0xbe]
            | [b'M', b'Z', _, _]
            | [b'#', b'!', _, _]
    )
}

static SHIM_OVERRIDE: RwLock<Option<PathBuf>> = RwLock::new(None);

pub fn set_shim_path(p: Option<PathBuf>) {
    if let Ok(mut g) = SHIM_OVERRIDE.write() {
        *g = p;
    }
}

pub fn pinned_shim() -> Option<PathBuf> {
    SHIM_OVERRIDE.read().ok().and_then(|g| g.clone())
}

fn exe_name(os: Os) -> String {
    if os == Os::Windows {
        format!("{SHIM_NAME}.exe")
    } else {
        SHIM_NAME.to_string()
    }
}

pub fn shim_path(env: &Env) -> PathBuf {
    if let Some(p) = SHIM_OVERRIDE.read().ok().and_then(|g| g.clone()) {
        return p;
    }
    let name = exe_name(env.os);
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

pub fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

#[derive(Debug, Clone, Default)]
pub struct ShimCall<'a> {
    pub tool: &'a str,
    pub event: &'a str,
    pub native_event: Option<&'a str>,
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

pub fn shim_command(env: &Env, call: &ShimCall, original: &str) -> String {
    let os = env.os;
    let mut parts = vec![quote(&shim_path(env).display().to_string(), os)];
    parts.extend(call.args().iter().map(|a| quote(a, os)));
    parts.push("--".into());
    parts.push(quote(original, os));
    parts.join(" ")
}

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

pub fn support_files(
    c: &Component,
    h: &HookSpec,
    target: &TargetAdapter,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<SupportPlan> {
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

pub(crate) fn apply_rewrites(cmd: &str, rewrites: &[(String, String)]) -> String {
    let mut out = cmd.to_string();
    for (from, to) in rewrites {
        out = rewrite_command(&out, from, to);
    }
    out
}

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

pub fn num(v: f64) -> Value {
    if v.fract() == 0.0 {
        Value::from(v as i64)
    } else {
        Value::from(v)
    }
}

pub fn shim_note(env: &Env) -> String {
    format!(
        "hook commands run through {} (translates the tool's hook input/output to Claude's); it must stay at that path",
        shim_path(env).display()
    )
}

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

pub fn unit_name(c: &Component, ctx: &ConvertCtx) -> String {
    ctx.name_for(c)
}

pub(crate) fn missing(target: &TargetAdapter, what: &str, scope: Scope) -> AgentkitError {
    AgentkitError::new(
        "AGENTKIT_NO_PATH",
        format!(
            "{} has no {what} location for scope {}",
            target.name,
            scope.as_str()
        ),
    )
}
