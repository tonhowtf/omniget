//! Claude stream-json → [`RuntimeEvent`]. Pure: no process, no clock beyond
//! the event timestamps, and file reads go through a closure, so the tests
//! replay the recorded fixtures line by line.
//!
//! Turn bookkeeping follows what 2.1.280 actually emits:
//! * every prompt we write carries our `uuid`; `command_lifecycle` says when
//!   it is `queued`, `started`, `completed` or `cancelled`;
//! * `result.user_message_uuids` lists the prompts a result closes (several
//!   when queued prompts were absorbed into one turn = steer);
//! * `result.total_cost_usd` is cumulative per process, `result.usage` is per
//!   turn, so the turn cost is the delta;
//! * output that arrives with no turn open (a background sub-agent finishing
//!   after the result) is emitted with no `turnId` and lands as loose rows.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::super::{
    ApprovalDecision, ApprovalOption, AuthStatusPayload, ContentDeltaPayload, ErrorClass,
    FilesPersistedPayload, HookPayload, ItemPayload, ItemStatus, ItemType, ModelReroutedPayload,
    PersistedFile, PlanStep, PlanUpdatedPayload, ProposedCompletedPayload, ProviderRefs,
    RateLimitWindow, RateLimitsPayload, RawPayload, RequestDetail, RequestOpenedPayload,
    RequestResolvedPayload, RequestType, RuntimeErrorPayload, RuntimeEvent, RuntimeEventKind,
    RuntimeWarningPayload, SessionStartedPayload, SessionState, SessionStatePayload, StreamKind,
    TaskPayload, ThreadStatePayload, TokenUsage, ToolDeniedPayload, ToolProgressPayload,
    ToolSummaryPayload, TurnCompletedPayload, TurnEndState, TurnStartedPayload,
    UsageUpdatedPayload, UserInputOption, UserInputQuestion, UserInputRequestedPayload,
    UserInputResolvedPayload, ValuePayload,
};
use super::cursor::{ClaudeCursor, TurnMark};
use super::diff;
use super::protocol;

pub const DRIVER: &str = "claude";
/// Tool output kept in an item (the model saw all of it; the UI needs a tail).
const OUTPUT_CLIP: usize = 16 * 1024;
const INPUT_CLIP: usize = 32 * 1024;

/// What one stdout line turns into.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    Event(RuntimeEvent),
    /// A line the driver must write to the CLI's stdin right away (answers
    /// the driver gives on its own: captured plans, unsupported requests).
    Send(Value),
    /// The CLI answered one of our control requests.
    ControlResult {
        request_id: String,
        ok: bool,
        payload: Value,
    },
}

/// A pending `can_use_tool` the user has to answer.
#[derive(Debug, Clone, PartialEq)]
pub enum OpenRequest {
    Approval {
        turn_id: Option<String>,
        tool_name: String,
        tool_use_id: Option<String>,
        input: Value,
        suggestions: Vec<Value>,
        request_type: RequestType,
    },
    Question {
        turn_id: Option<String>,
        tool_use_id: Option<String>,
        input: Value,
        questions: Vec<UserInputQuestion>,
    },
}

#[derive(Debug, Clone)]
enum Block {
    Text { item_id: String, text: String },
    Thinking { item_id: String, text: String },
    Tool { tool_use_id: String, json: String },
}

#[derive(Debug, Clone)]
struct ToolItem {
    turn_id: Option<String>,
    name: String,
    item_type: ItemType,
    input: Value,
    agent_id: Option<String>,
    parent: Option<String>,
    completed: bool,
}

#[derive(Debug, Clone)]
struct TaskInfo {
    turn_id: Option<String>,
    tool_use_id: Option<String>,
    description: Option<String>,
    task_type: Option<String>,
}

pub struct Translator {
    instance_id: String,
    thread_id: String,
    pub cwd: Option<PathBuf>,
    pub cursor: ClaudeCursor,
    cursor_dirty: bool,
    /// prompt uuid → engine turn id.
    prompts: HashMap<String, String>,
    queue: VecDeque<String>,
    active: Option<String>,
    started: HashSet<String>,
    completed: HashSet<String>,
    interrupt_requested: bool,
    /// Last main-chain transcript uuid seen (the rollback boundary).
    last_uuid: Option<String>,
    // per process
    last_total_cost: f64,
    streamed: HashSet<String>,
    current_message: Option<String>,
    snapshot_blocks: HashMap<String, usize>,
    blocks: HashMap<u64, Block>,
    tools: HashMap<String, ToolItem>,
    plans: HashSet<String>,
    requests: HashMap<String, OpenRequest>,
    tasks: HashMap<String, TaskInfo>,
    task_by_tool: HashMap<String, String>,
    failure_hint: Option<String>,
    rate_warned: HashSet<String>,
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    init_count: u64,
}

impl Translator {
    pub fn new(
        instance_id: &str,
        thread_id: &str,
        cwd: Option<PathBuf>,
        cursor: ClaudeCursor,
    ) -> Self {
        Self {
            instance_id: instance_id.to_string(),
            thread_id: thread_id.to_string(),
            cwd,
            cursor,
            cursor_dirty: false,
            prompts: HashMap::new(),
            queue: VecDeque::new(),
            active: None,
            started: HashSet::new(),
            completed: HashSet::new(),
            interrupt_requested: false,
            last_uuid: None,
            last_total_cost: 0.0,
            streamed: HashSet::new(),
            current_message: None,
            snapshot_blocks: HashMap::new(),
            blocks: HashMap::new(),
            tools: HashMap::new(),
            plans: HashSet::new(),
            requests: HashMap::new(),
            tasks: HashMap::new(),
            task_by_tool: HashMap::new(),
            failure_hint: None,
            rate_warned: HashSet::new(),
            model: None,
            permission_mode: None,
            init_count: 0,
        }
    }

    /// `system/init` messages seen so far (a process that got going).
    pub fn inits(&self) -> u64 {
        self.init_count
    }

    fn ev(&self, turn: Option<&str>, kind: RuntimeEventKind) -> RuntimeEvent {
        RuntimeEvent::new(DRIVER, &self.instance_id, &self.thread_id, turn, kind)
    }

    fn cur(&self) -> Option<String> {
        self.active.clone()
    }

    // ── turn bookkeeping ────────────────────────────────────────────────

    /// A prompt was written to stdin. Returns `turn.started` when no other
    /// turn is running; otherwise the prompt is queued (steer) and starts when
    /// the CLI says so.
    pub fn register_prompt(
        &mut self,
        turn_id: &str,
        uuid: &str,
        model: Option<&str>,
    ) -> Vec<RuntimeEvent> {
        self.prompts.insert(uuid.to_string(), turn_id.to_string());
        self.cursor.turns.push(TurnMark {
            turn_id: turn_id.to_string(),
            prompt_uuid: uuid.to_string(),
            last_uuid: None,
        });
        self.cursor_dirty = true;
        self.queue.push_back(uuid.to_string());
        if model.is_some() {
            self.model = model.map(str::to_string);
        }
        if self.active.is_none() {
            self.activate_next().into_iter().collect()
        } else {
            Vec::new()
        }
    }

    fn activate_next(&mut self) -> Option<RuntimeEvent> {
        let uuid = self.queue.pop_front()?;
        let turn = self.prompts.get(&uuid).cloned()?;
        self.activate(&turn)
    }

    fn activate(&mut self, turn: &str) -> Option<RuntimeEvent> {
        self.active = Some(turn.to_string());
        self.failure_hint = None;
        if self.started.insert(turn.to_string()) {
            Some(self.ev(
                Some(turn),
                RuntimeEventKind::TurnStarted(TurnStartedPayload {
                    model: self.model.clone(),
                    effort: None,
                }),
            ))
        } else {
            None
        }
    }

    /// Content arrived: make sure the queued prompt it belongs to is active.
    fn ensure_active(&mut self, out: &mut Vec<Output>) {
        if self.active.is_none() && !self.queue.is_empty() {
            if let Some(e) = self.activate_next() {
                out.push(Output::Event(e));
            }
        }
    }

    pub fn has_live_turn(&self) -> bool {
        self.active.is_some() || !self.queue.is_empty()
    }

    pub fn active_turn(&self) -> Option<String> {
        self.active.clone()
    }

    pub fn mark_interrupt_requested(&mut self) {
        self.interrupt_requested = true;
    }

    /// Forget everything tied to one process (new spawn).
    pub fn reset_process(&mut self) {
        self.last_total_cost = 0.0;
        self.streamed.clear();
        self.current_message = None;
        self.snapshot_blocks.clear();
        self.blocks.clear();
        self.requests.clear();
        self.interrupt_requested = false;
    }

    /// The cursor, when it changed since the last call.
    pub fn take_cursor_if_dirty(&mut self) -> Option<ClaudeCursor> {
        if self.cursor_dirty {
            self.cursor_dirty = false;
            Some(self.cursor.clone())
        } else {
            None
        }
    }

    fn cursor_event(&mut self) -> Option<RuntimeEvent> {
        if self.cursor.session_id.is_empty() {
            return None;
        }
        Some(self.ev(
            None,
            RuntimeEventKind::SessionStarted(SessionStartedPayload {
                message: None,
                resume: Some(self.cursor.to_value()),
            }),
        ))
    }

    /// Ends every live turn (process died, session stopped). Open tools and
    /// requests are closed; the engine marks the approvals stale.
    pub fn abort_all(&mut self, state: TurnEndState, reason: &str) -> Vec<RuntimeEvent> {
        let mut turns: Vec<String> = Vec::new();
        if let Some(a) = self.active.take() {
            turns.push(a);
        }
        while let Some(uuid) = self.queue.pop_front() {
            if let Some(t) = self.prompts.get(&uuid) {
                turns.push(t.clone());
            }
        }
        let mut out = Vec::new();
        for turn in turns {
            if !self.completed.insert(turn.clone()) {
                continue;
            }
            out.extend(self.close_open_items(Some(&turn), false));
            if self.started.insert(turn.clone()) {
                out.push(self.ev(
                    Some(&turn),
                    RuntimeEventKind::TurnStarted(TurnStartedPayload::default()),
                ));
            }
            out.push(self.ev(
                Some(&turn),
                RuntimeEventKind::TurnCompleted(TurnCompletedPayload {
                    state,
                    stop_reason: None,
                    usage: None,
                    total_cost_usd: None,
                    error_message: Some(reason.to_string()),
                }),
            ));
        }
        self.requests.clear();
        self.interrupt_requested = false;
        out
    }

    /// Open requests (for stop: each gets a deny so the CLI is never left
    /// waiting, and a resolution event).
    pub fn drain_requests(&mut self, resolution: &str) -> (Vec<Value>, Vec<RuntimeEvent>) {
        let mut sends = Vec::new();
        let mut events = Vec::new();
        let open: Vec<(String, OpenRequest)> = self.requests.drain().collect();
        for (id, req) in open {
            sends.push(protocol::control_success(
                &id,
                json!({"behavior": "deny", "message": protocol::CANCEL_MESSAGE, "interrupt": true}),
            ));
            match req {
                OpenRequest::Approval {
                    turn_id,
                    request_type,
                    ..
                } => events.push(
                    self.ev(
                        turn_id.as_deref(),
                        RuntimeEventKind::RequestResolved(RequestResolvedPayload {
                            request_type,
                            decision: Some(ApprovalDecision::Cancel),
                            resolution: Some(resolution.to_string()),
                        }),
                    )
                    .with_request(id),
                ),
                OpenRequest::Question { turn_id, .. } => events.push(
                    self.ev(
                        turn_id.as_deref(),
                        RuntimeEventKind::UserInputResolved(UserInputResolvedPayload {
                            answers: json!({}),
                        }),
                    )
                    .with_request(id),
                ),
            }
        }
        (sends, events)
    }

    pub fn open_request(&self, request_id: &str) -> Option<&OpenRequest> {
        self.requests.get(request_id)
    }

    // ── answers ─────────────────────────────────────────────────────────

    /// The control_response for an approval + the `request.resolved` event.
    pub fn answer_approval(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(Value, Vec<RuntimeEvent>), String> {
        let Some(OpenRequest::Approval { .. }) = self.requests.get(request_id) else {
            return Err(format!("no open approval `{request_id}`"));
        };
        let Some(OpenRequest::Approval {
            turn_id,
            tool_name,
            input,
            suggestions,
            request_type,
            ..
        }) = self.requests.remove(request_id)
        else {
            unreachable!()
        };
        let answer = protocol::permission_answer(decision, &tool_name, &input, &suggestions);
        if decision == ApprovalDecision::Cancel {
            self.interrupt_requested = true;
        }
        let ev = self
            .ev(
                turn_id.as_deref(),
                RuntimeEventKind::RequestResolved(RequestResolvedPayload {
                    request_type,
                    decision: Some(decision),
                    resolution: None,
                }),
            )
            .with_request(request_id);
        Ok((protocol::control_success(request_id, answer), vec![ev]))
    }

    /// The control_response for an `AskUserQuestion` + `user-input.resolved`.
    ///
    /// `answers` is `{questionId: "label" | ["a","b"] | {answer|value|label}}`;
    /// question ids are the question texts, which is also the key the CLI
    /// looks answers up by (T3 #2388). Multi-select answers are joined with
    /// ", " as the CLI's own dialog does.
    pub fn answer_questions(
        &mut self,
        request_id: &str,
        answers: &Value,
    ) -> Result<(Value, Vec<RuntimeEvent>), String> {
        let Some(OpenRequest::Question { .. }) = self.requests.get(request_id) else {
            return Err(format!("no open question `{request_id}`"));
        };
        let Some(OpenRequest::Question {
            turn_id,
            input,
            questions,
            ..
        }) = self.requests.remove(request_id)
        else {
            unreachable!()
        };
        let mut map = serde_json::Map::new();
        for (i, q) in questions.iter().enumerate() {
            let raw = answers
                .get(&q.id)
                .or_else(|| answers.get(&q.question))
                .or_else(|| answers.get(format!("q-{i}")))
                .or_else(|| answers.as_array().and_then(|a| a.get(i)));
            let text = match raw {
                Some(Value::String(s)) => s.clone(),
                Some(Value::Array(list)) => list
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_string).or_else(|| answer_text(v)))
                    .collect::<Vec<_>>()
                    .join(", "),
                Some(v) => answer_text(v).unwrap_or_default(),
                None => continue,
            };
            map.insert(q.question.clone(), Value::String(text));
        }
        let answered = Value::Object(map);
        let mut updated = input.clone();
        if let Some(obj) = updated.as_object_mut() {
            obj.insert("answers".into(), answered.clone());
        }
        let line = protocol::control_success(
            request_id,
            json!({"behavior": "allow", "updatedInput": updated}),
        );
        let ev = self
            .ev(
                turn_id.as_deref(),
                RuntimeEventKind::UserInputResolved(UserInputResolvedPayload { answers: answered }),
            )
            .with_request(request_id);
        Ok((line, vec![ev]))
    }

    // ── the stream ──────────────────────────────────────────────────────

    pub fn on_line(
        &mut self,
        line: &str,
        read_file: &dyn Fn(&Path) -> Option<String>,
    ) -> Vec<Output> {
        let line = line.trim();
        if line.is_empty() {
            return Vec::new();
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            // A stray log line on stdout is never fatal.
            return Vec::new();
        };
        let mut out = Vec::new();
        let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "system" => self.on_system(&v, &mut out),
            "stream_event" => self.on_stream_event(&v, &mut out),
            "assistant" => self.on_assistant(&v, &mut out),
            "user" => self.on_user(&v, &mut out),
            "result" => self.on_result(&v, &mut out),
            "rate_limit_event" => self.on_rate_limit(&v, &mut out),
            "control_request" => self.on_control_request(&v, read_file, &mut out),
            "control_cancel_request" => self.on_control_cancel(&v, &mut out),
            "control_response" => self.on_control_response(&v, &mut out),
            "command_lifecycle" => self.on_lifecycle(&v, &mut out),
            "auth_status" => {
                let p = AuthStatusPayload {
                    is_authenticating: v.get("isAuthenticating").and_then(Value::as_bool),
                    output: v
                        .get("output")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                    error: str_at(&v, "error"),
                };
                out.push(Output::Event(
                    self.ev(self.cur().as_deref(), RuntimeEventKind::AuthStatus(p)),
                ));
            }
            "tool_progress" => {
                let p = ToolProgressPayload {
                    tool_use_id: str_at(&v, "tool_use_id"),
                    tool_name: str_at(&v, "tool_name"),
                    summary: None,
                    elapsed_seconds: v.get("elapsed_time_seconds").and_then(Value::as_f64),
                    task_id: str_at(&v, "task_id"),
                    parent_tool_use_id: str_at(&v, "parent_tool_use_id"),
                };
                out.push(Output::Event(
                    self.ev(self.cur().as_deref(), RuntimeEventKind::ToolProgress(p)),
                ));
            }
            "tool_use_summary" => {
                let p = ToolSummaryPayload {
                    summary: str_at(&v, "summary").unwrap_or_default(),
                    preceding_tool_use_ids: v
                        .get("preceding_tool_use_ids")
                        .and_then(Value::as_array)
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                };
                out.push(Output::Event(
                    self.ev(self.cur().as_deref(), RuntimeEventKind::ToolSummary(p)),
                ));
            }
            "keep_alive"
            | "prompt_suggestion"
            | "transcript_mirror"
            | "streamlined_text"
            | "streamlined_tool_use_summary" => {}
            _ => out.push(Output::Event(self.ev(
                self.cur().as_deref(),
                RuntimeEventKind::Raw(RawPayload {
                    source: "claude.stream-json".into(),
                    method: Some(kind.to_string()),
                    payload: v.clone(),
                }),
            ))),
        }
        out
    }

    fn on_lifecycle(&mut self, v: &Value, out: &mut Vec<Output>) {
        let Some(uuid) = str_at(v, "command_uuid") else {
            return;
        };
        let Some(turn) = self.prompts.get(&uuid).cloned() else {
            return;
        };
        match v.get("state").and_then(Value::as_str).unwrap_or("") {
            "started" => {
                self.queue.retain(|u| u != &uuid);
                if self.active.as_deref() != Some(turn.as_str()) && !self.completed.contains(&turn)
                {
                    if let Some(e) = self.activate(&turn) {
                        out.push(Output::Event(e));
                    }
                }
            }
            "cancelled" => {
                self.queue.retain(|u| u != &uuid);
                if self.completed.insert(turn.clone()) {
                    if self.active.as_deref() == Some(turn.as_str()) {
                        self.active = None;
                    }
                    for e in self.close_open_items(Some(&turn), false) {
                        out.push(Output::Event(e));
                    }
                    if self.started.insert(turn.clone()) {
                        out.push(Output::Event(self.ev(
                            Some(&turn),
                            RuntimeEventKind::TurnStarted(TurnStartedPayload::default()),
                        )));
                    }
                    out.push(Output::Event(self.ev(
                        Some(&turn),
                        RuntimeEventKind::TurnCompleted(TurnCompletedPayload {
                            state: TurnEndState::Interrupted,
                            stop_reason: None,
                            usage: None,
                            total_cost_usd: None,
                            error_message: Some("cancelled before it ran".into()),
                        }),
                    )));
                }
            }
            _ => {}
        }
    }

    fn on_system(&mut self, v: &Value, out: &mut Vec<Output>) {
        let sub = v.get("subtype").and_then(Value::as_str).unwrap_or("");
        match sub {
            "init" => {
                self.init_count += 1;
                self.ensure_active(out);
                if let Some(model) = str_at(v, "model") {
                    self.model = Some(model);
                }
                if let Some(mode) = str_at(v, "permissionMode") {
                    self.permission_mode = Some(mode);
                }
                if let Some(sid) = str_at(v, "session_id") {
                    if sid != self.cursor.session_id {
                        self.cursor.session_id = sid;
                        self.cursor.resume_at = None;
                        self.cursor.fork = false;
                        self.cursor_dirty = true;
                        if let Some(e) = self.cursor_event() {
                            out.push(Output::Event(e));
                        }
                    }
                }
                let config = json!({
                    "model": v.get("model"),
                    "permissionMode": v.get("permissionMode"),
                    "cwd": v.get("cwd"),
                    "claudeCodeVersion": v.get("claude_code_version"),
                    "tools": v.get("tools").and_then(Value::as_array).map(|a| a.len()),
                    "outputStyle": v.get("output_style"),
                    "apiKeySource": v.get("apiKeySource"),
                });
                out.push(Output::Event(self.ev(
                    self.cur().as_deref(),
                    RuntimeEventKind::SessionConfigured(ValuePayload { value: config }),
                )));
                if let Some(servers) = v
                    .get("mcp_servers")
                    .filter(|s| s.as_array().map_or(false, |a| !a.is_empty()))
                {
                    out.push(Output::Event(self.ev(
                        None,
                        RuntimeEventKind::McpStatusUpdated(ValuePayload {
                            value: json!({"servers": servers}),
                        }),
                    )));
                }
                out.push(Output::Event(self.state(SessionState::Running, None)));
            }
            "status" => {
                if let Some(mode) = str_at(v, "permissionMode") {
                    if self.permission_mode.as_deref() != Some(mode.as_str()) {
                        self.permission_mode = Some(mode.clone());
                        out.push(Output::Event(self.ev(
                            self.cur().as_deref(),
                            RuntimeEventKind::SessionConfigured(ValuePayload {
                                value: json!({"permissionMode": mode}),
                            }),
                        )));
                    }
                }
                if v.get("status").and_then(Value::as_str) == Some("compacting") {
                    out.push(Output::Event(
                        self.state(SessionState::Waiting, Some("compacting".into())),
                    ));
                }
            }
            "compact_boundary" => {
                let meta = v.get("compact_metadata");
                let p = ThreadStatePayload {
                    state: "compacted".into(),
                    before_tokens: meta
                        .and_then(|m| m.get("pre_tokens"))
                        .and_then(Value::as_u64),
                    after_tokens: meta
                        .and_then(|m| m.get("post_tokens"))
                        .and_then(Value::as_u64),
                    detail: meta
                        .and_then(|m| m.get("trigger"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                };
                out.push(Output::Event(self.ev(
                    self.cur().as_deref(),
                    RuntimeEventKind::ThreadStateChanged(p),
                )));
            }
            "hook_started" | "hook_progress" | "hook_response" => {
                let p = HookPayload {
                    hook_id: str_at(v, "hook_id")
                        .unwrap_or_else(|| str_at(v, "uuid").unwrap_or_default()),
                    hook_name: str_at(v, "hook_name"),
                    hook_event: str_at(v, "hook_event"),
                    output: str_at(v, "output").map(|s| clip(&s, OUTPUT_CLIP)),
                    stdout: str_at(v, "stdout").map(|s| clip(&s, OUTPUT_CLIP)),
                    stderr: str_at(v, "stderr").map(|s| clip(&s, OUTPUT_CLIP)),
                    outcome: str_at(v, "outcome"),
                    exit_code: v.get("exit_code").and_then(Value::as_i64).map(|c| c as i32),
                };
                let kind = match sub {
                    "hook_started" => RuntimeEventKind::HookStarted(p),
                    "hook_progress" => RuntimeEventKind::HookProgress(p),
                    _ => RuntimeEventKind::HookCompleted(p),
                };
                out.push(Output::Event(self.ev(self.cur().as_deref(), kind)));
            }
            "task_started" | "task_progress" | "task_updated" | "task_notification" => {
                self.on_task(sub, v, out)
            }
            "permission_denied" => {
                let p = ToolDeniedPayload {
                    tool_name: str_at(v, "tool_name").unwrap_or_default(),
                    tool_use_id: str_at(v, "tool_use_id"),
                    reason: str_at(v, "reason").or_else(|| str_at(v, "message")),
                    agent_id: str_at(v, "agent_id"),
                };
                out.push(Output::Event(
                    self.ev(self.cur().as_deref(), RuntimeEventKind::ToolDenied(p)),
                ));
            }
            "api_retry" => {
                let attempt = v.get("attempt").and_then(Value::as_u64).unwrap_or(0);
                let max = v.get("max_retries").and_then(Value::as_u64).unwrap_or(0);
                out.push(Output::Event(self.state(
                    SessionState::Running,
                    Some(format!("api_retry:{attempt}/{max}")),
                )));
            }
            "session_state_changed" => {
                let state = match v.get("state").and_then(Value::as_str).unwrap_or("") {
                    "running" => SessionState::Running,
                    "requires_action" => SessionState::Waiting,
                    _ => SessionState::Ready,
                };
                out.push(Output::Event(self.state(state, None)));
            }
            "model_fallback" | "model_consent_fallback" | "model_refusal_fallback" => {
                let from = str_at(v, "from_model").or_else(|| str_at(v, "original_model"));
                let to = str_at(v, "to_model").or_else(|| str_at(v, "fallback_model"));
                let kind = match (from, to) {
                    (Some(from_model), Some(to_model)) => {
                        self.model = Some(to_model.clone());
                        RuntimeEventKind::ModelRerouted(ModelReroutedPayload {
                            from_model,
                            to_model,
                            reason: sub.to_string(),
                        })
                    }
                    _ => RuntimeEventKind::RuntimeWarning(RuntimeWarningPayload {
                        message: format!("Claude switched model ({sub})"),
                        detail: Some(clip(&v.to_string(), 600)),
                    }),
                };
                out.push(Output::Event(self.ev(self.cur().as_deref(), kind)));
            }
            "model_refusal_no_fallback" | "notification" | "informational" => {
                let level = str_at(v, "level")
                    .or_else(|| str_at(v, "priority"))
                    .unwrap_or_default();
                let loud = sub == "model_refusal_no_fallback"
                    || matches!(level.as_str(), "warning" | "error" | "high" | "immediate");
                if loud {
                    let message = str_at(v, "message")
                        .or_else(|| str_at(v, "text"))
                        .or_else(|| str_at(v, "api_refusal_explanation"))
                        .unwrap_or_else(|| sub.to_string());
                    out.push(Output::Event(self.ev(
                        self.cur().as_deref(),
                        RuntimeEventKind::RuntimeWarning(RuntimeWarningPayload {
                            message,
                            detail: None,
                        }),
                    )));
                }
            }
            "files_persisted" => {
                let files = v
                    .get("files")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|f| PersistedFile {
                                filename: str_at(f, "filename").unwrap_or_default(),
                                file_id: str_at(f, "file_id").unwrap_or_default(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let failed = v
                    .get("failed")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(|f| {
                                f.as_str()
                                    .map(str::to_string)
                                    .unwrap_or_else(|| f.to_string())
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                out.push(Output::Event(self.ev(
                    self.cur().as_deref(),
                    RuntimeEventKind::FilesPersisted(FilesPersistedPayload { files, failed }),
                )));
            }
            "mirror_error" => out.push(Output::Event(self.ev(
                self.cur().as_deref(),
                RuntimeEventKind::RuntimeError(RuntimeErrorPayload {
                    message: "Claude workspace mirror error".into(),
                    class: ErrorClass::Unknown,
                    code: None,
                    detail: str_at(v, "error").or_else(|| str_at(v, "message")),
                }),
            ))),
            // Progress noise the UI does not show.
            _ => {}
        }
    }

    fn state(&self, state: SessionState, reason: Option<String>) -> RuntimeEvent {
        self.ev(
            self.cur().as_deref(),
            RuntimeEventKind::SessionStateChanged(SessionStatePayload {
                state,
                reason,
                detail: None,
            }),
        )
    }

    fn on_task(&mut self, sub: &str, v: &Value, out: &mut Vec<Output>) {
        let Some(task_id) = str_at(v, "task_id") else {
            return;
        };
        if sub == "task_started" {
            let info = TaskInfo {
                turn_id: self.cur(),
                tool_use_id: str_at(v, "tool_use_id"),
                description: str_at(v, "description"),
                task_type: str_at(v, "task_type").or_else(|| str_at(v, "subagent_type")),
            };
            if let Some(t) = &info.tool_use_id {
                self.task_by_tool.insert(t.clone(), task_id.clone());
            }
            self.tasks.insert(task_id.clone(), info);
        }
        let info = self.tasks.get(&task_id).cloned().unwrap_or(TaskInfo {
            turn_id: self.cur(),
            tool_use_id: str_at(v, "tool_use_id"),
            description: None,
            task_type: None,
        });
        let usage = v.get("usage").map(|u| TokenUsage {
            used_tokens: u.get("total_tokens").and_then(Value::as_u64),
            tool_uses: u.get("tool_uses").and_then(Value::as_u64),
            duration_ms: u.get("duration_ms").and_then(Value::as_u64),
            ..Default::default()
        });
        let mut p = TaskPayload {
            task_id: task_id.clone(),
            description: str_at(v, "description").or_else(|| info.description.clone()),
            task_type: info.task_type.clone(),
            title: info.description.clone(),
            tool_use_id: info.tool_use_id.clone(),
            usage,
            last_tool_name: str_at(v, "last_tool_name"),
            is_backgrounded: v.get("is_backgrounded").and_then(Value::as_bool),
            agent_id: Some(task_id.clone()),
            model: str_at(v, "model"),
            ..Default::default()
        };
        let kind = match sub {
            "task_started" => {
                p.status = Some("running".into());
                RuntimeEventKind::TaskStarted(p)
            }
            "task_progress" => {
                p.status = Some("running".into());
                p.summary = str_at(v, "description");
                RuntimeEventKind::TaskProgress(p)
            }
            "task_updated" => {
                let patch = v.get("patch").cloned().unwrap_or(Value::Null);
                p.status = str_at(&patch, "status").map(|s| match s.as_str() {
                    "killed" => "cancelled".to_string(),
                    "paused" => "idle".to_string(),
                    other => other.to_string(),
                });
                p.error = str_at(&patch, "error");
                RuntimeEventKind::TaskUpdated(p)
            }
            _ => {
                p.status = str_at(v, "status").or(Some("completed".into()));
                p.summary = str_at(v, "summary").map(|s| clip(&s, OUTPUT_CLIP));
                RuntimeEventKind::TaskCompleted(p)
            }
        };
        out.push(Output::Event(self.ev(info.turn_id.as_deref(), kind)));
    }

    fn agent_of(&self, parent: Option<&str>) -> Option<String> {
        parent.and_then(|p| {
            self.task_by_tool
                .get(p)
                .cloned()
                .or_else(|| Some(p.to_string()))
        })
    }

    fn on_stream_event(&mut self, v: &Value, out: &mut Vec<Output>) {
        let Some(e) = v.get("event") else { return };
        let parent = str_at(v, "parent_tool_use_id");
        let et = e.get("type").and_then(Value::as_str).unwrap_or("");
        let index = e.get("index").and_then(Value::as_u64).unwrap_or(0);
        if parent.is_none() {
            self.ensure_active(out);
        }
        match et {
            "message_start" => {
                self.blocks.clear();
                if let Some(id) = e
                    .get("message")
                    .and_then(|m| m.get("id"))
                    .and_then(Value::as_str)
                {
                    self.streamed.insert(id.to_string());
                    self.current_message = Some(id.to_string());
                }
            }
            "content_block_start" => {
                let block = e.get("content_block").cloned().unwrap_or(Value::Null);
                let msg = self.current_message.clone().unwrap_or_default();
                match block.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text" if parent.is_none() => {
                        self.blocks.insert(
                            index,
                            Block::Text {
                                item_id: format!("{msg}:{index}"),
                                text: String::new(),
                            },
                        );
                    }
                    "thinking" | "redacted_thinking" if parent.is_none() => {
                        self.blocks.insert(
                            index,
                            Block::Thinking {
                                item_id: format!("{msg}:{index}"),
                                text: String::new(),
                            },
                        );
                    }
                    "tool_use" | "server_tool_use" | "mcp_tool_use" => {
                        let id = str_at(&block, "id").unwrap_or_else(|| format!("{msg}:{index}"));
                        let name = str_at(&block, "name").unwrap_or_else(|| "tool".into());
                        self.blocks.insert(
                            index,
                            Block::Tool {
                                tool_use_id: id.clone(),
                                json: String::new(),
                            },
                        );
                        if !self.tools.contains_key(&id) {
                            let input = block.get("input").cloned().unwrap_or(json!({}));
                            self.start_tool(&id, &name, input, parent.as_deref(), out);
                        }
                    }
                    _ => {}
                }
            }
            "content_block_delta" => {
                let d = e.get("delta").cloned().unwrap_or(Value::Null);
                match (
                    d.get("type").and_then(Value::as_str).unwrap_or(""),
                    self.blocks.get_mut(&index),
                ) {
                    ("text_delta", Some(Block::Text { item_id, text })) => {
                        let delta = str_at(&d, "text").unwrap_or_default();
                        text.push_str(&delta);
                        let item = item_id.clone();
                        self.delta(StreamKind::AssistantText, &item, delta, out);
                    }
                    ("thinking_delta", Some(Block::Thinking { item_id, text })) => {
                        let delta = str_at(&d, "thinking").unwrap_or_default();
                        text.push_str(&delta);
                        let item = item_id.clone();
                        self.delta(StreamKind::ReasoningSummaryText, &item, delta, out);
                    }
                    ("input_json_delta", Some(Block::Tool { json, .. })) => {
                        json.push_str(&str_at(&d, "partial_json").unwrap_or_default());
                    }
                    _ => {}
                }
            }
            "content_block_stop" => match self.blocks.remove(&index) {
                Some(Block::Text { item_id, text }) => {
                    if !text.is_empty() {
                        self.complete_text(ItemType::AssistantMessage, &item_id, text, out);
                    }
                }
                Some(Block::Thinking { item_id, text }) => {
                    if !text.trim().is_empty() {
                        self.complete_text(ItemType::Reasoning, &item_id, text, out);
                    }
                }
                Some(Block::Tool { tool_use_id, json }) => {
                    let input: Value = if json.trim().is_empty() {
                        json!({})
                    } else {
                        serde_json::from_str(&json)
                            .unwrap_or_else(|_| json!({"_raw": clip(&json, INPUT_CLIP)}))
                    };
                    self.update_tool_input(&tool_use_id, input, out);
                }
                None => {}
            },
            _ => {}
        }
    }

    fn delta(&self, kind: StreamKind, item: &str, delta: String, out: &mut Vec<Output>) {
        if delta.is_empty() {
            return;
        }
        out.push(Output::Event(
            self.ev(
                self.cur().as_deref(),
                RuntimeEventKind::ContentDelta(ContentDeltaPayload {
                    stream_kind: kind,
                    delta,
                    content_index: None,
                    summary_index: None,
                }),
            )
            .with_item(item),
        ));
    }

    fn complete_text(&self, item_type: ItemType, item: &str, text: String, out: &mut Vec<Output>) {
        let mut p = ItemPayload::new(item_type);
        p.status = Some(ItemStatus::Completed);
        p.detail = Some(text);
        out.push(Output::Event(
            self.ev(self.cur().as_deref(), RuntimeEventKind::ItemCompleted(p))
                .with_item(item),
        ));
    }

    fn start_tool(
        &mut self,
        id: &str,
        name: &str,
        input: Value,
        parent: Option<&str>,
        out: &mut Vec<Output>,
    ) {
        let item_type = classify_tool(name, &input);
        let agent_id = self.agent_of(parent);
        let turn = self.cur();
        self.tools.insert(
            id.to_string(),
            ToolItem {
                turn_id: turn.clone(),
                name: name.to_string(),
                item_type,
                input: input.clone(),
                agent_id: agent_id.clone(),
                parent: parent.map(str::to_string),
                completed: false,
            },
        );
        let mut p = ItemPayload::new(item_type);
        p.status = Some(ItemStatus::InProgress);
        p.title = Some(title_of(item_type).into());
        p.tool_name = Some(name.to_string());
        p.detail = tool_summary(name, &input);
        p.data = Some(json!({"input": clip_value(&input)}));
        p.agent_id = agent_id;
        p.parent_tool_use_id = parent.map(str::to_string);
        let mut ev = self
            .ev(turn.as_deref(), RuntimeEventKind::ItemStarted(p))
            .with_item(id);
        ev.provider_refs = Some(ProviderRefs {
            provider_item_id: Some(id.to_string()),
            ..Default::default()
        });
        out.push(Output::Event(ev));
        if name == "ExitPlanMode" {
            self.capture_plan(id, &input, out);
        }
    }

    fn update_tool_input(&mut self, id: &str, input: Value, out: &mut Vec<Output>) {
        let Some(tool) = self.tools.get_mut(id) else {
            return;
        };
        tool.input = input.clone();
        tool.item_type = classify_tool(&tool.name, &input);
        let (name, item_type, turn) = (tool.name.clone(), tool.item_type, tool.turn_id.clone());
        let mut p = ItemPayload::new(item_type);
        p.status = Some(ItemStatus::InProgress);
        p.detail = tool_summary(&name, &input);
        p.data = Some(json!({"input": clip_value(&input)}));
        out.push(Output::Event(
            self.ev(turn.as_deref(), RuntimeEventKind::ItemUpdated(p))
                .with_item(id),
        ));
        if name == "TodoWrite" {
            if let Some(todos) = input.get("todos").and_then(Value::as_array) {
                let plan = todos
                    .iter()
                    .map(|t| PlanStep {
                        step: str_at(t, "content")
                            .or_else(|| str_at(t, "activeForm"))
                            .unwrap_or_default(),
                        status: match str_at(t, "status").as_deref() {
                            Some("in_progress") => "inProgress".into(),
                            Some("completed") => "completed".into(),
                            _ => "pending".into(),
                        },
                    })
                    .collect();
                out.push(Output::Event(self.ev(
                    turn.as_deref(),
                    RuntimeEventKind::PlanUpdated(PlanUpdatedPayload {
                        explanation: None,
                        plan,
                    }),
                )));
            }
        }
        if name == "ExitPlanMode" {
            self.capture_plan(id, &input, out);
        }
    }

    fn capture_plan(&mut self, tool_use_id: &str, input: &Value, out: &mut Vec<Output>) {
        let Some(plan) = str_at(input, "plan").filter(|p| !p.trim().is_empty()) else {
            return;
        };
        if !self.plans.insert(tool_use_id.to_string()) || !self.plans.insert(format!("plan:{plan}"))
        {
            return;
        }
        let turn = self
            .tools
            .get(tool_use_id)
            .and_then(|t| t.turn_id.clone())
            .or_else(|| self.cur());
        out.push(Output::Event(self.ev(
            turn.as_deref(),
            RuntimeEventKind::ProposedCompleted(ProposedCompletedPayload {
                plan_markdown: plan,
            }),
        )));
    }

    fn on_assistant(&mut self, v: &Value, out: &mut Vec<Output>) {
        let parent = str_at(v, "parent_tool_use_id");
        let Some(msg) = v.get("message") else { return };
        if parent.is_none() {
            self.ensure_active(out);
            if let Some(uuid) = str_at(v, "uuid") {
                self.last_uuid = Some(uuid);
            }
            if let Some(model) = str_at(msg, "model") {
                self.model = Some(model);
            }
            if let Some(err) = str_at(v, "error") {
                let text = collect_text(msg);
                self.failure_hint = Some(if text.is_empty() {
                    err
                } else {
                    format!("{err}: {text}")
                });
            }
        }
        let id = str_at(msg, "id").unwrap_or_default();
        let streamed = self.streamed.contains(&id);
        let blocks = msg
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for block in blocks {
            let bt = block.get("type").and_then(Value::as_str).unwrap_or("");
            match bt {
                "tool_use" | "server_tool_use" | "mcp_tool_use" => {
                    let Some(tid) = str_at(&block, "id") else {
                        continue;
                    };
                    let name = str_at(&block, "name").unwrap_or_else(|| "tool".into());
                    let input = block.get("input").cloned().unwrap_or(json!({}));
                    if !self.tools.contains_key(&tid) {
                        self.start_tool(&tid, &name, input, parent.as_deref(), out);
                    } else if name == "ExitPlanMode" {
                        self.capture_plan(&tid, &input, out);
                    }
                }
                "text" | "thinking" if !streamed && parent.is_none() => {
                    // Backfill: this message never streamed (partial messages
                    // off, or a replayed snapshot). One delta + completion.
                    let n = self.snapshot_blocks.entry(id.clone()).or_insert(0);
                    let item = format!("{id}:s{n}");
                    *n += 1;
                    let (kind, item_type, text) = if bt == "text" {
                        (
                            StreamKind::AssistantText,
                            ItemType::AssistantMessage,
                            str_at(&block, "text").unwrap_or_default(),
                        )
                    } else {
                        (
                            StreamKind::ReasoningSummaryText,
                            ItemType::Reasoning,
                            str_at(&block, "thinking").unwrap_or_default(),
                        )
                    };
                    if text.trim().is_empty() {
                        continue;
                    }
                    self.delta(kind, &item, text.clone(), out);
                    self.complete_text(item_type, &item, text, out);
                }
                _ => {}
            }
        }
    }

    fn on_user(&mut self, v: &Value, out: &mut Vec<Output>) {
        let parent = str_at(v, "parent_tool_use_id");
        if parent.is_none() {
            if let Some(uuid) = str_at(v, "uuid") {
                self.last_uuid = Some(uuid);
            }
        }
        let content = v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let result_meta = v.get("tool_use_result").cloned();
        for block in content {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            let Some(id) = str_at(&block, "tool_use_id") else {
                continue;
            };
            let is_error = block
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let text = tool_result_text(block.get("content"));
            if !self.tools.contains_key(&id) {
                self.start_tool(&id, "tool", json!({}), parent.as_deref(), out);
            }
            let tool = self.tools.get_mut(&id).expect("inserted above");
            tool.completed = true;
            let (item_type, turn, input, agent, parent_id, name) = (
                tool.item_type,
                tool.turn_id.clone(),
                tool.input.clone(),
                tool.agent_id.clone(),
                tool.parent.clone(),
                tool.name.clone(),
            );
            if item_type == ItemType::CommandExecution && parent_id.is_none() && !text.is_empty() {
                out.push(Output::Event(
                    self.ev(
                        turn.as_deref(),
                        RuntimeEventKind::ContentDelta(ContentDeltaPayload {
                            stream_kind: StreamKind::CommandOutput,
                            delta: clip(&text, OUTPUT_CLIP),
                            content_index: None,
                            summary_index: None,
                        }),
                    )
                    .with_item(&id),
                ));
            }
            let mut p = ItemPayload::new(item_type);
            p.status = Some(if is_error {
                ItemStatus::Failed
            } else {
                ItemStatus::Completed
            });
            p.tool_name = Some(name);
            let mut data = json!({"input": clip_value(&input), "output": clip(&text, OUTPUT_CLIP)});
            if let Some(meta) = result_meta.as_ref().filter(|m| m.is_object()) {
                data["result"] = clip_value(meta);
            }
            p.data = Some(data);
            p.agent_id = agent;
            p.parent_tool_use_id = parent_id;
            out.push(Output::Event(
                self.ev(turn.as_deref(), RuntimeEventKind::ItemCompleted(p))
                    .with_item(&id),
            ));
        }
    }

    fn close_open_items(&mut self, turn: Option<&str>, completed: bool) -> Vec<RuntimeEvent> {
        let mut out = Vec::new();
        let ids: Vec<String> = self
            .tools
            .iter()
            .filter(|(_, t)| !t.completed && t.turn_id.as_deref() == turn)
            .map(|(k, _)| k.clone())
            .collect();
        for id in ids {
            let tool = self.tools.get_mut(&id).expect("listed above");
            tool.completed = true;
            // A background agent outlives the turn that launched it.
            if tool.item_type == ItemType::CollabAgentToolCall && completed {
                continue;
            }
            let mut p = ItemPayload::new(tool.item_type);
            p.status = Some(if completed {
                ItemStatus::Completed
            } else {
                ItemStatus::Failed
            });
            p.tool_name = Some(tool.name.clone());
            let ev = RuntimeEvent::new(
                DRIVER,
                &self.instance_id,
                &self.thread_id,
                turn,
                RuntimeEventKind::ItemCompleted(p),
            )
            .with_item(id);
            out.push(ev);
        }
        self.requests.retain(|_, r| match r {
            OpenRequest::Approval { turn_id, .. } | OpenRequest::Question { turn_id, .. } => {
                turn_id.as_deref() != turn
            }
        });
        out
    }

    fn on_result(&mut self, v: &Value, out: &mut Vec<Output>) {
        // Which turns this result closes.
        let mut uuids: Vec<String> = v
            .get("user_message_uuids")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        if uuids.is_empty() {
            if let Some(u) = str_at(v, "user_message_uuid") {
                uuids.push(u);
            }
        }
        let mut turns: Vec<String> = uuids
            .iter()
            .filter_map(|u| self.prompts.get(u).cloned())
            .filter(|t| !self.completed.contains(t))
            .collect();
        if turns.is_empty() {
            if let Some(a) = self.active.clone().filter(|t| !self.completed.contains(t)) {
                turns.push(a);
            }
        }
        for u in &uuids {
            self.queue.retain(|q| q != u);
        }

        let usage = result_usage(v);
        let total = v.get("total_cost_usd").and_then(Value::as_f64);
        let turn_cost = total.map(|t| {
            let d = if t >= self.last_total_cost {
                t - self.last_total_cost
            } else {
                t
            };
            self.last_total_cost = t;
            d
        });
        let mut usage = usage;
        if let Some(u) = usage.as_mut() {
            u.cost_usd = turn_cost;
            if u.model.is_none() {
                u.model = self.model.clone();
            }
        }

        // Boundaries: every closed turn (and a synthetic tail) ends at the
        // latest transcript message.
        if let Some(last) = self.last_uuid.clone() {
            let targets: HashSet<&String> = turns.iter().collect();
            let mut touched = false;
            for mark in self.cursor.turns.iter_mut() {
                if targets.contains(&mark.turn_id) {
                    mark.last_uuid = Some(last.clone());
                    touched = true;
                }
            }
            if turns.is_empty() {
                if let Some(mark) = self.cursor.turns.last_mut() {
                    mark.last_uuid = Some(last.clone());
                    touched = true;
                }
            }
            self.cursor_dirty |= touched;
        }

        if turns.is_empty() {
            // Background output after the turn ended (sub-agent reply).
            if let Some(u) = usage {
                out.push(Output::Event(self.ev(
                    None,
                    RuntimeEventKind::UsageUpdated(UsageUpdatedPayload { usage: u }),
                )));
            }
            if self.cursor_dirty {
                self.cursor_dirty = false;
                if let Some(e) = self.cursor_event() {
                    out.push(Output::Event(e));
                }
            }
            return;
        }

        let (state, error) = self.outcome(v);
        if let (TurnEndState::Failed, Some(msg)) = (state, error.clone()) {
            let code = crate::core::llm::cli_runtime::parse::classify_error_text(&msg);
            out.push(Output::Event(
                self.ev(
                    Some(&turns[0]),
                    RuntimeEventKind::RuntimeError(RuntimeErrorPayload {
                        message: msg.clone(),
                        class: ErrorClass::ProviderError,
                        code: code
                            .map(str::to_string)
                            .or_else(|| str_at(v, "terminal_reason")),
                        detail: str_at(v, "subtype"),
                    }),
                ),
            ));
        }
        for (i, turn) in turns.iter().enumerate() {
            self.completed.insert(turn.clone());
            for e in self.close_open_items(Some(turn), state == TurnEndState::Completed) {
                out.push(Output::Event(e));
            }
            if self.started.insert(turn.clone()) {
                out.push(Output::Event(self.ev(
                    Some(turn),
                    RuntimeEventKind::TurnStarted(TurnStartedPayload {
                        model: self.model.clone(),
                        effort: None,
                    }),
                )));
            }
            out.push(Output::Event(self.ev(
                Some(turn),
                RuntimeEventKind::TurnCompleted(TurnCompletedPayload {
                    state,
                    stop_reason: str_at(v, "stop_reason").or_else(|| str_at(v, "terminal_reason")),
                    usage: if i == 0 { usage.clone() } else { None },
                    total_cost_usd: if i == 0 { turn_cost } else { None },
                    error_message: error.clone(),
                }),
            )));
            if self.active.as_deref() == Some(turn.as_str()) {
                self.active = None;
            }
        }
        self.interrupt_requested = false;
        self.failure_hint = None;
        self.cursor_dirty = false;
        if let Some(e) = self.cursor_event() {
            out.push(Output::Event(e));
        }
        if self.active.is_none() && self.queue.is_empty() {
            out.push(Output::Event(self.state(SessionState::Ready, None)));
        }
    }

    /// T3's `resultOutcome`, trimmed to what 2.1.280 sends.
    fn outcome(&self, v: &Value) -> (TurnEndState, Option<String>) {
        let subtype = str_at(v, "subtype").unwrap_or_default();
        let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
        let terminal = str_at(v, "terminal_reason").unwrap_or_default();
        let errors: Vec<String> = v
            .get("errors")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .filter(|s| !s.starts_with("[ede_diagnostic]"))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let joined = errors.join("; ");
        if matches!(terminal.as_str(), "aborted_streaming" | "aborted_tools")
            || joined.to_ascii_lowercase().contains("interrupt")
            || (self.interrupt_requested && subtype == "error_during_execution")
        {
            return (TurnEndState::Interrupted, Some("Interrupted".into()));
        }
        if subtype == "success" && !is_error {
            if v.get("api_error_status").and_then(Value::as_u64) == Some(529) {
                return (
                    TurnEndState::Failed,
                    Some("Claude API is overloaded (529). Try again shortly.".into()),
                );
            }
            return (TurnEndState::Completed, None);
        }
        let message = if !joined.is_empty() {
            joined
        } else if let Some(hint) = &self.failure_hint {
            hint.clone()
        } else if let Some(r) = str_at(v, "result").filter(|r| !r.trim().is_empty()) {
            r
        } else if !terminal.is_empty() && terminal != "completed" {
            format!("Claude ended the turn: {terminal}")
        } else {
            format!("Claude ended with `{subtype}`")
        };
        if message.to_ascii_lowercase().contains("cancel") {
            return (TurnEndState::Cancelled, Some(message));
        }
        (TurnEndState::Failed, Some(message))
    }

    fn on_rate_limit(&mut self, v: &Value, out: &mut Vec<Output>) {
        let info = v.get("rate_limit_info").unwrap_or(v);
        let mut windows = Vec::new();
        let unified = info
            .get("unifiedWindows")
            .or_else(|| info.get("unified_windows"));
        for (id, label, minutes) in [
            ("five_hour", "Session", 300u64),
            ("seven_day", "Weekly", 10_080),
        ] {
            if let Some(w) = unified.and_then(|u| u.get(id)) {
                windows.push(window(id, label, Some(minutes), w, false));
            }
        }
        let bucket = str_at(info, "rateLimitType");
        if let Some(b) = bucket.as_deref() {
            if !windows.iter().any(|w| w.id == b) && info.get("utilization").is_some() {
                windows.push(window(b, &bucket_label(b), None, info, false));
            }
        }
        let credits = json!({
            "status": info.get("status"),
            "overageStatus": info.get("overageStatus"),
            "isUsingOverage": info.get("isUsingOverage"),
        });
        out.push(Output::Event(self.ev(
            self.cur().as_deref(),
            RuntimeEventKind::RateLimitsUpdated(RateLimitsPayload {
                windows,
                credits: Some(credits),
            }),
        )));
        if str_at(info, "status").as_deref() == Some("rejected") {
            let resets = info.get("resetsAt").and_then(epoch_to_iso);
            let key = format!(
                "{}:{}",
                bucket.clone().unwrap_or_default(),
                resets.clone().unwrap_or_default()
            );
            if self.rate_warned.insert(key) {
                out.push(Output::Event(self.ev(
                    self.cur().as_deref(),
                    RuntimeEventKind::RuntimeWarning(RuntimeWarningPayload {
                        message: match &resets {
                            Some(at) => format!("Claude usage limit reached. The turn is paused until the limit resets ({at})."),
                            None => "Claude usage limit reached. The turn is paused until the limit resets.".into(),
                        },
                        detail: bucket,
                    }),
                )));
            }
        }
    }

    fn on_control_response(&mut self, v: &Value, out: &mut Vec<Output>) {
        let r = v.get("response").cloned().unwrap_or(Value::Null);
        let request_id = str_at(&r, "request_id").unwrap_or_default();
        let ok = str_at(&r, "subtype").as_deref() == Some("success");
        let payload = if ok {
            r.get("response").cloned().unwrap_or(Value::Null)
        } else {
            json!({"error": r.get("error")})
        };
        if ok && payload.get("commands").is_some() && payload.get("models").is_some() {
            // initialize: capabilities of this CLI build. The account e-mail
            // is not forwarded.
            if let Some(mode) = str_at(&payload, "current_permission_mode") {
                self.permission_mode = Some(mode);
            }
            let value = json!({
                "pid": payload.get("pid"),
                "permissionMode": payload.get("current_permission_mode"),
                "subscriptionType": payload.get("account").and_then(|a| a.get("subscriptionType")),
                "apiProvider": payload.get("account").and_then(|a| a.get("apiProvider")),
                "outputStyle": payload.get("output_style"),
                "models": payload.get("models"),
                "commands": payload.get("commands").and_then(Value::as_array).map(|a| {
                    a.iter().filter_map(|c| c.get("name").cloned()).collect::<Vec<_>>()
                }),
            });
            out.push(Output::Event(self.ev(
                None,
                RuntimeEventKind::SessionConfigured(ValuePayload { value }),
            )));
        }
        if ok && payload.get("rate_limits").is_some() {
            if let Some(p) = usage_windows(&payload) {
                out.push(Output::Event(
                    self.ev(None, RuntimeEventKind::RateLimitsUpdated(p)),
                ));
            }
        }
        out.push(Output::ControlResult {
            request_id,
            ok,
            payload,
        });
    }

    fn on_control_cancel(&mut self, v: &Value, out: &mut Vec<Output>) {
        let Some(id) = str_at(v, "request_id") else {
            return;
        };
        match self.requests.remove(&id) {
            Some(OpenRequest::Approval {
                turn_id,
                request_type,
                ..
            }) => out.push(Output::Event(
                self.ev(
                    turn_id.as_deref(),
                    RuntimeEventKind::RequestResolved(RequestResolvedPayload {
                        request_type,
                        decision: None,
                        resolution: Some("cancelled".into()),
                    }),
                )
                .with_request(id),
            )),
            Some(OpenRequest::Question { turn_id, .. }) => out.push(Output::Event(
                self.ev(
                    turn_id.as_deref(),
                    RuntimeEventKind::UserInputResolved(UserInputResolvedPayload {
                        answers: json!({}),
                    }),
                )
                .with_request(id),
            )),
            None => {}
        }
    }

    fn on_control_request(
        &mut self,
        v: &Value,
        read_file: &dyn Fn(&Path) -> Option<String>,
        out: &mut Vec<Output>,
    ) {
        let request_id = str_at(v, "request_id").unwrap_or_default();
        let r = v.get("request").cloned().unwrap_or(Value::Null);
        let sub = str_at(&r, "subtype").unwrap_or_default();
        if sub != "can_use_tool" {
            // hook_callback, mcp_message, elicitation, …: nothing registered
            // on our side. An error answer keeps the CLI from waiting.
            out.push(Output::Send(protocol::control_error(
                &request_id,
                &format!("`{sub}` is not handled by OmniGet"),
            )));
            return;
        }
        self.ensure_active(out);
        let tool_name = str_at(&r, "tool_name").unwrap_or_default();
        let input = r.get("input").cloned().unwrap_or(json!({}));
        let tool_use_id = str_at(&r, "tool_use_id");
        let turn = tool_use_id
            .as_ref()
            .and_then(|t| self.tools.get(t))
            .and_then(|t| t.turn_id.clone())
            .or_else(|| self.cur());
        let refs = ProviderRefs {
            provider_item_id: tool_use_id.clone(),
            provider_request_id: Some(request_id.clone()),
            ..Default::default()
        };

        if tool_name == "ExitPlanMode" {
            if let Some(id) = &tool_use_id {
                self.capture_plan(id, &input, out);
            } else {
                self.capture_plan(&request_id, &input, out);
            }
            out.push(Output::Send(protocol::control_success(
                &request_id,
                json!({"behavior": "deny", "message": protocol::PLAN_CAPTURED_MESSAGE}),
            )));
            return;
        }

        if tool_name == "AskUserQuestion" {
            let questions = parse_questions(&input);
            self.requests.insert(
                request_id.clone(),
                OpenRequest::Question {
                    turn_id: turn.clone(),
                    tool_use_id: tool_use_id.clone(),
                    input: input.clone(),
                    questions: questions.clone(),
                },
            );
            let mut ev = self
                .ev(
                    turn.as_deref(),
                    RuntimeEventKind::UserInputRequested(UserInputRequestedPayload {
                        questions,
                        response_mode: None,
                    }),
                )
                .with_request(&request_id);
            ev.provider_refs = Some(refs);
            out.push(Output::Event(ev));
            return;
        }

        let request_type = classify_request(&tool_name);
        let suggestions: Vec<Value> = r
            .get("permission_suggestions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut detail = RequestDetail {
            tool_name: Some(tool_name.clone()),
            cwd: self.cwd.as_ref().map(|p| p.display().to_string()),
            reason: str_at(&r, "decision_reason").or_else(|| str_at(&input, "description")),
            preview: str_at(&r, "description").or_else(|| tool_summary(&tool_name, &input)),
            input: Some(clip_value(&input)),
            ..Default::default()
        };
        if matches!(request_type, RequestType::CommandExecutionApproval) {
            detail.command = str_at(&input, "command");
        }
        if let Some(path) = diff::target_path(&input) {
            let abs = self.absolute(&path);
            if matches!(tool_name.as_str(), "Write" | "Edit" | "MultiEdit") {
                let current = read_file(&abs);
                detail.diff = diff::preview_edit(&tool_name, &input, current.as_deref());
            }
            detail.paths.push(abs.display().to_string());
        }
        if let Some(blocked) = str_at(&r, "blocked_path") {
            if !detail.paths.contains(&blocked) {
                detail.paths.push(blocked);
            }
        }
        self.requests.insert(
            request_id.clone(),
            OpenRequest::Approval {
                turn_id: turn.clone(),
                tool_name: tool_name.clone(),
                tool_use_id: tool_use_id.clone(),
                input: input.clone(),
                suggestions: suggestions.clone(),
                request_type,
            },
        );
        let mut ev = self
            .ev(
                turn.as_deref(),
                RuntimeEventKind::RequestOpened(RequestOpenedPayload {
                    request_type,
                    detail: Some(detail),
                    app_name: Some("Claude Code".into()),
                    options: approval_options(),
                    args: Some(json!({
                        "toolName": tool_name,
                        "toolUseId": tool_use_id,
                        "suggestions": suggestions,
                    })),
                }),
            )
            .with_request(&request_id);
        ev.provider_refs = Some(refs);
        out.push(Output::Event(ev));
    }

    fn absolute(&self, path: &str) -> PathBuf {
        let p = PathBuf::from(path);
        if p.is_absolute() {
            p
        } else {
            match &self.cwd {
                Some(c) => c.join(p),
                None => p,
            }
        }
    }
}

// ── helpers ─────────────────────────────────────────────────────────────

fn str_at(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn answer_text(v: &Value) -> Option<String> {
    ["answer", "value", "label"]
        .iter()
        .find_map(|k| v.get(*k).and_then(Value::as_str))
        .map(str::to_string)
}

pub fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut cut = s.len() - max;
    while !s.is_char_boundary(cut) {
        cut += 1;
    }
    format!("…{}", &s[cut..])
}

fn clip_value(v: &Value) -> Value {
    let text = v.to_string();
    if text.len() <= INPUT_CLIP {
        return v.clone();
    }
    match v {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, x)| {
                    let x = match x {
                        Value::String(s) if s.len() > 4096 => {
                            Value::String(format!("{}… ({} bytes)", &s[..floor(s, 4096)], s.len()))
                        }
                        other if other.to_string().len() > 8192 => json!("(clipped)"),
                        other => other.clone(),
                    };
                    (k.clone(), x)
                })
                .collect(),
        ),
        _ => json!({"clipped": clip(&text, INPUT_CLIP)}),
    }
}

fn floor(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn collect_text(msg: &Value) -> String {
    msg.get("content")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn tool_result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| match p.get("type").and_then(Value::as_str) {
                Some("text") => p.get("text").and_then(Value::as_str).map(str::to_string),
                Some("tool_reference") => p
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .map(|n| format!("[{n}]")),
                Some("image") => Some("[image]".into()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Canonical item type of a Claude tool.
pub fn classify_tool(name: &str, input: &Value) -> ItemType {
    let lower = name.to_ascii_lowercase();
    if lower == "read" {
        let path = input
            .get("file_path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        if [".png", ".jpg", ".jpeg", ".gif", ".webp"]
            .iter()
            .any(|e| path.ends_with(e))
        {
            return ItemType::ImageView;
        }
    }
    if matches!(lower.as_str(), "agent" | "task") || lower.contains("subagent") {
        ItemType::CollabAgentToolCall
    } else if matches!(
        lower.as_str(),
        "bash" | "bashoutput" | "killshell" | "powershell"
    ) || lower.contains("shell")
        || lower.contains("terminal")
    {
        ItemType::CommandExecution
    } else if matches!(
        lower.as_str(),
        "edit" | "write" | "multiedit" | "notebookedit"
    ) {
        ItemType::FileChange
    } else if lower.starts_with("mcp__") {
        ItemType::McpToolCall
    } else if matches!(lower.as_str(), "websearch" | "webfetch") {
        ItemType::WebSearch
    } else {
        ItemType::DynamicToolCall
    }
}

fn title_of(t: ItemType) -> &'static str {
    match t {
        ItemType::CommandExecution => "Command run",
        ItemType::FileChange => "File change",
        ItemType::McpToolCall => "MCP tool call",
        ItemType::CollabAgentToolCall => "Subagent task",
        ItemType::WebSearch => "Web search",
        ItemType::ImageView => "Image view",
        _ => "Tool call",
    }
}

/// One-line summary of a tool call for the timeline.
pub fn tool_summary(name: &str, input: &Value) -> Option<String> {
    let s = |k: &str| input.get(k).and_then(Value::as_str).map(str::to_string);
    let text = match name {
        "Bash" | "PowerShell" => s("command"),
        "Read" | "Write" | "Edit" | "MultiEdit" => s("file_path"),
        "NotebookEdit" => s("notebook_path"),
        "Grep" | "Glob" => s("pattern"),
        "WebFetch" => s("url"),
        "WebSearch" => s("query"),
        "Agent" | "Task" => s("description"),
        "Skill" => s("skill"),
        _ => None,
    }?;
    Some(clip_head(&text, 400))
}

fn clip_head(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..floor(s, max)])
    }
}

/// `request.opened.requestType` by tool name (T3's `classifyRequestType`).
pub fn classify_request(name: &str) -> RequestType {
    let lower = name.to_ascii_lowercase();
    if ["read", "view", "grep", "glob", "search", "ls"]
        .iter()
        .any(|k| lower == *k || lower.ends_with(k))
        && !lower.contains("web")
    {
        RequestType::FileReadApproval
    } else if ["bash", "command", "shell", "terminal"]
        .iter()
        .any(|k| lower.contains(k))
    {
        RequestType::CommandExecutionApproval
    } else if [
        "edit", "write", "file", "patch", "replace", "create", "delete",
    ]
    .iter()
    .any(|k| lower.contains(k))
    {
        RequestType::FileChangeApproval
    } else {
        RequestType::DynamicToolCall
    }
}

pub fn approval_options() -> Vec<ApprovalOption> {
    [
        ("accept", "Allow", ApprovalDecision::Accept),
        (
            "acceptForSession",
            "Allow for this session",
            ApprovalDecision::AcceptForSession,
        ),
        (
            "acceptAlways",
            "Always allow",
            ApprovalDecision::AcceptAlways,
        ),
        ("decline", "Deny", ApprovalDecision::Decline),
        ("cancel", "Deny and stop", ApprovalDecision::Cancel),
    ]
    .into_iter()
    .map(|(id, label, d)| ApprovalOption {
        id: id.into(),
        label: label.into(),
        decision: Some(d),
    })
    .collect()
}

fn parse_questions(input: &Value) -> Vec<UserInputQuestion> {
    input
        .get("questions")
        .and_then(Value::as_array)
        .map(|qs| {
            qs.iter()
                .enumerate()
                .map(|(i, q)| {
                    let question = str_at(q, "question").unwrap_or_default();
                    UserInputQuestion {
                        // The CLI looks answers up by question text.
                        id: if question.is_empty() {
                            format!("q-{i}")
                        } else {
                            question.clone()
                        },
                        header: str_at(q, "header").unwrap_or_default(),
                        question,
                        options: q
                            .get("options")
                            .and_then(Value::as_array)
                            .map(|os| {
                                os.iter()
                                    .map(|o| UserInputOption {
                                        label: str_at(o, "label").unwrap_or_default(),
                                        description: str_at(o, "description"),
                                        value: None,
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        allow_custom_answer: true,
                        multi_select: q
                            .get("multiSelect")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn result_usage(v: &Value) -> Option<TokenUsage> {
    let u = v.get("usage")?;
    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
    let reasoning = u
        .get("output_tokens_details")
        .and_then(|d| d.get("thinking_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(n("output_tokens"));
    // Context in use = the last API iteration of the turn.
    let used = u
        .get("iterations")
        .and_then(Value::as_array)
        .and_then(|a| a.last())
        .map(|it| {
            let g = |k: &str| it.get(k).and_then(Value::as_u64).unwrap_or(0);
            g("input_tokens")
                + g("cache_read_input_tokens")
                + g("cache_creation_input_tokens")
                + g("output_tokens")
        });
    let model_usage = v.get("modelUsage").and_then(Value::as_object);
    let max = model_usage.and_then(|m| {
        m.values()
            .filter_map(|x| x.get("contextWindow").and_then(Value::as_u64))
            .max()
    });
    let model = model_usage.and_then(|m| m.keys().next().cloned());
    Some(TokenUsage {
        input_tokens: n("input_tokens"),
        cached_input_tokens: n("cache_read_input_tokens"),
        cache_write_tokens: n("cache_creation_input_tokens"),
        output_tokens: n("output_tokens"),
        reasoning_output_tokens: reasoning,
        used_tokens: used.filter(|u| *u > 0),
        max_tokens: max,
        cost_usd: None,
        model,
        duration_ms: v.get("duration_ms").and_then(Value::as_u64),
        tool_uses: None,
        usage_status: Some(
            if str_at(v, "subtype").as_deref() == Some("success") {
                "complete"
            } else {
                "partial"
            }
            .into(),
        ),
    })
}

fn window(
    id: &str,
    label: &str,
    minutes: Option<u64>,
    w: &Value,
    percent_scale: bool,
) -> RateLimitWindow {
    let util = w.get("utilization").and_then(Value::as_f64);
    RateLimitWindow {
        id: id.to_string(),
        label: label.to_string(),
        used_percent: util.map(|u| {
            if percent_scale || u > 1.0 {
                u
            } else {
                u * 100.0
            }
        }),
        window_minutes: minutes,
        resets_at: w
            .get("resetsAt")
            .or_else(|| w.get("resets_at"))
            .and_then(epoch_to_iso),
    }
}

fn bucket_label(id: &str) -> String {
    match id {
        "five_hour" => "Session".into(),
        "seven_day" => "Weekly".into(),
        "seven_day_opus" => "Weekly · Opus".into(),
        "seven_day_sonnet" => "Weekly · Sonnet".into(),
        "overage" => "Extra usage".into(),
        other => other.replace('_', " "),
    }
}

/// `get_usage` answer (utilization 0..100, ISO resets). Opt-in only.
fn usage_windows(payload: &Value) -> Option<RateLimitsPayload> {
    let limits = payload.get("rate_limits")?;
    let mut windows = Vec::new();
    for (id, label, minutes) in [
        ("five_hour", "Session", Some(300u64)),
        ("seven_day", "Weekly", Some(10_080)),
        ("seven_day_opus", "Weekly · Opus", Some(10_080)),
        ("seven_day_sonnet", "Weekly · Sonnet", Some(10_080)),
    ] {
        if let Some(w) = limits.get(id).filter(|w| w.is_object()) {
            windows.push(window(id, label, minutes, w, true));
        }
    }
    if windows.is_empty() {
        return None;
    }
    Some(RateLimitsPayload {
        windows,
        credits: payload
            .get("subscription_type")
            .map(|s| json!({"subscriptionType": s})),
    })
}

fn epoch_to_iso(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return Some(s.to_string());
    }
    let n = v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))?;
    let secs = if n > 100_000_000_000 { n / 1000 } else { n };
    chrono::DateTime::from_timestamp(secs, 0)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}
