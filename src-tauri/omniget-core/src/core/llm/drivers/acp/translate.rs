//! ACP → [`RuntimeEvent`] translation. Pure: no I/O, no clock, no ids from
//! outside except a counter, so the whole mapping is tested against real
//! recorded traffic (`fixtures/*.ndjson`).
//!
//! Mapping (ACP v1 schema, `session/update` discriminated by `sessionUpdate`):
//! - `agent_message_chunk` → `item.started{assistant_message}` once per
//!   segment + `content.delta{assistant_text}`; the segment closes with
//!   `item.completed{detail: full text}` when a tool call, a thought or the
//!   end of the turn interrupts it.
//! - `agent_thought_chunk` → same with `reasoning` / `reasoning_text`.
//! - `user_message_chunk` → `item.completed{user_message}` (echoes of the
//!   prompt we sent are dropped; replays outside a turn too).
//! - `tool_call` / `tool_call_update` → `item.started|updated|completed` with
//!   the item type from `kind` (`execute→command_execution`,
//!   `edit|delete|move→file_change`, `fetch→web_search`, MCP names →
//!   `mcp_tool_call`, the rest `dynamic_tool_call`), `data = {kind, input,
//!   output, locations, diffs[{path, oldText, newText, unifiedDiff}],
//!   terminals}`; command output text growth → `content.delta{command_output}`.
//! - `plan` → `turn.plan.updated`; `usage_update` → `thread.token-usage.updated`;
//!   `session_info_update` → `thread.metadata.updated`;
//!   `available_commands_update` / `current_mode_update` /
//!   `config_option_update` → `session.configured`; anything else → `raw`.
//! - `session/request_permission` → [`permission_ask`] (`request.opened` with
//!   the canonical request type, never `acp:other`).

use std::collections::HashMap;

use serde_json::{json, Value};

use super::diff::unified_diff;
use crate::core::llm::drivers::{
    ApprovalDecision, ApprovalOption, ContentDeltaPayload, ItemPayload, ItemStatus, ItemType,
    PlanStep, PlanUpdatedPayload, RawPayload, RequestDetail, RequestType, RuntimeEventKind,
    ThreadMetadataPayload, TokenUsage, TurnEndState, UsageUpdatedPayload, UserInputOption,
    UserInputQuestion, ValuePayload,
};

/// Text kept per tool field (input/output) inside `item.*` data.
pub const CLIP: usize = 8_000;

/// One event to emit, before the driver stamps thread/turn/ids on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Emit {
    pub kind: RuntimeEventKind,
    pub item_id: Option<String>,
    /// The agent's own id (message id, tool call id), for `providerRefs`.
    pub provider_item_id: Option<String>,
}

impl Emit {
    fn new(kind: RuntimeEventKind) -> Self {
        Self {
            kind,
            item_id: None,
            provider_item_id: None,
        }
    }

    fn item(kind: RuntimeEventKind, item_id: &str, provider: Option<&str>) -> Self {
        Self {
            kind,
            item_id: Some(item_id.to_string()),
            provider_item_id: provider.map(str::to_string),
        }
    }

    pub fn type_name(&self) -> &'static str {
        self.kind.type_name()
    }
}

#[derive(Debug, Clone)]
struct Segment {
    item_id: String,
    message_id: Option<String>,
    text: String,
}

#[derive(Debug, Clone, Default)]
struct ToolState {
    item_id: String,
    kind: String,
    title: String,
    name: Option<String>,
    status: String,
    raw_input: Value,
    raw_output: Value,
    content: Vec<Value>,
    locations: Vec<Value>,
    meta: Value,
    started: bool,
    done: bool,
    /// Command output already streamed as `content.delta`.
    streamed: String,
    last_data: String,
    last_title: String,
    last_status: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Which {
    Assistant,
    Thought,
}

/// Per-session translation state.
#[derive(Debug, Default)]
pub struct Translator {
    active: bool,
    prompt: String,
    assistant: Option<Segment>,
    thought: Option<Segment>,
    tools: HashMap<String, ToolState>,
    counter: u64,
    session_cost: Option<f64>,
    cost_at_start: Option<f64>,
    current_mode: Option<String>,
    config_options: Option<Value>,
    available_commands: Option<Value>,
    /// terminalId → toolCallId, learned from `{type: "terminal"}` contents.
    terminal_owner: HashMap<String, String>,
    /// Output of terminals whose tool call is not known yet.
    terminal_pending: HashMap<String, String>,
}

impl Translator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn current_mode(&self) -> Option<&str> {
        self.current_mode.as_deref()
    }

    pub fn set_current_mode(&mut self, mode: Option<String>) {
        self.current_mode = mode;
    }

    pub fn config_options(&self) -> Option<&Value> {
        self.config_options.as_ref()
    }

    pub fn set_config_options(&mut self, v: Option<Value>) {
        self.config_options = v.filter(|v| !v.is_null());
    }

    pub fn available_commands(&self) -> Option<&Value> {
        self.available_commands.as_ref()
    }

    fn next_id(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("{prefix}{}", self.counter)
    }

    /// A prompt was sent: chunks from now on belong to the turn.
    pub fn begin_turn(&mut self, prompt_text: &str) {
        self.active = true;
        self.prompt = prompt_text.to_string();
        self.assistant = None;
        self.thought = None;
        self.cost_at_start = self.session_cost;
        self.tools.retain(|_, t| !t.done);
    }

    /// The prompt returned: close open text segments. Tool calls still
    /// running stay known (background commands report later).
    pub fn end_turn(&mut self) -> Vec<Emit> {
        let mut out = self.close_segments();
        self.active = false;
        // Tools that never reported a final status would stay "in progress"
        // forever in the UI; the turn is over, so they are over too.
        let open: Vec<String> = self
            .tools
            .iter()
            .filter(|(_, t)| t.started && !t.done && t.kind != "execute")
            .map(|(k, _)| k.clone())
            .collect();
        for id in open {
            if let Some(t) = self.tools.get_mut(&id) {
                t.done = true;
                let mut p = payload(t);
                p.status = Some(ItemStatus::Completed);
                out.push(Emit::item(
                    RuntimeEventKind::ItemCompleted(p),
                    &t.item_id,
                    Some(&id),
                ));
            }
        }
        out
    }

    /// Cost of the turn in USD, from the cumulative `usage_update.cost`.
    pub fn turn_cost(&self) -> Option<f64> {
        match (self.session_cost, self.cost_at_start) {
            (Some(now), Some(start)) => Some((now - start).max(0.0)),
            (Some(now), None) => Some(now),
            _ => None,
        }
    }

    fn close_segment(&mut self, which: Which) -> Option<Emit> {
        let seg = match which {
            Which::Assistant => self.assistant.take(),
            Which::Thought => self.thought.take(),
        }?;
        let mut p = ItemPayload::new(match which {
            Which::Assistant => ItemType::AssistantMessage,
            Which::Thought => ItemType::Reasoning,
        });
        p.status = Some(ItemStatus::Completed);
        p.detail = Some(seg.text);
        Some(Emit::item(
            RuntimeEventKind::ItemCompleted(p),
            &seg.item_id,
            seg.message_id.as_deref(),
        ))
    }

    fn close_segments(&mut self) -> Vec<Emit> {
        [Which::Thought, Which::Assistant]
            .into_iter()
            .filter_map(|w| self.close_segment(w))
            .collect()
    }

    fn chunk(&mut self, which: Which, update: &Value) -> Vec<Emit> {
        if !self.active {
            return Vec::new();
        }
        let text = content_text(&update["content"]);
        if text.is_empty() {
            return Vec::new();
        }
        let mid = update
            .get("messageId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut out = Vec::new();
        let other = match which {
            Which::Assistant => Which::Thought,
            Which::Thought => Which::Assistant,
        };
        out.extend(self.close_segment(other));
        let slot = match which {
            Which::Assistant => &self.assistant,
            Which::Thought => &self.thought,
        };
        let switch = matches!(
            (slot, &mid),
            (Some(Segment { message_id: Some(a), .. }), Some(b)) if a != b
        );
        if switch {
            out.extend(self.close_segment(which));
        }
        let (item_type, stream_kind, prefix) = match which {
            Which::Assistant => (ItemType::AssistantMessage, StreamKindOf::Assistant, "msg-"),
            Which::Thought => (ItemType::Reasoning, StreamKindOf::Reasoning, "think-"),
        };
        let needs_open = match which {
            Which::Assistant => self.assistant.is_none(),
            Which::Thought => self.thought.is_none(),
        };
        if needs_open {
            let item_id = self.next_id(prefix);
            let mut p = ItemPayload::new(item_type);
            p.status = Some(ItemStatus::InProgress);
            out.push(Emit::item(
                RuntimeEventKind::ItemStarted(p),
                &item_id,
                mid.as_deref(),
            ));
            let seg = Segment {
                item_id,
                message_id: mid.clone(),
                text: String::new(),
            };
            match which {
                Which::Assistant => self.assistant = Some(seg),
                Which::Thought => self.thought = Some(seg),
            }
        }
        let seg = match which {
            Which::Assistant => self.assistant.as_mut(),
            Which::Thought => self.thought.as_mut(),
        }
        .expect("segment opened above");
        seg.text.push_str(&text);
        let item_id = seg.item_id.clone();
        out.push(Emit::item(
            RuntimeEventKind::ContentDelta(ContentDeltaPayload {
                stream_kind: stream_kind.kind(),
                delta: text,
                content_index: None,
                summary_index: None,
            }),
            &item_id,
            mid.as_deref(),
        ));
        out
    }

    fn user_chunk(&mut self, update: &Value) -> Vec<Emit> {
        if !self.active {
            return Vec::new();
        }
        let text = content_text(&update["content"]);
        let t = text.trim();
        if t.is_empty() || self.prompt.contains(t) {
            return Vec::new();
        }
        let item_id = self.next_id("user-");
        let mut p = ItemPayload::new(ItemType::UserMessage);
        p.status = Some(ItemStatus::Completed);
        p.detail = Some(text);
        vec![Emit::item(
            RuntimeEventKind::ItemCompleted(p),
            &item_id,
            update.get("messageId").and_then(Value::as_str),
        )]
    }

    fn tool(&mut self, update: &Value, is_new: bool) -> Vec<Emit> {
        let Some(id) = update.get("toolCallId").and_then(Value::as_str) else {
            return Vec::new();
        };
        let id = id.to_string();
        let mut out = self.close_segments();
        let known = self.tools.contains_key(&id);
        if !known {
            let item_id = if id.is_empty() {
                self.next_id("tool-")
            } else {
                id.clone()
            };
            self.tools.insert(
                id.clone(),
                ToolState {
                    item_id,
                    kind: "other".into(),
                    status: "pending".into(),
                    ..Default::default()
                },
            );
        }
        let st = self.tools.get_mut(&id).expect("inserted above");
        merge(st, update, is_new);
        // Terminals embedded in this call: map them and flush early output.
        let mut flush = Vec::new();
        for c in &st.content {
            if c.get("type").and_then(Value::as_str) == Some("terminal") {
                if let Some(tid) = c.get("terminalId").and_then(Value::as_str) {
                    self.terminal_owner.insert(tid.to_string(), id.clone());
                    if let Some(buf) = self.terminal_pending.remove(tid) {
                        flush.push(buf);
                    }
                }
            }
        }
        let st = self.tools.get_mut(&id).expect("inserted above");
        if !st.started {
            st.started = true;
            let p = payload(st);
            st.last_data = data_key(&p);
            st.last_title = st.title.clone();
            st.last_status = st.status.clone();
            out.push(Emit::item(
                RuntimeEventKind::ItemStarted(p),
                &st.item_id,
                Some(&id),
            ));
        } else if !st.done {
            let full = payload(st);
            let mut p = ItemPayload::new(full.item_type);
            let key = data_key(&full);
            let mut changed = false;
            if key != st.last_data {
                p.data = full.data.clone();
                p.detail = full.detail.clone();
                st.last_data = key;
                changed = true;
            }
            if st.title != st.last_title {
                p.title = full.title.clone();
                st.last_title = st.title.clone();
                changed = true;
            }
            if st.status != st.last_status {
                p.status = full.status;
                st.last_status = st.status.clone();
                changed = true;
            }
            if changed && !matches!(st.status.as_str(), "completed" | "failed") {
                p.tool_name = full.tool_name.clone();
                out.push(Emit::item(
                    RuntimeEventKind::ItemUpdated(p),
                    &st.item_id,
                    Some(&id),
                ));
            }
        }
        // Live command output (agents that run commands themselves send the
        // growing text as `content`).
        if st.kind == "execute" {
            let text = tool_text(&st.content);
            if text.len() > st.streamed.len() && text.starts_with(&st.streamed) {
                let delta = text[st.streamed.len()..].to_string();
                st.streamed = text;
                out.push(Emit::item(
                    RuntimeEventKind::ContentDelta(ContentDeltaPayload {
                        stream_kind: crate::core::llm::drivers::StreamKind::CommandOutput,
                        delta,
                        content_index: None,
                        summary_index: None,
                    }),
                    &st.item_id,
                    Some(&id),
                ));
            }
        }
        for buf in flush {
            out.push(Emit::item(
                RuntimeEventKind::ContentDelta(ContentDeltaPayload {
                    stream_kind: crate::core::llm::drivers::StreamKind::CommandOutput,
                    delta: buf,
                    content_index: None,
                    summary_index: None,
                }),
                &st.item_id,
                Some(&id),
            ));
        }
        if !st.done && matches!(st.status.as_str(), "completed" | "failed") {
            st.done = true;
            let p = payload(st);
            out.push(Emit::item(
                RuntimeEventKind::ItemCompleted(p),
                &st.item_id,
                Some(&id),
            ));
        }
        out
    }

    /// Output our own terminal (`terminal/create`) produced.
    pub fn on_terminal_output(&mut self, terminal_id: &str, chunk: &str) -> Vec<Emit> {
        if chunk.is_empty() {
            return Vec::new();
        }
        match self.terminal_owner.get(terminal_id) {
            Some(call) => {
                let item_id = self
                    .tools
                    .get(call)
                    .map(|t| t.item_id.clone())
                    .unwrap_or_else(|| call.clone());
                vec![Emit::item(
                    RuntimeEventKind::ContentDelta(ContentDeltaPayload {
                        stream_kind: crate::core::llm::drivers::StreamKind::CommandOutput,
                        delta: chunk.to_string(),
                        content_index: None,
                        summary_index: None,
                    }),
                    &item_id,
                    Some(call),
                )]
            }
            None => {
                let buf = self
                    .terminal_pending
                    .entry(terminal_id.to_string())
                    .or_default();
                if buf.len() < 256 * 1024 {
                    buf.push_str(chunk);
                }
                Vec::new()
            }
        }
    }

    /// Forget a released terminal.
    pub fn forget_terminal(&mut self, terminal_id: &str) {
        self.terminal_owner.remove(terminal_id);
        self.terminal_pending.remove(terminal_id);
    }

    /// One `session/update` notification's `update` object.
    pub fn on_update(&mut self, update: &Value) -> Vec<Emit> {
        if update
            .get("_meta")
            .and_then(|m| m.get("isReplay"))
            .and_then(Value::as_bool)
            == Some(true)
        {
            return Vec::new();
        }
        let kind = update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or("");
        match kind {
            "agent_message_chunk" => self.chunk(Which::Assistant, update),
            "agent_thought_chunk" => self.chunk(Which::Thought, update),
            "user_message_chunk" => self.user_chunk(update),
            "tool_call" => self.tool(update, true),
            "tool_call_update" => self.tool(update, false),
            "plan" => {
                let plan = update
                    .get("entries")
                    .and_then(Value::as_array)
                    .map(|a| a.as_slice())
                    .unwrap_or(&[])
                    .iter()
                    .map(|e| PlanStep {
                        step: e
                            .get("content")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        status: plan_status(e.get("status").and_then(Value::as_str)),
                    })
                    .collect();
                vec![Emit::new(RuntimeEventKind::PlanUpdated(
                    PlanUpdatedPayload {
                        explanation: None,
                        plan,
                    },
                ))]
            }
            "available_commands_update" => {
                let cmds = update
                    .get("availableCommands")
                    .cloned()
                    .unwrap_or(Value::Array(Vec::new()));
                self.available_commands = Some(cmds.clone());
                vec![configured(json!({ "availableCommands": cmds }))]
            }
            "current_mode_update" => {
                let mode = update
                    .get("currentModeId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                self.current_mode = mode.clone();
                vec![configured(json!({ "currentModeId": mode }))]
            }
            "config_option_update" => {
                let opts = update.get("configOptions").cloned().unwrap_or(Value::Null);
                self.set_config_options(Some(opts.clone()));
                vec![configured(json!({ "configOptions": opts }))]
            }
            "session_info_update" => {
                let name = update
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let meta = update
                    .get("updatedAt")
                    .filter(|v| !v.is_null())
                    .map(|v| json!({ "updatedAt": v }));
                if name.is_none() && meta.is_none() {
                    return Vec::new();
                }
                vec![Emit::new(RuntimeEventKind::ThreadMetadataUpdated(
                    ThreadMetadataPayload {
                        name,
                        metadata: meta,
                    },
                ))]
            }
            "usage_update" => {
                let cost = update.get("cost").filter(|c| !c.is_null());
                let usd = cost.and_then(|c| {
                    let cur = c.get("currency").and_then(Value::as_str).unwrap_or("USD");
                    cur.eq_ignore_ascii_case("USD")
                        .then(|| c.get("amount").and_then(Value::as_f64))
                        .flatten()
                });
                if usd.is_some() {
                    self.session_cost = usd;
                }
                let usage = TokenUsage {
                    used_tokens: update.get("used").and_then(Value::as_u64),
                    max_tokens: update.get("size").and_then(Value::as_u64),
                    cost_usd: usd,
                    usage_status: Some("partial".into()),
                    ..Default::default()
                };
                vec![Emit::new(RuntimeEventKind::UsageUpdated(
                    UsageUpdatedPayload { usage },
                ))]
            }
            other => vec![Emit::new(RuntimeEventKind::Raw(RawPayload {
                source: "acp.session_update".into(),
                method: Some(other.to_string()),
                payload: update.clone(),
            }))],
        }
    }
}

#[derive(Clone, Copy)]
enum StreamKindOf {
    Assistant,
    Reasoning,
}

impl StreamKindOf {
    fn kind(self) -> crate::core::llm::drivers::StreamKind {
        match self {
            StreamKindOf::Assistant => crate::core::llm::drivers::StreamKind::AssistantText,
            StreamKindOf::Reasoning => crate::core::llm::drivers::StreamKind::ReasoningText,
        }
    }
}

fn configured(value: Value) -> Emit {
    Emit::new(RuntimeEventKind::SessionConfigured(ValuePayload { value }))
}

fn plan_status(s: Option<&str>) -> String {
    match s {
        Some("in_progress") => "inProgress",
        Some("completed") => "completed",
        _ => "pending",
    }
    .to_string()
}

/// Text of one ACP `ContentBlock`.
pub fn content_text(block: &Value) -> String {
    match block.get("type").and_then(Value::as_str) {
        Some("text") => block
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        Some("image") => "[image]".into(),
        Some("audio") => "[audio]".into(),
        Some("resource_link") => format!(
            "[{}]({})",
            block.get("name").and_then(Value::as_str).unwrap_or("link"),
            block.get("uri").and_then(Value::as_str).unwrap_or("")
        ),
        Some("resource") => block
            .get("resource")
            .and_then(|r| r.get("text"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| "[resource]".into()),
        _ => block
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    }
}

fn merge(st: &mut ToolState, u: &Value, is_new: bool) {
    let s = |k: &str| u.get(k).and_then(Value::as_str).map(str::to_string);
    if let Some(v) = s("kind") {
        st.kind = v;
    }
    if let Some(v) = s("title") {
        st.title = v;
    }
    if let Some(v) = s("name") {
        st.name = Some(v);
    }
    if let Some(v) = s("status") {
        st.status = v;
    } else if is_new && st.status.is_empty() {
        st.status = "pending".into();
    }
    for (key, slot) in [
        ("rawInput", &mut st.raw_input),
        ("rawOutput", &mut st.raw_output),
    ] {
        if let Some(v) = u.get(key).filter(|v| !v.is_null()) {
            // Agents send `{}` as a placeholder before the real input.
            let empty = v.as_object().map(|o| o.is_empty()).unwrap_or(false);
            if !empty || slot.is_null() {
                *slot = v.clone();
            }
        }
    }
    // `content` and `locations` replace the previous collection (spec).
    if let Some(a) = u.get("content").and_then(Value::as_array) {
        st.content = a.clone();
    }
    if let Some(a) = u.get("locations").and_then(Value::as_array) {
        if !a.is_empty() || st.locations.is_empty() {
            st.locations = a.clone();
        }
    }
    if let Some(m) = u.get("_meta").filter(|m| !m.is_null()) {
        st.meta = m.clone();
    }
}

fn clip_str(s: &str) -> String {
    if s.len() <= CLIP {
        return s.to_string();
    }
    let mut cut = CLIP;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n[… {} more characters]", &s[..cut], s.len() - cut)
}

/// Clip every long string inside a JSON value.
pub fn clip(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(clip_str(s)),
        Value::Array(a) => Value::Array(a.iter().take(200).map(clip).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), clip(v))).collect()),
        other => other.clone(),
    }
}

fn tool_text(content: &[Value]) -> String {
    content
        .iter()
        .filter(|c| c.get("type").and_then(Value::as_str) == Some("content"))
        .map(|c| content_text(&c["content"]))
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_mcp_name(name: &str) -> bool {
    name.starts_with("mcp__") || name.starts_with("mcp_") || name.contains("__")
}

/// Item type of an ACP tool kind.
pub fn item_type(kind: &str, name: &str) -> ItemType {
    match kind {
        "execute" => ItemType::CommandExecution,
        "edit" | "delete" | "move" => ItemType::FileChange,
        "fetch" => ItemType::WebSearch,
        _ if is_mcp_name(name) => ItemType::McpToolCall,
        _ => ItemType::DynamicToolCall,
    }
}

fn command_of(input: &Value) -> Option<String> {
    let c = input.get("command").or_else(|| input.get("cmd"))?;
    match c {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => Some(
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => None,
    }
}

fn paths_of(locations: &[Value], content: &[Value], input: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut add = |p: &str| {
        if !p.is_empty() && !out.iter().any(|x| x == p) {
            out.push(p.to_string());
        }
    };
    for l in locations {
        if let Some(p) = l.get("path").and_then(Value::as_str) {
            add(p);
        }
    }
    for c in content {
        if c.get("type").and_then(Value::as_str) == Some("diff") {
            if let Some(p) = c.get("path").and_then(Value::as_str) {
                add(p);
            }
        }
    }
    for k in [
        "filePath",
        "filepath",
        "file_path",
        "path",
        "abs_path",
        "absolute_path",
    ] {
        if let Some(p) = input.get(k).and_then(Value::as_str) {
            add(p);
        }
    }
    out
}

fn diffs_of(content: &[Value]) -> Vec<Value> {
    content
        .iter()
        .filter(|c| c.get("type").and_then(Value::as_str) == Some("diff"))
        .map(|c| {
            let path = c.get("path").and_then(Value::as_str).unwrap_or("");
            let old = c.get("oldText").and_then(Value::as_str).unwrap_or("");
            let new = c.get("newText").and_then(Value::as_str).unwrap_or("");
            json!({
                "path": path,
                "oldText": clip_str(old),
                "newText": clip_str(new),
                "unifiedDiff": clip_str(&unified_diff(old, new, path)),
                "created": c.get("oldText").map(|v| v.is_null()).unwrap_or(true),
            })
        })
        .collect()
}

fn payload(st: &ToolState) -> ItemPayload {
    let tool_name = st
        .name
        .clone()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| st.kind.clone());
    let mut p = ItemPayload::new(item_type(&st.kind, &tool_name));
    p.status = Some(match st.status.as_str() {
        "completed" => ItemStatus::Completed,
        "failed" => ItemStatus::Failed,
        _ => ItemStatus::InProgress,
    });
    p.title = (!st.title.is_empty()).then(|| st.title.clone());
    p.tool_name = Some(tool_name);
    let paths = paths_of(&st.locations, &st.content, &st.raw_input);
    p.detail = match st.kind.as_str() {
        "execute" => command_of(&st.raw_input).or_else(|| p.title.clone()),
        "edit" | "delete" | "move" | "read" => paths.first().cloned().or_else(|| p.title.clone()),
        _ => p.title.clone(),
    };
    let output = if st.raw_output.is_null() {
        let t = tool_text(&st.content);
        if t.is_empty() {
            Value::Null
        } else {
            Value::String(t)
        }
    } else {
        st.raw_output.clone()
    };
    let terminals: Vec<Value> = st
        .content
        .iter()
        .filter(|c| c.get("type").and_then(Value::as_str) == Some("terminal"))
        .filter_map(|c| c.get("terminalId").cloned())
        .collect();
    let mut data = json!({
        "kind": st.kind,
        "input": clip(&st.raw_input),
        "output": clip(&output),
        "locations": st.locations,
        "paths": paths,
    });
    let diffs = diffs_of(&st.content);
    if !diffs.is_empty() {
        data["diffs"] = Value::Array(diffs);
    }
    if !terminals.is_empty() {
        data["terminals"] = Value::Array(terminals);
    }
    if !st.meta.is_null() {
        data["meta"] = clip(&st.meta);
    }
    p.data = Some(data);
    p
}

fn data_key(p: &ItemPayload) -> String {
    p.data.as_ref().map(|d| d.to_string()).unwrap_or_default()
}

// ── Permissions ─────────────────────────────────────────────────────────

/// A `session/request_permission`, normalized.
#[derive(Debug, Clone, PartialEq)]
pub struct PermissionAsk {
    pub tool_call_id: String,
    /// ACP tool kind (`edit`, `execute`, …; `other` when absent).
    pub kind: String,
    pub request_type: RequestType,
    pub detail: RequestDetail,
    pub options: Vec<ApprovalOption>,
    /// Same operation ⇒ same key; "accept for session" remembers it.
    pub memory_key: String,
}

/// Infer the kind when the agent did not send one.
fn infer_kind(call: &Value) -> String {
    if let Some(k) = call.get("kind").and_then(Value::as_str) {
        return k.to_string();
    }
    let content = call.get("content").and_then(Value::as_array);
    if content
        .map(|a| {
            a.iter()
                .any(|c| c.get("type").and_then(Value::as_str) == Some("diff"))
        })
        .unwrap_or(false)
    {
        return "edit".into();
    }
    if call
        .get("rawInput")
        .map(|i| command_of(i).is_some())
        .unwrap_or(false)
    {
        return "execute".into();
    }
    "other".into()
}

/// The canonical request type of an ACP tool kind.
pub fn request_type(kind: &str) -> RequestType {
    match kind {
        "edit" | "delete" | "move" => RequestType::FileChangeApproval,
        "execute" => RequestType::CommandExecutionApproval,
        "read" => RequestType::FileReadApproval,
        "switch_mode" => RequestType::PermissionApproval,
        _ => RequestType::DynamicToolCall,
    }
}

fn option_decision(kind: &str) -> Option<ApprovalDecision> {
    match kind {
        "allow_once" => Some(ApprovalDecision::Accept),
        "allow_always" => Some(ApprovalDecision::AcceptAlways),
        "reject_once" | "reject_always" => Some(ApprovalDecision::Decline),
        _ => None,
    }
}

/// Normalize the params of `session/request_permission`.
pub fn permission_ask(params: &Value) -> PermissionAsk {
    let call = params.get("toolCall").cloned().unwrap_or(Value::Null);
    let kind = infer_kind(&call);
    let input = call.get("rawInput").cloned().unwrap_or(Value::Null);
    let content: Vec<Value> = call
        .get("content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let locations: Vec<Value> = call
        .get("locations")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let title = call
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let name = call.get("name").and_then(Value::as_str).map(str::to_string);
    let paths = paths_of(&locations, &content, &input);
    let mut detail = RequestDetail {
        tool_name: Some(name.clone().unwrap_or_else(|| kind.clone())),
        paths: paths.clone(),
        input: (!input.is_null()).then(|| clip(&input)),
        ..Default::default()
    };
    match kind.as_str() {
        "execute" => {
            detail.command =
                command_of(&input).or_else(|| (!title.is_empty()).then(|| title.clone()));
            detail.cwd = input
                .get("cwd")
                .or_else(|| input.get("workdir"))
                .and_then(Value::as_str)
                .map(str::to_string);
            detail.reason = input
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        "edit" | "delete" | "move" => {
            let mut patch = String::new();
            for c in &content {
                if c.get("type").and_then(Value::as_str) == Some("diff") {
                    let path = c.get("path").and_then(Value::as_str).unwrap_or("");
                    let old = c.get("oldText").and_then(Value::as_str).unwrap_or("");
                    let new = c.get("newText").and_then(Value::as_str).unwrap_or("");
                    patch.push_str(&unified_diff(old, new, path));
                }
            }
            if patch.is_empty() {
                // OpenCode sends its own unified diff in `rawInput.diff`.
                if let Some(d) = input.get("diff").and_then(Value::as_str) {
                    patch = d.to_string();
                }
            }
            detail.diff = (!patch.is_empty()).then_some(patch);
        }
        _ => {}
    }
    let text = tool_text(&content);
    let preview = match (title.is_empty(), text.is_empty()) {
        (false, false) => format!("{title}\n{text}"),
        (false, true) => title.clone(),
        (true, false) => text,
        (true, true) => String::new(),
    };
    detail.preview = (!preview.is_empty()).then(|| clip_str(&preview));
    let options = params
        .get("options")
        .and_then(Value::as_array)
        .map(|a| a.as_slice())
        .unwrap_or(&[])
        .iter()
        .filter_map(|o| {
            Some(ApprovalOption {
                id: o.get("optionId")?.as_str()?.to_string(),
                label: o
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                decision: o
                    .get("kind")
                    .and_then(Value::as_str)
                    .and_then(option_decision),
            })
        })
        .collect();
    let op = match kind.as_str() {
        "execute" => detail
            .command
            .as_deref()
            .and_then(|c| c.split_whitespace().next())
            .unwrap_or("")
            .to_string(),
        "edit" | "delete" | "move" | "read" => String::new(),
        _ => name.clone().unwrap_or_else(|| title.clone()),
    };
    PermissionAsk {
        tool_call_id: call
            .get("toolCallId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        request_type: request_type(&kind),
        memory_key: format!("{kind}:{op}"),
        kind,
        detail,
        options,
    }
}

/// The `RequestPermissionOutcome` for a decision, picking the agent's own
/// option by kind (`allow_once` / `allow_always` / `reject_once` /
/// `reject_always`). "Accept for session" answers `allow_once` (the driver
/// remembers the operation), never widening to the agent's persistent
/// "always".
pub fn permission_outcome(options: &[Value], decision: ApprovalDecision) -> Value {
    let want: &[&str] = match decision {
        ApprovalDecision::Accept | ApprovalDecision::AcceptForSession => {
            &["allow_once", "allow_always"]
        }
        ApprovalDecision::AcceptAlways => &["allow_always", "allow_once"],
        ApprovalDecision::Decline => &["reject_once", "reject_always"],
        ApprovalDecision::Cancel => &[],
    };
    let pick = want.iter().find_map(|k| {
        options
            .iter()
            .find(|o| o.get("kind").and_then(Value::as_str) == Some(k))
    });
    match pick.and_then(|o| o.get("optionId")) {
        Some(id) => json!({ "outcome": { "outcome": "selected", "optionId": id } }),
        None => json!({ "outcome": { "outcome": "cancelled" } }),
    }
}

// ── Turn end and usage ──────────────────────────────────────────────────

/// `PromptResponse.stopReason` → how the turn ended.
pub fn turn_end(stop_reason: Option<&str>, interrupted: bool) -> (TurnEndState, Option<String>) {
    match stop_reason {
        Some("cancelled") => (TurnEndState::Interrupted, None),
        _ if interrupted => (TurnEndState::Interrupted, None),
        Some("refusal") => (
            TurnEndState::Failed,
            Some("The agent refused to continue.".into()),
        ),
        Some("error") => (
            TurnEndState::Failed,
            Some("The agent reported an error.".into()),
        ),
        _ => (TurnEndState::Completed, None),
    }
}

/// `PromptResponse.usage` (stabilized in 2026: `inputTokens`, `outputTokens`,
/// `totalTokens`, `cachedReadTokens`, `cachedWriteTokens`, `thoughtTokens`).
/// `inputTokens` already excludes cache reads (OpenCode: 305 + 24 + 7936 =
/// 8265 total).
pub fn prompt_usage(result: &Value) -> Option<TokenUsage> {
    let u = result.get("usage").filter(|u| u.is_object())?;
    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
    Some(TokenUsage {
        input_tokens: n("inputTokens"),
        cached_input_tokens: n("cachedReadTokens"),
        cache_write_tokens: n("cachedWriteTokens"),
        output_tokens: n("outputTokens"),
        reasoning_output_tokens: n("thoughtTokens"),
        used_tokens: u.get("totalTokens").and_then(Value::as_u64),
        usage_status: Some("complete".into()),
        ..Default::default()
    })
}

// ── Elicitation (form mode) ─────────────────────────────────────────────

/// Questions of an `elicitation/create` (form or url mode).
pub fn elicitation_questions(params: &Value) -> Vec<UserInputQuestion> {
    let message = params
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if params.get("mode").and_then(Value::as_str) == Some("url") {
        let url = params.get("url").and_then(Value::as_str).unwrap_or("");
        return vec![UserInputQuestion {
            id: "url".into(),
            header: "Open link".into(),
            question: format!("{message}\n{url}").trim().to_string(),
            options: vec![
                UserInputOption {
                    label: "Done".into(),
                    description: None,
                    value: Some("accept".into()),
                },
                UserInputOption {
                    label: "Cancel".into(),
                    description: None,
                    value: Some("cancel".into()),
                },
            ],
            allow_custom_answer: false,
            multi_select: false,
        }];
    }
    let schema = params
        .get("requestedSchema")
        .cloned()
        .unwrap_or(Value::Null);
    let props = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for (key, p) in props {
        let title = p
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(&key)
            .to_string();
        let desc = p
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string);
        let (options, multi) = enum_options(&p);
        let question = match (&desc, out.is_empty() && !message.is_empty()) {
            (Some(d), true) => format!("{message}\n{d}"),
            (Some(d), false) => d.clone(),
            (None, true) => message.clone(),
            (None, false) => title.clone(),
        };
        let custom = options.is_empty();
        out.push(UserInputQuestion {
            id: key,
            header: title,
            question,
            options,
            allow_custom_answer: custom,
            multi_select: multi,
        });
    }
    if out.is_empty() {
        out.push(UserInputQuestion {
            id: "confirm".into(),
            header: String::new(),
            question: message,
            options: vec![
                UserInputOption {
                    label: "OK".into(),
                    description: None,
                    value: Some("accept".into()),
                },
                UserInputOption {
                    label: "Cancel".into(),
                    description: None,
                    value: Some("cancel".into()),
                },
            ],
            allow_custom_answer: false,
            multi_select: false,
        });
    }
    out
}

fn enum_options(p: &Value) -> (Vec<UserInputOption>, bool) {
    let ty = p.get("type").and_then(Value::as_str).unwrap_or("");
    if ty == "boolean" {
        return (
            vec![
                UserInputOption {
                    label: "Yes".into(),
                    description: None,
                    value: Some("true".into()),
                },
                UserInputOption {
                    label: "No".into(),
                    description: None,
                    value: Some("false".into()),
                },
            ],
            false,
        );
    }
    let (src, multi) = if ty == "array" {
        (p.get("items").cloned().unwrap_or(Value::Null), true)
    } else {
        (p.clone(), false)
    };
    let mut out = Vec::new();
    if let Some(one_of) = src
        .get("oneOf")
        .or_else(|| src.get("anyOf"))
        .and_then(Value::as_array)
    {
        for o in one_of {
            if let Some(v) = o.get("const").and_then(Value::as_str) {
                out.push(UserInputOption {
                    label: o
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or(v)
                        .to_string(),
                    description: None,
                    value: Some(v.to_string()),
                });
            }
        }
    } else if let Some(vals) = src.get("enum").and_then(Value::as_array) {
        let names = src.get("enumNames").and_then(Value::as_array);
        for (i, v) in vals.iter().enumerate() {
            if let Some(v) = v.as_str() {
                out.push(UserInputOption {
                    label: names
                        .and_then(|n| n.get(i))
                        .and_then(Value::as_str)
                        .unwrap_or(v)
                        .to_string(),
                    description: None,
                    value: Some(v.to_string()),
                });
            }
        }
    }
    (out, multi)
}

/// Build the `CreateElicitationResponse` from the answers of the UI
/// (`{questionId: answer | [answers]}`). `null`/empty answers cancel.
pub fn elicitation_response(params: &Value, answers: &Value) -> Value {
    let empty = match answers {
        Value::Null => true,
        Value::Object(o) => o.is_empty(),
        _ => false,
    };
    if empty {
        return json!({ "action": "cancel" });
    }
    let first = |k: &str| -> Option<String> {
        match answers.get(k)? {
            Value::String(s) => Some(s.clone()),
            Value::Array(a) => a.first().and_then(Value::as_str).map(str::to_string),
            other => Some(other.to_string()),
        }
    };
    if params.get("mode").and_then(Value::as_str) == Some("url") {
        return json!({ "action": if first("url").as_deref() == Some("accept") { "accept" } else { "cancel" } });
    }
    let props = params
        .get("requestedSchema")
        .and_then(|s| s.get("properties"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if props.is_empty() {
        let ok = first("confirm").as_deref() != Some("cancel");
        return json!({ "action": if ok { "accept" } else { "decline" } });
    }
    let mut content = serde_json::Map::new();
    for (key, p) in props {
        let Some(raw) = answers.get(&key) else {
            continue;
        };
        let ty = p.get("type").and_then(Value::as_str).unwrap_or("string");
        let as_text = |v: &Value| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        let value = match ty {
            "boolean" => Value::Bool(matches!(
                as_text(raw).to_ascii_lowercase().as_str(),
                "true" | "yes" | "1"
            )),
            "integer" => as_text(raw)
                .trim()
                .parse::<i64>()
                .map(Value::from)
                .unwrap_or(Value::Null),
            "number" => as_text(raw)
                .trim()
                .parse::<f64>()
                .ok()
                .and_then(|f| serde_json::Number::from_f64(f).map(Value::Number))
                .unwrap_or(Value::Null),
            "array" => match raw {
                Value::Array(a) => Value::Array(a.clone()),
                other => Value::Array(vec![Value::String(as_text(other))]),
            },
            _ => Value::String(match raw {
                Value::Array(a) => a.first().map(as_text).unwrap_or_default(),
                other => as_text(other),
            }),
        };
        if !value.is_null() {
            content.insert(key, value);
        }
    }
    json!({ "action": "accept", "content": content })
}

#[cfg(test)]
mod tests;
