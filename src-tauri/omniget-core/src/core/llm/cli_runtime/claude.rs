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

#[derive(Debug, Clone, Default)]
pub struct ClaudeArgs {
    pub model: Option<String>,
    pub system_prompt: Option<String>,
    pub max_budget_usd: Option<f64>,
    pub partial_messages: bool,
    pub resume: Option<String>,
    pub settings: Option<String>,
    pub permission_mode: Option<&'static str>,
    pub mcp_config: Option<String>,
    pub allowed_tools: Vec<String>,
    pub tools: Option<String>,
    pub disallowed_tools: Vec<String>,
    pub permission_prompt_tool: Option<String>,
    pub restricted: bool,
    pub strict_mcp: bool,
    pub exclude_dynamic: bool,
    pub effort: Option<String>,
    pub replace_system_prompt: bool,
    pub no_session_persistence: bool,
}

pub const SIDE_EFFORT: &str = "low";

pub const SIDE_ENV: &[(&str, &str)] = &[("MAX_THINKING_TOKENS", "0")];

pub const EXTERNAL_DENIED: &str = "Bash,Edit,Write,MultiEdit,NotebookEdit,Read,Glob,Grep,LS,KillShell,BashOutput,WebSearch,WebFetch,Task,Agent,TodoWrite,ExitPlanMode,SlashCommand,Skill";

pub const PROJECTLESS_TOOLS: &str = "WebSearch,WebFetch";
pub const PROJECTLESS_DENIED: &str =
    "Bash,Edit,Write,MultiEdit,NotebookEdit,Read,Glob,Grep,LS,KillShell,BashOutput";

pub const PROJECT_TOOLS: &str = "Bash,Read,Edit,Write,Glob,Grep,WebFetch,WebSearch,ToolSearch";

pub const PROJECT_BATCHING: &str = "Work in as few turns as possible: batch independent tool calls in one message (read every file you need at once), make all the edits, then run the tests once at the end. Do not re-read files you just wrote or re-run checks that already passed. Stop as soon as the result is verified.";

pub const PROJECT_EFFORT: &str = "low";

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

pub fn permission_mode(sandbox: super::accounts::SandboxMode) -> &'static str {
    match sandbox {
        super::accounts::SandboxMode::ReadOnly => "plan",
        super::accounts::SandboxMode::Write | super::accounts::SandboxMode::Full => "acceptEdits",
    }
}

pub fn permission_flags(prompt_tool: Option<&str>, mode: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    match prompt_tool {
        Some(tool) => {
            out.push("--permission-prompts".into());
            out.push("host".into());
            out.push("--permission-prompt-tool".into());
            out.push(tool.to_string());
        }
        None => {
            out.push("--permission-prompts".into());
            out.push("none".into());
        }
    }
    if let Some(mode) = mode {
        out.push("--permission-mode".into());
        out.push(mode.into());
    }
    out
}

pub fn argv(args: &ClaudeArgs) -> Vec<String> {
    let mut out: Vec<String> = vec![
        "-p".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
    ];
    out.extend(permission_flags(
        args.permission_prompt_tool.as_deref(),
        args.permission_mode,
    ));
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
            out.push(if args.replace_system_prompt {
                "--system-prompt".into()
            } else {
                "--append-system-prompt".into()
            });
            out.push(system.clone());
        }
    }
    if args.no_session_persistence {
        out.push("--no-session-persistence".into());
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

pub struct ClaudeParser {
    partial: bool,
    started: AtomicBool,
    errored: AtomicBool,
    tool_blocks: Mutex<HashMap<u64, String>>,
    expect_auto: bool,
    denial_note: String,
    explained: Mutex<std::collections::HashSet<String>>,
    baselines: Option<String>,
}

impl ClaudeParser {
    pub fn new(partial: bool) -> Self {
        Self {
            partial,
            started: AtomicBool::new(false),
            errored: AtomicBool::new(false),
            tool_blocks: Mutex::new(HashMap::new()),
            expect_auto: false,
            denial_note: "[OmniGet] Denied: the CLI refused this call under its permission mode and nobody could be asked.".into(),
            explained: Mutex::new(std::collections::HashSet::new()),
            baselines: None,
        }
    }

    pub fn with_baselines(mut self, request: String) -> Self {
        self.baselines = Some(request);
        self
    }

    pub fn with_permission(mut self, expect_auto: bool, denial_note: String) -> Self {
        self.expect_auto = expect_auto;
        self.denial_note = denial_note;
        self
    }

    fn explain(&self, id: &str) -> bool {
        self.explained
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.to_string())
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
                if self.expect_auto && value.get("subtype").and_then(Value::as_str) == Some("init")
                {
                    let got = value
                        .get("permissionMode")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown");
                    if got != "auto" {
                        out.push(CliSignal::Event(TurnEvent::TextDelta {
                            text: crate::core::llm::run_mode::auto_fallback_note(got),
                        }));
                    }
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
                for denial in value
                    .get("permission_denials")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(id) = denial.get("tool_use_id").and_then(Value::as_str) {
                        if self.explain(id) {
                            out.push(CliSignal::Event(TurnEvent::ToolResult {
                                id: id.to_string(),
                                content: self.denial_note.clone(),
                                is_error: true,
                            }));
                        }
                    }
                }
                if let Some(usage) = result_usage(&value) {
                    out.push(CliSignal::Event(TurnEvent::Usage { usage }));
                }
                let is_error = value
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let subtype = value.get("subtype").and_then(Value::as_str).unwrap_or("");
                if is_error || subtype.starts_with("error") {
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
            "user" => {
                if let (Some(request), Some(touch)) = (
                    &self.baselines,
                    value
                        .get("tool_use_result")
                        .and_then(crate::core::llm::file_changes::from_tool_use_result),
                ) {
                    crate::core::llm::file_changes::record(request, touch);
                }
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
                    let mut content = match block.get("content") {
                        Some(Value::String(text)) => text.clone(),
                        Some(Value::Array(parts)) => parts
                            .iter()
                            .filter_map(|p| p.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => String::new(),
                    };
                    let is_error = block
                        .get("is_error")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    if is_error
                        && crate::core::llm::run_mode::looks_denied(&content)
                        && self.explain(id)
                    {
                        content.push_str("\n\n");
                        content.push_str(&self.denial_note);
                    }
                    out.push(CliSignal::Event(TurnEvent::ToolResult {
                        id: id.to_string(),
                        content,
                        is_error,
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
                    _ => {}
                }
            }
            "content_block_stop" => {
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

fn result_usage(value: &Value) -> Option<Usage> {
    let usage = value.get("usage")?;
    let n = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0) as u32;
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

fn epoch_seconds(value: &Value) -> Option<i64> {
    if let Some(n) = value.as_i64() {
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
