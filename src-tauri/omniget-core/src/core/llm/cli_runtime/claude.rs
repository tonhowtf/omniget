//! Claude Code (`claude -p --output-format stream-json --verbose`) as a turn.
//! Owned by f4-cli-runtime.
//!
//! Flags checked against the installed 2.1.276 (`claude --help`, 18/09/2026):
//! `-p/--print`, `--output-format stream-json`, `--verbose`,
//! `--include-partial-messages`, `--model`, `--append-system-prompt`,
//! `--max-budget-usd`, `--session-id`, `--no-session-persistence`,
//! `--permission-prompts none`, `--settings`. The prompt goes in on **stdin**,
//! so nothing about the conversation shows up in `ps`.
//!
//! Quota comes from `rate_limit_event` only (plan §9.2, `estudos/74` §A.3).
//! This file never opens `.credentials.json`, never touches the keychain and
//! never calls an HTTP endpoint.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::Value;

use super::super::error::LlmError;
use super::super::types::{ContentPart, FinishReason, Message, Role, TurnEvent, Usage};
use super::parse::{
    classify_error_text, CliSignal, QuotaSource, RateSnapshot, RateStatus, RateWindow,
};
use super::session::LineParser;
use super::ERR_CLI_EXIT;

/// Arguments for one non-interactive turn. `prompt` is **not** here: it goes
/// on stdin.
#[derive(Debug, Clone, Default)]
pub struct ClaudeArgs {
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub max_budget_usd: Option<f64>,
    /// Ask for token-level deltas. On by default: without it the CLI only
    /// emits whole assistant messages and the UI cannot stream.
    pub partial_messages: bool,
    /// Session id to resume, when the coordinator keeps one per conversation.
    pub resume: Option<String>,
    /// Extra settings file (`--settings`), used to install the status line.
    pub settings: Option<String>,
    /// `--permission-mode`: Claude Code's half of the sandbox knob. `plan` is
    /// the read-only mode (the model may read and reason, not edit) and
    /// `acceptEdits` is the write one. `None` leaves the CLI's own default,
    /// which the runtime never selects: [`super::mod`]'s plan always sets one.
    pub permission_mode: Option<&'static str>,
    /// `--mcp-config <file>`: OmniGet's scoped projection of the assistant
    /// tools (a private temp file, deleted after the turn).
    pub mcp_config: Option<String>,
    /// `--allowedTools`: pre-approved tools (the projection's server; its
    /// own grants are checked by OmniGet on every call).
    pub allowed_tools: Vec<String>,
    /// `--tools`: the built-in tools available at all. `Some` only for a
    /// personal (projectless) conversation, which gets web tools and no
    /// file/shell tools.
    pub tools: Option<String>,
    /// `--disallowedTools`, belt and braces with `tools`.
    pub disallowed_tools: Vec<String>,
    /// `--permission-prompt-tool`: routes permission prompts to OmniGet (via
    /// the projection) instead of denying them. Switches
    /// `--permission-prompts` to `host`.
    pub permission_prompt_tool: Option<String>,
    /// `--restricted`: drops code-running built-ins and ignores the user's
    /// own settings/hooks (external missions).
    pub restricted: bool,
    /// `--strict-mcp-config`: only the MCP servers of `--mcp-config` (none
    /// at all when there is no config).
    pub strict_mcp: bool,
    /// `--exclude-dynamic-system-prompt-sections`: cwd, env and git status go
    /// to the first user message, so the system prefix is the same for every
    /// job and is read from the prompt cache across jobs.
    pub exclude_dynamic: bool,
    /// `--effort` (`low`..`max`), already normalised by [`effort_level`].
    pub effort: Option<String>,
}

/// Built-in tools an external (MCP-controlled) mission never gets: every
/// file, shell, web and agent tool. It acts only through OmniGet's projection,
/// where the external grant is checked on each call.
pub const EXTERNAL_DENIED: &str = "Bash,Edit,Write,MultiEdit,NotebookEdit,Read,Glob,Grep,LS,KillShell,BashOutput,WebSearch,WebFetch,Task,Agent,TodoWrite,ExitPlanMode,SlashCommand,Skill";

/// Built-in tools a personal conversation may use: web only.
pub const PROJECTLESS_TOOLS: &str = "WebSearch,WebFetch";
/// Built-in tools a personal conversation never gets (file and shell).
pub const PROJECTLESS_DENIED: &str =
    "Bash,Edit,Write,MultiEdit,NotebookEdit,Read,Glob,Grep,LS,KillShell,BashOutput";

/// Built-in tools of a project job: the coding set plus web. `ToolSearch`
/// stays, or Claude Code loads every MCP tool schema up front (82k tokens of
/// context measured on 2.1.283 against ~13k with it).
pub const PROJECT_TOOLS: &str = "Bash,Read,Edit,Write,Glob,Grep,WebFetch,WebSearch,ToolSearch";

/// Appended to a project job's system prompt: fewer turns, fewer resends of
/// the whole context.
pub const PROJECT_BATCHING: &str = "Work in as few turns as possible: batch independent tool calls in one message (read every file you need at once), make all the edits, then run the tests once at the end. Do not re-read files you just wrote or re-run checks that already passed. Stop as soon as the result is verified.";

/// `--effort` of a lean project job when the turn does not ask for one.
/// Measured on 2.1.283 (missions, sonnet, 3 reps): median 152.8 s -> 125.6 s
/// for the 11 missions, cost unchanged (US$ 0.33-0.41 per round), 33/33 ok.
pub const PROJECT_EFFORT: &str = "low";

/// Maps a `reasoning_effort` (OpenAI-style or Claude Code's own) to a level
/// `claude --effort` accepts (2.1.283: low, medium, high, xhigh, max).
pub fn effort_level(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "none" | "minimal" | "low" => Some("low"),
        "medium" => Some("medium"),
        "high" => Some("high"),
        "xhigh" => Some("xhigh"),
        "max" => Some("max"),
        _ => None,
    }
}

/// `--permission-mode` value for a [`super::accounts::SandboxMode`]. Verified
/// against `claude --help` of 2.1.276: the accepted choices are `acceptEdits`,
/// `auto`, `bypassPermissions`, `manual`, `dontAsk` and `plan`. The bypass one
/// is never produced here.
pub fn permission_mode(sandbox: super::accounts::SandboxMode) -> &'static str {
    match sandbox {
        super::accounts::SandboxMode::ReadOnly => "plan",
        super::accounts::SandboxMode::Write => "acceptEdits",
    }
}

impl ClaudeArgs {
    pub fn streaming() -> Self {
        Self {
            partial_messages: true,
            ..Self::default()
        }
    }
}

/// Builds the argv. Pure, so the test can pin the flags of a version.
pub fn argv(args: &ClaudeArgs) -> Vec<String> {
    let mut out: Vec<String> = vec![
        "-p".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
    ];
    match &args.permission_prompt_tool {
        // Prompts go to OmniGet's user through the projection.
        Some(tool) => {
            out.push("--permission-prompts".into());
            out.push("host".into());
            out.push("--permission-prompt-tool".into());
            out.push(tool.clone());
        }
        // Nobody is at the keyboard: anything that would prompt is denied
        // instead of hanging the turn forever.
        None => {
            out.push("--permission-prompts".into());
            out.push("none".into());
        }
    }
    if let Some(mode) = args.permission_mode {
        out.push("--permission-mode".into());
        out.push(mode.into());
    }
    if args.partial_messages {
        out.push("--include-partial-messages".into());
    }
    if let Some(model) = &args.model {
        out.push("--model".into());
        out.push(model.clone());
    }
    if let Some(effort) = &args.effort {
        out.push("--effort".into());
        out.push(effort.clone());
    }
    if let Some(system) = &args.system_prompt {
        if !system.trim().is_empty() {
            out.push("--append-system-prompt".into());
            out.push(system.clone());
        }
    }
    if let Some(budget) = args.max_budget_usd {
        out.push("--max-budget-usd".into());
        out.push(budget.to_string());
    }
    if let Some(session) = &args.resume {
        out.push("--resume".into());
        out.push(session.clone());
    }
    if let Some(settings) = &args.settings {
        out.push("--settings".into());
        out.push(settings.clone());
    }
    if args.restricted {
        out.push("--restricted".into());
    }
    if let Some(tools) = &args.tools {
        out.push("--tools".into());
        out.push(tools.clone());
    }
    if !args.disallowed_tools.is_empty() {
        out.push("--disallowedTools".into());
        out.push(args.disallowed_tools.join(","));
    }
    if let Some(config) = &args.mcp_config {
        out.push("--mcp-config".into());
        out.push(config.clone());
    }
    if args.strict_mcp {
        out.push("--strict-mcp-config".into());
    }
    if args.exclude_dynamic {
        out.push("--exclude-dynamic-system-prompt-sections".into());
    }
    if !args.allowed_tools.is_empty() {
        out.push("--allowedTools".into());
        out.push(args.allowed_tools.join(","));
    }
    out
}

/// The text of every system message, for `--append-system-prompt` when the
/// provider keeps the history (resume): instructions and this turn's context
/// still reach the model, the transcript does not travel again.
pub fn system_text(messages: &[Message]) -> String {
    messages
        .iter()
        .filter(|m| m.role == Role::System)
        .flat_map(|m| m.parts.iter())
        .filter_map(|p| match p {
            ContentPart::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Only the newest user message: what a resumed session needs.
pub fn last_user_text(messages: &[Message]) -> String {
    messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User)
        .map(|m| {
            m.parts
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Flattens a conversation into the single prompt the CLI takes. The CLI owns
/// its own history when `--resume` is used; without it the whole conversation
/// is replayed, which is also what a reroute does (plan §9.2).
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
                ContentPart::ToolUse { name, input, .. } => {
                    format!("(called {name} with {input})")
                }
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

/// Line parser for Claude Code's stream-json. State is the minimum that keeps
/// the stream well formed: `Started` once, `Error` once, and the map from an
/// SSE block index to the tool id it opened (the deltas only carry `index`).
pub struct ClaudeParser {
    partial: bool,
    started: AtomicBool,
    errored: AtomicBool,
    tool_blocks: Mutex<HashMap<u64, String>>,
}

impl ClaudeParser {
    pub fn new(partial: bool) -> Self {
        Self {
            partial,
            started: AtomicBool::new(false),
            errored: AtomicBool::new(false),
            tool_blocks: Mutex::new(HashMap::new()),
        }
    }

    fn take_start(&self, id: &str, out: &mut Vec<CliSignal>) {
        if !self.started.swap(true, Ordering::Relaxed) {
            out.push(CliSignal::Event(TurnEvent::Started {
                request_id: id.to_string(),
            }));
        }
    }

    fn push_error(&self, error: LlmError, out: &mut Vec<CliSignal>) {
        if !self.errored.swap(true, Ordering::Relaxed) {
            out.push(CliSignal::Event(TurnEvent::Error { error }));
        }
    }
}

impl LineParser for ClaudeParser {
    fn parse_line(&self, line: &str) -> Vec<CliSignal> {
        let mut out = Vec::new();
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            // Not JSON: a stray log line. Never fatal (plan Fase 4 "Plano B").
            tracing::debug!("[claude] non-json line: {}", head(line));
            return vec![CliSignal::Ignored];
        };
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "system" => {
                if let Some(id) = value.get("session_id").and_then(Value::as_str) {
                    self.take_start(id, &mut out);
                    out.push(CliSignal::Session(id.to_string()));
                }
            }
            "stream_event" => {
                if self.partial {
                    if let Some(event) = value.get("event") {
                        self.partial_event(event, &mut out);
                    }
                }
            }
            "assistant" => {
                let message = value.get("message");
                let is_error = value
                    .get("is_api_error_message")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    || value.get("error").and_then(Value::as_str).is_some();
                if is_error {
                    let detail = message
                        .map(collect_text)
                        .filter(|t| !t.is_empty())
                        .or_else(|| {
                            value
                                .get("error")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                        .unwrap_or_else(|| "the CLI reported an error".into());
                    let combined = format!(
                        "{} {}",
                        value.get("error").and_then(Value::as_str).unwrap_or(""),
                        detail
                    );
                    let code = classify_error_text(&combined).unwrap_or(ERR_CLI_EXIT);
                    let mut error = LlmError::new(code, detail);
                    error.retryable = code == super::ERR_CLI_RATE;
                    self.push_error(error, &mut out);
                } else if !self.partial {
                    // Whole-message mode: the text arrives once, here.
                    if let Some(message) = message {
                        let text = collect_text(message);
                        if !text.is_empty() {
                            out.push(CliSignal::Event(TurnEvent::TextDelta { text }));
                        }
                        for (id, name) in tool_uses(message) {
                            out.push(CliSignal::Event(TurnEvent::ToolCallStart { id, name }));
                        }
                    }
                }
            }
            "result" => {
                if let Some(usage) = result_usage(&value) {
                    out.push(CliSignal::Event(TurnEvent::Usage { usage }));
                }
                let is_error = value
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let subtype = value.get("subtype").and_then(Value::as_str).unwrap_or("");
                if is_error || subtype.starts_with("error") {
                    // The success arm carries `result`; the error arm carries
                    // `errors: string[]` and no `result` at all.
                    let text = value
                        .get("result")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                        .or_else(|| {
                            value.get("errors").and_then(Value::as_array).map(|list| {
                                list.iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join("; ")
                            })
                        })
                        .filter(|t| !t.trim().is_empty())
                        .unwrap_or_else(|| format!("the CLI ended with `{subtype}`"));
                    let terminal = value
                        .get("terminal_reason")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let code = classify_error_text(&format!("{text} {subtype} {terminal}"))
                        .unwrap_or(ERR_CLI_EXIT);
                    let mut error = LlmError::new(code, text);
                    error.retryable = code == super::ERR_CLI_RATE;
                    self.push_error(error, &mut out);
                }
                out.push(CliSignal::Event(TurnEvent::Finished {
                    reason: match (is_error || subtype.starts_with("error"), subtype) {
                        (true, _) => FinishReason::Other,
                        (false, _)
                            if value.get("stop_reason").and_then(Value::as_str)
                                == Some("max_tokens") =>
                        {
                            FinishReason::Length
                        }
                        _ => FinishReason::Stop,
                    },
                }));
            }
            "rate_limit_event" => {
                if let Some(snapshot) = rate_snapshot(&value) {
                    if snapshot.status == RateStatus::Rejected {
                        let mut error = LlmError::new(
                            super::ERR_CLI_RATE,
                            "the CLI account is out of quota for this window",
                        );
                        error.retryable = true;
                        self.push_error(error, &mut out);
                    }
                    out.push(CliSignal::Rate(snapshot));
                }
            }
            // `user` records carry what the CLI's own tools answered:
            // `{"type":"user","message":{"content":[{"type":"tool_result",
            // "tool_use_id":"toolu_…","content":"…" | [{"type":"text",…}]}]}}`.
            "user" => {
                let blocks = value
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for block in blocks {
                    if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                        continue;
                    }
                    let Some(id) = block.get("tool_use_id").and_then(Value::as_str) else {
                        continue;
                    };
                    let content = match block.get("content") {
                        Some(Value::String(text)) => text.clone(),
                        Some(Value::Array(parts)) => parts
                            .iter()
                            .filter_map(|p| p.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => String::new(),
                    };
                    out.push(CliSignal::Event(TurnEvent::ToolResult {
                        id: id.to_string(),
                        content,
                        is_error: block
                            .get("is_error")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    }));
                }
            }
            "prompt_suggestion" | "hook_event" | "control_response" => {}
            other => {
                tracing::debug!("[claude] unknown event `{other}`");
                out.push(CliSignal::Ignored);
            }
        }
        if out.is_empty() {
            out.push(CliSignal::Ignored);
        }
        out
    }
}

fn head(line: &str) -> String {
    line.chars().take(120).collect()
}

impl ClaudeParser {
    /// One raw Anthropic SSE event, as `--include-partial-messages` wraps it
    /// (`stream_event.event` is the `data:` object verbatim).
    fn partial_event(&self, event: &Value, out: &mut Vec<CliSignal>) {
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        let index = event.get("index").and_then(Value::as_u64).unwrap_or(0);
        match kind {
            "content_block_start" => {
                let block = event.get("content_block");
                if block.and_then(|b| b.get("type")).and_then(Value::as_str) == Some("tool_use") {
                    let id = block
                        .and_then(|b| b.get("id"))
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .to_string();
                    let name = block
                        .and_then(|b| b.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or("tool")
                        .to_string();
                    if let Ok(mut map) = self.tool_blocks.lock() {
                        map.insert(index, id.clone());
                    }
                    out.push(CliSignal::Event(TurnEvent::ToolCallStart { id, name }));
                }
            }
            "content_block_delta" => {
                let delta = event.get("delta");
                let delta_kind = delta
                    .and_then(|d| d.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                match delta_kind {
                    "text_delta" => {
                        if let Some(text) =
                            delta.and_then(|d| d.get("text")).and_then(Value::as_str)
                        {
                            out.push(CliSignal::Event(TurnEvent::TextDelta {
                                text: text.to_string(),
                            }));
                        }
                    }
                    "thinking_delta" => {
                        if let Some(text) = delta
                            .and_then(|d| d.get("thinking"))
                            .and_then(Value::as_str)
                        {
                            out.push(CliSignal::Event(TurnEvent::ThinkingDelta {
                                text: text.to_string(),
                            }));
                        }
                    }
                    "input_json_delta" => {
                        if let Some(text) = delta
                            .and_then(|d| d.get("partial_json"))
                            .and_then(Value::as_str)
                        {
                            out.push(CliSignal::Event(TurnEvent::ToolCallDelta {
                                id: self.tool_id(index).unwrap_or_default(),
                                input_json_delta: text.to_string(),
                            }));
                        }
                    }
                    // `signature_delta` always precedes the stop of a thinking
                    // block and carries no user-visible text.
                    _ => {}
                }
            }
            "content_block_stop" => {
                // Only a tool block closes a tool call; a text block closing is
                // not a `ToolCallEnd`.
                let id = self
                    .tool_blocks
                    .lock()
                    .ok()
                    .and_then(|mut map| map.remove(&index));
                if let Some(id) = id {
                    out.push(CliSignal::Event(TurnEvent::ToolCallEnd { id }));
                }
            }
            _ => {}
        }
    }

    fn tool_id(&self, index: u64) -> Option<String> {
        self.tool_blocks.lock().ok()?.get(&index).cloned()
    }
}

fn collect_text(message: &Value) -> String {
    let Some(content) = message.get("content").and_then(Value::as_array) else {
        return message
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
    };
    content
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

fn tool_uses(message: &Value) -> Vec<(String, String)> {
    let Some(content) = message.get("content").and_then(Value::as_array) else {
        return Vec::new();
    };
    content
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
        .map(|b| {
            (
                b.get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string(),
                b.get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string(),
            )
        })
        .collect()
}

/// `result.usage` + `result.total_cost_usd` + `result.duration_ms`. The CLI
/// calls the cost a "client-side estimate" (`estudos/74` §A.2) and we keep it
/// as such: the Observatory shows it, the budget does not bill on it.
fn result_usage(value: &Value) -> Option<Usage> {
    let usage = value.get("usage")?;
    let n = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0) as u32;
    // The CLI reports Anthropic's raw shape (input = fresh only); the Usage
    // contract wants the whole input, cache included, like every runtime.
    Some(Usage {
        input_tokens: n("input_tokens")
            .saturating_add(n("cache_read_input_tokens"))
            .saturating_add(n("cache_creation_input_tokens")),
        output_tokens: n("output_tokens"),
        cache_read_tokens: n("cache_read_input_tokens"),
        cache_write_tokens: n("cache_creation_input_tokens"),
        first_token_ms: value
            .get("ttft_ms")
            .and_then(Value::as_u64)
            .map(|v| v as u32),
        total_ms: value
            .get("duration_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        cost_usd: value.get("total_cost_usd").and_then(Value::as_f64),
    })
}

/// Reads the quota out of a `rate_limit_event`.
///
/// Two shapes are in the wild and both are handled:
///
/// * **Documented** (`SDKRateLimitEvent`, code.claude.com/docs/en/agent-sdk/
///   typescript): `rate_limit_info: { status, resetsAt?, utilization?,
///   errorCode? }` — flat, with **no** per-window breakdown. The single number
///   lands on the 5 h window, because that is the one a session actually hits.
/// * **Observed live on 2.1.275** (`estudos/74` §A.2):
///   `unifiedWindows.{five_hour,seven_day}.{utilization, resetsAt}`.
///
/// Neither doc pins the range of `utilization`; the live reading was `0..1`
/// and the status line's sibling `used_percentage` is `0..100`, so
/// [`window_at`] decides by value: above 1.0 it can only be a percentage.
pub fn rate_snapshot(value: &Value) -> Option<RateSnapshot> {
    let info = value
        .get("rate_limit_info")
        .or_else(|| value.get("rateLimitInfo"))
        .unwrap_or(value);
    let status = info
        .get("status")
        .or_else(|| value.get("status"))
        .and_then(Value::as_str)
        .map(RateStatus::parse)
        .unwrap_or(RateStatus::Unknown);
    let windows = info
        .get("unifiedWindows")
        .or_else(|| info.get("unified_windows"))
        .or_else(|| value.get("unifiedWindows"))
        .unwrap_or(info);
    let mut five = window_at(windows, "five_hour").or_else(|| window_at(windows, "fiveHour"));
    let seven = window_at(windows, "seven_day").or_else(|| window_at(windows, "sevenDay"));
    if five.is_none() && seven.is_none() {
        // Flat documented shape: one utilization for the whole session.
        five = flat_window(info);
    }
    if five.is_none() && seven.is_none() && status == RateStatus::Unknown {
        return None;
    }
    Some(RateSnapshot {
        status,
        window_5h: five,
        window_7d: seven,
        source: QuotaSource::Real,
    })
}

fn flat_window(info: &Value) -> Option<RateWindow> {
    let resets = info
        .get("resetsAt")
        .or_else(|| info.get("resets_at"))
        .and_then(epoch_seconds);
    let util = info.get("utilization").and_then(Value::as_f64)?;
    Some(if util > 1.0 {
        RateWindow::from_percentage(util, resets)
    } else {
        RateWindow::from_utilization(util, resets)
    })
}

fn window_at(parent: &Value, key: &str) -> Option<RateWindow> {
    let node = parent.get(key)?;
    let resets = node
        .get("resetsAt")
        .or_else(|| node.get("resets_at"))
        .and_then(epoch_seconds);
    if let Some(util) = node.get("utilization").and_then(Value::as_f64) {
        // 0..1 on the stream; some builds send 0..100. Above 1 it can only be
        // a percentage.
        return Some(if util > 1.0 {
            RateWindow::from_percentage(util, resets)
        } else {
            RateWindow::from_utilization(util, resets)
        });
    }
    let pct = node
        .get("used_percentage")
        .or_else(|| node.get("usedPercentage"))
        .and_then(Value::as_f64)?;
    Some(RateWindow::from_percentage(pct, resets))
}

/// `resetsAt` is epoch seconds on the stream and an RFC 3339 string in some
/// builds; both land on epoch seconds.
fn epoch_seconds(value: &Value) -> Option<i64> {
    if let Some(n) = value.as_i64() {
        // Milliseconds would put the reset in the year 57000.
        return Some(if n > 100_000_000_000 { n / 1000 } else { n });
    }
    if let Some(f) = value.as_f64() {
        return Some(f as i64);
    }
    let text = value.as_str()?;
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|d| d.timestamp())
}

/// The status-line payload the CLI hands an installed status line
/// (`rate_limits.{five_hour,seven_day}.{used_percentage,resets_at}`), for the
/// interactive sessions the user opens from the Accounts tab.
pub fn statusline_snapshot(input: &str) -> Option<RateSnapshot> {
    let value: Value = serde_json::from_str(input).ok()?;
    let limits = value.get("rate_limits")?;
    let five = window_at(limits, "five_hour");
    let seven = window_at(limits, "seven_day");
    if five.is_none() && seven.is_none() {
        return None;
    }
    Some(RateSnapshot {
        status: RateStatus::Allowed,
        window_5h: five,
        window_7d: seven,
        source: QuotaSource::Real,
    })
}

#[cfg(test)]
mod tests {
    use super::super::fixture;
    use super::*;

    #[test]
    fn a_user_record_yields_the_result_of_the_clis_own_tool() {
        let parser = ClaudeParser::new(true);
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_01A","content":[{"type":"text","text":"line 1"},{"type":"text","text":"line 2"}]}]}}"#;
        let events: Vec<TurnEvent> = parser
            .parse_line(line)
            .into_iter()
            .filter_map(|s| match s {
                CliSignal::Event(e) => Some(e),
                _ => None,
            })
            .collect();
        assert_eq!(
            events,
            vec![TurnEvent::ToolResult {
                id: "toolu_01A".into(),
                content: "line 1\nline 2".into(),
                is_error: false
            }]
        );
    }

    fn parse_all(parser: &ClaudeParser, text: &str) -> Vec<CliSignal> {
        text.lines()
            .filter(|l| !l.trim().is_empty())
            .flat_map(|l| parser.parse_line(l))
            .collect()
    }

    fn events(signals: &[CliSignal]) -> Vec<TurnEvent> {
        signals
            .iter()
            .filter_map(|s| match s {
                CliSignal::Event(e) => Some(e.clone()),
                _ => None,
            })
            .collect()
    }

    fn text_of(events: &[TurnEvent]) -> String {
        events
            .iter()
            .filter_map(|e| match e {
                TurnEvent::TextDelta { text } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn external_missions_run_claude_with_no_builtin_tools_and_only_the_projection() {
        let joined = argv(&ClaudeArgs {
            mcp_config: Some("/tmp/mcp.json".into()),
            allowed_tools: vec!["mcp__omniget".into()],
            tools: Some(String::new()),
            disallowed_tools: vec![EXTERNAL_DENIED.into()],
            permission_prompt_tool: Some("mcp__omniget__omniget_permission".into()),
            restricted: true,
            strict_mcp: true,
            ..ClaudeArgs::default()
        });
        let pos = |f: &str| {
            joined
                .iter()
                .position(|a| a == f)
                .unwrap_or_else(|| panic!("{f} missing: {joined:?}"))
        };
        assert_eq!(joined[pos("--tools") + 1], "", "no built-in tools at all");
        assert!(
            joined[pos("--disallowedTools") + 1].contains("Bash")
                && joined[pos("--disallowedTools") + 1].contains("WebFetch")
        );
        pos("--restricted");
        assert_eq!(joined[pos("--mcp-config") + 2], "--strict-mcp-config");
        assert_eq!(joined[pos("--allowedTools") + 1], "mcp__omniget");
    }

    /// A project job drops the user's MCP servers even with no projection,
    /// keeps `ToolSearch` (without it the CLI loads every MCP schema eagerly:
    /// 82k tokens measured on 2.1.283) and moves per-machine sections out of
    /// the system prompt so the prefix is cached across jobs.
    #[test]
    fn effort_goes_on_the_argv_only_when_set_and_valid() {
        let with = |e: Option<&str>| {
            argv(&ClaudeArgs {
                model: Some("sonnet".into()),
                effort: e.map(String::from),
                ..ClaudeArgs::default()
            })
        };
        let a = with(Some("low"));
        let i = a.iter().position(|x| x == "--effort").expect("--effort");
        assert_eq!(a[i + 1], "low");
        assert!(!with(None).iter().any(|x| x == "--effort"));
        assert_eq!(effort_level("XHigh"), Some("xhigh"));
        assert_eq!(effort_level("minimal"), Some("low"));
        assert_eq!(effort_level("none"), Some("low"));
        assert_eq!(effort_level("turbo"), None);
    }

    #[test]
    fn a_lean_project_job_is_strict_without_a_projection_and_cache_friendly() {
        let joined = argv(&ClaudeArgs {
            tools: Some(PROJECT_TOOLS.into()),
            strict_mcp: true,
            exclude_dynamic: true,
            ..ClaudeArgs::default()
        });
        assert!(
            joined.iter().any(|a| a == "--strict-mcp-config"),
            "{joined:?}"
        );
        assert!(joined
            .iter()
            .any(|a| a == "--exclude-dynamic-system-prompt-sections"));
        let pos = joined.iter().position(|a| a == "--tools").unwrap();
        assert!(joined[pos + 1].split(',').any(|t| t == "ToolSearch"));
        assert!(!joined[pos + 1].contains("Task"));
        // Off by default: nothing changes for other launches.
        let plain = argv(&ClaudeArgs::default());
        assert!(
            !plain
                .iter()
                .any(|a| a == "--strict-mcp-config"
                    || a == "--exclude-dynamic-system-prompt-sections")
        );
    }

    #[test]
    fn argv_matches_the_installed_2_1_276_flags() {
        let args = ClaudeArgs {
            model: Some("sonnet".into()),
            system_prompt: Some("be brief".into()),
            max_budget_usd: Some(0.5),
            partial_messages: true,
            resume: None,
            settings: None,
            permission_mode: Some(permission_mode(
                crate::core::llm::cli_runtime::accounts::SandboxMode::ReadOnly,
            )),
            ..ClaudeArgs::default()
        };
        let argv = argv(&args);
        let joined = argv.join(" ");
        assert!(joined.starts_with("-p --output-format stream-json --verbose"));
        assert!(joined.contains("--permission-mode plan"));
        assert!(joined.contains("--include-partial-messages"));
        assert!(joined.contains("--permission-prompts none"));
        assert!(joined.contains("--model sonnet"));
        assert!(joined.contains("--append-system-prompt be brief"));
        assert!(joined.contains("--max-budget-usd 0.5"));
        // The prompt is never in argv: it goes on stdin.
        assert!(!joined.contains("be brief\n"));
        assert!(!argv.iter().any(|a| a == "--dangerously-skip-permissions"));
    }

    #[test]
    fn argv_without_partial_messages_stays_whole_message() {
        let argv = argv(&ClaudeArgs::default());
        assert!(!argv.iter().any(|a| a == "--include-partial-messages"));
        // No mode asked for, no mode passed: the flag is opt-in here, and the
        // runtime's `plan()` is what always sets one.
        assert!(!argv.iter().any(|a| a == "--permission-mode"));
    }

    /// The read-only mode is Claude's `plan`, the write one is `acceptEdits`,
    /// and neither of the two bypass modes is reachable from here.
    #[test]
    fn the_permission_mode_never_bypasses_permissions() {
        use crate::core::llm::cli_runtime::accounts::SandboxMode;
        assert_eq!(permission_mode(SandboxMode::ReadOnly), "plan");
        assert_eq!(permission_mode(SandboxMode::Write), "acceptEdits");
        for mode in [SandboxMode::ReadOnly, SandboxMode::Write] {
            let joined = argv(&ClaudeArgs {
                permission_mode: Some(permission_mode(mode)),
                ..ClaudeArgs::default()
            })
            .join(" ");
            assert!(!joined.contains("bypassPermissions"), "{joined}");
            assert!(!joined.contains("dangerously"), "{joined}");
            // Nobody is at the keyboard either way.
            assert!(joined.contains("--permission-prompts none"));
        }
    }

    /// Flags checked against `claude --help` of 2.1.282 (24/09/2026):
    /// `--resume`, `--mcp-config`, `--allowedTools`, `--tools`,
    /// `--disallowedTools`, `--permission-prompts host`.
    #[test]
    fn argv_for_a_resumed_personal_turn_with_the_projection() {
        let argv = argv(&ClaudeArgs {
            resume: Some("sess-1".into()),
            mcp_config: Some("/tmp/mcp.json".into()),
            allowed_tools: vec!["mcp__omniget".into()],
            tools: Some(PROJECTLESS_TOOLS.into()),
            disallowed_tools: vec![PROJECTLESS_DENIED.into()],
            permission_prompt_tool: Some("mcp__omniget__omniget_permission".into()),
            ..ClaudeArgs::default()
        });
        let joined = argv.join(" ");
        assert!(joined.contains("--resume sess-1"));
        assert!(joined.contains("--mcp-config /tmp/mcp.json"));
        assert!(joined.contains("--allowedTools mcp__omniget"));
        assert!(joined.contains("--tools WebSearch,WebFetch"));
        assert!(joined.contains("--disallowedTools Bash,Edit"));
        assert!(joined.contains(
            "--permission-prompts host --permission-prompt-tool mcp__omniget__omniget_permission"
        ));
        assert!(!joined.contains("--permission-prompts none"));
        // Every variadic flag is followed by a flag or nothing: no prompt in argv.
        assert_eq!(argv.last().map(String::as_str), Some("mcp__omniget"));
    }

    #[test]
    fn render_prompt_labels_every_role() {
        let prompt = render_prompt(&[
            Message::text(Role::User, "oi"),
            Message::text(Role::Assistant, "ola"),
            Message::text(Role::User, "tudo bem?"),
        ]);
        assert_eq!(prompt, "User: oi\n\nAssistant: ola\n\nUser: tudo bem?");
        assert!(render_prompt(&[]).is_empty());
    }

    #[test]
    fn the_real_not_logged_in_fixture_is_err_cli_auth() {
        // Recorded on 18/09/2026 with claude 2.1.276 in an EMPTY temporary
        // CLAUDE_CONFIG_DIR; exit code 1.
        let text = fixture("claude-2.1.276-not-logged-in.jsonl");
        let parser = ClaudeParser::new(true);
        let signals = parse_all(&parser, &text);
        let events = events(&signals);
        assert!(matches!(events.first(), Some(TurnEvent::Started { .. })));
        let error = events
            .iter()
            .find_map(|e| match e {
                TurnEvent::Error { error } => Some(error.clone()),
                _ => None,
            })
            .expect("the fixture must yield an error");
        assert_eq!(error.code, super::super::ERR_CLI_AUTH);
        assert!(error.message.contains("/login"), "{}", error.message);
        // Exactly one error, even though both the assistant line and the
        // result line report it.
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, TurnEvent::Error { .. }))
                .count(),
            1
        );
        assert!(matches!(
            events.last(),
            Some(TurnEvent::Finished {
                reason: FinishReason::Other
            })
        ));
    }

    #[test]
    fn the_happy_fixture_streams_text_usage_and_a_clean_finish() {
        let text = fixture("claude-2.1.276-hello.jsonl");
        let parser = ClaudeParser::new(true);
        let events = events(&parse_all(&parser, &text));
        assert!(matches!(events.first(), Some(TurnEvent::Started { .. })));
        assert_eq!(text_of(&events), "ok");
        let usage = events
            .iter()
            .find_map(|e| match e {
                TurnEvent::Usage { usage } => Some(usage.clone()),
                _ => None,
            })
            .expect("the result line carries usage");
        // One Usage convention: input is the whole input, cache included.
        assert_eq!(usage.input_tokens, 4 + 12_040);
        assert_eq!(usage.output_tokens, 2);
        assert_eq!(usage.cache_read_tokens, 12_040);
        assert_eq!(usage.billable_tokens(), 4 + 1_204 + 2);
        assert!(usage.cost_usd.unwrap() > 0.0);
        assert!(matches!(
            events.last(),
            Some(TurnEvent::Finished {
                reason: FinishReason::Stop
            })
        ));
        assert!(!events.iter().any(|e| matches!(e, TurnEvent::Error { .. })));
    }

    #[test]
    fn a_tool_call_streams_start_delta_and_end() {
        let text = fixture("claude-2.1.276-tool-use.jsonl");
        let parser = ClaudeParser::new(true);
        let events = events(&parse_all(&parser, &text));
        let start = events.iter().find_map(|e| match e {
            TurnEvent::ToolCallStart { id, name } => Some((id.clone(), name.clone())),
            _ => None,
        });
        assert_eq!(start, Some(("toolu_01A".into(), "Read".into())));
        let deltas: String = events
            .iter()
            .filter_map(|e| match e {
                TurnEvent::ToolCallDelta {
                    id,
                    input_json_delta,
                } => {
                    assert_eq!(id, "toolu_01A", "the delta must carry the tool id");
                    Some(input_json_delta.as_str())
                }
                _ => None,
            })
            .collect();
        assert_eq!(deltas, "{\"file_path\":\"/tmp/a.txt\"}");
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, TurnEvent::ToolCallEnd { id } if id == "toolu_01A"))
                .count(),
            1,
            "a text block closing is not a tool call end"
        );
        assert!(events
            .iter()
            .any(|e| matches!(e, TurnEvent::ThinkingDelta { .. })));
    }

    #[test]
    fn rate_limit_events_feed_the_quota_and_rejected_reroutes() {
        let text = fixture("claude-2.1.276-rate-limit.jsonl");
        let parser = ClaudeParser::new(true);
        let signals = parse_all(&parser, &text);
        let snaps: Vec<RateSnapshot> = signals
            .iter()
            .filter_map(|s| match s {
                CliSignal::Rate(r) => Some(r.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(snaps.len(), 3, "allowed, allowed_warning, rejected");
        assert_eq!(snaps[0].status, RateStatus::Allowed);
        assert!((snaps[0].window_5h.unwrap().used - 0.12).abs() < 1e-6);
        assert_eq!(snaps[0].window_5h.unwrap().resets_at, Some(1_760_000_000));
        assert_eq!(snaps[1].status, RateStatus::AllowedWarning);
        assert!((snaps[1].quota_remaining().unwrap() - 0.07).abs() < 1e-5);
        assert_eq!(snaps[2].status, RateStatus::Rejected);
        assert_eq!(snaps[2].quota_remaining(), Some(0.0));

        let error = events(&signals)
            .into_iter()
            .find_map(|e| match e {
                TurnEvent::Error { error } => Some(error),
                _ => None,
            })
            .expect("rejected must produce an error");
        assert_eq!(error.code, super::super::ERR_CLI_RATE);
        assert!(error.retryable);
        assert!(super::super::super::router::is_reroutable(&error.code));
    }

    #[test]
    fn whole_message_mode_yields_the_text_once() {
        let text = fixture("claude-2.1.276-hello.jsonl");
        let parser = ClaudeParser::new(false);
        let events = events(&parse_all(&parser, &text));
        // The assistant line carries the same "ok"; the stream_event lines are
        // dropped so the text is not duplicated.
        assert_eq!(text_of(&events), "ok");
    }

    #[test]
    fn an_unknown_event_and_a_log_line_are_ignored_not_fatal() {
        let parser = ClaudeParser::new(true);
        for line in [
            r#"{"type":"something_from_2027","payload":{"a":1}}"#,
            "npm notice New version available",
            "",
            "{not json",
        ] {
            let signals = parser.parse_line(line);
            assert!(
                signals.iter().all(|s| matches!(s, CliSignal::Ignored)),
                "line must be ignored: {line}"
            );
        }
    }

    #[test]
    fn statusline_payload_normalises_0_to_100() {
        let payload = r#"{"session_id":"x","rate_limits":{"five_hour":{"used_percentage":73.5,"resets_at":1760000000},"seven_day":{"used_percentage":10,"resets_at":1760500000}}}"#;
        let snap = statusline_snapshot(payload).unwrap();
        assert!((snap.window_5h.unwrap().used - 0.735).abs() < 1e-5);
        assert_eq!(snap.window_7d.unwrap().resets_at, Some(1_760_500_000));
        assert_eq!(snap.source, QuotaSource::Real);
        assert!(statusline_snapshot("{}").is_none());
    }

    #[test]
    fn resets_at_accepts_seconds_millis_and_rfc3339() {
        assert_eq!(
            epoch_seconds(&Value::from(1_760_000_000_i64)),
            Some(1_760_000_000)
        );
        assert_eq!(
            epoch_seconds(&Value::from(1_760_000_000_000_i64)),
            Some(1_760_000_000)
        );
        assert_eq!(
            epoch_seconds(&Value::from("2025-10-09T08:53:20Z")),
            Some(1_760_000_000)
        );
        assert_eq!(epoch_seconds(&Value::Null), None);
    }
}
