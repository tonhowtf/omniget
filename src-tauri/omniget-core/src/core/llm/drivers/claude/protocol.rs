//! Wire of `claude -p --input-format stream-json --output-format stream-json`
//! as measured on Claude Code 2.1.280 (22/09/2026, fixtures in `fixtures/`).
//!
//! Both directions are NDJSON on the child's stdio:
//!
//! * **stdin (us → CLI)**
//!   - prompt: `{"type":"user","message":{"role":"user","content":[…]},
//!     "parent_tool_use_id":null,"session_id":"","uuid":"<ours>"}`. The `uuid`
//!     becomes the transcript uuid and comes back in `command_lifecycle`
//!     (`queued|started|completed|cancelled`) and in `result.user_message_uuids`.
//!     A prompt written while a turn runs is queued (steer).
//!   - control: `{"type":"control_request","request_id":"<ours>","request":
//!     {"subtype":"initialize"|"interrupt"|"set_permission_mode"|"set_model"|
//!     "get_usage",…}}`; the CLI answers `{"type":"control_response",
//!     "response":{"subtype":"success"|"error","request_id":…,"response":{…}}}`.
//!   - answer to a CLI request: `{"type":"control_response","response":
//!     {"subtype":"success","request_id":"<the CLI's>","response":
//!     {"behavior":"allow","updatedInput":{…},"updatedPermissions":[…]} |
//!     {"behavior":"deny","message":"…","interrupt":true?}}}`.
//! * **stdout (CLI → us)**: the SDK message stream (`system/*`, `stream_event`,
//!   `assistant`, `user`, `result`, `rate_limit_event`, `command_lifecycle`, …)
//!   plus `control_request` `can_use_tool {tool_name, input, tool_use_id,
//!   permission_suggestions?, description?, requires_user_interaction?}` and
//!   `control_cancel_request {request_id}` when a pending ask is withdrawn.
//!
//! `--permission-prompt-tool stdio` is the hidden flag the Agent SDK passes;
//! it routes every permission prompt to the host over this same stdio (the
//! binary's own help string: "permission prompts reach the host over stdio").

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{json, Value};

use super::super::{AccessMode, ApprovalDecision, InteractionMode};

/// Variables that make a nested `claude` think it runs inside another Claude
/// Code session (set when OmniGet itself is launched from a Claude terminal).
pub const NESTING_ENV: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
];

/// Flags a user launch arg may not override (T3: "argv order never lets the
/// user's flag win"). The permission mode comes from the thread's access mode.
const RESERVED_FLAGS: &[&str] = &[
    "-p",
    "--print",
    "--input-format",
    "--output-format",
    "--permission-mode",
    "--permission-prompt-tool",
    "--permission-prompts",
    "--dangerously-skip-permissions",
    "--allow-dangerously-skip-permissions",
    "--resume",
    "-r",
    "--continue",
    "-c",
    "--fork-session",
    "--session-id",
    "--resume-session-at",
];

/// `--permission-mode` for a thread. `bypassPermissions` only for the mode
/// the user picked as full access; plan turns use `plan`.
pub fn permission_mode(access: AccessMode, interaction: InteractionMode) -> &'static str {
    if interaction == InteractionMode::Plan {
        return "plan";
    }
    match access {
        AccessMode::ApprovalRequired => "default",
        AccessMode::AutoAcceptEdits => "acceptEdits",
        AccessMode::Auto => "auto",
        AccessMode::FullAccess => "bypassPermissions",
    }
}

/// Everything one launch needs. Built by the driver, turned into argv here.
#[derive(Debug, Clone, Default)]
pub struct Launch {
    pub model: Option<String>,
    pub access: AccessMode,
    pub interaction: InteractionMode,
    /// `--resume <id>`.
    pub resume: Option<String>,
    /// `--resume-session-at <uuid>` (needs `resume`).
    pub resume_at: Option<String>,
    /// `--fork-session` (needs `resume`).
    pub fork: bool,
    /// `--session-id <uuid>` for a fresh session, so the cursor exists before
    /// the first `system/init`.
    pub session_id: Option<String>,
    /// JSON string or file path for `--mcp-config`.
    pub mcp_config: Option<String>,
    /// The instance's own launch args (filtered).
    pub extra_args: Vec<String>,
}

impl Launch {
    pub fn argv(&self) -> Vec<String> {
        let mut out: Vec<String> = [
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--include-hook-events",
            "--permission-prompt-tool",
            "stdio",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let mode = permission_mode(self.access, self.interaction);
        out.push("--permission-mode".into());
        out.push(mode.into());
        if self.access == AccessMode::FullAccess {
            // Needed for bypassPermissions at launch and for switching into it
            // later with `set_permission_mode`.
            out.push("--allow-dangerously-skip-permissions".into());
        }
        if let Some(model) = self.model.as_deref().filter(|m| !m.trim().is_empty()) {
            if model != "default" {
                out.push("--model".into());
                out.push(model.to_string());
            }
        }
        match &self.resume {
            Some(id) => {
                out.push("--resume".into());
                out.push(id.clone());
                if let Some(at) = &self.resume_at {
                    out.push("--resume-session-at".into());
                    out.push(at.clone());
                }
                if self.fork {
                    out.push("--fork-session".into());
                }
            }
            None => {
                if let Some(id) = &self.session_id {
                    out.push("--session-id".into());
                    out.push(id.clone());
                }
            }
        }
        if let Some(mcp) = self.mcp_config.as_deref().filter(|m| !m.trim().is_empty()) {
            out.push("--mcp-config".into());
            out.push(mcp.to_string());
        }
        out.extend(filter_user_args(&self.extra_args));
        out
    }
}

/// Drops reserved flags (and their value) from a user's launch args.
pub fn filter_user_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let name = arg.split('=').next().unwrap_or(arg);
        if RESERVED_FLAGS.contains(&name) {
            // `--flag value` form: skip the value too (flags that take none are
            // followed by another `-…` or nothing).
            let takes_value = !arg.contains('=')
                && !matches!(
                    name,
                    "-p" | "--print"
                        | "--continue"
                        | "-c"
                        | "--fork-session"
                        | "--dangerously-skip-permissions"
                        | "--allow-dangerously-skip-permissions"
                );
            i += 1;
            if takes_value && i < args.len() && !args[i].starts_with('-') {
                i += 1;
            }
            continue;
        }
        out.push(arg.clone());
        i += 1;
    }
    out
}

/// Environment of the child: the account's config dir plus the instance's
/// own variables. Empty `config_dir` = the CLI's default profile.
pub fn child_env(
    config_dir: Option<&PathBuf>,
    instance_env: &[super::super::EnvVar],
) -> BTreeMap<String, String> {
    let mut env = BTreeMap::new();
    if let Some(dir) = config_dir.filter(|d| !d.as_os_str().is_empty()) {
        env.insert("CLAUDE_CONFIG_DIR".into(), dir.display().to_string());
    }
    for var in instance_env {
        if !var.name.trim().is_empty() && !var.value_redacted {
            env.insert(var.name.clone(), var.value.clone());
        }
    }
    env
}

/// Variables removed before launch: API keys (the account decides who is
/// billed, `accounts::SCRUB_ENV`) and the nesting markers.
pub fn scrub_env() -> Vec<String> {
    crate::core::llm::cli_runtime::SCRUB_ENV
        .iter()
        .chain(NESTING_ENV.iter())
        .map(|s| s.to_string())
        .collect()
}

/// One prompt. Images first, the text block last: the CLI only expands a
/// slash command when the last block is text (T3 finding).
pub fn user_message(uuid: &str, text: &str, attachments: &[Value]) -> Value {
    let mut content = Vec::new();
    let mut text = text.to_string();
    for att in attachments {
        let mime = att
            .get("mime")
            .or_else(|| att.get("mimeType"))
            .or_else(|| att.get("mediaType"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let data = att
            .get("data")
            .or_else(|| att.get("base64"))
            .and_then(Value::as_str);
        if let (true, Some(data)) = (mime.starts_with("image/"), data) {
            content.push(json!({
                "type": "image",
                "source": {"type": "base64", "media_type": mime, "data": data}
            }));
        } else if let Some(path) = att.get("path").and_then(Value::as_str) {
            text.push_str(&format!("\n\n[attached file is saved at: {path}]"));
        }
    }
    content.push(json!({"type": "text", "text": text}));
    json!({
        "type": "user",
        "message": {"role": "user", "content": content},
        "parent_tool_use_id": null,
        "session_id": "",
        "uuid": uuid,
    })
}

pub fn control_request(request_id: &str, request: Value) -> Value {
    json!({"type": "control_request", "request_id": request_id, "request": request})
}

pub fn control_success(request_id: &str, response: Value) -> Value {
    json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": request_id, "response": response}
    })
}

pub fn control_error(request_id: &str, error: &str) -> Value {
    json!({
        "type": "control_response",
        "response": {"subtype": "error", "request_id": request_id, "error": error}
    })
}

pub const DECLINE_MESSAGE: &str = "User declined tool execution.";
pub const CANCEL_MESSAGE: &str = "User cancelled tool execution.";
pub const PLAN_CAPTURED_MESSAGE: &str = "The client captured your proposed plan. Stop here and wait for the user's feedback or implementation request in a later turn.";

/// The `can_use_tool` answer for a decision.
///
/// * `accept` → allow once.
/// * `acceptForSession` → allow + the CLI's own suggestions **rescoped to
///   `session`** (echoing them verbatim could write `.claude/settings.local.json`);
///   with no suggestion, a session `addRules` for the tool. For an edit the
///   CLI suggests `setMode acceptEdits` (measured), i.e. "allow edits for the
///   rest of the session", the same as its own dialog.
/// * `acceptAlways` → allow + the suggestions as the CLI proposed them
///   (their own destination, typically `localSettings`); `setMode` stays
///   session-scoped; with none, a `localSettings` rule for the tool.
/// * `decline` → deny with the message the model sees.
/// * `cancel` → deny + `interrupt: true` (the CLI aborts the turn).
pub fn permission_answer(
    decision: ApprovalDecision,
    tool_name: &str,
    input: &Value,
    suggestions: &[Value],
) -> Value {
    let rule = |destination: &str| {
        json!({
            "type": "addRules",
            "rules": [{"toolName": tool_name}],
            "behavior": "allow",
            "destination": destination
        })
    };
    match decision {
        ApprovalDecision::Accept => json!({"behavior": "allow", "updatedInput": input}),
        ApprovalDecision::AcceptForSession => {
            let mut updates: Vec<Value> = suggestions
                .iter()
                .map(|s| {
                    let mut s = s.clone();
                    if let Some(obj) = s.as_object_mut() {
                        obj.insert("destination".into(), json!("session"));
                    }
                    s
                })
                .collect();
            if updates.is_empty() {
                updates.push(rule("session"));
            }
            json!({"behavior": "allow", "updatedInput": input, "updatedPermissions": updates})
        }
        ApprovalDecision::AcceptAlways => {
            let mut updates: Vec<Value> = suggestions
                .iter()
                .map(|s| {
                    let mut s = s.clone();
                    let is_mode = s.get("type").and_then(Value::as_str) == Some("setMode");
                    if let Some(obj) = s.as_object_mut() {
                        if is_mode {
                            obj.insert("destination".into(), json!("session"));
                        } else if !obj.contains_key("destination") {
                            obj.insert("destination".into(), json!("localSettings"));
                        }
                    }
                    s
                })
                .collect();
            if !updates
                .iter()
                .any(|u| u.get("type").and_then(Value::as_str) == Some("addRules"))
            {
                updates.push(rule("localSettings"));
            }
            json!({"behavior": "allow", "updatedInput": input, "updatedPermissions": updates})
        }
        ApprovalDecision::Decline => json!({"behavior": "deny", "message": DECLINE_MESSAGE}),
        ApprovalDecision::Cancel => {
            json!({"behavior": "deny", "message": CANCEL_MESSAGE, "interrupt": true})
        }
    }
}

/// `--mcp-config` body for MCP servers handed over by the host (the embedded
/// OmniGet MCP with the session token): Claude's `.mcp.json` shape.
pub fn mcp_config_json(servers: &[super::super::acp::McpServerSpec]) -> Value {
    use super::super::acp::McpServerSpec as S;
    let headers = |h: &[(String, String)]| -> Value {
        Value::Object(
            h.iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect(),
        )
    };
    let mut out = serde_json::Map::new();
    for s in servers {
        let (name, cfg) = match s {
            S::Http {
                name,
                url,
                headers: h,
            } => (
                name,
                json!({ "type": "http", "url": url, "headers": headers(h) }),
            ),
            S::Sse {
                name,
                url,
                headers: h,
            } => (
                name,
                json!({ "type": "sse", "url": url, "headers": headers(h) }),
            ),
            S::Stdio {
                name,
                command,
                args,
                env,
            } => (
                name,
                json!({ "type": "stdio", "command": command, "args": args, "env": headers(env) }),
            ),
        };
        out.insert(name.clone(), cfg);
    }
    json!({ "mcpServers": out })
}

#[cfg(test)]
mod mcp_config_tests {
    use super::*;
    use crate::core::llm::drivers::acp::McpServerSpec;

    #[test]
    fn embedded_server_becomes_an_http_entry_with_its_header() {
        let v = mcp_config_json(&[McpServerSpec::Http {
            name: "omniget".into(),
            url: "http://127.0.0.1:47720/mcp".into(),
            headers: vec![("Authorization".into(), "Bearer t".into())],
        }]);
        assert_eq!(v["mcpServers"]["omniget"]["type"], "http");
        assert_eq!(
            v["mcpServers"]["omniget"]["url"],
            "http://127.0.0.1:47720/mcp"
        );
        assert_eq!(
            v["mcpServers"]["omniget"]["headers"]["Authorization"],
            "Bearer t"
        );
        assert_eq!(mcp_config_json(&[])["mcpServers"], json!({}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_is_the_measured_long_lived_launch() {
        let launch = Launch {
            model: Some("haiku".into()),
            ..Default::default()
        };
        let joined = launch.argv().join(" ");
        assert!(joined.starts_with(
            "-p --input-format stream-json --output-format stream-json --verbose --include-partial-messages"
        ));
        assert!(joined.contains("--permission-prompt-tool stdio"));
        assert!(joined.contains("--permission-mode default"));
        assert!(joined.contains("--model haiku"));
        assert!(!joined.contains("dangerously"));
        assert!(!joined.contains("--resume"));
    }

    #[test]
    fn access_modes_map_to_cli_modes_and_bypass_only_for_full_access() {
        use AccessMode::*;
        assert_eq!(
            permission_mode(ApprovalRequired, InteractionMode::Default),
            "default"
        );
        assert_eq!(
            permission_mode(AutoAcceptEdits, InteractionMode::Default),
            "acceptEdits"
        );
        assert_eq!(permission_mode(Auto, InteractionMode::Default), "auto");
        assert_eq!(
            permission_mode(FullAccess, InteractionMode::Default),
            "bypassPermissions"
        );
        assert_eq!(permission_mode(FullAccess, InteractionMode::Plan), "plan");
        for mode in [ApprovalRequired, AutoAcceptEdits, Auto] {
            let argv = Launch {
                access: mode,
                ..Default::default()
            }
            .argv()
            .join(" ");
            assert!(
                !argv.contains("bypass") && !argv.contains("dangerously"),
                "{argv}"
            );
        }
        let argv = Launch {
            access: FullAccess,
            ..Default::default()
        }
        .argv()
        .join(" ");
        assert!(argv.contains("--permission-mode bypassPermissions"));
        assert!(argv.contains("--allow-dangerously-skip-permissions"));
    }

    #[test]
    fn resume_fork_and_rollback_flags() {
        let argv = Launch {
            resume: Some("sid".into()),
            resume_at: Some("m1".into()),
            fork: true,
            session_id: Some("ignored".into()),
            ..Default::default()
        }
        .argv()
        .join(" ");
        assert!(
            argv.contains("--resume sid --resume-session-at m1 --fork-session"),
            "{argv}"
        );
        assert!(!argv.contains("--session-id"));
        let fresh = Launch {
            session_id: Some("new".into()),
            ..Default::default()
        }
        .argv()
        .join(" ");
        assert!(fresh.contains("--session-id new"));
    }

    #[test]
    fn user_args_cannot_override_reserved_flags() {
        let args: Vec<String> = [
            "--chrome",
            "--permission-mode",
            "bypassPermissions",
            "--dangerously-skip-permissions",
            "--effort=high",
            "--resume=x",
            "--add-dir",
            "/tmp",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            filter_user_args(&args),
            vec!["--chrome", "--effort=high", "--add-dir", "/tmp"]
        );
    }

    #[test]
    fn the_text_block_goes_last() {
        let m = user_message(
            "u1",
            "/compact",
            &[
                json!({"mime": "image/png", "data": "AAAA"}),
                json!({"path": "/x/y.pdf"}),
            ],
        );
        let content = m["message"]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "image");
        assert_eq!(content.last().unwrap()["type"], "text");
        assert!(content.last().unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("/x/y.pdf"));
        assert_eq!(m["uuid"], "u1");
    }

    #[test]
    fn decisions_become_the_sdk_permission_results() {
        let input = json!({"file_path": "/t/a.txt", "content": "oi"});
        let sugg =
            vec![json!({"type": "setMode", "mode": "acceptEdits", "destination": "session"})];
        let a = permission_answer(ApprovalDecision::Accept, "Write", &input, &sugg);
        assert_eq!(a, json!({"behavior": "allow", "updatedInput": input}));
        let s = permission_answer(ApprovalDecision::AcceptForSession, "Write", &input, &sugg);
        assert_eq!(s["updatedPermissions"][0]["mode"], "acceptEdits");
        assert_eq!(s["updatedPermissions"][0]["destination"], "session");
        let mcp = permission_answer(
            ApprovalDecision::AcceptForSession,
            "mcp__x__y",
            &json!({}),
            &[],
        );
        assert_eq!(
            mcp["updatedPermissions"][0]["rules"][0]["toolName"],
            "mcp__x__y"
        );
        let bash_sugg = vec![
            json!({"type": "addRules", "rules": [{"toolName": "Bash", "ruleContent": "npm test:*"}], "behavior": "allow", "destination": "localSettings"}),
        ];
        let al = permission_answer(
            ApprovalDecision::AcceptAlways,
            "Bash",
            &json!({}),
            &bash_sugg,
        );
        assert_eq!(al["updatedPermissions"].as_array().unwrap().len(), 1);
        assert_eq!(al["updatedPermissions"][0]["destination"], "localSettings");
        let d = permission_answer(ApprovalDecision::Decline, "Write", &input, &sugg);
        assert_eq!(d, json!({"behavior": "deny", "message": DECLINE_MESSAGE}));
        let c = permission_answer(ApprovalDecision::Cancel, "Write", &input, &sugg);
        assert_eq!(c["interrupt"], true);
    }
}
