//! The `native` driver: the existing `Coordinator` (API models, local models,
//! the code harness, and the roster's CLI/ACP agents) behind the `Driver`
//! trait. It translates `TurnEvent` + `BusEvent` into `RuntimeEvent`s.
//!
//! Durable approvals: the thread id is the coordinator's conversation id and
//! is marked with `drivers::set_durable_conversation`, so the broker's `Ask`
//! waits for the answer without its 120 s timeout. The ask becomes a
//! persisted `request.opened` (request id = the broker's tool call id);
//! `respond_request` answers the broker. Jobs and the old chat never mark
//! their conversations and keep the timeout.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use futures::StreamExt;
use omniget_core::core::llm::code_tools;
use omniget_core::core::llm::coordinator;
use omniget_core::core::llm::drivers::*;
use omniget_core::core::llm::types::{ContentPart, FinishReason, ModelRef, ProviderId, TurnEvent};
use omniget_core::core::omni::bus::BusEvent;
use omniget_core::core::threads::decider::{item_type_of_tool, request_type_of_tool};
use serde_json::{json, Value};
use tauri::AppHandle;

use crate::llm_manager::{sanitize_id, LlmManager};

pub const NATIVE: &str = "native";
const OUTPUT_MAX: usize = 16_000;

fn clip(s: &str) -> String {
    if s.len() <= OUTPUT_MAX {
        return s.to_string();
    }
    let mut end = OUTPUT_MAX;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n[clipped: {} of {} bytes]", &s[..end], end, s.len())
}

pub fn capabilities() -> DriverCapabilities {
    DriverCapabilities {
        rollback: true,
        fork: true,
        interrupt: true,
        approvals: true,
        user_input: false,
        model_switch: true,
        plan_mode: false,
        compaction: false,
        continuation: false,
        steer: false,
        rate_limits: false,
    }
}

#[derive(Default)]
struct ThreadSlot {
    agent_id: Option<String>,
    access: AccessMode,
    /// Tools accepted "for this session".
    session_allow: HashSet<String>,
    turn: Option<ActiveTurn>,
}

#[derive(Clone)]
struct ActiveTurn {
    turn_id: String,
    request_id: String,
    /// tool_call_id → tool name, asks still waiting.
    open_asks: HashMap<String, String>,
}

pub struct NativeDriver {
    instance: DriverInstance,
    sink: RuntimeSink,
    llm: Arc<LlmManager>,
    app: AppHandle,
    threads: Arc<Mutex<HashMap<String, ThreadSlot>>>,
}

impl NativeDriver {
    pub fn new(
        instance: DriverInstance,
        sink: RuntimeSink,
        llm: Arc<LlmManager>,
        app: AppHandle,
    ) -> Self {
        Self {
            instance,
            sink,
            llm,
            app,
            threads: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn emit(&self, thread_id: &str, turn_id: Option<&str>, kind: RuntimeEventKind) {
        let _ = self.sink.send(RuntimeEvent::new(
            NATIVE,
            &self.instance.id,
            thread_id,
            turn_id,
            kind,
        ));
    }

    fn slots(&self) -> std::sync::MutexGuard<'_, HashMap<String, ThreadSlot>> {
        self.threads.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn pick_agent(&self, wanted: Option<&str>) -> Option<String> {
        if let Some(id) = wanted.filter(|s| !s.is_empty()) {
            if self.llm.agent(id).is_some() {
                return Some(id.to_string());
            }
        }
        self.llm.roster().first().map(|a| a.id.clone())
    }

    /// Index of the message that opens turn `turn_count + 1`, i.e. how many
    /// records to keep so the first `turn_count` turns remain.
    fn keep_for(&self, conv: &str, turn_count: u32) -> Option<(PathBuf, usize)> {
        let dir = self.llm.coordinator().dir()?.to_path_buf();
        let messages = coordinator::load_conversation(&dir, conv).ok()?;
        let mut users = 0u32;
        for (i, m) in messages.iter().enumerate() {
            if m.role == omniget_core::core::llm::types::Role::User {
                if users == turn_count {
                    return Some((dir, i));
                }
                users += 1;
            }
        }
        Some((dir, messages.len()))
    }
}

/// `provider/model` (or `provider:model`) → `ModelRef`.
fn parse_model(s: &str) -> Option<ModelRef> {
    let s = s.trim();
    let (p, m) = s.split_once('/').or_else(|| s.split_once(':'))?;
    if p.is_empty() || m.is_empty() {
        return None;
    }
    Some(ModelRef {
        provider: ProviderId::new(p),
        model: m.to_string(),
    })
}

fn error_class(code: &str) -> ErrorClass {
    if code.starts_with("ERR_TOOL_DENIED") || code.contains("PERMISSION") {
        ErrorClass::PermissionError
    } else if code.contains("NETWORK") || code.contains("TIMEOUT") || code.contains("HTTP") {
        ErrorClass::TransportError
    } else if code.contains("PARSE") || code.contains("INVALID") {
        ErrorClass::ValidationError
    } else if code.starts_with("ERR_LLM") {
        ErrorClass::ProviderError
    } else {
        ErrorClass::Unknown
    }
}

/// What the approval card shows, from the tool name and its input.
fn request_detail(tool: &str, input: &Value, preview: &str) -> RequestDetail {
    let mut d = RequestDetail {
        tool_name: Some(tool.to_string()),
        preview: (!preview.is_empty()).then(|| preview.to_string()),
        input: (!input.is_null()).then(|| input.clone()),
        ..Default::default()
    };
    if let Some(c) = input.get("command").and_then(Value::as_str) {
        d.command = Some(c.to_string());
    }
    if let Some(c) = input.get("cwd").and_then(Value::as_str) {
        d.cwd = Some(c.to_string());
    }
    for key in ["path", "file", "file_path"] {
        if let Some(p) = input.get(key).and_then(Value::as_str) {
            d.paths.push(p.to_string());
        }
    }
    if let Some(list) = input.get("paths").and_then(Value::as_array) {
        d.paths
            .extend(list.iter().filter_map(|v| v.as_str().map(str::to_string)));
    }
    if item_type_of_tool(tool) == ItemType::FileChange {
        d.diff = input
            .get("patch")
            .or_else(|| input.get("diff"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| (!preview.is_empty()).then(|| preview.to_string()));
    }
    d
}

struct TurnCtx {
    sink: RuntimeSink,
    instance_id: String,
    thread_id: String,
    conv: String,
    turn_id: String,
    request_id: String,
    agent_id: String,
    llm: Arc<LlmManager>,
    threads: Arc<Mutex<HashMap<String, ThreadSlot>>>,
    /// tool id → (name, input json being assembled)
    calls: HashMap<String, (String, String)>,
    /// Tool calls whose result is in the JSONL once the round ends.
    awaiting: Vec<String>,
    segment: u32,
    text_open: bool,
    last_usage: Option<omniget_core::core::llm::types::Usage>,
    last_error: Option<String>,
    started: Instant,
    model: String,
}

impl TurnCtx {
    fn emit(&self, kind: RuntimeEventKind) -> RuntimeEvent {
        RuntimeEvent::new(
            NATIVE,
            &self.instance_id,
            &self.thread_id,
            Some(&self.turn_id),
            kind,
        )
    }

    fn send(&self, ev: RuntimeEvent) {
        let _ = self.sink.send(ev);
    }

    fn item(&self, id: &str, name: &str, status: ItemStatus, data: Option<Value>) -> ItemPayload {
        let _ = id;
        let mut p = ItemPayload::new(item_type_of_tool(name));
        p.status = Some(status);
        p.title = Some(name.to_string());
        p.tool_name = Some(name.to_string());
        p.data = data;
        p.agent_id = Some(self.agent_id.clone());
        p
    }

    fn input_of(&self, id: &str) -> Value {
        self.calls
            .get(id)
            .and_then(|(_, raw)| serde_json::from_str(raw).ok())
            .unwrap_or(Value::Null)
    }

    /// The coordinator persisted the results of the last tool round before it
    /// asked the model again; read them back from the JSONL.
    fn resolve_results(&mut self, finishing: bool) {
        if self.awaiting.is_empty() {
            return;
        }
        let dir = self.llm.coordinator().dir().map(|d| d.to_path_buf());
        let history = dir
            .as_deref()
            .and_then(|d| coordinator::load_conversation(d, &self.conv).ok())
            .unwrap_or_default();
        let mut results: HashMap<String, (String, bool)> = HashMap::new();
        for m in history.iter().rev().take(64) {
            for p in &m.parts {
                if let ContentPart::ToolResult {
                    tool_use_id,
                    content,
                    is_error,
                } = p
                {
                    results
                        .entry(tool_use_id.clone())
                        .or_insert((content.clone(), *is_error));
                }
            }
        }
        let pending = std::mem::take(&mut self.awaiting);
        for id in pending {
            let name = self.calls.get(&id).map(|c| c.0.clone()).unwrap_or_default();
            let input = self.input_of(&id);
            match results.get(&id) {
                Some((out, failed)) => {
                    let status = if *failed {
                        ItemStatus::Failed
                    } else {
                        ItemStatus::Completed
                    };
                    let p = self.item(
                        &id,
                        &name,
                        status,
                        Some(json!({ "input": input, "output": clip(out) })),
                    );
                    self.send(
                        self.emit(RuntimeEventKind::ItemCompleted(p))
                            .with_item(id.clone()),
                    );
                }
                None if finishing => {
                    let mut p = self.item(
                        &id,
                        &name,
                        ItemStatus::Failed,
                        Some(json!({ "input": input })),
                    );
                    p.detail = Some("no result".into());
                    self.send(
                        self.emit(RuntimeEventKind::ItemCompleted(p))
                            .with_item(id.clone()),
                    );
                }
                None => self.awaiting.push(id),
            }
        }
    }

    fn on_turn_event(&mut self, event: TurnEvent) -> bool {
        match event {
            TurnEvent::Started { .. } | TurnEvent::PruneReceipt { .. } => {}
            TurnEvent::TextDelta { text } => {
                self.resolve_results(false);
                if !self.text_open {
                    self.segment += 1;
                    self.text_open = true;
                }
                let ev = self
                    .emit(RuntimeEventKind::ContentDelta(ContentDeltaPayload {
                        stream_kind: StreamKind::AssistantText,
                        delta: text,
                        content_index: None,
                        summary_index: None,
                    }))
                    .with_item(format!("text{}", self.segment));
                self.send(ev);
            }
            TurnEvent::ThinkingDelta { text } => {
                self.resolve_results(false);
                let ev = self
                    .emit(RuntimeEventKind::ContentDelta(ContentDeltaPayload {
                        stream_kind: StreamKind::ReasoningText,
                        delta: text,
                        content_index: None,
                        summary_index: None,
                    }))
                    .with_item(format!("reasoning{}", self.segment + 1));
                self.send(ev);
            }
            TurnEvent::ToolCallStart { id, name } => {
                self.resolve_results(false);
                self.text_open = false;
                self.calls.insert(id.clone(), (name.clone(), String::new()));
                let p = self.item(&id, &name, ItemStatus::InProgress, None);
                self.send(self.emit(RuntimeEventKind::ItemStarted(p)).with_item(id));
            }
            TurnEvent::ToolCallDelta {
                id,
                input_json_delta,
            } => {
                if let Some(c) = self.calls.get_mut(&id) {
                    c.1.push_str(&input_json_delta);
                }
            }
            TurnEvent::ToolCallEnd { id } => {
                let name = self.calls.get(&id).map(|c| c.0.clone()).unwrap_or_default();
                let input = self.input_of(&id);
                let p = self.item(
                    &id,
                    &name,
                    ItemStatus::InProgress,
                    Some(json!({ "input": input })),
                );
                self.send(
                    self.emit(RuntimeEventKind::ItemUpdated(p))
                        .with_item(id.clone()),
                );
                self.awaiting.push(id);
            }
            TurnEvent::ToolResult {
                id,
                content,
                is_error,
            } => {
                // A CLI/ACP agent ran its own tool: the result is in the stream.
                self.awaiting.retain(|a| a != &id);
                let name = self.calls.get(&id).map(|c| c.0.clone()).unwrap_or_default();
                let input = self.input_of(&id);
                let status = if is_error {
                    ItemStatus::Failed
                } else {
                    ItemStatus::Completed
                };
                let p = self.item(
                    &id,
                    &name,
                    status,
                    Some(json!({ "input": input, "output": clip(&content) })),
                );
                self.send(self.emit(RuntimeEventKind::ItemCompleted(p)).with_item(id));
            }
            TurnEvent::Usage { usage } => {
                self.last_usage = Some(usage);
            }
            TurnEvent::Error { error } => {
                self.last_error = Some(format!("{}: {}", error.code, error.message));
                self.send(
                    self.emit(RuntimeEventKind::RuntimeError(RuntimeErrorPayload {
                        message: error.message.clone(),
                        class: error_class(&error.code),
                        code: Some(error.code.to_string()),
                        detail: None,
                    })),
                );
            }
            TurnEvent::Finished { reason } => {
                self.resolve_results(true);
                let state = match reason {
                    FinishReason::Cancelled => TurnEndState::Interrupted,
                    _ if self.last_error.is_some() => TurnEndState::Failed,
                    _ => TurnEndState::Completed,
                };
                let usage = self.last_usage.as_ref().map(|u| TokenUsage {
                    input_tokens: u.input_tokens as u64,
                    cached_input_tokens: u.cache_read_tokens as u64,
                    cache_write_tokens: u.cache_write_tokens as u64,
                    output_tokens: u.output_tokens as u64,
                    cost_usd: u.cost_usd,
                    model: (!self.model.is_empty()).then(|| self.model.clone()),
                    duration_ms: Some(self.started.elapsed().as_millis() as u64),
                    usage_status: Some("complete".into()),
                    ..Default::default()
                });
                let total = usage.as_ref().and_then(|u| u.cost_usd);
                self.send(
                    self.emit(RuntimeEventKind::TurnCompleted(TurnCompletedPayload {
                        state,
                        stop_reason: Some(format!("{reason:?}").to_ascii_lowercase()),
                        usage,
                        total_cost_usd: total,
                        error_message: self.last_error.clone(),
                    })),
                );
                return true;
            }
        }
        false
    }

    /// Asks and reroutes from the bus that belong to this turn.
    fn on_bus(&mut self, event: BusEvent) {
        match event {
            BusEvent::ToolAsk {
                request_id,
                tool_call_id,
                tool,
                preview,
                ..
            } if request_id == self.request_id => {
                let auto = {
                    let slots = self.threads.lock().unwrap_or_else(|e| e.into_inner());
                    let slot = slots.get(&self.thread_id);
                    let access = slot.map(|s| s.access).unwrap_or_default();
                    let allowed = slot
                        .map(|s| s.session_allow.contains(&tool))
                        .unwrap_or(false);
                    allowed
                        || access == AccessMode::FullAccess
                        || (matches!(access, AccessMode::AutoAcceptEdits | AccessMode::Auto)
                            && item_type_of_tool(&tool) == ItemType::FileChange)
                };
                if auto {
                    let _ = self
                        .llm
                        .answer_tool(&self.request_id, &tool_call_id, true, false);
                    return;
                }
                {
                    let mut slots = self.threads.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(turn) = slots.get_mut(&self.thread_id).and_then(|s| s.turn.as_mut())
                    {
                        turn.open_asks.insert(tool_call_id.clone(), tool.clone());
                    }
                }
                let input = self.input_of(&tool_call_id);
                let request_type = if tool == "external_directory" {
                    RequestType::PermissionApproval
                } else {
                    request_type_of_tool(&tool)
                };
                let options = vec![
                    ApprovalOption {
                        id: "accept".into(),
                        label: "Allow once".into(),
                        decision: Some(ApprovalDecision::Accept),
                    },
                    ApprovalOption {
                        id: "acceptForSession".into(),
                        label: "Allow for this thread".into(),
                        decision: Some(ApprovalDecision::AcceptForSession),
                    },
                    ApprovalOption {
                        id: "acceptAlways".into(),
                        label: "Always allow".into(),
                        decision: Some(ApprovalDecision::AcceptAlways),
                    },
                    ApprovalOption {
                        id: "decline".into(),
                        label: "Deny".into(),
                        decision: Some(ApprovalDecision::Decline),
                    },
                ];
                let ev = self
                    .emit(RuntimeEventKind::RequestOpened(RequestOpenedPayload {
                        request_type,
                        detail: Some(request_detail(&tool, &input, &preview)),
                        app_name: None,
                        options,
                        args: None,
                    }))
                    .with_request(tool_call_id.clone())
                    .with_item(tool_call_id);
                self.send(ev);
            }
            BusEvent::ToolCalled {
                agent, tool, ok, ..
            } if agent == self.agent_id => {
                // The call ran (or was refused): if its ask was answered
                // somewhere else (the pet, the old chat), close it here.
                let closed: Option<String> = {
                    let mut slots = self.threads.lock().unwrap_or_else(|e| e.into_inner());
                    slots
                        .get_mut(&self.thread_id)
                        .and_then(|s| s.turn.as_mut())
                        .and_then(|t| {
                            let id = t
                                .open_asks
                                .iter()
                                .find(|(_, name)| **name == tool)
                                .map(|(id, _)| id.clone())?;
                            t.open_asks.remove(&id);
                            Some(id)
                        })
                };
                if let Some(id) = closed {
                    let ev = self
                        .emit(RuntimeEventKind::RequestResolved(RequestResolvedPayload {
                            request_type: request_type_of_tool(&tool),
                            decision: Some(if ok {
                                ApprovalDecision::Accept
                            } else {
                                ApprovalDecision::Decline
                            }),
                            resolution: Some("answered-elsewhere".into()),
                        }))
                        .with_request(id);
                    self.send(ev);
                }
            }
            BusEvent::Rerouted {
                agent,
                from,
                to,
                why,
                ..
            } if agent == self.agent_id => {
                self.send(
                    self.emit(RuntimeEventKind::ModelRerouted(ModelReroutedPayload {
                        from_model: from,
                        to_model: to.clone(),
                        reason: why,
                    })),
                );
                self.model = to;
            }
            _ => {}
        }
    }
}

#[async_trait]
impl Driver for NativeDriver {
    fn kind(&self) -> &str {
        NATIVE
    }

    fn capabilities(&self) -> DriverCapabilities {
        capabilities()
    }

    async fn start_session(&self, input: SessionStart) -> Result<(), DriverError> {
        crate::commands::llm::ensure_wired(&self.app);
        let conv = sanitize_id(&input.thread_id);
        set_durable_conversation(&conv, true);
        if let Some(cwd) = input.cwd.clone().filter(|p| p.is_dir()) {
            if let Err(e) = code_tools::set_conversation_workspace(&conv, Some(cwd)) {
                tracing::warn!("[threads] workspace of {conv}: {e}");
            }
        }
        if let Some(m) = input.model.as_deref().and_then(parse_model) {
            let _ = self.llm.switch_model(&conv, m);
        }
        let fresh = {
            let mut slots = self.slots();
            let slot = slots.entry(input.thread_id.clone()).or_default();
            let fresh = slot.agent_id.is_none();
            slot.agent_id = self.pick_agent(input.agent_id.as_deref());
            slot.access = input.access_mode;
            fresh
        };
        if fresh {
            self.emit(
                &input.thread_id,
                None,
                RuntimeEventKind::SessionStarted(SessionStartedPayload {
                    message: None,
                    resume: Some(json!(conv)),
                }),
            );
        }
        Ok(())
    }

    async fn start_turn(&self, input: TurnStart) -> Result<TurnStartResult, DriverError> {
        let conv = sanitize_id(&input.thread_id);
        let agent_id = {
            let slots = self.slots();
            let slot = slots.get(&input.thread_id);
            if slot.and_then(|s| s.turn.as_ref()).is_some() {
                return Err(DriverError::new(
                    ERR_DRIVER_FAILED,
                    "a turn is already running on this thread",
                ));
            }
            slot.and_then(|s| s.agent_id.clone())
        }
        .or_else(|| self.pick_agent(None))
        .ok_or_else(|| {
            DriverError::new(
                ERR_DRIVER_FAILED,
                "ERR_NO_AGENT: create an agent in /llm first",
            )
        })?;
        if let Some(m) = input.model.as_deref().and_then(parse_model) {
            let _ = self.llm.switch_model(&conv, m);
        }
        // Subscribed before the turn exists, so no ask can slip by.
        let mut bus = self.llm.bus().subscribe();
        let (request_id, _cancel, mut stream) = self
            .llm
            .turn_stream(&conv, &agent_id, &input.text)
            .await
            .map_err(|e| DriverError::new(ERR_DRIVER_FAILED, e))?;
        {
            let mut slots = self.slots();
            let slot = slots.entry(input.thread_id.clone()).or_default();
            slot.agent_id = Some(agent_id.clone());
            slot.turn = Some(ActiveTurn {
                turn_id: input.turn_id.clone(),
                request_id: request_id.clone(),
                open_asks: HashMap::new(),
            });
        }
        crate::commands::llm::spawn_telemetry_ticker(self.app.clone());
        // The conversation's override (the thread's model) wins over the
        // agent's default, same as the coordinator picks it.
        let model = self
            .llm
            .model_override(&conv)
            .map(|m| format!("{}/{}", m.provider.as_str(), m.model))
            .unwrap_or_else(|| self.llm.model_label(&agent_id));
        let mut ctx = TurnCtx {
            sink: self.sink.clone(),
            instance_id: self.instance.id.clone(),
            thread_id: input.thread_id.clone(),
            conv,
            turn_id: input.turn_id.clone(),
            request_id: request_id.clone(),
            agent_id: agent_id.clone(),
            llm: self.llm.clone(),
            threads: self.threads.clone(),
            calls: HashMap::new(),
            awaiting: Vec::new(),
            segment: 0,
            text_open: false,
            last_usage: None,
            last_error: None,
            started: Instant::now(),
            model: model.clone(),
        };
        ctx.send(ctx.emit(RuntimeEventKind::TurnStarted(TurnStartedPayload {
            model: (!model.is_empty()).then(|| model.clone()),
            effort: None,
        })));
        let llm = self.llm.clone();
        let threads = self.threads.clone();
        let thread_id = input.thread_id.clone();
        tauri::async_runtime::spawn(async move {
            let mut finished = false;
            loop {
                tokio::select! {
                    next = stream.next() => match next {
                        Some(event) => {
                            llm.note_event(&ctx.agent_id, &event);
                            if ctx.on_turn_event(event) {
                                finished = true;
                            }
                        }
                        None => break,
                    },
                    bus_event = bus.recv() => match bus_event {
                        Ok(ev) => ctx.on_bus(ev),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {}
                    },
                }
            }
            if !finished {
                ctx.on_turn_event(TurnEvent::Finished {
                    reason: FinishReason::Other,
                });
            }
            llm.finish_turn(&ctx.request_id, &ctx.agent_id);
            let mut slots = threads.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(slot) = slots.get_mut(&thread_id) {
                if slot.turn.as_ref().map(|t| t.request_id == ctx.request_id) == Some(true) {
                    slot.turn = None;
                }
            }
        });
        Ok(TurnStartResult {
            resume_cursor: Some(json!(sanitize_id(&input.thread_id))),
        })
    }

    async fn interrupt(&self, thread_id: &str, turn_id: Option<&str>) -> Result<(), DriverError> {
        let turn = self.slots().get(thread_id).and_then(|s| s.turn.clone());
        match turn {
            Some(t) => {
                for id in t.open_asks.keys() {
                    self.llm.broker().answer(id, false);
                }
                let _ = self.llm.cancel(&t.request_id);
                Ok(())
            }
            None => {
                // Nothing runs here (the app restarted mid-turn): close the
                // turn so the thread is not stuck "running".
                if let Some(turn_id) = turn_id {
                    self.emit(
                        thread_id,
                        Some(turn_id),
                        RuntimeEventKind::TurnAborted(TurnAbortedPayload {
                            reason: "no running turn".into(),
                            usage: None,
                        }),
                    );
                }
                Ok(())
            }
        }
    }

    async fn respond_request(
        &self,
        thread_id: &str,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), DriverError> {
        let found = {
            let mut slots = self.slots();
            slots.get_mut(thread_id).and_then(|slot| {
                let turn = slot.turn.as_mut()?;
                let tool = turn.open_asks.remove(request_id)?;
                if decision == ApprovalDecision::AcceptForSession {
                    slot.session_allow.insert(tool.clone());
                }
                Some((turn.request_id.clone(), turn.turn_id.clone(), tool))
            })
        };
        let Some((turn_request, turn_id, tool)) = found else {
            return Err(DriverError::new(
                ERR_DRIVER_NO_SESSION,
                format!("no pending ask {request_id}"),
            ));
        };
        let answered = self.llm.answer_tool(
            &turn_request,
            request_id,
            decision.allows(),
            decision == ApprovalDecision::AcceptAlways,
        );
        let ev = RuntimeEvent::new(
            NATIVE,
            &self.instance.id,
            thread_id,
            Some(&turn_id),
            RuntimeEventKind::RequestResolved(RequestResolvedPayload {
                request_type: request_type_of_tool(&tool),
                decision: Some(decision),
                resolution: answered.as_ref().err().map(|e| format!("stale: {e}")),
            }),
        )
        .with_request(request_id);
        let _ = self.sink.send(ev);
        if decision == ApprovalDecision::Cancel {
            let _ = self.llm.cancel(&turn_request);
        }
        Ok(())
    }

    async fn respond_user_input(
        &self,
        _thread_id: &str,
        _request_id: &str,
        _answers: Value,
    ) -> Result<(), DriverError> {
        Err(DriverError::unsupported("user input"))
    }

    async fn rollback(&self, thread_id: &str, turn_count: u32) -> Result<(), DriverError> {
        let conv = sanitize_id(thread_id);
        let Some((dir, keep)) = self.keep_for(&conv, turn_count) else {
            return Ok(());
        };
        coordinator::truncate_conversation(&dir, &conv, keep)
            .map(|_| ())
            .map_err(|e| DriverError::new(ERR_DRIVER_FAILED, format!("{}: {}", e.code, e.message)))
    }

    async fn fork(
        &self,
        source_thread_id: &str,
        target_thread_id: &str,
        turn_count: u32,
    ) -> Result<(), DriverError> {
        let src = sanitize_id(source_thread_id);
        let dst = sanitize_id(target_thread_id);
        let Some((dir, keep)) = self.keep_for(&src, turn_count) else {
            return Ok(());
        };
        let messages = coordinator::load_conversation(&dir, &src)
            .map_err(|e| DriverError::new(ERR_DRIVER_FAILED, e.message))?;
        for m in messages.iter().take(keep) {
            coordinator::append_message(&dir, &dst, None, m);
        }
        if let Some(ws) = code_tools::workspace_of(&src) {
            let _ = code_tools::set_conversation_workspace(&dst, Some(ws));
        }
        if let Some(m) = self.llm.model_override(&src) {
            let _ = self.llm.switch_model(&dst, m);
        }
        Ok(())
    }

    async fn set_access_mode(&self, thread_id: &str, mode: AccessMode) -> Result<(), DriverError> {
        self.slots()
            .entry(thread_id.to_string())
            .or_default()
            .access = mode;
        Ok(())
    }

    async fn stop(&self, thread_id: &str) -> Result<(), DriverError> {
        let turn = self.slots().remove(thread_id).and_then(|s| s.turn);
        if let Some(t) = turn {
            for id in t.open_asks.keys() {
                self.llm.broker().answer(id, false);
            }
            let _ = self.llm.cancel(&t.request_id);
        }
        set_durable_conversation(&sanitize_id(thread_id), false);
        self.emit(
            thread_id,
            None,
            RuntimeEventKind::SessionExited(SessionExitedPayload {
                reason: Some("stopped".into()),
                recoverable: Some(true),
                exit_kind: Some("graceful".into()),
            }),
        );
        Ok(())
    }
}
