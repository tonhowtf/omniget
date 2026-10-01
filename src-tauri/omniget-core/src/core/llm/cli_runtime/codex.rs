//! Codex CLI (`codex exec --json`) as a turn. Owned by f4-cli-runtime.
//!
//! Event schema from the serde definitions that produce the JSONL,
//! `codex-rs/exec/src/exec_events.rs` in openai/codex, and from
//! learn.chatgpt.com/docs/non-interactive-mode. Top level is
//! `#[serde(tag = "type")]`: `thread.started`, `turn.started`,
//! `turn.completed`, `turn.failed`, `item.started|updated|completed`, `error`.
//! A `ThreadItem` flattens its own `type` next to `id`.
//!
//! Isolation is `CODEX_HOME` (default `~/.codex`, holds `config.toml` and
//! `auth.json`). This file never opens `auth.json`.
//!
//! Codex was **not installed on the machine that wrote this** (`which codex`
//! empty, 18/09/2026), so the fixtures are reconstructed from the schema and
//! from the sample stream in the docs; `SOURCES.txt` in the fixture folder
//! says which lines are verbatim and which are not.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use super::super::error::LlmError;
use super::super::types::{ContentPart, FinishReason, Message, Role, TurnEvent, Usage};
use super::parse::{classify_error_text, CliSignal};
use super::session::LineParser;
use super::ERR_CLI_EXIT;

/// Sandbox for a turn we drive.
///
/// The three upstream values are `read-only`, `workspace-write` and
/// `danger-full-access` (`SandboxModeCliArg`, kebab-cased, in
/// `codex-rs/utils/cli/src/sandbox_mode_cli_arg.rs`); `--sandbox`/`-s` is a
/// shared flag, so `codex exec` takes it (`codex-rs/utils/cli/src/shared_options.rs`).
/// `read-only` is the CLI's own `#[default]`
/// (`codex-rs/protocol/src/config_types.rs`), and it is the only mode OmniGet
/// picks unless the user changed the account. **`danger-full-access` is not
/// modelled here on purpose**: there is no code path that can produce it.
///
/// Worth knowing when reading the tool stream: the sandbox governs the shell
/// commands the model runs, not the MCP servers — those are started as children
/// of the Codex process itself
/// (`codex-rs/rmcp-client/src/stdio_server_launcher.rs`, `LocalStdioServerLauncher`:
/// "starts the configured command as a child of the orchestrator process"), so
/// `read-only` does not constrain what an MCP server does.
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
            // Without a run mode a `full` account is still only a write one:
            // `danger-full-access` comes from `run_mode::codex_flags` alone.
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
    /// `codex exec resume <id>`: continue an earlier thread.
    pub resume: Option<String>,
    /// The run mode's flags (`run_mode::codex_flags`, 2.7). When set they
    /// replace `--sandbox <sandbox>`; empty = the account's sandbox alone.
    pub permission: Vec<String>,
}

/// Builds the argv. The trailing `-` makes Codex read the prompt from stdin,
/// which keeps the conversation out of `ps` exactly like the Claude path.
pub fn argv(args: &CodexArgs) -> Vec<String> {
    let mut out: Vec<String> = vec!["exec".into()];
    if let Some(thread) = &args.resume {
        out.push("resume".into());
        out.push(thread.clone());
    }
    out.push("--json".into());
    // Turns in a plain directory must not be refused for not being a repo.
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

/// Same flattening as the Claude path: Codex takes one prompt.
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
    /// Said next to a call Codex declined (2.7, S5): `codex exec` has no
    /// approval surface, so a declined call is a denial the user must see
    /// with its reason.
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

    /// The reason given next to a declined call (the turn's run mode).
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
        // `file_change` is the one item type that is *only* ever emitted as
        // completed ("emitted only as a completed event once the patch succeeds
        // or fails", `codex-rs/exec/src/exec_events.rs`), so there is no
        // `item.started` to pair with. Open and close it here, otherwise the
        // single thing a turn can do that a read-only sandbox forbids would be
        // the only thing the timeline never shows.
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

/// `(id, display name)` for the item types that are a tool call. `None` for
/// everything else.
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
        // `apply_patch` is what the user sees Codex call; the item carries the
        // paths, so the name says how many files moved.
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
        // Codex reports no dollar figure; the Observatory prices the tokens.
        cost_usd: None,
    })
}
