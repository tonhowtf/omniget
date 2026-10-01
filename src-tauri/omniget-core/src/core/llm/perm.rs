//! Permission rules on top of the per-tool grant: `{tool, pattern, action}` per
//! agent, the last rule that matches wins, no match means "ask". Shape read
//! from opencode (`permission/next.ts`, `bash arity`): "Always" on a shell ask
//! stores the command prefix up to the arity of the program (`git status`,
//! `npm run test`), never the whole line.
//!
//! One source of truth (3.4, S19): the native broker, the Claude Code
//! `PreToolUse` hook (`omniget hook claude`, [`super::claude_hooks`]) and the
//! CLI's permission prompt tool all decide with [`gate`] over the same rules,
//! after mapping Claude's built-in tools onto the native names
//! ([`from_claude`]): a "deny `fs_write` `**/.env`" bites a `Write` of the CLI
//! exactly as it bites the native `fs_write`, and an "Always" given in one
//! place counts in the other.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::agent::GrantMode;
use super::run_mode::RunMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    Deny,
    Ask,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub tool: String,
    pub pattern: String,
    pub action: Action,
}

static RULES: RwLock<Option<HashMap<String, Vec<Rule>>>> = RwLock::new(None);
static FILE: RwLock<Option<PathBuf>> = RwLock::new(None);

pub fn set_store_file(path: PathBuf) {
    let loaded: HashMap<String, Vec<Rule>> = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    *RULES.write().unwrap_or_else(|e| e.into_inner()) = Some(loaded);
    *FILE.write().unwrap_or_else(|e| e.into_inner()) = Some(path);
}

/// The file the rules live in (the hook of a CLI turn reads it on every
/// call, so a rule saved mid-turn counts from the next call on).
pub fn store_file() -> Option<PathBuf> {
    FILE.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// The rules of `agent` as saved in `file` (the hook's side of
/// [`rules_of`]). A missing file is no rules; an unreadable or malformed one
/// is an error, because the hook must not read "no rules" into it.
pub fn rules_in_file(file: &Path, agent: &str) -> Result<Vec<Rule>, String> {
    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{}: {e}", file.display())),
    };
    let map: HashMap<String, Vec<Rule>> =
        serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", file.display()))?;
    Ok(map.get(agent).cloned().unwrap_or_default())
}

fn save(map: &HashMap<String, Vec<Rule>>) {
    if let Some(file) = FILE.read().unwrap_or_else(|e| e.into_inner()).clone() {
        if let Ok(bytes) = serde_json::to_vec_pretty(map) {
            let _ = std::fs::write(file, bytes);
        }
    }
}

pub fn rules_of(agent: &str) -> Vec<Rule> {
    RULES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(agent).cloned())
        .unwrap_or_default()
}

pub fn set_rules(agent: &str, rules: Vec<Rule>) {
    let mut guard = RULES.write().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(Default::default);
    if rules.is_empty() {
        map.remove(agent);
    } else {
        map.insert(agent.to_string(), rules);
    }
    save(map);
}

pub fn add_rule(agent: &str, rule: Rule) {
    let mut rules = rules_of(agent);
    rules.retain(|r| !(r.tool == rule.tool && r.pattern == rule.pattern));
    rules.push(rule);
    set_rules(agent, rules);
}

/// How many leading words identify "the same command". Default is 1, which is
/// also what interpreters get (`node *`): their second word is a script path,
/// not a subcommand.
const ARITY: &[(&str, usize)] = &[
    ("npm run", 3),
    ("pnpm run", 3),
    ("yarn run", 3),
    ("bun run", 3),
    ("cargo run", 2),
    ("docker compose", 3),
    ("git", 2),
    ("npm", 2),
    ("pnpm", 2),
    ("yarn", 2),
    ("bun", 2),
    ("cargo", 2),
    ("go", 2),
    ("docker", 2),
    ("kubectl", 2),
    ("gh", 3),
    ("make", 2),
    ("pip", 2),
    ("uv", 2),
    ("brew", 2),
];

/// `git status --short` → `git status`; `npm run test -- -t x` → `npm run test`.
pub fn command_prefix(command: &str) -> String {
    let words: Vec<&str> = command.split_whitespace().collect();
    if words.is_empty() {
        return String::new();
    }
    let mut arity = 1;
    for (head, n) in ARITY {
        let head_words: Vec<&str> = head.split(' ').collect();
        if words.len() >= head_words.len() && words[..head_words.len()] == head_words[..] {
            arity = *n;
            break;
        }
    }
    words[..arity.min(words.len())].join(" ")
}

/// A shell line is only as safe as each command in it: `git status && rm -rf x`
/// must not ride on an "always git status".
fn segments(command: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let chars: Vec<char> = command.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let two = chars.get(i + 1).map(|n| (c, *n));
        if matches!(two, Some(('&', '&')) | Some(('|', '|'))) {
            out.push(std::mem::take(&mut cur));
            i += 2;
            continue;
        }
        if matches!(c, ';' | '|' | '&' | '\n') {
            out.push(std::mem::take(&mut cur));
            i += 1;
            continue;
        }
        cur.push(c);
        i += 1;
    }
    out.push(cur);
    out.into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn opaque(command: &str) -> bool {
    command.contains("$(")
        || command.contains('`')
        || command.contains("<(")
        || command.contains('>')
}

/// `*` matches anything; a pattern ending in ` *` also matches the bare prefix.
pub fn matches(pattern: &str, subject: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    // Glob habit: `**/.env` also means the `.env` at the top.
    if let Some(rest) = pattern.strip_prefix("**/") {
        if matches(rest, subject) {
            return true;
        }
    }
    if let Some(prefix) = pattern.strip_suffix(" *") {
        if subject == prefix {
            return true;
        }
    }
    let mut re = String::from("^");
    for c in pattern.chars() {
        match c {
            '*' => re.push_str(".*"),
            c => re.push_str(&regex::escape(&c.to_string())),
        }
    }
    re.push('$');
    regex::Regex::new(&re)
        .map(|r| r.is_match(subject))
        .unwrap_or(false)
}

/// What the rules are matched against: the command for the shell, the path for
/// the file tools, `*` otherwise.
pub fn subject(tool: &str, input: &Value) -> String {
    let get = |k: &str| {
        input
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    match tool {
        "shell_exec" => get("command"),
        "fs_read" | "fs_list" | "fs_glob" | "fs_grep" | "fs_edit" | "fs_write" => get("path"),
        _ => "*".to_string(),
    }
}

fn decide_one(rules: &[Rule], tool: &str, subject: &str) -> Option<Action> {
    rules
        .iter()
        .rev()
        .find(|r| (r.tool == tool || r.tool == "*") && matches(&r.pattern, subject))
        .map(|r| r.action)
}

/// `None` = no rule has an opinion, the grant mode stands.
pub fn decide(agent: &str, tool: &str, input: &Value) -> Option<Action> {
    decide_in(&rules_of(agent), tool, input)
}

/// [`decide`] over a given rule list (the hook reads its own copy).
pub fn decide_in(rules: &[Rule], tool: &str, input: &Value) -> Option<Action> {
    if rules.is_empty() {
        return None;
    }
    let subject = subject(tool, input);
    if tool != "shell_exec" {
        return decide_one(rules, tool, &subject);
    }
    let parts = segments(&subject);
    let mut verdict = Some(Action::Allow);
    for part in &parts {
        match decide_one(rules, tool, part) {
            Some(Action::Deny) => return Some(Action::Deny),
            Some(Action::Allow) => {}
            _ => verdict = None,
        }
    }
    if verdict == Some(Action::Allow) && (parts.is_empty() || opaque(&subject)) {
        return None;
    }
    verdict
}

/// The rule behind a [`decide_in`] answer, for the reason shown next to a
/// denial (the last matching rule with that action, on the whole subject or
/// on one segment of a shell line).
pub fn rule_for(rules: &[Rule], tool: &str, input: &Value, action: Action) -> Option<Rule> {
    let subject = subject(tool, input);
    let mut subjects = vec![subject.clone()];
    if tool == "shell_exec" {
        subjects.extend(segments(&subject));
    }
    rules
        .iter()
        .rev()
        .find(|r| {
            r.action == action
                && (r.tool == tool || r.tool == "*")
                && subjects.iter().any(|s| matches(&r.pattern, s))
        })
        .cloned()
}

// ── one gate for every harness (3.4) ────────────────────────────────────

/// What settled a [`gate`] verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The agent's grant for the tool (the CLI's: [`cli_grant`]).
    Grant,
    /// The turn's run mode moved the grant (2.7).
    RunMode,
    /// A stored rule.
    Rule,
    /// A tool that asks every time (washmybelly plans).
    AlwaysAsks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verdict {
    pub mode: GrantMode,
    pub source: Source,
}

/// The permission of one call, in the order the native broker has always
/// applied it: the grant, the run mode inside the grant (a `Deny` grant is
/// never lifted), the stored rules, then the always-ask tools. The
/// "outside the workspace" question is not here: it is the broker's own
/// extra question (Claude Code asks its own for a path outside its folders).
pub fn gate(
    rules: &[Rule],
    run: Option<RunMode>,
    grant: GrantMode,
    tool: &str,
    input: &Value,
) -> Verdict {
    let mut verdict = Verdict {
        mode: grant,
        source: Source::Grant,
    };
    if let Some(run) = run {
        let moved = super::run_mode::native_grant(run, tool, grant);
        if moved != grant {
            verdict = Verdict {
                mode: moved,
                source: Source::RunMode,
            };
        }
    }
    if matches!(verdict.mode, GrantMode::Deny) {
        return verdict;
    }
    if let Some(action) = decide_in(rules, tool, input) {
        verdict = Verdict {
            mode: match action {
                Action::Allow => GrantMode::Auto,
                Action::Ask => GrantMode::Ask,
                Action::Deny => GrantMode::Deny,
            },
            source: Source::Rule,
        };
    }
    if super::tool_table::always_asks(tool) && matches!(verdict.mode, GrantMode::Auto) {
        verdict = Verdict {
            mode: GrantMode::Ask,
            source: Source::AlwaysAsks,
        };
    }
    verdict
}

/// The grant a Claude Code built-in has before the run mode moves it: the
/// read tools go through (the CLI never asks for them in any mode), edits
/// and the shell are the CLI's to ask about. The same grants on a native
/// agent give the same verdicts (BN8).
pub fn cli_grant(native_tool: &str) -> GrantMode {
    if super::code_tools::READ_TOOLS.contains(&native_tool) {
        GrantMode::Auto
    } else {
        GrantMode::Ask
    }
}

/// Claude Code's built-in tool call as the native harness's `(name, input)`,
/// so one rule covers both. File paths are made relative to `workspace`
/// when they are inside it (the native tools take relative paths). A tool
/// with no native twin keeps the `cli:<Tool>` name the prompt tool has
/// always stored its rules under.
pub fn from_claude(tool: &str, input: &Value, workspace: Option<&Path>) -> (String, Value) {
    let s = |k: &str| input.get(k).and_then(Value::as_str).unwrap_or("");
    let rel = |raw: &str| relative(raw, workspace);
    match tool {
        "Bash" => ("shell_exec".into(), json!({ "command": s("command") })),
        "Write" => ("fs_write".into(), json!({ "path": rel(s("file_path")) })),
        "Edit" | "MultiEdit" => ("fs_edit".into(), json!({ "path": rel(s("file_path")) })),
        "NotebookEdit" => ("fs_edit".into(), json!({ "path": rel(s("notebook_path")) })),
        "Read" => ("fs_read".into(), json!({ "path": rel(s("file_path")) })),
        "LS" => ("fs_list".into(), json!({ "path": rel(s("path")) })),
        "Glob" => (
            "fs_glob".into(),
            json!({ "pattern": s("pattern"), "path": rel(s("path")) }),
        ),
        "Grep" => (
            "fs_grep".into(),
            json!({ "pattern": s("pattern"), "path": rel(s("path")) }),
        ),
        other => (format!("cli:{other}"), input.clone()),
    }
}

/// A native call's input with its `path` made relative the same way as
/// [`from_claude`] does (the model may send either form).
pub fn native_input(tool: &str, input: &Value, workspace: Option<&Path>) -> Value {
    let mut out = input.clone();
    if tool.starts_with("fs_") {
        if let Some(raw) = input.get("path").and_then(Value::as_str) {
            out["path"] = Value::String(relative(raw, workspace));
        }
    }
    out
}

/// `raw` relative to `workspace` when it is inside it; `./x` → `x`.
pub fn relative(raw: &str, workspace: Option<&Path>) -> String {
    let raw = raw.trim();
    let p = Path::new(raw);
    if let Some(ws) = workspace {
        if let Ok(rest) = p.strip_prefix(ws) {
            return rest.to_string_lossy().replace('\\', "/");
        }
        // macOS: /var → /private/var and the like.
        if let (Ok(a), Ok(b)) = (
            std::fs::canonicalize(ws),
            p.parent()
                .map(std::fs::canonicalize)
                .unwrap_or(Ok(PathBuf::new())),
        ) {
            if let (Ok(rest), Some(name)) = (b.strip_prefix(&a), p.file_name()) {
                return rest.join(name).to_string_lossy().replace('\\', "/");
            }
        }
    }
    raw.strip_prefix("./").unwrap_or(raw).to_string()
}

/// The rule "Always" stores for this call.
pub fn always_rule(tool: &str, input: &Value) -> Rule {
    let subject = subject(tool, input);
    let pattern = match tool {
        "shell_exec" => {
            let first = segments(&subject).into_iter().next().unwrap_or_default();
            format!("{} *", command_prefix(&first))
        }
        _ => "*".to_string(),
    };
    Rule {
        tool: tool.to_string(),
        pattern,
        action: Action::Allow,
    }
}
