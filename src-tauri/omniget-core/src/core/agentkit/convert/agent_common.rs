//! Helpers shared by the agent, command, rule and skill converters of round 2
//! (`agent_*`, `command_*`, `rule_*`, `skill_link`): tool-name, model and
//! permission mapping, loss bookkeeping, reading installed files back into
//! canonical components, and small text emitters (YAML block scalars, TOML).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::{kind_path, ConvertCtx};
use crate::core::agentkit::edit::{toml as etoml, yaml as eyaml};
use crate::core::agentkit::model::*;
use crate::core::agentkit::parse::{self, placeholders};
use crate::core::agentkit::targets::TargetAdapter;
use crate::core::agentkit::{AgentkitError, Result, Scope};

/// Error for a target that has no location for a kind in a scope.
pub(crate) fn no_path(target: &TargetAdapter, what: &str, scope: Scope) -> AgentkitError {
    AgentkitError::new(
        "AGENTKIT_NO_PATH",
        format!(
            "{} has no {what} location for scope {}",
            target.name,
            scope.as_str()
        ),
    )
}

/// `kind_path` or an `AGENTKIT_NO_PATH` error.
pub(crate) fn dir_for(
    target: &TargetAdapter,
    key: &str,
    scope: Scope,
    ctx: &ConvertCtx,
) -> Result<PathBuf> {
    kind_path(target, key, scope, ctx).ok_or_else(|| no_path(target, key, scope))
}

pub(crate) fn push_unique(v: &mut Vec<String>, s: impl Into<String>) {
    let s = s.into();
    if !v.contains(&s) {
        v.push(s);
    }
}

// ------------------------------------------------------------------ tool names

/// Claude spellings that are aliases of another Claude tool.
fn claude_alias(t: &str) -> &str {
    match t {
        "Task" => "Agent",
        "MultiEdit" | "NotebookEdit" => "Edit",
        "NotebookRead" => "Read",
        "PowerShell" => "Bash",
        "LS" => "Glob",
        "TaskCreate" | "TaskUpdate" | "TaskList" | "TaskGet" => "TodoWrite",
        other => other,
    }
}

/// `Bash(git:*)` → (`Bash`, true).
pub(crate) fn tool_base(t: &str) -> (&str, bool) {
    match t.find('(') {
        Some(i) if t.ends_with(')') => (t[..i].trim(), true),
        _ => (t.trim(), false),
    }
}

/// `mcp__srv__tool` / `mcp__srv` / `mcp__srv__*` → (server, tool or `*`).
pub(crate) fn split_mcp(t: &str) -> Option<(String, String)> {
    let rest = t.strip_prefix("mcp__")?;
    let (srv, tool) = match rest.split_once("__") {
        Some((s, t)) => (s, if t.is_empty() { "*" } else { t }),
        None => (rest, "*"),
    };
    (!srv.is_empty()).then(|| (srv.to_string(), tool.to_string()))
}

/// Copilot chatmode / `.agent.md` tool id → Claude tools. `None` = no equivalent.
pub fn copilot_to_claude(id: &str) -> Option<Vec<String>> {
    let raw = id.trim().trim_matches(['\'', '"']);
    let lower = raw.to_ascii_lowercase();
    let pick = |s: &str| -> Option<&'static [&'static str]> {
        Some(match s {
            "execute"
            | "shell"
            | "bash"
            | "powershell"
            | "runcommands"
            | "runincommand"
            | "runinterminal"
            | "terminallastcommand"
            | "terminalselection"
            | "runtasks"
            | "runtests"
            | "gettaskoutput"
            | "getterminaloutput"
            | "testfailure"
            | "createandruntask"
            | "runtask" => &["Bash"],
            "read"
            | "readfile"
            | "notebookread"
            | "problems"
            | "changes"
            | "readcelloutput"
            | "readnotebookcelloutput"
            | "getnotebooksummary"
            | "view" => &["Read"],
            "edit" | "editfiles" | "multiedit" | "write" | "notebookedit" | "new"
            | "createfile" | "createdirectory" | "editnotebook" | "newworkspace" | "create"
            | "str_replace_editor" | "apply_patch" => &["Edit", "Write"],
            "search" | "codebase" | "usages" | "searchresults" | "findtestfiles" | "grep"
            | "glob" | "filesearch" | "textsearch" | "listdirectory" | "rg" => {
                &["Grep", "Glob", "Read"]
            }
            "web" => &["WebFetch", "WebSearch"],
            "fetch" | "web_fetch" | "opensimplebrowser" | "githubrepo" => &["WebFetch"],
            "websearch" | "web_search" => &["WebSearch"],
            "agent" | "runsubagent" | "custom-agent" | "task" => &["Agent"],
            "todo" | "todos" | "todowrite" | "update_todo" | "manage_todo_list" => &["TodoWrite"],
            "askquestions" | "ask_user" | "askuser" => &["AskUserQuestion"],
            _ => return None,
        })
    };
    if let Some((ns, leaf)) = lower.split_once('/') {
        if let Some(v) = pick(leaf) {
            return Some(v.iter().map(|s| s.to_string()).collect());
        }
        const BUILTIN_NS: &[&str] = &[
            "search",
            "web",
            "edit",
            "read",
            "execute",
            "vscode",
            "agent",
            "todo",
            "runcommands",
            "runtasks",
            "runnotebooks",
            "new",
        ];
        if BUILTIN_NS.contains(&ns) {
            return pick(ns).map(|v| v.iter().map(|s| s.to_string()).collect());
        }
        // `github/*`, `server/tool`: an MCP server
        let srv = raw.split_once('/').map(|(a, _)| a).unwrap_or(raw);
        let tool = raw.split_once('/').map(|(_, b)| b).unwrap_or("*");
        return Some(vec![if tool == "*" {
            format!("mcp__{srv}")
        } else {
            format!("mcp__{srv}__{tool}")
        }]);
    }
    pick(&lower).map(|v| v.iter().map(|s| s.to_string()).collect())
}

/// Claude tool → Copilot `.agent.md` tool alias (`execute`, `read`, `edit` …).
pub fn claude_to_copilot(t: &str) -> Option<String> {
    let (base, _) = tool_base(t);
    if let Some((srv, tool)) = split_mcp(base) {
        return Some(format!("{srv}/{tool}"));
    }
    Some(
        match claude_alias(base) {
            "Bash" => "execute",
            "Read" => "read",
            "Edit" | "Write" => "edit",
            "Grep" | "Glob" => "search",
            "Agent" => "agent",
            "WebFetch" | "WebSearch" => "web",
            "TodoWrite" => "todo",
            _ => return None,
        }
        .to_string(),
    )
}

/// An agent's tools in Claude names. Copilot chatmodes (`origin_tool = copilot`)
/// are translated back; ids with no Claude equivalent come back as losses.
pub fn claude_tools(tools: &[String], origin: &str) -> (Vec<String>, Vec<String>) {
    let mut out: Vec<String> = Vec::new();
    let mut lost: Vec<String> = Vec::new();
    for t in tools {
        if origin == "copilot" {
            match copilot_to_claude(t) {
                Some(v) => v.into_iter().for_each(|x| push_unique(&mut out, x)),
                None => push_unique(&mut lost, format!("Copilot tool `{t}`")),
            }
        } else {
            push_unique(&mut out, t.clone());
        }
    }
    (out, lost)
}

/// Claude tool names → the target's names via `tool_name_map`. Returns the
/// mapped names and what could not be expressed.
pub fn map_tools(tools: &[String], target: &TargetAdapter) -> (Vec<String>, Vec<String>) {
    let mut out: Vec<String> = Vec::new();
    let mut lost: Vec<String> = Vec::new();
    for t in tools {
        let (base, patterned) = tool_base(t);
        if patterned && target.id != "claude" {
            push_unique(
                &mut lost,
                "tool argument patterns such as Bash(git:*) (the whole tool is allowed)",
            );
        }
        if let Some((srv, tool)) = split_mcp(base) {
            match target.tool_name_map.get("mcp").filter(|p| !p.is_empty()) {
                Some(p) => push_unique(
                    &mut out,
                    p.replace("{server}", &srv).replace("{tool}", &tool),
                ),
                None if target.id == "claude" => push_unique(&mut out, base.to_string()),
                None => push_unique(&mut lost, format!("MCP tool `{base}`")),
            }
            continue;
        }
        let name = claude_alias(base);
        let mapped = target
            .tool_name_map
            .get(base)
            .or_else(|| target.tool_name_map.get(name));
        match mapped {
            Some(m) if m.trim().is_empty() => {
                push_unique(&mut lost, format!("tool `{base}` (no equivalent)"))
            }
            Some(m) => {
                for part in m.split(',') {
                    push_unique(&mut out, part.trim().to_string());
                }
            }
            None if target.id == "claude" || target.tool_name_map.is_empty() => {
                push_unique(&mut out, base.to_string())
            }
            None => push_unique(&mut lost, format!("tool `{base}` (no equivalent)")),
        }
    }
    (out, lost)
}

/// The target's tool name → Claude name (inverse of `tool_name_map`).
pub fn reverse_tool(target: &TargetAdapter, name: &str) -> Option<String> {
    const ORDER: &[&str] = &[
        "Bash",
        "Read",
        "Edit",
        "Write",
        "Glob",
        "Grep",
        "WebFetch",
        "WebSearch",
        "Agent",
        "TodoWrite",
        "Skill",
        "AskUserQuestion",
    ];
    let n = name.trim();
    for k in ORDER {
        if let Some(v) = target.tool_name_map.get(*k) {
            if v.split(',').any(|p| p.trim().eq_ignore_ascii_case(n)) && !v.is_empty() {
                return Some((*k).to_string());
            }
        }
    }
    for (k, v) in &target.tool_name_map {
        if k != "mcp" && !v.is_empty() && v.split(',').any(|p| p.trim().eq_ignore_ascii_case(n)) {
            return Some(k.clone());
        }
    }
    if let Some(p) = target
        .tool_name_map
        .get("mcp")
        .filter(|p| p.contains("{server}"))
    {
        if let Some((srv, tool)) = unpattern(p, n) {
            return Some(if tool == "*" {
                format!("mcp__{srv}")
            } else {
                format!("mcp__{srv}__{tool}")
            });
        }
    }
    // already a Claude name (tools that accept Claude aliases)
    ORDER
        .iter()
        .find(|k| k.eq_ignore_ascii_case(n))
        .map(|k| k.to_string())
}

/// Inverse of an MCP id pattern like `mcp_{server}_{tool}`.
fn unpattern(pattern: &str, name: &str) -> Option<(String, String)> {
    let (pre, rest) = pattern.split_once("{server}")?;
    let (sep, post) = rest.split_once("{tool}")?;
    let mid = name.strip_prefix(pre)?.strip_suffix(post)?;
    if sep.is_empty() {
        return None;
    }
    let (s, t) = mid.split_once(sep)?;
    (!s.is_empty() && !t.is_empty()).then(|| (s.to_string(), t.to_string()))
}

/// Reverses a list of target tool names; unknown ones are kept as-is.
pub fn reverse_tools(target: &TargetAdapter, names: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for n in names {
        push_unique(
            &mut out,
            reverse_tool(target, n).unwrap_or_else(|| n.clone()),
        );
    }
    out
}

/// A list of Claude tools grants no edit and no shell.
pub fn is_read_only(tools: &[String]) -> bool {
    !tools.is_empty()
        && tools.iter().all(|t| {
            let (b, _) = tool_base(t);
            !matches!(claude_alias(b), "Bash" | "Edit" | "Write")
        })
}

// ------------------------------------------------------------------ model

const CLAUDE_ALIASES: &[&str] = &["sonnet", "opus", "haiku", "fable", "inherit"];

/// Model value for an agent/command on a target, plus a loss when dropped.
pub fn model_for(
    model: Option<&str>,
    origin: &str,
    target: &TargetAdapter,
) -> (Option<String>, Option<String>) {
    let Some(m) = model.map(str::trim).filter(|m| !m.is_empty()) else {
        return (None, None);
    };
    if target.id == "claude" {
        return (Some(m.to_string()), None);
    }
    if origin == "copilot" && target.id != "copilot" {
        return (
            None,
            Some(format!(
                "model `{m}` (a Copilot model name; {} picks its own)",
                target.name
            )),
        );
    }
    if CLAUDE_ALIASES.contains(&m) {
        return match target.model_map.get(m).filter(|x| !x.is_empty()) {
            Some(x) => (Some(x.clone()), None),
            None if m == "inherit" => (None, None),
            None => (
                None,
                Some(format!(
                    "model `{m}` (a Claude alias; {} uses its default model)",
                    target.name
                )),
            ),
        };
    }
    // a concrete model id
    let lower = m.to_ascii_lowercase();
    let keep = match target.id.as_str() {
        "opencode" | "kilo" => {
            if m.contains('/') {
                return (Some(m.to_string()), None);
            }
            if lower.starts_with("claude") {
                return (Some(format!("anthropic/{m}")), None);
            }
            if lower.starts_with("gpt") || lower.starts_with('o') {
                return (Some(format!("openai/{m}")), None);
            }
            if lower.starts_with("gemini") {
                return (Some(format!("google/{m}")), None);
            }
            false
        }
        "codex" => {
            lower.starts_with("gpt")
                || lower.starts_with("o3")
                || lower.starts_with("o4")
                || lower.contains("codex")
        }
        "gemini" => lower.starts_with("gemini"),
        "vibe" => ["mistral", "devstral", "codestral", "magistral"]
            .iter()
            .any(|p| lower.starts_with(p)),
        _ => true,
    };
    if keep {
        (Some(m.to_string()), None)
    } else {
        (
            None,
            Some(format!("model `{m}` (not available on {})", target.name)),
        )
    }
}

/// Undoes `model_for` for import (`anthropic/claude-x` → `claude-x`).
pub fn model_back(m: &str) -> String {
    m.strip_prefix("anthropic/").unwrap_or(m).to_string()
}

// ------------------------------------------------------------------ losses

/// Losses for every set agent field that is not in `keep` (Claude names).
pub fn dropped_fields(a: &AgentSpec, keep: &[&str], target: &TargetAdapter) -> Vec<String> {
    let mut v = Vec::new();
    let mut lose = |set: bool, field: &str| {
        if set && !keep.contains(&field) {
            v.push(format!(
                "agent field `{field}` (not supported by {})",
                target.name
            ));
        }
    };
    lose(!a.tools.is_empty(), "tools");
    lose(!a.disallowed_tools.is_empty(), "disallowedTools");
    lose(
        a.permission_mode
            .as_deref()
            .map(|p| p != "default")
            .unwrap_or(false),
        "permissionMode",
    );
    lose(a.max_turns.is_some(), "maxTurns");
    lose(!a.skills.is_empty(), "skills");
    lose(!a.mcp_servers.is_empty(), "mcpServers");
    lose(!a.hooks.is_null(), "hooks");
    lose(a.color.is_some(), "color");
    lose(a.background.is_some(), "background");
    lose(a.effort.is_some(), "effort");
    lose(a.isolation.is_some(), "isolation");
    lose(a.memory.is_some(), "memory");
    lose(a.initial_prompt.is_some(), "initialPrompt");
    for k in a.extra.keys() {
        if !keep.contains(&k.as_str()) {
            v.push(format!(
                "agent field `{k}` (not supported by {})",
                target.name
            ));
        }
    }
    v
}

/// The agent spec of an agent component.
pub(crate) fn agent_of(c: &Component) -> Option<&AgentSpec> {
    match &c.body {
        ComponentBody::Agent(a) => Some(a),
        _ => None,
    }
}

/// The command spec of a command component.
pub(crate) fn command_of(c: &Component) -> Option<&CommandSpec> {
    match &c.body {
        ComponentBody::Command(x) => Some(x),
        _ => None,
    }
}

/// Body with CRLF folded and a single leading newline dropped.
pub(crate) fn clean_body(s: &str) -> String {
    let t = s.replace("\r\n", "\n");
    t.strip_prefix('\n').unwrap_or(&t).to_string()
}

/// Claude colors → hex (OpenCode wants hex or a theme token).
pub(crate) fn color_hex(c: &str) -> String {
    match c {
        "red" => "#ef4444",
        "blue" => "#3b82f6",
        "green" => "#22c55e",
        "yellow" => "#eab308",
        "purple" => "#a855f7",
        "orange" => "#f97316",
        "pink" => "#ec4899",
        "cyan" => "#06b6d4",
        other => return other.to_string(),
    }
    .to_string()
}

/// Hex → Claude color name when it is one of ours.
pub(crate) fn color_name(c: &str) -> String {
    for n in [
        "red", "blue", "green", "yellow", "purple", "orange", "pink", "cyan",
    ] {
        if color_hex(n).eq_ignore_ascii_case(c) {
            return n.to_string();
        }
    }
    c.to_string()
}

// ------------------------------------------------------------------ placeholders

/// Neutral command body → a target dialect. Dialects without an arguments
/// placeholder (the typed text is appended) get a readable stand-in.
pub fn render_body(body: &str, map: &BTreeMap<String, String>) -> (String, Vec<String>) {
    let no_args = map.get("args").map(|s| s.is_empty()).unwrap_or(true);
    let src = if no_args && body.contains("{{args}}") {
        body.replace("{{args}}", "<arguments>")
    } else {
        body.to_string()
    };
    let (mut text, mut lost) = placeholders::render(&src, map);
    if no_args && body.contains("{{args}}") {
        push_unique(
            &mut lost,
            "arguments placeholder (the text typed after the command is appended instead)",
        );
        if !text.ends_with('\n') {
            text.push('\n');
        }
    }
    (text, lost)
}

fn is_claude_dialect(map: &BTreeMap<String, String>) -> bool {
    map.get("args").map(String::as_str) == Some("$ARGUMENTS")
        && map.get("shell").map(String::as_str) == Some("!`{cmd}`")
}

/// A target dialect → neutral markers (inverse of `render_body` for the parts
/// the dialect has).
pub fn canonical_body(text: &str, map: &BTreeMap<String, String>) -> String {
    if is_claude_dialect(map) {
        return placeholders::to_canonical(text);
    }
    let mut out = text.to_string();
    // `{{ args }}` (Goose) and dialect args → {{args}}
    if let Some(a) = map.get("args").filter(|a| !a.is_empty()) {
        if a != "{{args}}" {
            out = out.replace(a.as_str(), "{{args}}");
        }
    }
    for (key, var) in [("shell", "{cmd}"), ("file", "{path}")] {
        let Some(tpl) = map.get(key).filter(|t| !t.is_empty()) else {
            continue;
        };
        // `!{{cmd}}` renders `!{` + cmd + `}`: split on the bare variable
        let Some((pre, post)) = tpl.split_once(var) else {
            continue;
        };
        if post.is_empty() || pre.is_empty() {
            continue;
        }
        let mut res = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(a) = rest.find(pre) {
            let after = &rest[a + pre.len()..];
            match after.find(post) {
                Some(e) if !after[..e].contains('\n') && !after[..e].is_empty() => {
                    res.push_str(&rest[..a]);
                    res.push_str(&format!("{{{{{}:{}}}}}", key, &after[..e]));
                    rest = &after[e + post.len()..];
                }
                _ => {
                    res.push_str(&rest[..a + pre.len()]);
                    rest = after;
                }
            }
        }
        res.push_str(rest);
        out = res;
    }
    if let Some(a) = map.get("arg").filter(|a| a.contains('N')) {
        // `$N` style: `$1`..`$9`
        if let Some(pre) = a.split('N').next().filter(|p| !p.is_empty()) {
            let mut res = String::new();
            let mut rest = out.as_str();
            while let Some(i) = rest.find(pre) {
                let after = &rest[i + pre.len()..];
                let d: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
                res.push_str(&rest[..i]);
                if d.len() == 1 && d != "0" {
                    res.push_str(&format!("{{{{arg:{d}}}}}"));
                    rest = &after[1..];
                } else {
                    res.push_str(pre);
                    rest = after;
                }
            }
            res.push_str(rest);
            out = res;
        }
    }
    out
}

// ------------------------------------------------------------------ import helpers

/// Files with one of the suffixes directly inside `dir`, sorted.
pub(crate) fn files_with(dir: &Path, suffixes: &[&str]) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| suffixes.iter().any(|s| n.ends_with(s)))
                            .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// File stem without one of the suffixes (`x.agent.md` → `x`).
pub(crate) fn stem_of(p: &Path, suffixes: &[&str]) -> String {
    let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    for s in suffixes {
        if let Some(x) = n.strip_suffix(s) {
            return x.to_string();
        }
    }
    n.to_string()
}

/// Canonical agent component from a spec read back from a tool's file.
pub(crate) fn agent_component(
    spec: &AgentSpec,
    target: &TargetAdapter,
    path: &Path,
) -> Option<Component> {
    let name = if spec.name.trim().is_empty() {
        stem_of(path, &[".agent.md", ".md", ".toml", ".json", ".yaml"])
    } else {
        spec.name.clone()
    };
    let text =
        parse::render_frontmatter(&super::claude::agent_frontmatter(spec, &name), &spec.prompt);
    let entry = format!("{}.md", parse::sanitize_name(&name));
    let files: parse::RawFiles = [(entry.clone(), text.into_bytes())].into_iter().collect();
    let mut c = parse::parse_raw(ComponentKind::Agent, &entry, &files).ok()?;
    super::claude::tag_installed(&mut c, target, path);
    Some(c)
}

/// Canonical command component from a spec read back from a tool's file.
pub(crate) fn command_component(
    name: &str,
    spec: &CommandSpec,
    target: &TargetAdapter,
    path: &Path,
) -> Option<Component> {
    let mut fm: Vec<(String, Value)> = Vec::new();
    if !spec.allowed_tools.is_empty() {
        fm.push(("allowed-tools".into(), json!(spec.allowed_tools.join(", "))));
    }
    if let Some(h) = &spec.argument_hint {
        fm.push(("argument-hint".into(), json!(h)));
    }
    if !spec.description.is_empty() {
        fm.push(("description".into(), json!(spec.description)));
    }
    if let Some(m) = &spec.model {
        fm.push(("model".into(), json!(m)));
    }
    if let Some(a) = &spec.agent {
        fm.push(("agent".into(), json!(a)));
    }
    let text = parse::render_frontmatter(&fm, &placeholders::to_claude(&spec.body));
    let entry = format!("{}.md", parse::sanitize_name(name));
    let files: parse::RawFiles = [(entry.clone(), text.into_bytes())].into_iter().collect();
    let mut c = parse::parse_raw(ComponentKind::Command, &entry, &files).ok()?;
    super::claude::tag_installed(&mut c, target, path);
    Some(c)
}

pub(crate) fn str_of(m: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| match m.get(*k) {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        _ => None,
    })
}

pub(crate) fn u32_of(m: &Map<String, Value>, keys: &[&str]) -> Option<u32> {
    keys.iter().find_map(|k| {
        m.get(*k).and_then(|v| {
            v.as_u64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
                .map(|n| n as u32)
        })
    })
}

// ------------------------------------------------------------------ emitters

/// `key: value` YAML, with multi-line strings as literal block scalars.
pub(crate) fn yaml_entry(key: &str, v: &Value, indent: usize) -> String {
    if let Value::String(s) = v {
        // a first line that starts with a space would need an indentation
        // indicator; those strings stay quoted
        let lead_space = s.starts_with(' ') || s.starts_with('\t');
        if s.contains('\n') && !lead_space {
            let pad = " ".repeat(indent + 2);
            let chomp = if s.ends_with('\n') { "" } else { "-" };
            let mut out = format!("{}{}: |{chomp}\n", " ".repeat(indent), eyaml_key(key));
            let body = s.strip_suffix('\n').unwrap_or(s);
            for line in body.split('\n') {
                if line.is_empty() {
                    out.push('\n');
                } else {
                    out.push_str(&pad);
                    out.push_str(line);
                    out.push('\n');
                }
            }
            return out;
        }
    }
    eyaml::entry_text(key, v, indent)
}

fn eyaml_key(k: &str) -> String {
    let t = eyaml::scalar_text(&json!(k));
    if t.starts_with('"') {
        t
    } else {
        k.to_string()
    }
}

/// Whole YAML document from ordered entries.
pub(crate) fn yaml_doc(entries: &[(&str, Value)]) -> String {
    let mut out = String::new();
    for (k, v) in entries {
        if v.is_null() {
            continue;
        }
        out.push_str(&yaml_entry(k, v, 0));
    }
    out
}

/// `key = value` TOML line.
pub(crate) fn toml_line(k: &str, v: &Value) -> String {
    format!("{} = {}\n", etoml::key_text(k), etoml::inline(v))
}

/// Claude MCP record (agent `mcpServers` entry) normalised to a plain
/// `{command,args,env,cwd}` / `{url,headers}` object, `type` dropped.
pub(crate) fn plain_mcp(v: &Value) -> Value {
    let mut m = match v {
        Value::Object(m) => m.clone(),
        other => return other.clone(),
    };
    m.remove("type");
    Value::Object(m)
}

/// Claude MCP record → Codex TOML table body (`command`, `args`, `env`, `cwd` /
/// `url`, `http_headers`).
pub(crate) fn codex_mcp_lines(v: &Value) -> (String, Option<String>) {
    let Some(m) = v.as_object() else {
        return (
            String::new(),
            Some("inline MCP server that is only a name".into()),
        );
    };
    let mut out = String::new();
    let t = m.get("type").and_then(|x| x.as_str()).unwrap_or("");
    if let Some(url) = m.get("url").or(m.get("httpUrl")).or(m.get("serverUrl")) {
        if t == "sse" {
            return (
                String::new(),
                Some("SSE MCP server (Codex has no SSE)".into()),
            );
        }
        out.push_str(&toml_line("url", url));
        if let Some(h) = m
            .get("headers")
            .filter(|h| h.as_object().map(|o| !o.is_empty()).unwrap_or(false))
        {
            out.push_str(&toml_line("http_headers", h));
        }
        return (out, None);
    }
    for k in ["command", "args", "env", "cwd"] {
        if let Some(x) = m.get(k) {
            let empty = match x {
                Value::Array(a) => a.is_empty(),
                Value::Object(o) => o.is_empty(),
                Value::Null => true,
                _ => false,
            };
            if !empty {
                out.push_str(&toml_line(k, x));
            }
        }
    }
    (out, None)
}

/// Codex TOML MCP table → Claude MCP record.
pub(crate) fn codex_mcp_back(v: &Value) -> Value {
    let Some(m) = v.as_object() else {
        return v.clone();
    };
    let mut o = Map::new();
    if let Some(u) = m.get("url") {
        o.insert("type".into(), json!("http"));
        o.insert("url".into(), u.clone());
        if let Some(h) = m.get("http_headers") {
            o.insert("headers".into(), h.clone());
        }
    } else {
        for k in ["command", "args", "env", "cwd"] {
            if let Some(x) = m.get(k) {
                o.insert(k.into(), x.clone());
            }
        }
    }
    Value::Object(o)
}

/// Relative path from `from_dir` to `to` when `to` is under `from_dir`, with `/`.
pub(crate) fn rel_slash(from_dir: &Path, to: &Path) -> Option<String> {
    let r = to.strip_prefix(from_dir).ok()?;
    Some(
        r.components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::agentkit::targets;

    #[test]
    fn copilot_tools_come_back_as_claude_names() {
        let tools: Vec<String> = [
            "codebase",
            "editFiles",
            "runCommands",
            "fetch",
            "github/*",
            "vscodeAPI",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let (t, lost) = claude_tools(&tools, "copilot");
        for want in [
            "Grep",
            "Glob",
            "Read",
            "Edit",
            "Write",
            "Bash",
            "WebFetch",
            "mcp__github",
        ] {
            assert!(t.contains(&want.to_string()), "{want} in {t:?}");
        }
        assert_eq!(lost, vec!["Copilot tool `vscodeAPI`".to_string()]);
    }

    #[test]
    fn tools_map_and_reverse() {
        let g = targets::target("gemini").unwrap();
        let (m, lost) = map_tools(
            &[
                "Read".into(),
                "Edit".into(),
                "Bash(git:*)".into(),
                "mcp__gh__issue".into(),
            ],
            g,
        );
        assert_eq!(
            m,
            vec!["read_file", "replace", "run_shell_command", "mcp_gh_issue"]
        );
        assert_eq!(lost.len(), 1);
        assert_eq!(
            reverse_tools(g, &m),
            vec!["Read", "Edit", "Bash", "mcp__gh__issue"]
        );
    }

    #[test]
    fn body_placeholders_round_trip_per_dialect() {
        let canon =
            "Do {{args}} now; branch {{shell:git branch}} and {{file:src/a.rs}}; first {{arg:1}}.";
        for id in ["gemini", "qwen", "opencode", "pi", "kiro"] {
            let t = targets::target(id).unwrap();
            let (text, _) = render_body(canon, &t.placeholder_map);
            let back = canonical_body(&text, &t.placeholder_map);
            assert!(back.contains("{{args}}"), "{id}: {back}");
            if t.placeholder_map
                .get("shell")
                .map(|s| !s.is_empty())
                .unwrap_or(false)
            {
                assert!(back.contains("{{shell:git branch}}"), "{id}: {back}");
                assert!(back.contains("{{file:src/a.rs}}"), "{id}: {back}");
            }
            if t.placeholder_map
                .get("arg")
                .map(|s| !s.is_empty())
                .unwrap_or(false)
            {
                assert!(back.contains("{{arg:1}}"), "{id}: {back}");
            }
        }
    }

    #[test]
    fn yaml_block_scalars_parse_back() {
        let text = yaml_doc(&[
            ("title", json!("x")),
            (
                "prompt",
                json!("line one\n  indented: yes # not a comment\n\nlast\n"),
            ),
            ("instructions", json!("  starts indented\nsecond")),
        ]);
        let v = crate::core::agentkit::edit::yaml::parse(&text).unwrap();
        assert_eq!(
            v["prompt"],
            "line one\n  indented: yes # not a comment\n\nlast\n"
        );
        assert_eq!(v["instructions"], "  starts indented\nsecond");
    }
    #[test]
    fn compat_matrix_has_no_errors() {
        use crate::core::agentkit::convert::compute_compat;
        let mk = |kind: ComponentKind, entry: &str, text: &str| {
            let files: parse::RawFiles = [(entry.to_string(), text.as_bytes().to_vec())]
                .into_iter()
                .collect();
            parse::parse_raw(kind, entry, &files).unwrap()
        };
        let comps = [
            mk(
                ComponentKind::Agent,
                "a.md",
                "---\nname: a\ndescription: d\ntools: Read, Bash\nmodel: sonnet\n---\nBody\n",
            ),
            mk(
                ComponentKind::Command,
                "c.md",
                "---\ndescription: d\n---\nDo $ARGUMENTS\n",
            ),
            mk(
                ComponentKind::Rule,
                "r.md",
                "---\npaths: [\"src/**\"]\n---\nRule\n",
            ),
            mk(ComponentKind::Rule, "root.md", "Root rule\n"),
            mk(
                ComponentKind::Skill,
                "s/SKILL.md",
                "---\nname: s\ndescription: d\n---\nB\n",
            ),
        ];
        let all = targets::all_targets();
        for c in &comps {
            for (t, compat) in compute_compat(c, all) {
                if let Compat::Unsupported { reason } = &compat {
                    assert!(!reason.contains("AGENTKIT_"), "{} on {t}: {reason}", c.kind);
                }
            }
        }
    }
}
