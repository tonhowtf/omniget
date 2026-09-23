//! Runtime of `omniget-hook-shim` (the binary in `omniget-cli` is a thin
//! wrapper around [`main_with`]).
//!
//! ```text
//! omniget-hook-shim --tool <id> --event <Claude event> [--native-event <name>]
//!                   [--matcher <Claude matcher>] [--timeout <s>] -- <original command>
//! omniget-hook-shim --tool cline --dispatch <registry.json> [--native-event <name>]
//! omniget-hook-shim --tool claude --observe --as <id> --data-dir <dir> [--gate] [--wait <s>]
//! ```
//!
//! 1. Reads the tool's hook stdin and normalises it to Claude's hook JSON
//!    (`session_id, transcript_path, cwd, hook_event_name, tool_name` with the
//!    canonical Claude tool name, `tool_input` with Claude's field names).
//! 2. Runs the original command with that stdin, `CLAUDE_PROJECT_DIR` set and
//!    the project as working directory (or, with `--observe`, reports to the
//!    OmniGet bridge and waits for the owner's answer on permission events).
//! 3. Reads Claude's answer (exit 2 + stderr, `hookSpecificOutput`, `decision`,
//!    `continue`) and writes it back in the tool's own dialect.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use super::hook_common::matcher_matches;

/// Parsed command line.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ShimArgs {
    /// Dialect of stdin/stdout (target id).
    pub tool: String,
    /// Canonical (Claude) event.
    pub event: Option<String>,
    pub native_event: Option<String>,
    /// Canonical matcher checked here (tools without native matchers).
    pub matcher: Option<String>,
    pub timeout: Option<f64>,
    /// Cline: registry of Claude-shaped hook groups keyed by native event.
    pub dispatch: Option<PathBuf>,
    /// Report to the bridge instead of running a command.
    pub observe: bool,
    /// Observe: the tool this installation reports as.
    pub as_tool: Option<String>,
    /// OmniGet data dir (settings.json with the bridge port, observe tokens).
    pub data_dir: Option<PathBuf>,
    /// Observe: hold `PreToolUse` too (not only `PermissionRequest`).
    pub gate: bool,
    /// Observe: seconds to wait for the owner before letting the tool ask.
    pub wait: Option<u64>,
    /// Statusline mode: translate the tool's statusline stdin to Claude's and
    /// print what the original command prints.
    pub statusline: bool,
    pub command: Vec<String>,
}

pub fn parse_args(args: &[String]) -> Result<ShimArgs, String> {
    let mut a = ShimArgs::default();
    let mut i = 0;
    let val = |i: &mut usize, name: &str| -> Result<String, String> {
        *i += 1;
        args.get(*i)
            .cloned()
            .ok_or_else(|| format!("{name} needs a value"))
    };
    while i < args.len() {
        match args[i].as_str() {
            "--" => {
                a.command = args[i + 1..].to_vec();
                break;
            }
            "--tool" => a.tool = val(&mut i, "--tool")?,
            "--event" => a.event = Some(val(&mut i, "--event")?),
            "--native-event" => a.native_event = Some(val(&mut i, "--native-event")?),
            "--matcher" => a.matcher = Some(val(&mut i, "--matcher")?),
            "--timeout" => {
                a.timeout = val(&mut i, "--timeout")?.parse().ok();
            }
            "--dispatch" => a.dispatch = Some(PathBuf::from(val(&mut i, "--dispatch")?)),
            "--observe" => a.observe = true,
            "--as" => a.as_tool = Some(val(&mut i, "--as")?),
            "--data-dir" => a.data_dir = Some(PathBuf::from(val(&mut i, "--data-dir")?)),
            "--gate" => a.gate = true,
            "--wait" => a.wait = val(&mut i, "--wait")?.parse().ok(),
            "--statusline" => a.statusline = true,
            other => return Err(format!("unknown argument `{other}`")),
        }
        i += 1;
    }
    if a.tool.is_empty() {
        a.tool = "claude".into();
    }
    if a.command.is_empty() && a.dispatch.is_none() && !a.observe {
        return Err("nothing to run: pass `-- <command>`, --dispatch or --observe".into());
    }
    Ok(a)
}

/// What the shim hands back to the tool.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    pub exit: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Where environment variables come from (the process, or a test map).
pub type EnvFn<'a> = &'a dyn Fn(&str) -> Option<String>;

pub fn process_env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

// ------------------------------------------------------------------ normalise

fn first_str(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| {
        v.get(*k).and_then(|x| match x {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    })
}

fn first_val(v: &Value, keys: &[&str]) -> Option<Value> {
    keys.iter()
        .find_map(|k| v.get(*k).filter(|x| !x.is_null()).cloned())
}

/// A string that holds JSON becomes the JSON.
fn unstring(v: Value) -> Value {
    match &v {
        Value::String(s) => {
            let t = s.trim();
            if (t.starts_with('{') && t.ends_with('}')) || (t.starts_with('[') && t.ends_with(']'))
            {
                serde_json::from_str(t).unwrap_or(v)
            } else {
                v
            }
        }
        _ => v,
    }
}

const CANONICAL_ORDER: &[&str] = &[
    "Bash",
    "Read",
    "Edit",
    "Write",
    "MultiEdit",
    "Glob",
    "Grep",
    "WebFetch",
    "WebSearch",
    "Agent",
    "Task",
    "TodoWrite",
    "Skill",
    "AskUserQuestion",
];

/// Native tool name → Claude's, through the target's `tool_name_map` (reversed)
/// plus aliases the runtimes use besides the documented names.
pub fn canonical_tool(tool: &str, native: &str) -> String {
    if native.is_empty() {
        return String::new();
    }
    if native.starts_with("mcp__") {
        return native.to_string();
    }
    let extra: &[(&str, &str)] = match tool {
        "cursor" => &[("Shell", "Bash"), ("Delete", "Bash"), ("Task", "Agent")],
        "copilot" => &[
            ("powershell", "Bash"),
            ("str_replace_editor", "Edit"),
            ("apply_patch", "Edit"),
            ("rg", "Grep"),
        ],
        "codex" => &[
            ("exec_command", "Bash"),
            ("shell", "Bash"),
            ("apply_patch", "Edit"),
            ("spawn_agent", "Agent"),
        ],
        "kiro" => &[
            ("execute_bash", "Bash"),
            ("fs_read", "Read"),
            ("fs_write", "Write"),
        ],
        "cline" => &[
            ("run_commands", "Bash"),
            ("read_files", "Read"),
            ("editor", "Edit"),
        ],
        "gemini" | "qwen" => &[("read_many_files", "Read"), ("search_file_content", "Grep")],
        "auggie" => &[
            ("launch-process", "Bash"),
            ("view", "Read"),
            ("str-replace-editor", "Edit"),
            ("save-file", "Write"),
        ],
        "opencode" | "kilo" => &[("patch", "Edit"), ("list", "Glob")],
        _ => &[],
    };
    if let Some((_, c)) = extra.iter().find(|(n, _)| *n == native) {
        return c.to_string();
    }
    if let Some(t) = crate::core::agentkit::targets::target(tool) {
        let mut hits: Vec<&str> = t
            .tool_name_map
            .iter()
            .filter(|(k, v)| {
                k.as_str() != "mcp" && v.split(',').map(str::trim).any(|x| x == native)
            })
            .map(|(k, _)| k.as_str())
            .collect();
        if hits.contains(&native) {
            return native.to_string();
        }
        hits.sort_by_key(|k| CANONICAL_ORDER.iter().position(|c| c == k).unwrap_or(99));
        if let Some(h) = hits.first() {
            return h.to_string();
        }
        // MCP id pattern (`mcp_{server}_{tool}`, `{server}__{tool}` …)
        if let Some(pat) = t.tool_name_map.get("mcp").filter(|p| !p.is_empty()) {
            if let Some((s, tl)) = match_mcp(pat, native) {
                return format!("mcp__{s}__{tl}");
            }
        }
    }
    if let Some(rest) = native.strip_prefix("MCP:") {
        return format!("mcp__unknown__{rest}");
    }
    CANONICAL_ORDER
        .iter()
        .find(|c| c.eq_ignore_ascii_case(native))
        .map(|c| c.to_string())
        .unwrap_or_else(|| native.to_string())
}

fn match_mcp(pat: &str, name: &str) -> Option<(String, String)> {
    let (pre, rest) = pat.split_once("{server}")?;
    let (mid, post) = rest.split_once("{tool}")?;
    if !post.is_empty() || mid.is_empty() {
        return None;
    }
    let body = name.strip_prefix(pre)?;
    if pre.is_empty() && !name.contains(mid) {
        return None;
    }
    let (s, t) = body.split_once(mid)?;
    (!s.is_empty() && !t.is_empty()).then(|| (s.to_string(), t.to_string()))
}

/// Tool input with Claude's field names added next to the native ones.
pub fn canonical_input(input: Value) -> Value {
    let Value::Object(mut m) = unstring(input) else {
        return json!({});
    };
    let alias = |m: &mut Map<String, Value>, to: &str, from: &[&str]| {
        if m.contains_key(to) {
            return;
        }
        if let Some(v) = from.iter().find_map(|k| m.get(*k).cloned()) {
            m.insert(to.to_string(), v);
        }
    };
    alias(
        &mut m,
        "file_path",
        &[
            "filePath",
            "absolute_path",
            "path",
            "file",
            "target_file",
            "TargetFile",
            "filename",
        ],
    );
    alias(
        &mut m,
        "command",
        &["cmd", "command_line", "commandLine", "CommandLine"],
    );
    alias(&mut m, "old_string", &["oldString", "old_str"]);
    alias(&mut m, "new_string", &["newString", "new_str"]);
    alias(&mut m, "content", &["contents", "file_text", "fileText"]);
    alias(&mut m, "replace_all", &["replaceAll"]);
    if let Some(Value::Array(edits)) = m.get("edits").cloned() {
        if let Some(first) = edits.first() {
            for (to, from) in [("old_string", "old_string"), ("new_string", "new_string")] {
                if !m.contains_key(to) {
                    if let Some(v) = first.get(from).cloned() {
                        m.insert(to.into(), v);
                    }
                }
            }
        }
    }
    Value::Object(m)
}

/// Claude event from a native one (reverse of `hook_event_map`).
pub fn canonical_event(tool: &str, native: &str) -> Option<String> {
    let t = crate::core::agentkit::targets::target(tool)?;
    t.hook_event_map
        .iter()
        .find(|(_, v)| v.split(',').map(str::trim).any(|x| x == native))
        .map(|(k, _)| k.clone())
}

/// Cascade has no tool name: the event says what kind of action it is.
fn cascade_tool(native_event: &str, info: &Value) -> String {
    match native_event.split_once('_').map(|x| x.1).unwrap_or("") {
        "run_command" => "Bash".into(),
        "write_code" => "Edit".into(),
        "read_code" => "Read".into(),
        "mcp_tool_use" => format!(
            "mcp__{}__{}",
            first_str(info, &["mcp_server_name"]).unwrap_or_else(|| "unknown".into()),
            first_str(info, &["mcp_tool_name"]).unwrap_or_else(|| "tool".into())
        ),
        _ => String::new(),
    }
}

const PROJECT_ENVS: &[&str] = &[
    "CLAUDE_PROJECT_DIR",
    "GEMINI_PROJECT_DIR",
    "QWEN_PROJECT_DIR",
    "CURSOR_PROJECT_DIR",
    "DEVIN_PROJECT_DIR",
    "FACTORY_PROJECT_DIR",
    "TRAE_PROJECT_DIR",
    "OPENHANDS_PROJECT_DIR",
    "CRUSH_PROJECT_DIR",
    "GROK_WORKSPACE_ROOT",
    "GEMINI_CWD",
];

/// Builds Claude's hook JSON from what the tool sent.
pub fn normalize(
    tool: &str,
    event: &str,
    native_event: Option<&str>,
    raw: &Value,
    env: EnvFn,
) -> Value {
    // Cline (VS Code) nests the event payload under its camelCase name.
    let nested = [
        "preToolUse",
        "postToolUse",
        "userPromptSubmit",
        "taskStart",
        "taskResume",
        "taskCancel",
        "taskComplete",
        "preCompact",
        "notification",
    ]
    .iter()
    .find_map(|k| raw.get(*k).filter(|v| v.is_object()))
    .cloned()
    .unwrap_or(Value::Null);
    let info = raw.get("tool_info").cloned().unwrap_or(Value::Null);

    let session = first_str(
        raw,
        &[
            "session_id",
            "sessionId",
            "sessionID",
            "conversation_id",
            "taskId",
            "trajectory_id",
        ],
    )
    .unwrap_or_default();
    let root_of = |k: &str| {
        raw.get(k)
            .and_then(|a| a.as_array())
            .and_then(|a| a.first())
            .and_then(|x| x.as_str())
            .map(str::to_string)
    };
    let cwd = first_str(
        raw,
        &[
            "cwd",
            "working_dir",
            "workingDir",
            "workspaceRoot",
            "project_path",
        ],
    )
    .or_else(|| first_str(&info, &["cwd"]))
    .or_else(|| root_of("workspace_roots"))
    .or_else(|| root_of("workspaceRoots"))
    .or_else(|| PROJECT_ENVS.iter().find_map(|k| env(k)))
    .or_else(|| {
        std::env::current_dir()
            .ok()
            .map(|p| p.display().to_string())
    })
    .unwrap_or_default();
    let transcript = first_str(raw, &["transcript_path", "transcriptPath"])
        .or_else(|| first_str(&info, &["transcript_path"]));

    let native_tool = first_str(raw, &["tool_name", "toolName", "tool"])
        .or_else(|| first_str(&nested, &["toolName", "tool_name"]))
        .or_else(|| raw.get("tool_result").and_then(|t| first_str(t, &["name"])))
        .or_else(|| raw.get("tool_call").and_then(|t| first_str(t, &["name"])))
        .unwrap_or_default();
    let tool_name = if tool == "windsurf" && native_tool.is_empty() {
        cascade_tool(native_event.unwrap_or(""), &info)
    } else {
        canonical_tool(tool, &native_tool)
    };
    let mut tool_input = first_val(
        raw,
        &["tool_input", "toolInput", "toolArgs", "args", "input"],
    )
    .or_else(|| first_val(&nested, &["parameters", "toolInput"]))
    .or_else(|| {
        raw.get("tool_call")
            .and_then(|t| first_val(t, &["input", "arguments"]))
    })
    .or_else(|| {
        info.as_object().map(|_| {
            info.get("mcp_tool_arguments")
                .cloned()
                .unwrap_or_else(|| info.clone())
        })
    })
    .unwrap_or_else(|| json!({}));
    tool_input = canonical_input(tool_input);
    let response = first_val(
        raw,
        &[
            "tool_response",
            "tool_output",
            "toolResult",
            "tool_result",
            "result",
        ],
    )
    .or_else(|| first_val(&nested, &["result"]))
    .or_else(|| info.get("mcp_result").cloned())
    .map(unstring);
    let prompt = first_str(raw, &["prompt", "user_prompt", "message"])
        .or_else(|| first_str(&nested, &["prompt"]))
        .or_else(|| first_str(&info, &["user_prompt"]));

    let mut out = Map::new();
    out.insert("session_id".into(), json!(session));
    if let Some(t) = transcript {
        out.insert("transcript_path".into(), json!(t));
    }
    out.insert("cwd".into(), json!(cwd));
    out.insert("hook_event_name".into(), json!(event));
    if let Some(pm) = first_str(raw, &["permission_mode", "permissionMode"]) {
        out.insert("permission_mode".into(), json!(pm));
    }
    let tool_event = matches!(
        event,
        "PreToolUse" | "PostToolUse" | "PostToolUseFailure" | "PermissionRequest"
    );
    if tool_event || !tool_name.is_empty() {
        out.insert("tool_name".into(), json!(tool_name));
        out.insert("tool_input".into(), tool_input);
        if let Some(id) = first_str(
            raw,
            &[
                "tool_use_id",
                "toolUseId",
                "tool_call_id",
                "callID",
                "toolUseID",
            ],
        ) {
            out.insert("tool_use_id".into(), json!(id));
        }
    }
    if let Some(r) = response {
        if event != "PreToolUse" {
            out.insert("tool_response".into(), r);
        }
    }
    if let Some(p) = prompt {
        if matches!(event, "UserPromptSubmit" | "Notification") {
            out.insert(
                if event == "Notification" {
                    "message"
                } else {
                    "prompt"
                }
                .into(),
                json!(p),
            );
        }
    }
    if let Some(s) = first_str(raw, &["source", "trigger", "reason", "notification_type"]) {
        let key = match event {
            "SessionStart" => "source",
            "PreCompact" => "trigger",
            "SessionEnd" => "reason",
            "Notification" => "notification_type",
            _ => "",
        };
        if !key.is_empty() {
            out.insert(key.into(), json!(s));
        }
    }
    for k in [
        "stop_hook_active",
        "last_assistant_message",
        "agent_id",
        "agent_type",
    ] {
        if let Some(v) = raw.get(k) {
            out.insert(k.into(), v.clone());
        }
    }
    out.insert(
        "omniget".into(),
        json!({ "tool": tool, "native_event": native_event, "native_tool": native_tool }),
    );
    Value::Object(out)
}

// ------------------------------------------------------------------ verdict

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Decision {
    /// No opinion: the tool does what it would have done.
    #[default]
    None,
    Allow,
    Deny,
    /// Ask the user (the tool's own prompt).
    Ask,
}

/// Claude's answer, read once and rendered per dialect.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Verdict {
    pub decision: Decision,
    pub reason: String,
    pub context: Option<String>,
    pub system_message: Option<String>,
    pub updated_input: Option<Value>,
    /// `continue: false`: stop the whole agent.
    pub stop: bool,
}

/// Reads a Claude hook result.
pub fn claude_verdict(event: &str, exit: i32, stdout: &str, stderr: &str) -> Verdict {
    let mut v = Verdict::default();
    if exit == 2 {
        v.decision = Decision::Deny;
        v.reason = stderr.trim().to_string();
        if v.reason.is_empty() {
            v.reason = "Blocked by a hook".into();
        }
        return v;
    }
    if exit != 0 {
        return v;
    }
    let t = stdout.trim();
    if t.starts_with('{') && t.ends_with('}') {
        if let Ok(j) = serde_json::from_str::<Value>(t) {
            let hso = j.get("hookSpecificOutput").cloned().unwrap_or(Value::Null);
            let pd = hso
                .get("permissionDecision")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let behavior = hso
                .get("decision")
                .and_then(|d| d.get("behavior"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            let top = j.get("decision").and_then(|x| x.as_str()).unwrap_or("");
            let d = if !pd.is_empty() {
                pd
            } else if !behavior.is_empty() {
                behavior
            } else {
                top
            };
            v.decision = match d {
                "deny" | "block" => Decision::Deny,
                "allow" | "approve" => Decision::Allow,
                "ask" => Decision::Ask,
                _ => Decision::None,
            };
            v.reason = hso
                .get("permissionDecisionReason")
                .or_else(|| hso.get("decision").and_then(|d| d.get("message")))
                .or_else(|| j.get("reason"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            v.context = hso
                .get("additionalContext")
                .and_then(|x| x.as_str())
                .map(str::to_string);
            v.updated_input = hso
                .get("updatedInput")
                .or_else(|| hso.get("decision").and_then(|d| d.get("updatedInput")))
                .cloned();
            v.system_message = j
                .get("systemMessage")
                .and_then(|x| x.as_str())
                .map(str::to_string);
            if j.get("continue") == Some(&Value::Bool(false)) {
                v.stop = true;
                if v.reason.is_empty() {
                    v.reason = j
                        .get("stopReason")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                }
            }
            return v;
        }
    }
    if !t.is_empty() && matches!(event, "SessionStart" | "UserPromptSubmit") {
        v.context = Some(t.to_string());
    }
    v
}

/// Claude's own output for a verdict (used for `--tool claude` observe and dispatch).
pub fn render_claude(event: &str, v: &Verdict) -> Outcome {
    let mut o = Map::new();
    let mut hso = Map::new();
    match (event, v.decision) {
        (_, Decision::None) => {}
        ("PreToolUse", d) => {
            hso.insert(
                "permissionDecision".into(),
                json!(match d {
                    Decision::Allow => "allow",
                    Decision::Deny => "deny",
                    _ => "ask",
                }),
            );
            if !v.reason.is_empty() {
                hso.insert("permissionDecisionReason".into(), json!(v.reason));
            }
        }
        ("PermissionRequest", Decision::Ask) => {}
        ("PermissionRequest", d) => {
            let mut dec = Map::new();
            dec.insert(
                "behavior".into(),
                json!(if d == Decision::Allow {
                    "allow"
                } else {
                    "deny"
                }),
            );
            if d == Decision::Deny && !v.reason.is_empty() {
                dec.insert("message".into(), json!(v.reason));
            }
            hso.insert("decision".into(), Value::Object(dec));
        }
        (_, Decision::Deny) => {
            o.insert("decision".into(), json!("block"));
            o.insert("reason".into(), json!(v.reason));
        }
        _ => {}
    }
    if let Some(c) = &v.context {
        hso.insert("additionalContext".into(), json!(c));
    }
    if let Some(u) = &v.updated_input {
        hso.insert("updatedInput".into(), u.clone());
    }
    if !hso.is_empty() {
        hso.insert("hookEventName".into(), json!(event));
        o.insert("hookSpecificOutput".into(), Value::Object(hso));
    }
    if let Some(m) = &v.system_message {
        o.insert("systemMessage".into(), json!(m));
    }
    if v.stop {
        o.insert("continue".into(), json!(false));
        o.insert("stopReason".into(), json!(v.reason));
    }
    Outcome {
        exit: 0,
        stdout: if o.is_empty() {
            String::new()
        } else {
            Value::Object(o).to_string()
        },
        stderr: String::new(),
    }
}

fn blocking(event: &str) -> bool {
    matches!(
        event,
        "PreToolUse" | "PermissionRequest" | "UserPromptSubmit"
    )
}

/// The verdict in the tool's own dialect.
pub fn render(tool: &str, event: &str, native: Option<&str>, v: &Verdict) -> Outcome {
    let deny = v.decision == Decision::Deny;
    let json_out = |o: Value| Outcome {
        exit: 0,
        stdout: if o.as_object().map(|m| m.is_empty()).unwrap_or(false) {
            String::new()
        } else {
            o.to_string()
        },
        stderr: String::new(),
    };
    let exit2 = |reason: &str| Outcome {
        exit: 2,
        stdout: String::new(),
        stderr: if reason.is_empty() {
            "Blocked by a hook".into()
        } else {
            reason.to_string()
        },
    };
    let native = native.unwrap_or("");
    match tool {
        "cursor" => {
            let mut o = Map::new();
            match native {
                "preToolUse" | "beforeShellExecution" | "beforeMCPExecution" | "beforeReadFile" => {
                    match v.decision {
                        Decision::Allow => {
                            o.insert("permission".into(), json!("allow"));
                        }
                        Decision::Deny => {
                            o.insert("permission".into(), json!("deny"));
                            o.insert("user_message".into(), json!(v.reason));
                            o.insert("agent_message".into(), json!(v.reason));
                        }
                        Decision::Ask => {
                            o.insert("permission".into(), json!("ask"));
                        }
                        Decision::None => {}
                    }
                    if let Some(u) = &v.updated_input {
                        o.insert("updated_input".into(), u.clone());
                    }
                }
                "beforeSubmitPrompt" => {
                    if deny || v.stop {
                        o.insert("continue".into(), json!(false));
                        o.insert("user_message".into(), json!(v.reason));
                    }
                }
                "stop" | "subagentStop" => {
                    if deny {
                        o.insert("followup_message".into(), json!(v.reason));
                    }
                }
                _ => {}
            }
            if let Some(c) = &v.context {
                o.insert("additional_context".into(), json!(c));
            }
            json_out(Value::Object(o))
        }
        "copilot" => {
            let mut o = Map::new();
            match native {
                "preToolUse" | "permissionRequest" => {
                    match v.decision {
                        Decision::None => {}
                        d => {
                            o.insert(
                                "permissionDecision".into(),
                                json!(match d {
                                    Decision::Allow => "allow",
                                    Decision::Deny => "deny",
                                    _ => "ask",
                                }),
                            );
                            if !v.reason.is_empty() {
                                o.insert("permissionDecisionReason".into(), json!(v.reason));
                            }
                        }
                    }
                    if let Some(u) = &v.updated_input {
                        o.insert("modifiedArgs".into(), u.clone());
                    }
                }
                "agentStop" | "subagentStop" => {
                    if deny {
                        o.insert("decision".into(), json!("block"));
                        o.insert("reason".into(), json!(v.reason));
                    }
                }
                "userPromptSubmitted" if deny => return exit2(&v.reason),
                _ => {}
            }
            if let Some(c) = &v.context {
                o.insert("additionalContext".into(), json!(c));
            }
            json_out(Value::Object(o))
        }
        "gemini" => {
            let mut o = Map::new();
            match v.decision {
                Decision::Deny => {
                    o.insert("decision".into(), json!("deny"));
                    o.insert("reason".into(), json!(v.reason));
                }
                Decision::Allow => {
                    o.insert("decision".into(), json!("allow"));
                }
                _ => {}
            }
            let mut hso = Map::new();
            if let Some(c) = &v.context {
                hso.insert("additionalContext".into(), json!(c));
            }
            if let Some(u) = &v.updated_input {
                hso.insert("tool_input".into(), u.clone());
            }
            if !hso.is_empty() {
                o.insert("hookSpecificOutput".into(), Value::Object(hso));
            }
            if let Some(m) = &v.system_message {
                o.insert("systemMessage".into(), json!(m));
            }
            if v.stop {
                o.insert("continue".into(), json!(false));
                o.insert("stopReason".into(), json!(v.reason));
            }
            json_out(Value::Object(o))
        }
        "cline" => {
            let mut o = Map::new();
            o.insert("cancel".into(), json!(deny || v.stop));
            if deny || v.stop {
                o.insert("errorMessage".into(), json!(v.reason));
            }
            if let Some(c) = &v.context {
                o.insert("contextModification".into(), json!(c));
            }
            Outcome {
                exit: 0,
                stdout: Value::Object(o).to_string(),
                stderr: String::new(),
            }
        }
        "vibe" => {
            let mut o = Map::new();
            match v.decision {
                Decision::Deny => {
                    o.insert("decision".into(), json!("deny"));
                    o.insert("reason".into(), json!(v.reason));
                }
                Decision::Allow => {
                    o.insert("decision".into(), json!("allow"));
                }
                _ => {}
            }
            if let Some(m) = &v.system_message {
                o.insert("system_message".into(), json!(m));
            }
            let mut hso = Map::new();
            if let Some(c) = &v.context {
                hso.insert("additional_context".into(), json!(c));
            }
            if let Some(u) = &v.updated_input {
                hso.insert("tool_input".into(), u.clone());
            }
            if !hso.is_empty() {
                o.insert("hook_specific_output".into(), Value::Object(hso));
            }
            json_out(Value::Object(o))
        }
        "goose" => {
            if deny {
                return json_out(json!({ "decision": "block", "reason": v.reason }));
            }
            Outcome {
                stdout: v.context.clone().unwrap_or_default(),
                ..Default::default()
            }
        }
        "crush" => {
            let mut o = Map::new();
            o.insert("version".into(), json!(1));
            o.insert(
                "decision".into(),
                match v.decision {
                    Decision::Allow => json!("allow"),
                    Decision::Deny => json!("deny"),
                    _ => Value::Null,
                },
            );
            if !v.reason.is_empty() {
                o.insert("reason".into(), json!(v.reason));
            }
            if let Some(c) = &v.context {
                o.insert("context".into(), json!(c));
            }
            if let Some(u) = &v.updated_input {
                o.insert("updated_input".into(), u.clone());
            }
            json_out(Value::Object(o))
        }
        "devin" => {
            let mut o = Map::new();
            match v.decision {
                Decision::Deny => {
                    o.insert("decision".into(), json!("block"));
                    o.insert("reason".into(), json!(v.reason));
                }
                Decision::Allow => {
                    o.insert("decision".into(), json!("approve"));
                }
                _ => {}
            }
            let mut hso = Map::new();
            if let Some(c) = &v.context {
                hso.insert("additionalContext".into(), json!(c));
            }
            if let Some(u) = &v.updated_input {
                hso.insert("updatedInput".into(), u.clone());
            }
            if !hso.is_empty() {
                hso.insert("hookEventName".into(), json!(native));
                o.insert("hookSpecificOutput".into(), Value::Object(hso));
            }
            json_out(Value::Object(o))
        }
        // exit-code dialects: block with 2 + stderr, context on stdout
        "kiro" | "windsurf" => {
            if deny && (blocking(event) || native.starts_with("pre_")) {
                return exit2(&v.reason);
            }
            Outcome {
                stdout: v.context.clone().unwrap_or_default(),
                ..Default::default()
            }
        }
        // our generated code plugins read this JSON
        "opencode" | "kilo" | "amp" | "pi" => json_out(json!({
            "decision": match v.decision {
                Decision::Allow => json!("allow"),
                Decision::Deny => json!("deny"),
                Decision::Ask => json!("ask"),
                Decision::None => Value::Null,
            },
            "reason": v.reason,
            "context": v.context,
            "updated_input": v.updated_input,
            "stop": v.stop,
        })),
        // Claude dialect (Claude, Codex, Qwen, Kimi, Droid, Junie …)
        _ => {
            if deny && blocking(event) && v.updated_input.is_none() && v.context.is_none() {
                return exit2(&v.reason);
            }
            render_claude(event, v)
        }
    }
}

// ------------------------------------------------------------------ running

/// Joins `-- a b c` back into one shell line (one argument = already a line).
pub fn command_line(cmd: &[String]) -> String {
    if cmd.len() == 1 {
        cmd[0].clone()
    } else {
        cmd.join(" ")
    }
}

/// Runs a shell line with `stdin`, in `dir` when it exists, with a timeout.
pub fn run_shell(
    line: &str,
    stdin: &[u8],
    dir: Option<&Path>,
    timeout: Option<Duration>,
) -> Outcome {
    let mut cmd = if cfg!(windows) {
        let mut c = crate::core::process::std_command("cmd");
        c.arg("/C").arg(line);
        c
    } else {
        let mut c = crate::core::process::std_command("sh");
        c.arg("-c").arg(line);
        c
    };
    if let Some(d) = dir.filter(|d| d.is_dir()) {
        cmd.current_dir(d);
        if std::env::var_os("CLAUDE_PROJECT_DIR").is_none() {
            cmd.env("CLAUDE_PROJECT_DIR", d);
        }
    }
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = match crate::core::process::spawn_retrying_busy(|| cmd.spawn()) {
        Ok(c) => c,
        Err(e) => {
            return Outcome {
                exit: 1,
                stdout: String::new(),
                stderr: format!("omniget-hook-shim: cannot start the hook: {e}"),
            }
        }
    };
    if let Some(mut si) = child.stdin.take() {
        let data = stdin.to_vec();
        std::thread::spawn(move || {
            let _ = si.write_all(&data);
        });
    }
    let mut so = child.stdout.take();
    let mut se = child.stderr.take();
    let out_t = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(s) = so.as_mut() {
            let _ = s.read_to_end(&mut b);
        }
        b
    });
    let err_t = std::thread::spawn(move || {
        let mut b = Vec::new();
        if let Some(s) = se.as_mut() {
            let _ = s.read_to_end(&mut b);
        }
        b
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) => {
                if timeout.map(|t| start.elapsed() > t).unwrap_or(false) {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break None,
        }
    };
    let stdout = String::from_utf8_lossy(&out_t.join().unwrap_or_default()).to_string();
    let stderr = String::from_utf8_lossy(&err_t.join().unwrap_or_default()).to_string();
    match status {
        Some(s) => Outcome {
            exit: s.code().unwrap_or(1),
            stdout,
            stderr,
        },
        None => Outcome {
            exit: 1,
            stdout,
            stderr: format!("{stderr}\nomniget-hook-shim: the hook timed out"),
        },
    }
}

fn project_dir(claude_in: &Value) -> Option<PathBuf> {
    claude_in
        .get("cwd")
        .and_then(|c| c.as_str())
        .filter(|c| !c.is_empty())
        .map(PathBuf::from)
}

fn subject(claude_in: &Value, event: &str) -> String {
    let tool = claude_in
        .get("tool_name")
        .and_then(|t| t.as_str())
        .unwrap_or("");
    if !tool.is_empty() {
        return tool.to_string();
    }
    let key = match event {
        "SessionStart" => "source",
        "PreCompact" => "trigger",
        "SessionEnd" => "reason",
        "Notification" => "notification_type",
        _ => return String::new(),
    };
    claude_in
        .get(key)
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}

/// Does a matcher let this call through? No subject (events without one) = yes.
pub fn passes(matcher: Option<&str>, claude_in: &Value, event: &str) -> bool {
    match matcher {
        None => true,
        Some(m) => {
            let s = subject(claude_in, event);
            s.is_empty() || matcher_matches(m, &s)
        }
    }
}

/// The whole shim: arguments + stdin → what to print and the exit code.
pub fn main_with(args: &ShimArgs, stdin: &[u8], env: EnvFn) -> Outcome {
    let raw: Value = serde_json::from_slice(stdin).unwrap_or_else(|_| json!({}));
    if let Some(reg) = &args.dispatch {
        return dispatch(args, reg, &raw, env);
    }
    let native = args.native_event.clone().or_else(|| {
        first_str(
            &raw,
            &[
                "hook_event_name",
                "hookEventName",
                "hookName",
                "event",
                "agent_action_name",
            ],
        )
    });
    let event = args
        .event
        .clone()
        .or_else(|| {
            native
                .as_deref()
                .and_then(|n| canonical_event(&args.tool, n))
        })
        .or_else(|| native.clone())
        .unwrap_or_else(|| "PreToolUse".into());
    let claude_in = if args.tool == "claude" && raw.get("hook_event_name").is_some() {
        let mut v = raw.clone();
        if let Some(m) = v.as_object_mut() {
            if let Some(Value::String(n)) = m.get("tool_name").cloned() {
                m.insert(
                    "tool_name".into(),
                    json!(canonical_tool(
                        args.as_tool.as_deref().unwrap_or("claude"),
                        &n
                    )),
                );
            }
        }
        v
    } else {
        normalize(&args.tool, &event, native.as_deref(), &raw, env)
    };
    if !passes(args.matcher.as_deref(), &claude_in, &event) {
        return render(&args.tool, &event, native.as_deref(), &Verdict::default());
    }
    if args.observe {
        let v = observe(args, &event, &claude_in, &raw, env);
        return if args.tool == "claude" {
            render_claude(&event, &v)
        } else {
            render(&args.tool, &event, native.as_deref(), &v)
        };
    }
    let body = serde_json::to_vec(&claude_in).unwrap_or_default();
    let res = run_shell(
        &command_line(&args.command),
        &body,
        project_dir(&claude_in).as_deref(),
        args.timeout.map(Duration::from_secs_f64),
    );
    if args.tool == "claude" {
        // same dialect: hand the hook's own answer back untouched
        return res;
    }
    let v = claude_verdict(&event, res.exit, &res.stdout, &res.stderr);
    let mut out = render(&args.tool, &event, native.as_deref(), &v);
    if res.exit != 0 && res.exit != 2 && !res.stderr.is_empty() {
        // a failing hook stays non-blocking, but its message is kept
        out.stderr = res.stderr;
    }
    out
}

/// Cline: one executable per event runs every registered hook for it.
fn dispatch(args: &ShimArgs, reg: &Path, raw: &Value, env: EnvFn) -> Outcome {
    let native = args
        .native_event
        .clone()
        .or_else(|| first_str(raw, &["hookName", "hook_event_name"]))
        .unwrap_or_default();
    let registry: Value = std::fs::read(reg)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let groups = registry
        .get("hooks")
        .and_then(|h| h.get(&native))
        .and_then(|g| g.as_array())
        .cloned()
        .unwrap_or_default();
    let mut total = Verdict::default();
    let mut contexts: Vec<String> = Vec::new();
    let mut last_event = native.clone();
    'outer: for g in groups {
        let matcher = g
            .get("matcher")
            .and_then(|m| m.as_str())
            .map(str::to_string);
        for h in g
            .get("hooks")
            .and_then(|h| h.as_array())
            .cloned()
            .unwrap_or_default()
        {
            let event = h
                .get("event")
                .and_then(|e| e.as_str())
                .map(str::to_string)
                .or_else(|| canonical_event(&args.tool, &native))
                .unwrap_or_else(|| native.clone());
            last_event = event.clone();
            let claude_in = normalize(&args.tool, &event, Some(&native), raw, env);
            if !passes(matcher.as_deref(), &claude_in, &event) {
                continue;
            }
            let Some(cmd) = h.get("command").and_then(|c| c.as_str()) else {
                continue;
            };
            let timeout = h
                .get("timeout")
                .and_then(|t| t.as_f64())
                .map(Duration::from_secs_f64);
            let res = run_shell(
                cmd,
                &serde_json::to_vec(&claude_in).unwrap_or_default(),
                project_dir(&claude_in).as_deref(),
                timeout,
            );
            let v = claude_verdict(&event, res.exit, &res.stdout, &res.stderr);
            if let Some(c) = &v.context {
                contexts.push(c.clone());
            }
            if v.decision == Decision::Deny || v.stop {
                total = v;
                break 'outer;
            }
            if v.decision != Decision::None && total.decision == Decision::None {
                total.decision = v.decision;
            }
        }
    }
    if !contexts.is_empty() {
        total.context = Some(contexts.join("\n"));
    }
    render(&args.tool, &last_event, Some(&native), &total)
}

// ------------------------------------------------------------------ observe

/// Which coding tool is running this hook right now, from what it leaves in
/// the environment and the payload. `None` = cannot tell.
pub fn running_tool(raw: &Value, env: EnvFn) -> Option<&'static str> {
    if raw.get("cursor_version").is_some() || env("CURSOR_VERSION").is_some() {
        return Some("cursor");
    }
    let by_env: &[(&str, &str)] = &[
        ("GEMINI_SESSION_ID", "gemini"),
        ("QWEN_PROJECT_DIR", "qwen"),
        ("TRAE_PROJECT_DIR", "trae"),
        ("DEVIN_PROJECT_DIR", "devin"),
        ("GROK_HOOK_EVENT", "grok"),
        ("FACTORY_PROJECT_DIR", "droid"),
        ("OPENHANDS_PROJECT_DIR", "openhands"),
        ("CRUSH_EVENT", "crush"),
        ("KIMI_CODE_HOME", "kimi"),
    ];
    for (k, t) in by_env {
        if env(k).is_some() {
            return Some(t);
        }
    }
    if raw.get("clineVersion").is_some() {
        return Some("cline");
    }
    let claude_transcript = raw
        .get("transcript_path")
        .and_then(|t| t.as_str())
        .map(|t| t.replace('\\', "/").contains("/.claude/projects/"))
        .unwrap_or(false);
    if env("CLAUDECODE").is_some() || env("CLAUDE_CODE_ENTRYPOINT").is_some() || claude_transcript {
        return Some("claude");
    }
    None
}

fn data_dir(args: &ShimArgs) -> Option<PathBuf> {
    args.data_dir
        .clone()
        .or_else(crate::core::paths::app_data_dir)
}

/// Bridge port from `<data>/settings.json` (`app_settings.bridge.port`).
pub fn bridge_port(data: &Path) -> Option<u16> {
    let v: Value = serde_json::from_slice(&std::fs::read(data.join("settings.json")).ok()?).ok()?;
    let b = &v["app_settings"]["bridge"];
    if b["enabled"] == Value::Bool(false) {
        return None;
    }
    b["port"].as_u64().map(|p| p as u16).filter(|p| *p != 0)
}

/// Observe tokens file: `<data>/agentkit/observe/tokens.json` = `{tool: token}`.
pub fn tokens_path(data: &Path) -> PathBuf {
    data.join("agentkit").join("observe").join("tokens.json")
}

pub fn read_token(data: &Path, tool: &str) -> Option<String> {
    let v: Value = serde_json::from_slice(&std::fs::read(tokens_path(data)).ok()?).ok()?;
    v.get(tool)
        .and_then(|t| t.get("token").or(Some(t)))
        .and_then(|t| t.as_str())
        .map(str::to_string)
}

/// Longest the bridge holds a permission question (it stays under the ask
/// timeout of the tool broker).
pub const MAX_WAIT_SECS: u64 = 115;
/// Default wait when the command line does not say.
pub const DEFAULT_WAIT_SECS: u64 = 100;

fn observe(args: &ShimArgs, event: &str, claude_in: &Value, raw: &Value, env: EnvFn) -> Verdict {
    let as_tool = args.as_tool.clone().unwrap_or_else(|| args.tool.clone());
    // a hook written into Claude's settings on behalf of another tool runs in
    // Claude too: report only when the runner is the tool this entry is for
    if let Some(running) = running_tool(raw, env) {
        if running != as_tool {
            return Verdict::default();
        }
    }
    let Some(data) = data_dir(args) else {
        return Verdict::default();
    };
    let (Some(port), Some(token)) = (bridge_port(&data), read_token(&data, &as_tool)) else {
        return Verdict::default();
    };
    let hold = event == "PermissionRequest" || (args.gate && event == "PreToolUse");
    let wait = args.wait.unwrap_or(DEFAULT_WAIT_SECS).min(MAX_WAIT_SECS);
    let mut body = claude_in.clone();
    if let Some(m) = body.as_object_mut() {
        m.insert(
            "omniget".into(),
            json!({ "hold": hold, "wait_ms": wait * 1000, "tool": as_tool }),
        );
    }
    let read_timeout = Duration::from_secs(if hold { wait + 10 } else { 5 });
    let Some(resp) = http_post(
        port,
        &format!("/v1/observe/{as_tool}"),
        &token,
        &serde_json::to_vec(&body).unwrap_or_default(),
        read_timeout,
    ) else {
        return Verdict::default();
    };
    let mut v = Verdict::default();
    if hold {
        v.decision = match resp.get("decision").and_then(|d| d.as_str()) {
            Some("allow") => Decision::Allow,
            Some("deny") => Decision::Deny,
            _ => Decision::None,
        };
        v.reason = resp
            .get("reason")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();
        if v.decision == Decision::Deny && v.reason.is_empty() {
            v.reason = "Denied in OmniGet".into();
        }
    }
    v
}

/// Minimal HTTP/1.1 POST to the loopback bridge (no TLS, no proxies).
pub fn http_post(
    port: u16,
    path: &str,
    token: &str,
    body: &[u8],
    read_timeout: Duration,
) -> Option<Value> {
    use std::net::{SocketAddr, TcpStream};
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_millis(800)).ok()?;
    s.set_read_timeout(Some(read_timeout)).ok()?;
    s.set_write_timeout(Some(Duration::from_secs(5))).ok()?;
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    s.write_all(head.as_bytes()).ok()?;
    s.write_all(body).ok()?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf).ok()?;
    let split = buf.windows(4).position(|w| w == b"\r\n\r\n")?;
    let headers = String::from_utf8_lossy(&buf[..split]).to_ascii_lowercase();
    let status_ok = headers
        .lines()
        .next()
        .map(|l| l.contains(" 200"))
        .unwrap_or(false);
    let mut payload = buf[split + 4..].to_vec();
    if headers.contains("transfer-encoding: chunked") {
        payload = dechunk(&payload);
    }
    if !status_ok {
        return None;
    }
    serde_json::from_slice(&payload).ok()
}

fn dechunk(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let Some(nl) = b[i..].windows(2).position(|w| w == b"\r\n") else {
            break;
        };
        let size_s = String::from_utf8_lossy(&b[i..i + nl]).to_string();
        let size =
            usize::from_str_radix(size_s.split(';').next().unwrap_or("0").trim(), 16).unwrap_or(0);
        i += nl + 2;
        if size == 0 || i + size > b.len() {
            break;
        }
        out.extend_from_slice(&b[i..i + size]);
        i += size + 2;
    }
    out
}

/// Entry point of the binary: process args + stdin → stdout/stderr/exit.
pub fn main_entry() -> i32 {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse_args(&argv) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("omniget-hook-shim: {e}");
            return 1;
        }
    };
    let mut stdin = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut stdin);
    if args.statusline {
        let (code, text) = super::statusline_stdin::statusline_main(
            &args.tool,
            &command_line(&args.command),
            &stdin,
        );
        let mut o = std::io::stdout();
        let _ = o.write_all(text.as_bytes());
        let _ = o.flush();
        return code;
    }
    let out = main_with(&args, &stdin, &process_env);
    if !out.stdout.is_empty() {
        let mut o = std::io::stdout();
        let _ = o.write_all(out.stdout.as_bytes());
        let _ = o.flush();
    }
    if !out.stderr.is_empty() {
        let mut e = std::io::stderr();
        let _ = e.write_all(out.stderr.as_bytes());
        let _ = e.flush();
    }
    out.exit
}

/// A registry of Claude-shaped groups keyed by native event (Cline dispatch).
pub type Registry = BTreeMap<String, Vec<Value>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn args_round_trip() {
        let a = parse_args(&[
            "--tool".into(),
            "cursor".into(),
            "--event".into(),
            "PreToolUse".into(),
            "--".into(),
            "python3 x.py".into(),
        ])
        .unwrap();
        assert_eq!(a.tool, "cursor");
        assert_eq!(a.event.as_deref(), Some("PreToolUse"));
        assert_eq!(a.command, vec!["python3 x.py".to_string()]);
        assert!(parse_args(&["--tool".into(), "x".into()]).is_err());
    }

    #[test]
    fn cursor_input_becomes_claude() {
        let raw = json!({"conversation_id":"c1","hook_event_name":"preToolUse","tool_name":"Shell",
            "tool_input":{"command":"ls"},"workspace_roots":["/p"],"cursor_version":"1.9"});
        let v = normalize("cursor", "PreToolUse", Some("preToolUse"), &raw, &no_env);
        assert_eq!(v["tool_name"], "Bash");
        assert_eq!(v["session_id"], "c1");
        assert_eq!(v["cwd"], "/p");
        assert_eq!(v["tool_input"]["command"], "ls");
        assert_eq!(v["hook_event_name"], "PreToolUse");
    }

    #[test]
    fn gemini_and_cline_and_cascade_names() {
        assert_eq!(canonical_tool("gemini", "run_shell_command"), "Bash");
        assert_eq!(canonical_tool("gemini", "replace"), "Edit");
        assert_eq!(
            canonical_tool("gemini", "mcp_github_create_issue"),
            "mcp__github__create_issue"
        );
        assert_eq!(canonical_tool("cline", "write_to_file"), "Write");
        assert_eq!(canonical_tool("cursor", "Write"), "Write");
        assert_eq!(canonical_tool("codex", "apply_patch"), "Edit");
        let raw = json!({"hookName":"PreToolUse","workspaceRoots":["/w"],
            "preToolUse":{"toolName":"execute_command","parameters":{"command":"rm -rf /"}}});
        let v = normalize("cline", "PreToolUse", Some("PreToolUse"), &raw, &no_env);
        assert_eq!(v["tool_name"], "Bash");
        assert_eq!(v["tool_input"]["command"], "rm -rf /");
        let raw = json!({"agent_action_name":"pre_run_command","trajectory_id":"t",
            "tool_info":{"command_line":"ls -la","cwd":"/c"}});
        let v = normalize(
            "windsurf",
            "PreToolUse",
            Some("pre_run_command"),
            &raw,
            &no_env,
        );
        assert_eq!(v["tool_name"], "Bash");
        assert_eq!(v["tool_input"]["command"], "ls -la");
        assert_eq!(v["cwd"], "/c");
    }

    #[test]
    fn verdicts_render_per_dialect() {
        let deny = claude_verdict("PreToolUse", 2, "", "no way");
        assert_eq!(deny.decision, Decision::Deny);
        let c = render("cursor", "PreToolUse", Some("preToolUse"), &deny);
        let j: Value = serde_json::from_str(&c.stdout).unwrap();
        assert_eq!(j["permission"], "deny");
        let g = render("gemini", "PreToolUse", Some("BeforeTool"), &deny);
        let j: Value = serde_json::from_str(&g.stdout).unwrap();
        assert_eq!(j["decision"], "deny");
        let cl = render("cline", "PreToolUse", Some("PreToolUse"), &deny);
        let j: Value = serde_json::from_str(&cl.stdout).unwrap();
        assert_eq!(j["cancel"], true);
        assert_eq!(
            render("kiro", "PreToolUse", Some("PreToolUse"), &deny).exit,
            2
        );
        let json_deny = claude_verdict(
            "PreToolUse",
            0,
            r#"{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"r"}}"#,
            "",
        );
        assert_eq!(json_deny.decision, Decision::Deny);
        assert_eq!(json_deny.reason, "r");
        let co = render("copilot", "PreToolUse", Some("preToolUse"), &json_deny);
        let j: Value = serde_json::from_str(&co.stdout).unwrap();
        assert_eq!(j["permissionDecision"], "deny");
        let allow = Verdict::default();
        assert_eq!(
            render("cursor", "PreToolUse", Some("preToolUse"), &allow).stdout,
            ""
        );
        let pr = render_claude(
            "PermissionRequest",
            &Verdict {
                decision: Decision::Allow,
                ..Default::default()
            },
        );
        let j: Value = serde_json::from_str(&pr.stdout).unwrap();
        assert_eq!(j["hookSpecificOutput"]["decision"]["behavior"], "allow");
    }

    #[test]
    fn runner_detection_keeps_observe_honest() {
        let env = |k: &str| (k == "CLAUDECODE").then(|| "1".to_string());
        assert_eq!(running_tool(&json!({}), &env), Some("claude"));
        assert_eq!(
            running_tool(&json!({"cursor_version":"1"}), &env),
            Some("cursor")
        );
        assert_eq!(running_tool(&json!({}), &no_env), None);
    }
}
