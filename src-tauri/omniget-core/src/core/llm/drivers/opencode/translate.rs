//! `opencode serve` SSE (`GET /event`) → [`RuntimeEvent`] translation, and
//! the SSE framing itself. Pure; tested against a recorded real turn
//! (`fixtures/serve-turn.ndjson`, OpenCode 1.18.32).
//!
//! Mapping (events are `{id, type, properties}`; only our session's pass):
//! - `message.updated` → roles/parents (turn ownership); assistant `error`
//!   → `runtime.error` (+ `auth.status` for `ProviderAuthError`).
//! - `message.part.updated` / `message.part.delta` of assistant `text` /
//!   `reasoning` parts → `item.started` + `content.delta` (only the new
//!   suffix of a snapshot) + `item.completed{detail}`; `tool` parts →
//!   `item.started|updated|completed` typed by tool name (`bash →
//!   command_execution`, `edit|write|patch|multiedit → file_change`,
//!   `webfetch|websearch|codesearch → web_search`, `task → collab_agent_tool_call`),
//!   live `bash` output → `content.delta{command_output}`; `step-finish`
//!   → token usage of the turn.
//! - `permission.asked|replied` → `request.opened|resolved`;
//!   `question.asked|replied|rejected` → `user-input.requested|resolved`;
//!   `todo.updated` → `turn.plan.updated`; `session.updated` (title) →
//!   `thread.metadata.updated`; `session.compacted` → `thread.state.changed`;
//!   `session.status{retry}` → `runtime.warning`; `session.error` →
//!   `runtime.error` (aborts are not errors); `session.idle` /
//!   `session.status{idle}` after the prompt was admitted → end of turn.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

use crate::core::llm::drivers::acp::diff::unified_diff;
use crate::core::llm::drivers::acp::translate::clip;
use crate::core::llm::drivers::{
    ApprovalDecision, ApprovalOption, AuthStatusPayload, ContentDeltaPayload, ErrorClass,
    ItemPayload, ItemStatus, ItemType, PlanStep, PlanUpdatedPayload, RequestDetail,
    RequestOpenedPayload, RequestResolvedPayload, RequestType, RuntimeErrorPayload,
    RuntimeEventKind, RuntimeWarningPayload, StreamKind, ThreadMetadataPayload, ThreadStatePayload,
    TokenUsage, UserInputOption, UserInputQuestion, UserInputRequestedPayload,
    UserInputResolvedPayload,
};

// ── SSE framing ─────────────────────────────────────────────────────────

/// Server-Sent Events parser (WHATWG framing: `data:` lines joined by `\n`,
/// dispatched on a blank line; comments and other fields ignored).
#[derive(Debug, Default)]
pub struct SseParser {
    buf: Vec<u8>,
    data: Vec<String>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed bytes; returns the `data` payload of every complete event.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(nl) = self.buf.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = self.buf.drain(..=nl).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8_lossy(&line).into_owned();
            if line.is_empty() {
                if !self.data.is_empty() {
                    out.push(self.data.join("\n"));
                    self.data.clear();
                }
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = match line.split_once(':') {
                Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                None => (line.as_str(), ""),
            };
            if field == "data" {
                self.data.push(value.to_string());
            }
        }
        out
    }
}

// ── Translation ─────────────────────────────────────────────────────────

/// One output of the translator.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    Event {
        kind: RuntimeEventKind,
        item_id: Option<String>,
        request_id: Option<String>,
        provider_item_id: Option<String>,
    },
    /// The session went idle after our prompt: the turn is over.
    TurnIdle,
    /// `session.error` during a turn (not an abort).
    TurnFailed(String),
    /// A permission OpenCode asked for (the driver may answer it itself).
    Permission {
        request_id: String,
        permission: String,
    },
}

fn ev(kind: RuntimeEventKind) -> Out {
    Out::Event {
        kind,
        item_id: None,
        request_id: None,
        provider_item_id: None,
    }
}

fn item_ev(kind: RuntimeEventKind, item_id: &str, provider: Option<&str>) -> Out {
    Out::Event {
        kind,
        item_id: Some(item_id.to_string()),
        request_id: None,
        provider_item_id: provider.map(str::to_string),
    }
}

fn req_ev(kind: RuntimeEventKind, request_id: &str, item_id: Option<&str>) -> Out {
    Out::Event {
        kind,
        item_id: item_id.map(str::to_string),
        request_id: Some(request_id.to_string()),
        provider_item_id: None,
    }
}

#[derive(Debug, Clone, Default)]
struct Part {
    item_id: String,
    kind: String,
    message_id: String,
    text: String,
    started: bool,
    done: bool,
    // tool parts
    tool: String,
    status: String,
    streamed: String,
    last_data: String,
    last_title: String,
}

/// Pending question of the session (answers are sent in question order).
#[derive(Debug, Clone)]
pub struct Question {
    pub ids: Vec<String>,
    pub texts: Vec<String>,
}

#[derive(Debug, Default)]
pub struct Translator {
    session_id: String,
    active: bool,
    admitted: bool,
    roles: HashMap<String, String>,
    parents: HashMap<String, String>,
    seen_users: HashSet<String>,
    turn_users: HashSet<String>,
    parts: HashMap<String, Part>,
    counted_steps: HashSet<String>,
    usage: TokenUsage,
    cost: f64,
    title: Option<String>,
    resolved: HashSet<String>,
    permissions: HashMap<String, RequestType>,
    pub questions: HashMap<String, Question>,
}

/// Item type of an OpenCode tool.
pub fn item_type(tool: &str) -> ItemType {
    match tool {
        "bash" | "shell" => ItemType::CommandExecution,
        "edit" | "write" | "patch" | "multiedit" | "apply_patch" => ItemType::FileChange,
        "webfetch" | "websearch" | "codesearch" => ItemType::WebSearch,
        "task" | "agent" | "subtask" => ItemType::CollabAgentToolCall,
        t if t.contains("__") || t.starts_with("mcp") => ItemType::McpToolCall,
        _ => ItemType::DynamicToolCall,
    }
}

/// Request type of an OpenCode permission.
pub fn permission_type(permission: &str) -> RequestType {
    match permission {
        "bash" => RequestType::CommandExecutionApproval,
        "edit" | "write" | "patch" => RequestType::FileChangeApproval,
        "read" | "list" | "glob" | "grep" => RequestType::FileReadApproval,
        "webfetch" | "websearch" | "codesearch" => RequestType::DynamicToolCall,
        _ => RequestType::PermissionApproval,
    }
}

/// `permission.reply` body value for a decision.
pub fn reply_for(decision: ApprovalDecision) -> &'static str {
    match decision {
        ApprovalDecision::Accept => "once",
        ApprovalDecision::AcceptForSession | ApprovalDecision::AcceptAlways => "always",
        ApprovalDecision::Decline | ApprovalDecision::Cancel => "reject",
    }
}

fn decision_of(reply: &str) -> ApprovalDecision {
    match reply {
        "once" => ApprovalDecision::Accept,
        "always" => ApprovalDecision::AcceptAlways,
        _ => ApprovalDecision::Decline,
    }
}

/// The session a raw event belongs to.
pub fn event_session(ev: &Value) -> Option<&str> {
    let p = ev.get("properties")?;
    p.get("sessionID")
        .or_else(|| p.get("part").and_then(|x| x.get("sessionID")))
        .or_else(|| p.get("info").and_then(|x| x.get("sessionID")))
        .and_then(Value::as_str)
}

fn n(v: &Value, path: &str) -> u64 {
    v.pointer(path).and_then(Value::as_f64).unwrap_or(0.0) as u64
}

impl Translator {
    pub fn new(session_id: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            ..Default::default()
        }
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Follow a new session id (fork/rollback) without losing state.
    pub fn set_session(&mut self, id: &str) {
        self.session_id = id.to_string();
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn begin_turn(&mut self) {
        self.active = true;
        self.admitted = false;
        self.turn_users.clear();
        self.counted_steps.clear();
        self.usage = TokenUsage::default();
        self.cost = 0.0;
    }

    /// Close open text items; returns them plus the turn's usage and cost.
    pub fn end_turn(&mut self) -> (Vec<Out>, TokenUsage, f64) {
        let mut out = Vec::new();
        let open: Vec<String> = self
            .parts
            .iter()
            .filter(|(_, p)| p.started && !p.done)
            .map(|(k, _)| k.clone())
            .collect();
        for id in open {
            out.extend(self.complete_part(&id, None));
        }
        self.active = false;
        let mut usage = std::mem::take(&mut self.usage);
        usage.usage_status = Some("complete".into());
        (out, usage, self.cost)
    }

    /// The user already answered this request through us.
    pub fn mark_resolved(&mut self, request_id: &str) {
        self.resolved.insert(request_id.to_string());
    }

    pub fn permission_type_of(&self, request_id: &str) -> RequestType {
        self.permissions
            .get(request_id)
            .copied()
            .unwrap_or(RequestType::PermissionApproval)
    }

    fn complete_part(&mut self, id: &str, status: Option<ItemStatus>) -> Vec<Out> {
        let Some(p) = self.parts.get_mut(id) else {
            return Vec::new();
        };
        if !p.started || p.done {
            return Vec::new();
        }
        p.done = true;
        let mut payload = ItemPayload::new(match p.kind.as_str() {
            "reasoning" => ItemType::Reasoning,
            "tool" => item_type(&p.tool),
            _ => ItemType::AssistantMessage,
        });
        payload.status = Some(status.unwrap_or(ItemStatus::Completed));
        if p.kind != "tool" {
            payload.detail = Some(p.text.clone());
        }
        vec![item_ev(
            RuntimeEventKind::ItemCompleted(payload),
            &p.item_id,
            Some(id),
        )]
    }

    fn text_part(&mut self, part: &Value) -> Vec<Out> {
        let id = part
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let kind = part
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("text")
            .to_string();
        let mid = part
            .get("messageID")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let mut out = Vec::new();
        let entry = self.parts.entry(id.clone()).or_insert_with(|| Part {
            item_id: id.clone(),
            kind: kind.clone(),
            message_id: mid,
            ..Default::default()
        });
        if entry.done {
            return out;
        }
        if !entry.started {
            entry.started = true;
            let mut p = ItemPayload::new(if kind == "reasoning" {
                ItemType::Reasoning
            } else {
                ItemType::AssistantMessage
            });
            p.status = Some(ItemStatus::InProgress);
            out.push(item_ev(
                RuntimeEventKind::ItemStarted(p),
                &entry.item_id,
                Some(&id),
            ));
        }
        let snapshot = part.get("text").and_then(Value::as_str).unwrap_or("");
        if snapshot.len() > entry.text.len() && snapshot.starts_with(&entry.text) {
            let delta = snapshot[entry.text.len()..].to_string();
            entry.text = snapshot.to_string();
            out.push(item_ev(
                RuntimeEventKind::ContentDelta(ContentDeltaPayload {
                    stream_kind: if kind == "reasoning" {
                        StreamKind::ReasoningText
                    } else {
                        StreamKind::AssistantText
                    },
                    delta,
                    content_index: None,
                    summary_index: None,
                }),
                &entry.item_id,
                Some(&id),
            ));
        } else if !snapshot.is_empty() && !snapshot.starts_with(&entry.text) {
            // Rewritten text: the final item carries the truth.
            entry.text = snapshot.to_string();
        }
        if part.pointer("/time/end").is_some() {
            out.extend(self.complete_part(&id, None));
        }
        out
    }

    fn tool_part(&mut self, part: &Value) -> Vec<Out> {
        let pid = part
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let call = part
            .get("callID")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| pid.clone());
        let tool = part
            .get("tool")
            .and_then(Value::as_str)
            .unwrap_or("tool")
            .to_string();
        let state = part.get("state").cloned().unwrap_or(Value::Null);
        let status = state
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("pending")
            .to_string();
        let mut out = Vec::new();
        let entry = self.parts.entry(pid.clone()).or_insert_with(|| Part {
            item_id: call.clone(),
            kind: "tool".into(),
            message_id: part
                .get("messageID")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            tool: tool.clone(),
            ..Default::default()
        });
        if entry.done {
            return out;
        }
        let input = state.get("input").cloned().unwrap_or(Value::Null);
        let meta = state.get("metadata").cloned().unwrap_or(Value::Null);
        let title = state
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| tool.clone());
        let output = state
            .get("output")
            .cloned()
            .or_else(|| state.get("error").cloned())
            .unwrap_or(Value::Null);
        let mut data = json!({ "tool": tool, "input": clip(&input), "output": clip(&output) });
        if let Some(d) = meta.get("diff").and_then(Value::as_str) {
            data["unifiedDiff"] = Value::String(d.to_string());
        } else if let (Some(old), Some(new)) = (
            input.get("oldString").and_then(Value::as_str),
            input.get("newString").and_then(Value::as_str),
        ) {
            let path = input.get("filePath").and_then(Value::as_str).unwrap_or("");
            data["unifiedDiff"] = Value::String(unified_diff(old, new, path));
        }
        let detail = match item_type(&tool) {
            ItemType::CommandExecution => input
                .get("command")
                .and_then(Value::as_str)
                .map(str::to_string),
            ItemType::FileChange => input
                .get("filePath")
                .or_else(|| input.get("path"))
                .and_then(Value::as_str)
                .map(str::to_string),
            _ => None,
        }
        .or_else(|| Some(title.clone()));
        let item_status = match status.as_str() {
            "completed" => ItemStatus::Completed,
            "error" => ItemStatus::Failed,
            _ => ItemStatus::InProgress,
        };
        let key = data.to_string();
        let item_id = entry.item_id.clone();
        if !entry.started {
            entry.started = true;
            let mut p = ItemPayload::new(item_type(&tool));
            p.status = Some(ItemStatus::InProgress);
            p.title = Some(title.clone());
            p.tool_name = Some(tool.clone());
            p.detail = detail.clone();
            p.data = Some(data.clone());
            entry.last_data = key.clone();
            entry.last_title = title.clone();
            out.push(item_ev(
                RuntimeEventKind::ItemStarted(p),
                &item_id,
                Some(&call),
            ));
        } else if status != "completed" && status != "error" {
            let mut p = ItemPayload::new(item_type(&tool));
            let mut changed = false;
            if key != entry.last_data {
                p.data = Some(data.clone());
                p.detail = detail.clone();
                entry.last_data = key.clone();
                changed = true;
            }
            if title != entry.last_title {
                p.title = Some(title.clone());
                entry.last_title = title.clone();
                changed = true;
            }
            if status != entry.status {
                p.status = Some(item_status);
                changed = true;
            }
            if changed {
                out.push(item_ev(
                    RuntimeEventKind::ItemUpdated(p),
                    &item_id,
                    Some(&call),
                ));
            }
        }
        entry.status = status.clone();
        if item_type(&tool) == ItemType::CommandExecution {
            let text = meta
                .get("output")
                .and_then(Value::as_str)
                .or_else(|| state.get("output").and_then(Value::as_str))
                .unwrap_or("")
                .to_string();
            if text.len() > entry.streamed.len() && text.starts_with(&entry.streamed) {
                let delta = text[entry.streamed.len()..].to_string();
                entry.streamed = text;
                out.push(item_ev(
                    RuntimeEventKind::ContentDelta(ContentDeltaPayload {
                        stream_kind: StreamKind::CommandOutput,
                        delta,
                        content_index: None,
                        summary_index: None,
                    }),
                    &item_id,
                    Some(&call),
                ));
            }
        }
        if status == "completed" || status == "error" {
            entry.done = true;
            let mut p = ItemPayload::new(item_type(&tool));
            p.status = Some(item_status);
            p.title = Some(title);
            p.tool_name = Some(tool);
            p.detail = detail;
            p.data = Some(data);
            out.push(item_ev(
                RuntimeEventKind::ItemCompleted(p),
                &item_id,
                Some(&call),
            ));
        }
        out
    }

    /// One event of `GET /event`.
    pub fn on_event(&mut self, event: &Value) -> Vec<Out> {
        let ty = event.get("type").and_then(Value::as_str).unwrap_or("");
        let p = event.get("properties").cloned().unwrap_or(Value::Null);
        let session = event_session(event).or_else(|| {
            (ty == "session.updated" || ty == "session.created")
                .then(|| p.pointer("/info/id").and_then(Value::as_str))
                .flatten()
        });
        if session != Some(self.session_id.as_str()) {
            return Vec::new();
        }
        match ty {
            "message.updated" => {
                let info = &p["info"];
                let id = info
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let role = info
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.roles.insert(id.clone(), role.clone());
                let mut out = Vec::new();
                if role == "user" {
                    if self.active && !self.seen_users.contains(&id) {
                        self.turn_users.insert(id.clone());
                        self.admitted = true;
                    }
                    self.seen_users.insert(id);
                } else if role == "assistant" {
                    if let Some(parent) = info.get("parentID").and_then(Value::as_str) {
                        self.parents.insert(id.clone(), parent.to_string());
                    }
                    if let Some(err) = info.get("error").filter(|e| !e.is_null()) {
                        if self.active {
                            let name = err.get("name").and_then(Value::as_str).unwrap_or("");
                            let msg = err
                                .pointer("/data/message")
                                .and_then(Value::as_str)
                                .unwrap_or(name)
                                .to_string();
                            if name == "ProviderAuthError" {
                                out.push(ev_auth(&msg));
                            }
                            if name != "MessageAbortedError" {
                                out.push(ev(RuntimeEventKind::RuntimeError(RuntimeErrorPayload {
                                    message: msg,
                                    class: if name == "ProviderAuthError" {
                                        ErrorClass::PermissionError
                                    } else {
                                        ErrorClass::ProviderError
                                    },
                                    code: Some(name.to_string()),
                                    detail: None,
                                })));
                            }
                        }
                    }
                }
                out
            }
            "message.part.updated" => {
                let part = &p["part"];
                let mid = part.get("messageID").and_then(Value::as_str).unwrap_or("");
                if self.roles.get(mid).map(String::as_str) != Some("assistant") || !self.active {
                    return Vec::new();
                }
                match part.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text" | "reasoning" => self.text_part(part),
                    "tool" => self.tool_part(part),
                    "step-finish" => {
                        let pid = part
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let mine = self
                            .parents
                            .get(mid)
                            .map(|par| self.turn_users.contains(par))
                            .unwrap_or(true);
                        if mine && self.counted_steps.insert(pid) {
                            self.usage.input_tokens += n(part, "/tokens/input");
                            self.usage.output_tokens += n(part, "/tokens/output");
                            self.usage.reasoning_output_tokens += n(part, "/tokens/reasoning");
                            self.usage.cached_input_tokens += n(part, "/tokens/cache/read");
                            self.usage.cache_write_tokens += n(part, "/tokens/cache/write");
                            self.usage.used_tokens = part
                                .pointer("/tokens/total")
                                .and_then(Value::as_f64)
                                .map(|t| t as u64)
                                .or(self.usage.used_tokens);
                            self.cost += part.get("cost").and_then(Value::as_f64).unwrap_or(0.0);
                            self.usage.tool_uses = Some(
                                self.usage.tool_uses.unwrap_or(0)
                                    + self
                                        .parts
                                        .values()
                                        .filter(|x| x.kind == "tool" && x.message_id == mid)
                                        .count() as u64,
                            );
                        }
                        // Text of this step is final.
                        let ids: Vec<String> = self
                            .parts
                            .iter()
                            .filter(|(_, x)| x.message_id == mid && x.kind != "tool")
                            .map(|(k, _)| k.clone())
                            .collect();
                        ids.iter()
                            .flat_map(|id| self.complete_part(id, None))
                            .collect()
                    }
                    _ => Vec::new(),
                }
            }
            "message.part.delta" => {
                if !self.active || p.get("field").and_then(Value::as_str) != Some("text") {
                    return Vec::new();
                }
                let pid = p.get("partID").and_then(Value::as_str).unwrap_or("");
                let delta = p.get("delta").and_then(Value::as_str).unwrap_or("");
                let Some(part) = self.parts.get_mut(pid) else {
                    return Vec::new();
                };
                if part.done || part.kind == "tool" || delta.is_empty() {
                    return Vec::new();
                }
                part.text.push_str(delta);
                vec![item_ev(
                    RuntimeEventKind::ContentDelta(ContentDeltaPayload {
                        stream_kind: if part.kind == "reasoning" {
                            StreamKind::ReasoningText
                        } else {
                            StreamKind::AssistantText
                        },
                        delta: delta.to_string(),
                        content_index: None,
                        summary_index: None,
                    }),
                    &part.item_id.clone(),
                    Some(pid),
                )]
            }
            "permission.asked" => {
                let id = p
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let perm = p
                    .get("permission")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let rt = permission_type(&perm);
                self.permissions.insert(id.clone(), rt);
                let meta = p.get("metadata").cloned().unwrap_or(Value::Null);
                let patterns: Vec<String> = p
                    .get("patterns")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect()
                    })
                    .unwrap_or_default();
                let mut detail = RequestDetail {
                    tool_name: Some(perm.clone()),
                    input: Some(clip(&meta)),
                    ..Default::default()
                };
                match rt {
                    RequestType::CommandExecutionApproval => {
                        detail.command = meta
                            .get("command")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .or_else(|| (!patterns.is_empty()).then(|| patterns.join(" ")));
                        detail.reason = meta
                            .get("description")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                    }
                    RequestType::FileChangeApproval => {
                        detail.diff = meta.get("diff").and_then(Value::as_str).map(str::to_string);
                        if let Some(f) = meta.get("filepath").and_then(Value::as_str) {
                            detail.paths.push(f.to_string());
                        }
                    }
                    _ => {}
                }
                if detail.paths.is_empty() && rt != RequestType::CommandExecutionApproval {
                    detail.paths = patterns.clone();
                }
                let call = p.pointer("/tool/callID").and_then(Value::as_str);
                let options = vec![
                    ApprovalOption {
                        id: "once".into(),
                        label: "Allow once".into(),
                        decision: Some(ApprovalDecision::Accept),
                    },
                    ApprovalOption {
                        id: "always".into(),
                        label: "Always allow".into(),
                        decision: Some(ApprovalDecision::AcceptAlways),
                    },
                    ApprovalOption {
                        id: "reject".into(),
                        label: "Reject".into(),
                        decision: Some(ApprovalDecision::Decline),
                    },
                ];
                vec![
                    req_ev(
                        RuntimeEventKind::RequestOpened(RequestOpenedPayload {
                            request_type: rt,
                            detail: Some(detail),
                            app_name: Some("OpenCode".into()),
                            options,
                            args: Some(
                                json!({ "permission": perm, "patterns": patterns, "always": p.get("always") }),
                            ),
                        }),
                        &id,
                        call,
                    ),
                    Out::Permission {
                        request_id: id,
                        permission: perm,
                    },
                ]
            }
            "permission.replied" => {
                let id = p
                    .get("requestID")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if self.resolved.remove(&id) {
                    return Vec::new();
                }
                let reply = p.get("reply").and_then(Value::as_str).unwrap_or("reject");
                vec![req_ev(
                    RuntimeEventKind::RequestResolved(RequestResolvedPayload {
                        request_type: self.permission_type_of(&id),
                        decision: Some(decision_of(reply)),
                        resolution: Some("answered-elsewhere".into()),
                    }),
                    &id,
                    None,
                )]
            }
            "question.asked" => {
                let id = p
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let qs = p
                    .get("questions")
                    .and_then(Value::as_array)
                    .map(|a| a.as_slice())
                    .unwrap_or(&[]);
                let mut questions = Vec::new();
                let mut ids = Vec::new();
                let mut texts = Vec::new();
                for (i, q) in qs.iter().enumerate() {
                    let qid = format!("q{i}");
                    let text = q
                        .get("question")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    ids.push(qid.clone());
                    texts.push(text.clone());
                    questions.push(UserInputQuestion {
                        id: qid,
                        header: q
                            .get("header")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        question: text,
                        options: q
                            .get("options")
                            .and_then(Value::as_array)
                            .map(|a| a.as_slice())
                            .unwrap_or(&[])
                            .iter()
                            .map(|o| UserInputOption {
                                label: o
                                    .get("label")
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                                    .to_string(),
                                description: o
                                    .get("description")
                                    .and_then(Value::as_str)
                                    .map(str::to_string),
                                value: o.get("label").and_then(Value::as_str).map(str::to_string),
                            })
                            .collect(),
                        allow_custom_answer: q
                            .get("custom")
                            .and_then(Value::as_bool)
                            .unwrap_or(true),
                        multi_select: q.get("multiple").and_then(Value::as_bool).unwrap_or(false),
                    });
                }
                self.questions.insert(id.clone(), Question { ids, texts });
                vec![req_ev(
                    RuntimeEventKind::UserInputRequested(UserInputRequestedPayload {
                        questions,
                        response_mode: None,
                    }),
                    &id,
                    p.pointer("/tool/callID").and_then(Value::as_str),
                )]
            }
            "question.replied" | "question.rejected" => {
                let id = p
                    .get("requestID")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.questions.remove(&id);
                if self.resolved.remove(&id) {
                    return Vec::new();
                }
                vec![req_ev(
                    RuntimeEventKind::UserInputResolved(UserInputResolvedPayload {
                        answers: p.get("answers").cloned().unwrap_or(Value::Null),
                    }),
                    &id,
                    None,
                )]
            }
            "todo.updated" => {
                let plan = p
                    .get("todos")
                    .and_then(Value::as_array)
                    .map(|a| a.as_slice())
                    .unwrap_or(&[])
                    .iter()
                    .map(|t| PlanStep {
                        step: t
                            .get("content")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        status: match t.get("status").and_then(Value::as_str) {
                            Some("completed") => "completed",
                            Some("in_progress") => "inProgress",
                            _ => "pending",
                        }
                        .to_string(),
                    })
                    .collect();
                vec![ev(RuntimeEventKind::PlanUpdated(PlanUpdatedPayload {
                    explanation: None,
                    plan,
                }))]
            }
            "session.updated" => {
                let title = p.pointer("/info/title").and_then(Value::as_str);
                match title {
                    Some(t)
                        if !t.is_empty()
                            && !t.starts_with("New session")
                            && !t.starts_with("Child session")
                            && !t.starts_with("OmniGet")
                            && self.title.as_deref() != Some(t) =>
                    {
                        self.title = Some(t.to_string());
                        vec![ev(RuntimeEventKind::ThreadMetadataUpdated(
                            ThreadMetadataPayload {
                                name: Some(t.to_string()),
                                metadata: None,
                            },
                        ))]
                    }
                    _ => Vec::new(),
                }
            }
            "session.compacted" => vec![ev(RuntimeEventKind::ThreadStateChanged(
                ThreadStatePayload {
                    state: "compacted".into(),
                    before_tokens: None,
                    after_tokens: None,
                    detail: None,
                },
            ))],
            "session.status" => match p.pointer("/status/type").and_then(Value::as_str) {
                Some("busy") => {
                    if self.active {
                        self.admitted = true;
                    }
                    Vec::new()
                }
                Some("retry") => vec![ev(RuntimeEventKind::RuntimeWarning(
                    RuntimeWarningPayload {
                        message: p
                            .pointer("/status/message")
                            .and_then(Value::as_str)
                            .unwrap_or("OpenCode is retrying")
                            .to_string(),
                        detail: p.pointer("/status/attempt").map(|a| format!("attempt {a}")),
                    },
                ))],
                Some("idle") if self.active && self.admitted => vec![Out::TurnIdle],
                _ => Vec::new(),
            },
            "session.idle" if self.active && self.admitted => vec![Out::TurnIdle],
            "session.error" => {
                let err = p.get("error").cloned().unwrap_or(Value::Null);
                let name = err.get("name").and_then(Value::as_str).unwrap_or("");
                if name == "MessageAbortedError" {
                    return Vec::new();
                }
                let msg = err
                    .pointer("/data/message")
                    .and_then(Value::as_str)
                    .unwrap_or(if name.is_empty() {
                        "OpenCode error"
                    } else {
                        name
                    })
                    .to_string();
                let mut out = Vec::new();
                if name == "ProviderAuthError" {
                    out.push(ev_auth(&msg));
                }
                out.push(ev(RuntimeEventKind::RuntimeError(RuntimeErrorPayload {
                    message: msg.clone(),
                    class: ErrorClass::ProviderError,
                    code: (!name.is_empty()).then(|| name.to_string()),
                    detail: None,
                })));
                if self.active {
                    out.push(Out::TurnFailed(msg));
                }
                out
            }
            _ => Vec::new(),
        }
    }
}

fn ev_auth(msg: &str) -> Out {
    ev(RuntimeEventKind::AuthStatus(AuthStatusPayload {
        is_authenticating: Some(false),
        output: vec![
            "OpenCode has no working credentials for this provider.".into(),
            "$ opencode auth login".into(),
        ],
        error: Some(msg.to_string()),
    }))
}

/// Body of `POST /question/{id}/reply` from the UI answers
/// (`{q0: "label" | ["a", "b"], …}`), in question order.
pub fn question_answers(q: &Question, answers: &Value) -> Value {
    let list: Vec<Value> = q
        .ids
        .iter()
        .zip(&q.texts)
        .map(|(id, text)| {
            let a = answers.get(id).or_else(|| answers.get(text));
            match a {
                Some(Value::Array(v)) => Value::Array(
                    v.iter()
                        .map(|x| {
                            Value::String(
                                x.as_str()
                                    .map(str::to_string)
                                    .unwrap_or_else(|| x.to_string()),
                            )
                        })
                        .collect(),
                ),
                Some(Value::String(s)) => json!([s]),
                Some(Value::Null) | None => json!([]),
                Some(other) => json!([other.to_string()]),
            }
        })
        .collect();
    json!({ "answers": list })
}

/// Session permission ruleset per access mode (last match wins in OpenCode).
pub fn ruleset(access: crate::core::llm::drivers::AccessMode) -> Value {
    use crate::core::llm::drivers::AccessMode;
    let r = |perm: &str, pattern: &str, action: &str| json!({ "permission": perm, "pattern": pattern, "action": action });
    if access == AccessMode::FullAccess {
        return json!([r("*", "*", "allow"), r("external_directory", "*", "allow")]);
    }
    let edit = if access == AccessMode::AutoAcceptEdits {
        "allow"
    } else {
        "ask"
    };
    json!([
        r("*", "*", "ask"),
        r("read", "*", "allow"),
        r("read", "*.env", "ask"),
        r("read", "*.env.*", "ask"),
        r("read", "*.env.example", "allow"),
        r("glob", "*", "allow"),
        r("grep", "*", "allow"),
        r("list", "*", "allow"),
        r("lsp", "*", "allow"),
        r("skill", "*", "allow"),
        r("todowrite", "*", "allow"),
        r("todoread", "*", "allow"),
        r("question", "*", "allow"),
        r("bash", "*", "ask"),
        r("webfetch", "*", "ask"),
        r("websearch", "*", "ask"),
        r("codesearch", "*", "ask"),
        r("external_directory", "*", "ask"),
        r("doom_loop", "*", "ask"),
        r("edit", "*", edit),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::llm::drivers::AccessMode;

    const SERVE: &str = include_str!("fixtures/serve-turn.ndjson");
    const SID: &str = "ses_f34ba6d80ffexlfsspdU6VNDwA";

    fn events() -> Vec<Value> {
        SERVE
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn kinds(out: &[Out]) -> Vec<&RuntimeEventKind> {
        out.iter()
            .filter_map(|o| match o {
                Out::Event { kind, .. } => Some(kind),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn sse_framing_joins_data_lines_and_skips_comments() {
        let mut p = SseParser::new();
        assert!(p.push(b": ping\n\ndata: {\"a\"").is_empty());
        let out = p.push(b":1}\r\n\r\ndata: x\ndata: y\n\n");
        assert_eq!(out, vec!["{\"a\":1}".to_string(), "x\ny".to_string()]);
    }

    #[test]
    fn recorded_serve_turn_translates() {
        let mut t = Translator::new(SID);
        t.begin_turn();
        let mut out = Vec::new();
        let mut idle_at = None;
        for (i, e) in events().iter().enumerate() {
            let o = t.on_event(e);
            if o.iter().any(|x| matches!(x, Out::TurnIdle)) && idle_at.is_none() {
                idle_at = Some(i);
            }
            out.extend(o);
            if idle_at.is_some() {
                break;
            }
        }
        assert!(idle_at.is_some(), "the turn ends on idle");
        let (closing, usage, cost) = t.end_turn();
        out.extend(closing);
        let ks = kinds(&out);
        // Two permissions, typed, with the diff and the command.
        let opened: Vec<_> = ks
            .iter()
            .filter_map(|k| match k {
                RuntimeEventKind::RequestOpened(p) => Some(p.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(opened.len(), 2);
        assert_eq!(opened[0].request_type, RequestType::FileChangeApproval);
        assert!(opened[0]
            .detail
            .as_ref()
            .unwrap()
            .diff
            .as_deref()
            .unwrap()
            .contains("+serve edited"));
        assert_eq!(
            opened[1].request_type,
            RequestType::CommandExecutionApproval
        );
        assert_eq!(
            opened[1].detail.as_ref().unwrap().command.as_deref(),
            Some("echo ok")
        );
        // replied elsewhere (the probe answered): resolved events.
        assert_eq!(
            ks.iter()
                .filter(|k| matches!(k, RuntimeEventKind::RequestResolved(_)))
                .count(),
            2
        );
        // Tool items: read, edit (file change with diff), bash (command).
        let completed: Vec<_> = ks
            .iter()
            .filter_map(|k| match k {
                RuntimeEventKind::ItemCompleted(p) => Some(p.clone()),
                _ => None,
            })
            .collect();
        let edit = completed
            .iter()
            .find(|p| p.tool_name.as_deref() == Some("edit"))
            .unwrap();
        assert_eq!(edit.item_type, ItemType::FileChange);
        assert!(edit.data.as_ref().unwrap()["unifiedDiff"]
            .as_str()
            .unwrap()
            .contains("+serve edited"));
        let bash = completed
            .iter()
            .find(|p| p.item_type == ItemType::CommandExecution)
            .unwrap();
        assert_eq!(bash.detail.as_deref(), Some("echo ok"));
        // Final answer text, streamed once.
        let text: String = out
            .iter()
            .filter_map(|o| match o {
                Out::Event {
                    kind: RuntimeEventKind::ContentDelta(p),
                    ..
                } if p.stream_kind == StreamKind::AssistantText => Some(p.delta.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "Edited and ran.");
        let reasoning = completed
            .iter()
            .find(|p| p.item_type == ItemType::Reasoning)
            .unwrap();
        assert!(reasoning
            .detail
            .as_deref()
            .unwrap()
            .starts_with("The user wants me to edit README.txt"));
        // Usage: three steps of this turn.
        assert_eq!(usage.input_tokens, 3816 + 209 + 218);
        assert_eq!(usage.output_tokens, 135 + 166 + 6);
        assert_eq!(usage.cached_input_tokens, 4608 + 8448 + 8704);
        assert_eq!(cost, 0.0);
        let starts = ks
            .iter()
            .filter(|k| matches!(k, RuntimeEventKind::ItemStarted(_)))
            .count();
        assert_eq!(starts, completed.len(), "every item closes");
    }

    #[test]
    fn events_of_other_sessions_are_ignored() {
        let mut t = Translator::new("ses_other");
        t.begin_turn();
        let n: usize = events().iter().map(|e| t.on_event(e).len()).sum();
        assert_eq!(n, 0);
    }

    #[test]
    fn replies_rulesets_and_answers() {
        assert_eq!(reply_for(ApprovalDecision::Accept), "once");
        assert_eq!(reply_for(ApprovalDecision::AcceptForSession), "always");
        assert_eq!(reply_for(ApprovalDecision::Cancel), "reject");
        let full = ruleset(AccessMode::FullAccess);
        assert_eq!(full[0]["action"], "allow");
        let edits = ruleset(AccessMode::AutoAcceptEdits);
        assert_eq!(edits.as_array().unwrap().last().unwrap()["action"], "allow");
        let strict = ruleset(AccessMode::ApprovalRequired);
        assert_eq!(strict.as_array().unwrap().last().unwrap()["action"], "ask");
        let q = Question {
            ids: vec!["q0".into(), "q1".into()],
            texts: vec!["Color?".into(), "Size?".into()],
        };
        assert_eq!(
            question_answers(&q, &json!({ "q0": "red", "q1": ["S", "M"] })),
            json!({ "answers": [["red"], ["S", "M"]] })
        );
        assert_eq!(
            permission_type("external_directory"),
            RequestType::PermissionApproval
        );
        assert_eq!(item_type("task"), ItemType::CollabAgentToolCall);
    }
}
