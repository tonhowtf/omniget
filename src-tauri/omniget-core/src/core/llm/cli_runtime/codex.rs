use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use super::super::error::LlmError;
use super::super::types::{ContentPart, FinishReason, Message, Role, TurnEvent, Usage};
use super::parse::{classify_error_text, CliSignal};
use super::session::LineParser;
use super::ERR_CLI_EXIT;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sandbox {
    #[default]
    ReadOnly,
    WorkspaceWrite,
}

impl Sandbox {
    pub fn as_str(self) -> &'static str {
        match self {
            Sandbox::ReadOnly => "read-only",
            Sandbox::WorkspaceWrite => "workspace-write",
        }
    }
}

impl From<super::accounts::SandboxMode> for Sandbox {
    fn from(mode: super::accounts::SandboxMode) -> Self {
        match mode {
            super::accounts::SandboxMode::ReadOnly => Sandbox::ReadOnly,
            super::accounts::SandboxMode::Write | super::accounts::SandboxMode::Full => {
                Sandbox::WorkspaceWrite
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CodexArgs {
    pub model: Option<String>,
    pub cwd: Option<String>,
    pub sandbox: Sandbox,
    pub resume: Option<String>,
    pub permission: Vec<String>,
}

pub fn argv(args: &CodexArgs) -> Vec<String> {
    let mut out: Vec<String> = vec!["exec".into()];
    if let Some(thread) = &args.resume {
        out.push("resume".into());
        out.push(thread.clone());
    }
    out.push("--json".into());
    out.push("--skip-git-repo-check".into());
    if args.permission.is_empty() {
        out.push("--sandbox".into());
        out.push(args.sandbox.as_str().into());
    } else {
        out.extend(args.permission.iter().cloned());
    }
    if let Some(model) = &args.model {
        out.push("-m".into());
        out.push(model.clone());
    }
    if let Some(dir) = &args.cwd {
        out.push("-C".into());
        out.push(dir.clone());
    }
    out.push("-".into());
    out
}

pub fn render_prompt(messages: &[Message]) -> String {
    let mut out = String::new();
    for message in messages {
        let label = match message.role {
            Role::System => "System",
            Role::User => "User",
            Role::Assistant => "Assistant",
            Role::Tool => "Tool",
        };
        for part in &message.parts {
            let text = match part {
                ContentPart::Text { text } => text.clone(),
                ContentPart::ToolResult { content, .. } => content.clone(),
                ContentPart::ToolUse { name, input, .. } => format!("(called {name} with {input})"),
                ContentPart::Image { mime, .. } => format!("(image: {mime})"),
            };
            if text.trim().is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(label);
            out.push_str(": ");
            out.push_str(&text);
        }
    }
    out
}

pub struct CodexParser {
    started: AtomicBool,
    errored: AtomicBool,
    denial_note: Option<String>,
}

impl Default for CodexParser {
    fn default() -> Self {
        Self::new()
    }
}

impl CodexParser {
    pub fn new() -> Self {
        Self {
            started: AtomicBool::new(false),
            errored: AtomicBool::new(false),
            denial_note: None,
        }
    }

    pub fn with_denial_note(mut self, note: String) -> Self {
        self.denial_note = Some(note);
        self
    }

    fn declined(&self, item: &Value, out: &mut Vec<CliSignal>) {
        if item.get("status").and_then(Value::as_str) != Some("declined") {
            return;
        }
        if let Some((id, _)) = tool_identity(item) {
            let note = self.denial_note.clone().unwrap_or_else(|| {
                "[OmniGet] Denied: Codex declined this call under its sandbox and approval policy, and `codex exec` cannot ask a person.".into()
            });
            out.push(CliSignal::Event(TurnEvent::ToolResult {
                id,
                content: note,
                is_error: true,
            }));
        }
    }

    fn take_start(&self, id: &str, out: &mut Vec<CliSignal>) {
        if !self.started.swap(true, Ordering::Relaxed) {
            out.push(CliSignal::Event(TurnEvent::Started {
                request_id: id.to_string(),
            }));
        }
    }

    fn push_error(&self, message: &str, out: &mut Vec<CliSignal>) {
        if self.errored.swap(true, Ordering::Relaxed) {
            return;
        }
        let code = classify_error_text(message).unwrap_or(ERR_CLI_EXIT);
        let mut error = LlmError::new(code, message.to_string());
        error.retryable = code == super::ERR_CLI_RATE;
        out.push(CliSignal::Event(TurnEvent::Error { error }));
    }
}

impl LineParser for CodexParser {
    fn parse_line(&self, line: &str) -> Vec<CliSignal> {
        let mut out = Vec::new();
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            tracing::debug!(
                "[codex] non-json line: {}",
                line.chars().take(120).collect::<String>()
            );
            return vec![CliSignal::Ignored];
        };
        match value.get("type").and_then(Value::as_str).unwrap_or("") {
            "thread.started" => {
                if let Some(id) = value.get("thread_id").and_then(Value::as_str) {
                    self.take_start(id, &mut out);
                    out.push(CliSignal::Session(id.to_string()));
                }
            }
            "turn.started" => {
                self.take_start("codex-turn", &mut out);
            }
            "item.started" => {
                if let Some(item) = value.get("item") {
                    if let Some((id, name)) = tool_identity(item) {
                        out.push(CliSignal::Event(TurnEvent::ToolCallStart { id, name }));
                    }
                }
            }
            "item.updated" => {}
            "item.completed" => {
                if let Some(item) = value.get("item") {
                    item_completed(self, item, &mut out);
                }
            }
            "turn.completed" => {
                if let Some(usage) = value.get("usage").and_then(usage_of) {
                    out.push(CliSignal::Event(TurnEvent::Usage { usage }));
                }
                out.push(CliSignal::Event(TurnEvent::Finished {
                    reason: FinishReason::Stop,
                }));
            }
            "turn.failed" => {
                let message = value
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("the turn failed");
                self.push_error(message, &mut out);
                out.push(CliSignal::Event(TurnEvent::Finished {
                    reason: FinishReason::Other,
                }));
            }
            "error" => {
                let message = value
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("the CLI reported an error");
                self.push_error(message, &mut out);
            }
            other => {
                tracing::debug!("[codex] unknown event `{other}`");
                out.push(CliSignal::Ignored);
            }
        }
        if out.is_empty() {
            out.push(CliSignal::Ignored);
        }
        out
    }
}

fn item_completed(parser: &CodexParser, item: &Value, out: &mut Vec<CliSignal>) {
    match item.get("type").and_then(Value::as_str).unwrap_or("") {
        "agent_message" => {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                out.push(CliSignal::Event(TurnEvent::TextDelta {
                    text: text.to_string(),
                }));
            }
        }
        "reasoning" => {
            if let Some(text) = item.get("text").and_then(Value::as_str) {
                out.push(CliSignal::Event(TurnEvent::ThinkingDelta {
                    text: text.to_string(),
                }));
            }
        }
        "error" => {
            let message = item
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("the CLI reported an item error");
            parser.push_error(message, out);
        }
        "file_change" => {
            if let Some((id, name)) = tool_identity(item) {
                out.push(CliSignal::Event(TurnEvent::ToolCallStart {
                    id: id.clone(),
                    name,
                }));
                out.push(CliSignal::Event(TurnEvent::ToolCallEnd { id }));
            }
            parser.declined(item, out);
        }
        "command_execution" | "mcp_tool_call" | "collab_tool_call" | "web_search" => {
            if let Some((id, _)) = tool_identity(item) {
                out.push(CliSignal::Event(TurnEvent::ToolCallEnd { id }));
            }
            parser.declined(item, out);
        }
        _ => {}
    }
}

fn tool_identity(item: &Value) -> Option<(String, String)> {
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("item")
        .to_string();
    let name = match item.get("type").and_then(Value::as_str)? {
        "command_execution" => item
            .get("command")
            .and_then(Value::as_str)
            .map(|c| c.split_whitespace().next().unwrap_or(c).to_string())
            .unwrap_or_else(|| "command".into()),
        "mcp_tool_call" => format!(
            "{}/{}",
            item.get("server").and_then(Value::as_str).unwrap_or("mcp"),
            item.get("tool").and_then(Value::as_str).unwrap_or("tool")
        ),
        "collab_tool_call" => item
            .get("tool")
            .and_then(Value::as_str)
            .unwrap_or("collab")
            .to_string(),
        "web_search" => "web_search".to_string(),
        "file_change" => {
            let n = item
                .get("changes")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            format!("apply_patch({n})")
        }
        _ => return None,
    };
    Some((id, name))
}

fn usage_of(usage: &Value) -> Option<Usage> {
    let n = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0) as u32;
    Some(Usage {
        input_tokens: n("input_tokens"),
        output_tokens: n("output_tokens"),
        cache_read_tokens: n("cached_input_tokens"),
        cache_write_tokens: n("cache_write_input_tokens"),
        first_token_ms: None,
        total_ms: 0,
        cost_usd: None,
    })
}
