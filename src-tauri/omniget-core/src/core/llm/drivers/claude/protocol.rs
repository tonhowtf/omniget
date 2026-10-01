use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::{json, Value};

use super::super::{AccessMode, ApprovalDecision, InteractionMode};

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

#[derive(Debug, Clone, Default)]
pub struct Launch {
    pub model: Option<String>,
    pub access: AccessMode,
    pub interaction: InteractionMode,
    pub resume: Option<String>,
    pub resume_at: Option<String>,
    pub fork: bool,
    pub session_id: Option<String>,
    pub mcp_config: Option<String>,
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

pub fn filter_user_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let name = arg.split('=').next().unwrap_or(arg);
        if RESERVED_FLAGS.contains(&name) {
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

pub fn scrub_env() -> Vec<String> {
    crate::core::llm::cli_runtime::SCRUB_ENV
        .iter()
        .chain(NESTING_ENV.iter())
        .map(|s| s.to_string())
        .collect()
}

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
