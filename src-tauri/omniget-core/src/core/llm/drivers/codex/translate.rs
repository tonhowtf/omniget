//! Codex app-server → [`RuntimeEvent`] translation. Pure: no I/O, no clock
//! except the event timestamps, so every rule is tested against fixtures.
//!
//! Mapping source: T3 `CodexAdapter.ts` (estudo 76, 01-server.md §4-§6), the
//! JSON Schema of codex-cli 0.156.0 (`protocol.rs`) and the live wire of that
//! version (`fixtures/app-server-*.jsonl`).
//!
//! Ids: `item_id` is Codex's own item id (stable across started/delta/
//! completed); `turn_id` is the engine's turn id, bound to Codex's turn id when
//! `turn/start` answers or `turn/started` arrives; `request_id` is a fresh
//! UUID per server request (Codex's JSON-RPC ids restart at 0 with every
//! process, so they cannot key a durable approval).

use std::collections::{BTreeMap, HashMap};

use serde_json::{json, Map, Value};

use super::super::{
    ApprovalDecision, ApprovalOption, ContentDeltaPayload, DiffUpdatedPayload, DriverError,
    ErrorClass, HookPayload, ItemPayload, ItemStatus, ItemType, McpOauthPayload,
    ModelReroutedPayload, NoticePayload, PlanStep, PlanUpdatedPayload, ProposedCompletedPayload,
    ProviderRefs, RateLimitWindow, RateLimitsPayload, RawPayload, RequestDetail,
    RequestOpenedPayload, RequestResolvedPayload, RequestType, RuntimeErrorPayload, RuntimeEvent,
    RuntimeEventKind, RuntimeWarningPayload, StreamKind, TaskPayload, ThreadMetadataPayload,
    ThreadStatePayload, TokenUsage, ToolProgressPayload, TurnAbortedPayload, TurnCompletedPayload,
    TurnEndState, TurnStartedPayload, UsageUpdatedPayload, UserInputOption, UserInputQuestion,
    UserInputRequestedPayload, UserInputResolvedPayload, ValuePayload, ERR_DRIVER_NO_SESSION,
};
use super::protocol as p;
use super::rpc::CODE_METHOD_NOT_FOUND;

pub const DRIVER: &str = "codex";
/// Longest tool output kept in an activity (`data.output`). The full text
/// already streamed as `content.delta`.
const OUTPUT_CLIP: usize = 16 * 1024;
const DIFF_CLIP: usize = 256 * 1024;

/// What kind of server request a pending approval answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingKind {
    Command,
    FileChange,
    Permissions,
    Elicitation,
    UserInput,
    LegacyPatch,
    LegacyExec,
}

impl PendingKind {
    fn request_type(self) -> RequestType {
        match self {
            PendingKind::Command => RequestType::CommandExecutionApproval,
            PendingKind::FileChange => RequestType::FileChangeApproval,
            PendingKind::Permissions => RequestType::PermissionApproval,
            PendingKind::Elicitation => RequestType::McpElicitationApproval,
            PendingKind::UserInput => RequestType::ToolUserInput,
            PendingKind::LegacyPatch => RequestType::ApplyPatchApproval,
            PendingKind::LegacyExec => RequestType::ExecCommandApproval,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PendingRequest {
    pub rpc_id: Value,
    pub kind: PendingKind,
    pub turn_id: Option<String>,
    /// The request params, needed to build some answers (permissions echo the
    /// requested profile; elicitation fills the form).
    pub params: Value,
}

/// A server request the driver answers by itself, right away.
#[derive(Debug, Clone, PartialEq)]
pub enum AutoReply {
    Result(Value),
    Error { code: i64, message: String },
}

#[derive(Debug, Default)]
pub struct ServerRequestOutcome {
    pub events: Vec<RuntimeEvent>,
    /// `Some` when nothing waits for the user.
    pub reply: Option<AutoReply>,
}

/// One JSON-RPC answer to send: `(rpc id, result)`.
pub type Answer = (Value, Value);

#[derive(Debug, Clone, Default)]
struct ItemCache {
    diff: Option<String>,
    paths: Vec<String>,
    command: Option<String>,
    cwd: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct RateView {
    primary: Option<p::RateLimitWindow>,
    secondary: Option<p::RateLimitWindow>,
    credits: Option<p::CreditsSnapshot>,
    plan_type: Option<p::PlanType>,
    reached: Option<p::RateLimitReachedType>,
    reset_credits: Option<Value>,
}

/// Per-thread translation state.
#[derive(Debug)]
pub struct Translator {
    instance_id: String,
    thread_id: String,
    provider_thread: Option<String>,
    model: Option<String>,
    /// Engine turn waiting for Codex's turn id.
    pending_engine_turn: Option<String>,
    /// Codex turn id → engine turn id.
    turn_map: HashMap<String, String>,
    active_provider_turn: Option<String>,
    active_engine_turn: Option<String>,
    turn_error_sent: bool,
    items: HashMap<String, ItemCache>,
    pending: HashMap<String, PendingRequest>,
    /// Codex rpc id (as string) → our request id, for `serverRequest/resolved`.
    by_rpc: HashMap<String, String>,
    usage_base: Option<p::TokenUsageBreakdown>,
    usage_last: Option<p::ThreadTokenUsage>,
    rates: RateView,
    /// Live turns of sub-agent threads (child thread → turn), for Stop.
    child_turns: BTreeMap<String, String>,
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n… ({} bytes more)", &s[..end], s.len() - end)
}

fn decode<T: serde::de::DeserializeOwned>(v: &Value) -> Option<T> {
    match serde_json::from_value(v.clone()) {
        Ok(t) => Some(t),
        Err(e) => {
            tracing::debug!("[codex] params did not decode: {e}");
            None
        }
    }
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// `--- a/x` / `+++ b/x` headers around Codex's per-file diff, unless it
/// already has them.
pub fn file_diff(path: &str, kind: &str, move_path: Option<&str>, body: &str) -> String {
    if body.starts_with("--- ") || body.starts_with("diff ") {
        return body.to_string();
    }
    let (from, to) = match kind {
        "add" => ("/dev/null".to_string(), format!("b/{path}")),
        "delete" => (format!("a/{path}"), "/dev/null".to_string()),
        _ => (
            format!("a/{path}"),
            format!("b/{}", move_path.unwrap_or(path)),
        ),
    };
    let mut out = format!("--- {from}\n+++ {to}\n");
    if body.contains("@@") || body.is_empty() {
        out.push_str(body);
    } else {
        // A whole file (add/delete): make it a hunk.
        let lines: Vec<&str> = body.lines().collect();
        let sign = if kind == "delete" { '-' } else { '+' };
        let n = lines.len();
        if kind == "delete" {
            out.push_str(&format!("@@ -1,{n} +0,0 @@\n"));
        } else {
            out.push_str(&format!("@@ -0,0 +1,{n} @@\n"));
        }
        for l in lines {
            out.push(sign);
            out.push_str(l);
            out.push('\n');
        }
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn change_kind(kind: &p::PatchChangeKind) -> (&'static str, Option<String>) {
    match kind {
        p::PatchChangeKind::Add {} => ("add", None),
        p::PatchChangeKind::Delete {} => ("delete", None),
        p::PatchChangeKind::Update { move_path } => ("update", move_path.clone()),
        p::PatchChangeKind::Unknown => ("update", None),
    }
}

fn changes_diff(changes: &[p::FileUpdateChange]) -> (String, Vec<String>) {
    let mut diff = String::new();
    let mut paths = Vec::new();
    for c in changes {
        let (kind, mv) = change_kind(&c.kind);
        diff.push_str(&file_diff(&c.path, kind, mv.as_deref(), &c.diff));
        paths.push(c.path.clone());
    }
    (clip(&diff, DIFF_CLIP), paths)
}

fn legacy_changes_diff(changes: &BTreeMap<String, p::FileChange>) -> (String, Vec<String>) {
    let mut diff = String::new();
    let mut paths = Vec::new();
    for (path, c) in changes {
        let piece = match c {
            p::FileChange::Add { content } => file_diff(path, "add", None, content),
            p::FileChange::Delete { content } => file_diff(path, "delete", None, content),
            p::FileChange::Update {
                move_path,
                unified_diff,
            } => file_diff(path, "update", move_path.as_deref(), unified_diff),
            p::FileChange::Unknown => String::new(),
        };
        diff.push_str(&piece);
        paths.push(path.clone());
    }
    (clip(&diff, DIFF_CLIP), paths)
}

fn epoch_to_iso(secs: i64) -> Option<String> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0)
        .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

fn window_label(minutes: Option<i64>) -> String {
    match minutes {
        Some(m) if m >= 28 * 24 * 60 => "Monthly".into(),
        Some(m) if m >= 7 * 24 * 60 => "Weekly".into(),
        Some(m) if m >= 24 * 60 => format!("{} d", m / (24 * 60)),
        Some(m) if m >= 60 => format!("{} h", m / 60),
        Some(m) => format!("{m} min"),
        None => String::new(),
    }
}

/// "5d 5h", "3h 20m", "12m".
pub fn format_wait(secs: i64) -> String {
    let secs = secs.max(0);
    let d = secs / 86_400;
    let h = (secs % 86_400) / 3600;
    let m = (secs % 3600) / 60;
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{}m", m.max(1))
    }
}

/// Error class and stable code for a Codex turn error. `info` is the raw
/// `codexErrorInfo` (string or single-key object), `message` the text.
pub fn classify_error(info: Option<&Value>, message: &str) -> (ErrorClass, &'static str) {
    let status = info
        .and_then(Value::as_object)
        .and_then(|o| o.values().next())
        .and_then(|v| v.get("httpStatusCode"))
        .and_then(Value::as_u64);
    let unauthorized = matches!(status, Some(401) | Some(403))
        || message.contains("401 Unauthorized")
        || message.contains("403 Forbidden")
        || message.contains("Missing bearer");
    let tag = match info {
        Some(Value::String(s)) => s.as_str(),
        Some(Value::Object(o)) => o.keys().next().map(String::as_str).unwrap_or(""),
        _ => "",
    };
    if tag == "unauthorized" || unauthorized {
        return (ErrorClass::ProviderError, "ERR_CODEX_AUTH");
    }
    match tag {
        "usageLimitExceeded" => (ErrorClass::ProviderError, "ERR_CODEX_USAGE_LIMIT"),
        "rateLimitExceeded" => (ErrorClass::ProviderError, "ERR_CODEX_RATE_LIMIT"),
        "serverOverloaded" => (ErrorClass::ProviderError, "ERR_CODEX_OVERLOADED"),
        "internalServerError" => (ErrorClass::ProviderError, "ERR_CODEX_SERVER"),
        "contextWindowExceeded" | "sessionBudgetExceeded" => {
            (ErrorClass::ValidationError, "ERR_CODEX_CONTEXT")
        }
        "badRequest" | "activeTurnNotSteerable" => {
            (ErrorClass::ValidationError, "ERR_CODEX_BAD_REQUEST")
        }
        "sandboxError" => (ErrorClass::PermissionError, "ERR_CODEX_SANDBOX"),
        "cyberPolicy" | "misalignmentPolicyViolation" => {
            (ErrorClass::ProviderError, "ERR_CODEX_POLICY")
        }
        "threadRollbackFailed" => (ErrorClass::ProviderError, "ERR_CODEX_ROLLBACK"),
        "httpConnectionFailed"
        | "responseStreamConnectionFailed"
        | "responseStreamDisconnected"
        | "responseTooManyFailedAttempts" => (ErrorClass::TransportError, "ERR_CODEX_TRANSPORT"),
        _ => (ErrorClass::ProviderError, "ERR_CODEX"),
    }
}

fn usage_of(b: &p::TokenUsageBreakdown, window: Option<i64>, model: Option<&str>) -> TokenUsage {
    let n = |x: i64| x.max(0) as u64;
    let cached = n(b.cached_input_tokens);
    TokenUsage {
        // Codex counts cached tokens inside inputTokens (OpenAI semantics);
        // OmniGet's TokenUsage keeps them apart.
        input_tokens: n(b.input_tokens).saturating_sub(cached),
        cached_input_tokens: cached,
        cache_write_tokens: n(b.cache_write_input_tokens.unwrap_or(0)),
        output_tokens: n(b.output_tokens),
        reasoning_output_tokens: n(b.reasoning_output_tokens),
        used_tokens: Some(n(b.total_tokens)),
        max_tokens: window.map(n),
        cost_usd: None,
        model: model.map(str::to_string),
        duration_ms: None,
        tool_uses: None,
        usage_status: Some("complete".into()),
    }
}

fn sub(a: &p::TokenUsageBreakdown, b: &p::TokenUsageBreakdown) -> p::TokenUsageBreakdown {
    p::TokenUsageBreakdown {
        cache_write_input_tokens: Some(
            a.cache_write_input_tokens.unwrap_or(0) - b.cache_write_input_tokens.unwrap_or(0),
        ),
        cached_input_tokens: a.cached_input_tokens - b.cached_input_tokens,
        input_tokens: a.input_tokens - b.input_tokens,
        output_tokens: a.output_tokens - b.output_tokens,
        reasoning_output_tokens: a.reasoning_output_tokens - b.reasoning_output_tokens,
        total_tokens: a.total_tokens - b.total_tokens,
    }
}

fn status_of_command(s: &p::CommandExecutionStatus) -> ItemStatus {
    match s {
        p::CommandExecutionStatus::Completed => ItemStatus::Completed,
        p::CommandExecutionStatus::Failed => ItemStatus::Failed,
        p::CommandExecutionStatus::Declined => ItemStatus::Declined,
        _ => ItemStatus::InProgress,
    }
}

fn status_of_patch(s: &p::PatchApplyStatus) -> ItemStatus {
    match s {
        p::PatchApplyStatus::Completed => ItemStatus::Completed,
        p::PatchApplyStatus::Failed => ItemStatus::Failed,
        p::PatchApplyStatus::Declined => ItemStatus::Declined,
        _ => ItemStatus::InProgress,
    }
}

fn status_of_mcp(s: &p::McpToolCallStatus) -> ItemStatus {
    match s {
        p::McpToolCallStatus::Completed => ItemStatus::Completed,
        p::McpToolCallStatus::Failed => ItemStatus::Failed,
        _ => ItemStatus::InProgress,
    }
}

fn status_of_dynamic(s: &p::DynamicToolCallStatus) -> ItemStatus {
    match serde_json::to_value(s)
        .ok()
        .as_ref()
        .and_then(Value::as_str)
    {
        Some("completed") => ItemStatus::Completed,
        Some("failed") => ItemStatus::Failed,
        _ => ItemStatus::InProgress,
    }
}

fn opt(id: &str, label: &str, d: ApprovalDecision) -> ApprovalOption {
    ApprovalOption {
        id: id.into(),
        label: label.into(),
        decision: Some(d),
    }
}

fn approval_options(session: bool) -> Vec<ApprovalOption> {
    let mut v = vec![opt("accept", "Approve", ApprovalDecision::Accept)];
    if session {
        v.push(opt(
            "acceptForSession",
            "Approve for this session",
            ApprovalDecision::AcceptForSession,
        ));
    }
    v.push(opt("decline", "Decline", ApprovalDecision::Decline));
    v.push(opt("cancel", "Stop the turn", ApprovalDecision::Cancel));
    v
}

impl Translator {
    pub fn new(instance_id: &str, thread_id: &str) -> Self {
        Self {
            instance_id: instance_id.to_string(),
            thread_id: thread_id.to_string(),
            provider_thread: None,
            model: None,
            pending_engine_turn: None,
            turn_map: HashMap::new(),
            active_provider_turn: None,
            active_engine_turn: None,
            turn_error_sent: false,
            items: HashMap::new(),
            pending: HashMap::new(),
            by_rpc: HashMap::new(),
            usage_base: None,
            usage_last: None,
            rates: RateView::default(),
            child_turns: BTreeMap::new(),
        }
    }

    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    pub fn provider_thread(&self) -> Option<&str> {
        self.provider_thread.as_deref()
    }

    pub fn set_provider_thread(&mut self, id: &str) {
        self.provider_thread = Some(id.to_string());
    }

    pub fn set_model(&mut self, model: Option<String>) {
        if model.is_some() {
            self.model = model;
        }
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    pub fn active_provider_turn(&self) -> Option<&str> {
        self.active_provider_turn.as_deref()
    }

    pub fn active_engine_turn(&self) -> Option<&str> {
        self.active_engine_turn
            .as_deref()
            .or(self.pending_engine_turn.as_deref())
    }

    pub fn child_turns(&self) -> Vec<(String, String)> {
        self.child_turns
            .iter()
            .map(|(a, b)| (a.clone(), b.clone()))
            .collect()
    }

    pub fn has_pending(&self, request_id: &str) -> bool {
        self.pending.contains_key(request_id)
    }

    fn ev(&self, turn: Option<&str>, kind: RuntimeEventKind) -> RuntimeEvent {
        RuntimeEvent::new(DRIVER, &self.instance_id, &self.thread_id, turn, kind)
    }

    fn refs(mut ev: RuntimeEvent, turn: Option<&str>, item: Option<&str>) -> RuntimeEvent {
        if turn.is_some() || item.is_some() {
            ev.provider_refs = Some(ProviderRefs {
                provider_turn_id: turn.map(str::to_string),
                provider_item_id: item.map(str::to_string),
                provider_request_id: None,
            });
        }
        ev
    }

    /// A turn the engine just asked for; its Codex id comes later.
    pub fn begin_turn(&mut self, engine_turn: &str) {
        self.pending_engine_turn = Some(engine_turn.to_string());
        self.active_engine_turn = Some(engine_turn.to_string());
        self.active_provider_turn = None;
        self.turn_error_sent = false;
        self.usage_last = None;
    }

    /// `turn/start` answered with Codex's turn id.
    pub fn bind_provider_turn(&mut self, provider_turn: &str) {
        if self.turn_map.contains_key(provider_turn) {
            return;
        }
        if let Some(engine) = self.pending_engine_turn.take() {
            self.turn_map.insert(provider_turn.to_string(), engine);
            if self.active_provider_turn.is_none() {
                self.active_provider_turn = Some(provider_turn.to_string());
            }
        }
    }

    /// The engine turn for a Codex turn id, binding a pending one on first
    /// sight (a notification can beat the `turn/start` answer).
    fn engine_turn(&mut self, provider_turn: Option<&str>) -> Option<String> {
        match provider_turn {
            Some(pt) => {
                if let Some(t) = self.turn_map.get(pt) {
                    return Some(t.clone());
                }
                if self.pending_engine_turn.is_some() {
                    self.bind_provider_turn(pt);
                    return self.turn_map.get(pt).cloned();
                }
                self.active_engine_turn.clone()
            }
            None => self.active_engine_turn.clone(),
        }
    }

    fn is_child(&self, thread: Option<&str>) -> bool {
        match (thread, self.provider_thread.as_deref()) {
            (Some(t), Some(root)) => t != root,
            _ => false,
        }
    }

    // ── notifications ───────────────────────────────────────────────────

    pub fn on_notification(&mut self, method: &str, params: &Value) -> Vec<RuntimeEvent> {
        let thread = params
            .get("threadId")
            .and_then(Value::as_str)
            .map(str::to_string);
        if self.is_child(thread.as_deref()) {
            if let Some(out) =
                self.child_notification(method, params, thread.as_deref().unwrap_or(""))
            {
                return out;
            }
        }
        let provider_turn = params
            .get("turnId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                params
                    .get("turn")
                    .and_then(|t| t.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
        let turn = self.engine_turn(provider_turn.as_deref());
        let t = turn.as_deref();
        let pt = provider_turn.as_deref();
        let mut out = Vec::new();
        match method {
            "turn/started" => {
                if let Some(pt) = pt {
                    if self.active_provider_turn.is_none() {
                        self.active_provider_turn = Some(pt.to_string());
                    }
                }
                let ev = self.ev(
                    t,
                    RuntimeEventKind::TurnStarted(TurnStartedPayload {
                        model: self.model.clone(),
                        effort: None,
                    }),
                );
                out.push(Self::refs(ev, pt, None));
            }
            "turn/completed" => out.extend(self.turn_completed(params, t, pt)),
            "error" => {
                let Some(n) = decode::<p::ErrorNotification>(params) else {
                    return self.raw(method, params);
                };
                let info = params.get("error").and_then(|e| e.get("codexErrorInfo"));
                if n.will_retry {
                    out.push(self.ev(
                        t,
                        RuntimeEventKind::RuntimeWarning(RuntimeWarningPayload {
                            message: n.error.message.clone(),
                            detail: n.error.additional_details.clone(),
                        }),
                    ));
                } else {
                    self.turn_error_sent = true;
                    out.push(self.runtime_error(
                        t,
                        info,
                        &n.error.message,
                        n.error.additional_details.clone(),
                    ));
                }
            }
            "warning" => {
                let message = str_of(params, "message").unwrap_or_default();
                out.push(self.ev(
                    t,
                    RuntimeEventKind::RuntimeWarning(RuntimeWarningPayload {
                        message,
                        detail: None,
                    }),
                ));
            }
            "item/started" | "item/completed" => {
                let completed = method == "item/completed";
                let Some(item) = params.get("item") else {
                    return self.raw(method, params);
                };
                out.extend(self.item_event(item, completed, t, pt));
            }
            "item/agentMessage/delta" => {
                out.extend(self.delta(params, StreamKind::AssistantText, t, pt))
            }
            "item/plan/delta" => out.extend(self.delta(params, StreamKind::PlanText, t, pt)),
            "item/reasoning/summaryTextDelta" => {
                out.extend(self.delta(params, StreamKind::ReasoningSummaryText, t, pt))
            }
            "item/reasoning/textDelta" => {
                out.extend(self.delta(params, StreamKind::ReasoningText, t, pt))
            }
            "item/commandExecution/outputDelta" => {
                out.extend(self.delta(params, StreamKind::CommandOutput, t, pt))
            }
            "item/fileChange/outputDelta" => {
                out.extend(self.delta(params, StreamKind::FileChangeOutput, t, pt))
            }
            "item/reasoning/summaryPartAdded" => {}
            "item/fileChange/patchUpdated" => {
                let Some(n) = decode::<p::FileChangePatchUpdatedNotification>(params) else {
                    return self.raw(method, params);
                };
                let (diff, paths) = changes_diff(&n.changes);
                let cache = self.items.entry(n.item_id.clone()).or_default();
                cache.diff = Some(diff.clone());
                cache.paths = paths.clone();
                let mut payload = ItemPayload::new(ItemType::FileChange);
                payload.data = Some(json!({ "input": { "paths": paths, "diff": diff } }));
                let ev = self
                    .ev(t, RuntimeEventKind::ItemUpdated(payload))
                    .with_item(n.item_id.clone());
                out.push(Self::refs(ev, pt, Some(&n.item_id)));
            }
            "item/commandExecution/terminalInteraction" => {
                let item_id = str_of(params, "itemId").unwrap_or_default();
                let mut payload = ItemPayload::new(ItemType::CommandExecution);
                payload.data = Some(
                    json!({ "input": { "stdin": params.get("stdin").cloned().unwrap_or(Value::Null) } }),
                );
                out.push(
                    self.ev(t, RuntimeEventKind::ItemUpdated(payload))
                        .with_item(item_id),
                );
            }
            "item/mcpToolCall/progress" => {
                let item_id = str_of(params, "itemId");
                out.push(self.ev(
                    t,
                    RuntimeEventKind::ToolProgress(ToolProgressPayload {
                        tool_use_id: item_id,
                        summary: str_of(params, "message"),
                        ..Default::default()
                    }),
                ));
            }
            "turn/plan/updated" => {
                let Some(n) = decode::<p::TurnPlanUpdatedNotification>(params) else {
                    return self.raw(method, params);
                };
                let plan = n
                    .plan
                    .iter()
                    .map(|s| PlanStep {
                        step: s.step.clone(),
                        status: serde_json::to_value(&s.status)
                            .ok()
                            .and_then(|v| v.as_str().map(str::to_string))
                            .filter(|s| s != "Unknown")
                            .unwrap_or_else(|| "pending".into()),
                    })
                    .collect();
                out.push(self.ev(
                    t,
                    RuntimeEventKind::PlanUpdated(PlanUpdatedPayload {
                        explanation: n.explanation,
                        plan,
                    }),
                ));
            }
            "turn/diff/updated" => {
                let diff = str_of(params, "diff").unwrap_or_default();
                out.push(self.ev(
                    t,
                    RuntimeEventKind::DiffUpdated(DiffUpdatedPayload {
                        unified_diff: clip(&diff, DIFF_CLIP),
                    }),
                ));
            }
            "thread/tokenUsage/updated" => {
                let Some(n) = decode::<p::ThreadTokenUsageUpdatedNotification>(params) else {
                    return self.raw(method, params);
                };
                let for_active = self.active_provider_turn.as_deref() == Some(n.turn_id.as_str())
                    || (self.active_provider_turn.is_none() && self.pending_engine_turn.is_some())
                    || self.turn_map.get(&n.turn_id).map(String::as_str)
                        == self.active_engine_turn.as_deref();
                if for_active && self.active_engine_turn.is_some() {
                    self.usage_last = Some(n.token_usage.clone());
                } else {
                    // A late update of an earlier turn only moves the baseline.
                    self.usage_base = Some(n.token_usage.total.clone());
                }
                let mut usage = usage_of(
                    &n.token_usage.last,
                    n.token_usage.model_context_window,
                    self.model.as_deref(),
                );
                usage.usage_status = None;
                out.push(self.ev(
                    t,
                    RuntimeEventKind::UsageUpdated(UsageUpdatedPayload { usage }),
                ));
            }
            "thread/started" => {
                // The root's own `thread/started` is already announced by the
                // driver when it opened the thread; a sub-agent's is a task.
                if let Some(thread) = params.get("thread") {
                    let id = str_of(thread, "id");
                    if id.is_some() && self.provider_thread.is_some() && id != self.provider_thread
                    {
                        out.push(
                            self.ev(
                                t,
                                RuntimeEventKind::TaskStarted(TaskPayload {
                                    task_id: id.clone().unwrap_or_default(),
                                    title: str_of(thread, "agentNickname")
                                        .or_else(|| str_of(thread, "agentRole")),
                                    agent_id: id,
                                    parent_agent_id: str_of(thread, "parentThreadId"),
                                    model: str_of(thread, "model"),
                                    status: Some("running".into()),
                                    task_type: Some("subagent".into()),
                                    ..Default::default()
                                }),
                            ),
                        );
                    }
                }
            }
            "thread/status/changed" => {
                let state = match params
                    .get("status")
                    .and_then(|s| s.get("type"))
                    .and_then(Value::as_str)
                {
                    Some("systemError") => "error",
                    Some("active") => "active",
                    Some("idle") => "idle",
                    Some("notLoaded") => "idle",
                    Some(other) => other,
                    None => "idle",
                }
                .to_string();
                let flags = params
                    .get("status")
                    .and_then(|s| s.get("activeFlags"))
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .filter(|s| !s.is_empty());
                out.push(self.ev(
                    t,
                    RuntimeEventKind::ThreadStateChanged(ThreadStatePayload {
                        state,
                        before_tokens: None,
                        after_tokens: None,
                        detail: flags,
                    }),
                ));
            }
            "thread/closed" => out.push(self.ev(
                t,
                RuntimeEventKind::ThreadStateChanged(ThreadStatePayload {
                    state: "closed".into(),
                    before_tokens: None,
                    after_tokens: None,
                    detail: None,
                }),
            )),
            "thread/name/updated" => out.push(self.ev(
                t,
                RuntimeEventKind::ThreadMetadataUpdated(ThreadMetadataPayload {
                    name: str_of(params, "threadName"),
                    metadata: None,
                }),
            )),
            "serverRequest/resolved" => {
                let key = params.get("requestId").map(|v| match v {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                });
                if let Some(req) = key.and_then(|k| self.by_rpc.remove(&k)) {
                    if let Some(pending) = self.pending.remove(&req) {
                        let kind = if pending.kind == PendingKind::UserInput {
                            RuntimeEventKind::UserInputResolved(UserInputResolvedPayload {
                                answers: Value::Object(Map::new()),
                            })
                        } else {
                            RuntimeEventKind::RequestResolved(RequestResolvedPayload {
                                request_type: pending.kind.request_type(),
                                decision: None,
                                resolution: Some("answered-elsewhere".into()),
                            })
                        };
                        out.push(self.ev(pending.turn_id.as_deref(), kind).with_request(req));
                    }
                }
            }
            "account/updated" => out.push(self.ev(
                None,
                RuntimeEventKind::AccountUpdated(ValuePayload {
                    value: params.clone(),
                }),
            )),
            "account/rateLimits/updated" => {
                if let Some(snap) = params
                    .get("rateLimits")
                    .and_then(decode::<p::RateLimitSnapshot>)
                {
                    if let Some(ev) = self.merge_rate_limits(&snap, None) {
                        out.push(ev);
                    }
                }
            }
            "mcpServer/oauthLogin/completed" => out.push(
                self.ev(
                    t,
                    RuntimeEventKind::McpOauthCompleted(McpOauthPayload {
                        success: params
                            .get("success")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                        name: str_of(params, "name"),
                        error: str_of(params, "error"),
                    }),
                ),
            ),
            "model/rerouted" => {
                let to = str_of(params, "toModel").unwrap_or_default();
                if !to.is_empty() {
                    self.model = Some(to.clone());
                }
                out.push(
                    self.ev(
                        t,
                        RuntimeEventKind::ModelRerouted(ModelReroutedPayload {
                            from_model: str_of(params, "fromModel").unwrap_or_default(),
                            to_model: to,
                            reason: params
                                .get("reason")
                                .map(|r| {
                                    r.as_str()
                                        .map(str::to_string)
                                        .unwrap_or_else(|| r.to_string())
                                })
                                .unwrap_or_default(),
                        }),
                    ),
                );
            }
            "deprecationNotice" => out.push(self.ev(
                t,
                RuntimeEventKind::DeprecationNotice(NoticePayload {
                    summary: str_of(params, "summary").unwrap_or_default(),
                    details: str_of(params, "details"),
                    path: None,
                }),
            )),
            "configWarning" => out.push(self.ev(
                t,
                RuntimeEventKind::ConfigWarning(NoticePayload {
                    summary: str_of(params, "summary").unwrap_or_default(),
                    details: str_of(params, "details"),
                    path: str_of(params, "path"),
                }),
            )),
            "hook/started" | "hook/completed" => {
                let run = params.get("run").cloned().unwrap_or(Value::Null);
                let payload = HookPayload {
                    hook_id: str_of(&run, "id").unwrap_or_default(),
                    hook_name: str_of(&run, "sourcePath"),
                    hook_event: str_of(&run, "eventName"),
                    output: str_of(&run, "statusMessage"),
                    outcome: if method == "hook/completed" {
                        str_of(&run, "status")
                    } else {
                        None
                    },
                    ..Default::default()
                };
                let kind = if method == "hook/started" {
                    RuntimeEventKind::HookStarted(payload)
                } else {
                    RuntimeEventKind::HookCompleted(payload)
                };
                out.push(self.ev(t, kind));
            }
            _ => return self.raw(method, params),
        }
        out
    }

    fn raw(&self, method: &str, params: &Value) -> Vec<RuntimeEvent> {
        vec![self.ev(
            None,
            RuntimeEventKind::Raw(RawPayload {
                source: "codex.app-server.notification".into(),
                method: Some(method.to_string()),
                payload: params.clone(),
            }),
        )]
    }

    fn runtime_error(
        &self,
        turn: Option<&str>,
        info: Option<&Value>,
        message: &str,
        detail: Option<String>,
    ) -> RuntimeEvent {
        let (class, code) = classify_error(info, message);
        let message = if code == "ERR_CODEX_USAGE_LIMIT" {
            self.usage_limit_message()
                .unwrap_or_else(|| message.to_string())
        } else if code == "ERR_CODEX_AUTH" {
            format!("Codex is not logged in on this account. Run `codex login` in its terminal. ({message})")
        } else {
            message.to_string()
        };
        self.ev(
            turn,
            RuntimeEventKind::RuntimeError(RuntimeErrorPayload {
                message,
                class,
                code: Some(code.into()),
                detail,
            }),
        )
    }

    /// "Codex usage limit reached. The weekly limit resets in 5d 5h."
    pub fn usage_limit_message(&self) -> Option<String> {
        let now = chrono::Utc::now().timestamp();
        let window = [self.rates.secondary.as_ref(), self.rates.primary.as_ref()]
            .into_iter()
            .flatten()
            .filter(|w| w.used_percent >= 100)
            .find_map(|w| w.resets_at.map(|r| (w.window_duration_mins, r)))
            .or_else(|| {
                [self.rates.primary.as_ref(), self.rates.secondary.as_ref()]
                    .into_iter()
                    .flatten()
                    .find_map(|w| w.resets_at.map(|r| (w.window_duration_mins, r)))
            })?;
        let (mins, at) = window;
        let label = window_label(mins).to_lowercase();
        let which = if label.is_empty() {
            "the limit".to_string()
        } else {
            format!("the {label} limit")
        };
        let next = match self.rates.reached {
            Some(p::RateLimitReachedType::WorkspaceOwnerCreditsDepleted)
            | Some(p::RateLimitReachedType::WorkspaceMemberCreditsDepleted) => {
                " Ask the workspace owner to add credits."
            }
            Some(p::RateLimitReachedType::WorkspaceOwnerUsageLimitReached)
            | Some(p::RateLimitReachedType::WorkspaceMemberUsageLimitReached) => {
                " Raise the workspace spend limit to continue."
            }
            _ => " Send again once it resets.",
        };
        Some(format!(
            "Codex usage limit reached. {} resets in {}.{next}",
            capitalize(&which),
            format_wait(at - now)
        ))
    }

    fn delta(
        &mut self,
        params: &Value,
        kind: StreamKind,
        turn: Option<&str>,
        pt: Option<&str>,
    ) -> Vec<RuntimeEvent> {
        let item_id = str_of(params, "itemId").unwrap_or_default();
        let delta = str_of(params, "delta").unwrap_or_default();
        if delta.is_empty() {
            return Vec::new();
        }
        let ev = self
            .ev(
                turn,
                RuntimeEventKind::ContentDelta(ContentDeltaPayload {
                    stream_kind: kind,
                    delta,
                    content_index: params
                        .get("contentIndex")
                        .and_then(Value::as_u64)
                        .map(|n| n as u32),
                    summary_index: params
                        .get("summaryIndex")
                        .and_then(Value::as_u64)
                        .map(|n| n as u32),
                }),
            )
            .with_item(item_id.clone());
        vec![Self::refs(ev, pt, Some(&item_id))]
    }

    fn item_event(
        &mut self,
        raw_item: &Value,
        completed: bool,
        turn: Option<&str>,
        pt: Option<&str>,
    ) -> Vec<RuntimeEvent> {
        let item_id = str_of(raw_item, "id").unwrap_or_default();
        let Some(item) = decode::<p::ThreadItem>(raw_item) else {
            // Undecodable: still show it, untyped.
            let mut payload = ItemPayload::new(ItemType::Unknown);
            payload.title = str_of(raw_item, "type");
            payload.status = Some(if completed {
                ItemStatus::Completed
            } else {
                ItemStatus::InProgress
            });
            payload.data = Some(raw_item.clone());
            let kind = if completed {
                RuntimeEventKind::ItemCompleted(payload)
            } else {
                RuntimeEventKind::ItemStarted(payload)
            };
            return vec![self.ev(turn, kind).with_item(item_id)];
        };
        let mut extra = Vec::new();
        let default_status = if completed {
            ItemStatus::Completed
        } else {
            ItemStatus::InProgress
        };
        let payload: Option<ItemPayload> = match item {
            p::ThreadItem::UserMessage { .. } | p::ThreadItem::HookPrompt { .. } => None,
            p::ThreadItem::AgentMessage {
                text,
                delivery,
                questions,
                ..
            } => {
                if completed && delivery.is_some() {
                    if let Some(qs) = questions.filter(|q| !q.is_empty()) {
                        // An async question does not end the turn; the user
                        // answers it with a normal message.
                        let questions = qs
                            .iter()
                            .enumerate()
                            .map(|(i, q)| UserInputQuestion {
                                id: format!("q{i}"),
                                header: String::new(),
                                question: q.title.clone(),
                                options: q
                                    .options
                                    .clone()
                                    .unwrap_or_default()
                                    .into_iter()
                                    .map(|o| UserInputOption {
                                        label: o,
                                        description: None,
                                        value: None,
                                    })
                                    .collect(),
                                allow_custom_answer: true,
                                multi_select: false,
                            })
                            .collect();
                        extra.push(
                            self.ev(
                                turn,
                                RuntimeEventKind::UserInputRequested(UserInputRequestedPayload {
                                    questions,
                                    response_mode: Some("message".into()),
                                }),
                            )
                            .with_request(format!(
                                "codex-async:{}:{}",
                                self.provider_thread.as_deref().unwrap_or(""),
                                item_id
                            )),
                        );
                    }
                }
                let mut p = ItemPayload::new(ItemType::AssistantMessage);
                p.status = Some(default_status);
                if completed {
                    p.detail = Some(text);
                }
                Some(p)
            }
            p::ThreadItem::Plan { text, .. } => {
                let mut p = ItemPayload::new(ItemType::Plan);
                p.status = Some(default_status);
                if completed {
                    extra.push(self.ev(
                        turn,
                        RuntimeEventKind::ProposedCompleted(ProposedCompletedPayload {
                            plan_markdown: text.clone(),
                        }),
                    ));
                    p.detail = Some(text);
                }
                Some(p)
            }
            p::ThreadItem::Reasoning {
                content, summary, ..
            } => {
                let mut p = ItemPayload::new(ItemType::Reasoning);
                p.status = Some(default_status);
                if completed {
                    let summary = summary.unwrap_or_default().join("\n\n");
                    let content = content.unwrap_or_default().join("\n\n");
                    let text = if !summary.is_empty() {
                        summary
                    } else {
                        content
                    };
                    if !text.is_empty() {
                        p.detail = Some(text);
                    }
                }
                Some(p)
            }
            p::ThreadItem::CommandExecution {
                aggregated_output,
                command,
                cwd,
                duration_ms,
                exit_code,
                status,
                ..
            } => {
                let cache = self.items.entry(item_id.clone()).or_default();
                cache.command = Some(command.clone());
                cache.cwd = Some(cwd.clone());
                let mut p = ItemPayload::new(ItemType::CommandExecution);
                p.status = Some(status_of_command(&status));
                p.title = Some("Ran command".into());
                p.tool_name = Some("shell".into());
                p.detail = Some(command.clone());
                let mut data = json!({ "input": { "command": command, "cwd": cwd } });
                if completed {
                    data["output"] = json!({
                        "exitCode": exit_code,
                        "durationMs": duration_ms,
                        "text": aggregated_output.map(|o| clip(&o, OUTPUT_CLIP)),
                    });
                }
                p.data = Some(data);
                Some(p)
            }
            p::ThreadItem::FileChange {
                changes, status, ..
            } => {
                let (diff, paths) = changes_diff(&changes);
                let cache = self.items.entry(item_id.clone()).or_default();
                cache.diff = Some(diff.clone());
                cache.paths = paths.clone();
                let mut p = ItemPayload::new(ItemType::FileChange);
                p.status = Some(status_of_patch(&status));
                p.title = Some("File change".into());
                p.tool_name = Some("apply_patch".into());
                p.detail = Some(paths.join(", "));
                p.data = Some(json!({ "input": { "paths": paths, "diff": diff } }));
                Some(p)
            }
            p::ThreadItem::McpToolCall {
                arguments,
                error,
                result,
                server,
                status,
                tool,
                ..
            } => {
                let mut p = ItemPayload::new(ItemType::McpToolCall);
                p.status = Some(status_of_mcp(&status));
                let title = arguments
                    .get("title")
                    .and_then(Value::as_str)
                    .filter(|_| tool == "js")
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("{server} · {tool}"));
                p.title = Some(title);
                p.tool_name = Some(format!("mcp__{server}__{tool}"));
                let mut data = json!({ "input": arguments });
                if completed {
                    data["output"] = match (result, error) {
                        (_, Some(e)) => json!({ "error": e.message }),
                        (Some(r), None) => {
                            let text = serde_json::to_string(&r.content).unwrap_or_default();
                            json!(clip(&text, OUTPUT_CLIP))
                        }
                        _ => Value::Null,
                    };
                }
                p.data = Some(data);
                Some(p)
            }
            p::ThreadItem::DynamicToolCall {
                arguments,
                status,
                tool,
                success,
                ..
            } => {
                let mut p = ItemPayload::new(ItemType::DynamicToolCall);
                p.status = Some(match success {
                    Some(false) if completed => ItemStatus::Failed,
                    _ => status_of_dynamic(&status),
                });
                p.title = Some(tool.clone());
                p.tool_name = Some(tool);
                p.data = Some(json!({ "input": arguments }));
                Some(p)
            }
            p::ThreadItem::CollabAgentToolCall {
                prompt,
                receiver_thread_ids,
                tool,
                model,
                ..
            } => {
                let mut p = ItemPayload::new(ItemType::CollabAgentToolCall);
                p.status = Some(default_status);
                let tool = serde_json::to_value(&tool)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_else(|| "agent".into());
                p.title = Some(format!("Agent · {tool}"));
                p.tool_name = Some(tool);
                p.detail = prompt.clone();
                p.data = Some(
                    json!({ "input": { "prompt": prompt, "model": model, "threads": receiver_thread_ids } }),
                );
                Some(p)
            }
            p::ThreadItem::WebSearch { query, .. } => {
                let mut p = ItemPayload::new(ItemType::WebSearch);
                p.status = Some(default_status);
                p.title = Some("Web search".into());
                p.tool_name = Some("web_search".into());
                p.detail = Some(query.clone());
                p.data = Some(json!({ "input": { "query": query } }));
                Some(p)
            }
            p::ThreadItem::ImageView { path, .. } => {
                let mut p = ItemPayload::new(ItemType::ImageView);
                p.status = Some(default_status);
                p.title = Some("Viewed image".into());
                p.detail = Some(path);
                Some(p)
            }
            p::ThreadItem::EnteredReviewMode { review, .. } => {
                let mut p = ItemPayload::new(ItemType::ReviewEntered);
                p.status = Some(default_status);
                p.detail = Some(review);
                Some(p)
            }
            p::ThreadItem::ExitedReviewMode { review, .. } => {
                let mut p = ItemPayload::new(ItemType::ReviewExited);
                p.status = Some(default_status);
                p.detail = Some(review);
                Some(p)
            }
            p::ThreadItem::ContextCompaction { .. } => {
                if completed {
                    extra.push(self.ev(
                        turn,
                        RuntimeEventKind::ThreadStateChanged(ThreadStatePayload {
                            state: "compacted".into(),
                            before_tokens: None,
                            after_tokens: None,
                            detail: None,
                        }),
                    ));
                }
                let mut p = ItemPayload::new(ItemType::ContextCompaction);
                p.status = Some(default_status);
                p.title = Some("Context compacted".into());
                Some(p)
            }
            p::ThreadItem::SubAgentActivity { .. } => None,
            other => {
                let mut p = ItemPayload::new(ItemType::Unknown);
                p.status = Some(default_status);
                p.title = str_of(raw_item, "type");
                p.data = serde_json::to_value(&other).ok();
                Some(p)
            }
        };
        let mut out = Vec::new();
        if let Some(payload) = payload {
            let kind = if completed {
                RuntimeEventKind::ItemCompleted(payload)
            } else {
                RuntimeEventKind::ItemStarted(payload)
            };
            let ev = self.ev(turn, kind).with_item(item_id.clone());
            out.push(Self::refs(ev, pt, Some(&item_id)));
        }
        out.extend(extra);
        out
    }

    fn turn_completed(
        &mut self,
        params: &Value,
        turn: Option<&str>,
        pt: Option<&str>,
    ) -> Vec<RuntimeEvent> {
        let mut out = Vec::new();
        let decoded = params.get("turn").and_then(decode::<p::Turn>);
        let (state, error, duration) = match &decoded {
            Some(t) => (
                match t.status {
                    p::TurnStatus::Failed => TurnEndState::Failed,
                    p::TurnStatus::Interrupted => TurnEndState::Interrupted,
                    _ => TurnEndState::Completed,
                },
                t.error.clone(),
                t.duration_ms,
            ),
            None => (TurnEndState::Completed, None, None),
        };
        let raw_info = params
            .get("turn")
            .and_then(|t| t.get("error"))
            .and_then(|e| e.get("codexErrorInfo"))
            .cloned();
        if state == TurnEndState::Failed && !self.turn_error_sent {
            if let Some(e) = &error {
                out.push(self.runtime_error(
                    turn,
                    raw_info.as_ref(),
                    &e.message,
                    e.additional_details.clone(),
                ));
            }
        }
        let usage = self.take_turn_usage(duration);
        let error_message = error.map(|e| {
            let (_, code) = classify_error(raw_info.as_ref(), &e.message);
            if code == "ERR_CODEX_USAGE_LIMIT" {
                self.usage_limit_message().unwrap_or(e.message)
            } else {
                e.message
            }
        });
        let ev = self.ev(
            turn,
            RuntimeEventKind::TurnCompleted(TurnCompletedPayload {
                state,
                stop_reason: None,
                usage,
                total_cost_usd: None,
                error_message,
            }),
        );
        out.push(Self::refs(ev, pt, None));
        // Parked requests of this turn are settled by Codex itself; the engine
        // marks what is still open as stale.
        self.pending.retain(|_, p| p.turn_id.as_deref() != turn);
        self.by_rpc.retain(|_, req| self.pending.contains_key(req));
        if pt.is_none() || self.active_provider_turn.as_deref() == pt {
            self.active_provider_turn = None;
            self.active_engine_turn = None;
            self.pending_engine_turn = None;
        }
        out
    }

    /// Usage of the turn that just ended: the last cumulative total seen in
    /// the turn minus the total at its start (a counter that went backwards is
    /// a reset: the last total is the turn's).
    fn take_turn_usage(&mut self, duration: Option<i64>) -> Option<TokenUsage> {
        let last = self.usage_last.take()?;
        let delta = match &self.usage_base {
            Some(base) if last.total.total_tokens >= base.total_tokens => sub(&last.total, base),
            _ => last.total.clone(),
        };
        self.usage_base = Some(last.total.clone());
        let mut u = usage_of(&delta, last.model_context_window, self.model.as_deref());
        u.used_tokens = Some(last.last.total_tokens.max(0) as u64);
        u.duration_ms = duration.map(|d| d.max(0) as u64);
        Some(u)
    }

    /// After a rollback the thread's cumulative counter restarts from what
    /// is left; the next update sets a fresh baseline.
    pub fn reset_usage_baseline(&mut self) {
        self.usage_base = None;
        self.usage_last = None;
    }

    fn child_notification(
        &mut self,
        method: &str,
        params: &Value,
        child: &str,
    ) -> Option<Vec<RuntimeEvent>> {
        let task =
            |status: Option<&str>, summary: Option<String>, error: Option<String>| TaskPayload {
                task_id: child.to_string(),
                agent_id: Some(child.to_string()),
                status: status.map(str::to_string),
                summary,
                error,
                task_type: Some("subagent".into()),
                ..Default::default()
            };
        let turn = self.active_engine_turn.clone();
        let t = turn.as_deref();
        let out = match method {
            "turn/started" => {
                if let Some(id) = params
                    .get("turn")
                    .and_then(|t| t.get("id"))
                    .and_then(Value::as_str)
                {
                    self.child_turns.insert(child.to_string(), id.to_string());
                }
                vec![self.ev(
                    t,
                    RuntimeEventKind::TaskUpdated(task(Some("running"), None, None)),
                )]
            }
            "turn/completed" => {
                self.child_turns.remove(child);
                let status = match params
                    .get("turn")
                    .and_then(|t| t.get("status"))
                    .and_then(Value::as_str)
                {
                    Some("failed") => "failed",
                    Some("interrupted") => "interrupted",
                    _ => "idle",
                };
                vec![self.ev(
                    t,
                    RuntimeEventKind::TaskUpdated(task(Some(status), None, None)),
                )]
            }
            "thread/status/changed" => {
                let s = params.get("status");
                let status = match s.and_then(|s| s.get("type")).and_then(Value::as_str) {
                    Some("systemError") => "failed",
                    Some("active") => {
                        let waiting = s
                            .and_then(|s| s.get("activeFlags"))
                            .and_then(Value::as_array)
                            .map(|a| !a.is_empty())
                            .unwrap_or(false);
                        if waiting {
                            "waiting"
                        } else {
                            "running"
                        }
                    }
                    _ => "idle",
                };
                vec![self.ev(
                    t,
                    RuntimeEventKind::TaskUpdated(task(Some(status), None, None)),
                )]
            }
            "thread/tokenUsage/updated" => {
                let usage = params
                    .get("tokenUsage")
                    .and_then(decode::<p::ThreadTokenUsage>)
                    .map(|u| usage_of(&u.total, u.model_context_window, None));
                let mut payload = task(None, None, None);
                payload.usage = usage;
                vec![self.ev(t, RuntimeEventKind::TaskProgress(payload))]
            }
            "item/started" | "item/completed" => {
                let item = params.get("item").cloned().unwrap_or(Value::Null);
                let summary = str_of(&item, "command")
                    .or_else(|| str_of(&item, "query"))
                    .or_else(|| str_of(&item, "tool"))
                    .or_else(|| str_of(&item, "type"));
                let mut payload = task(None, summary, None);
                payload.last_tool_name = str_of(&item, "tool");
                vec![self.ev(t, RuntimeEventKind::TaskProgress(payload))]
            }
            "thread/closed" => {
                self.child_turns.remove(child);
                vec![self.ev(
                    t,
                    RuntimeEventKind::TaskCompleted(task(Some("completed"), None, None)),
                )]
            }
            "error" => {
                if params
                    .get("willRetry")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                {
                    return Some(Vec::new());
                }
                let msg = params
                    .get("error")
                    .and_then(|e| e.get("message"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                vec![self.ev(
                    t,
                    RuntimeEventKind::TaskUpdated(task(Some("failed"), None, msg)),
                )]
            }
            "item/agentMessage/delta"
            | "item/plan/delta"
            | "item/reasoning/summaryTextDelta"
            | "item/reasoning/summaryPartAdded"
            | "item/reasoning/textDelta"
            | "item/commandExecution/outputDelta"
            | "item/fileChange/outputDelta"
            | "item/fileChange/patchUpdated"
            | "turn/plan/updated"
            | "turn/diff/updated"
            | "thread/name/updated"
            | "thread/settings/updated"
            | "rawResponseItem/completed"
            | "thread/archived"
            | "thread/unarchived"
            | "thread/compacted"
            | "thread/started" => Vec::new(),
            // Anything else (serverRequest/resolved, new methods) goes the
            // parent's way: never swallow what we do not know.
            _ => return None,
        };
        Some(out)
    }

    // ── rate limits ─────────────────────────────────────────────────────

    /// `account/rateLimits/read` answer.
    pub fn on_rate_limits_read(&mut self, resp: &Value) -> Option<RuntimeEvent> {
        let r = decode::<p::GetAccountRateLimitsResponse>(resp)?;
        let snap = r
            .rate_limits_by_limit_id
            .as_ref()
            .and_then(|m| m.get("codex").cloned())
            .unwrap_or(r.rate_limits);
        let credits = resp
            .get("rateLimitResetCredits")
            .cloned()
            .filter(|v| !v.is_null());
        self.merge_rate_limits(&snap, credits)
    }

    /// Merges a (possibly sparse) snapshot over the last view and emits the
    /// normalized windows. Model-specific buckets (`limitId` not `codex`)
    /// are ignored.
    pub fn merge_rate_limits(
        &mut self,
        snap: &p::RateLimitSnapshot,
        reset_credits: Option<Value>,
    ) -> Option<RuntimeEvent> {
        if let Some(id) = snap.limit_id.as_deref() {
            if id != "codex" {
                return None;
            }
        }
        if snap.primary.is_some() {
            self.rates.primary = snap.primary.clone();
        }
        if snap.secondary.is_some() {
            self.rates.secondary = snap.secondary.clone();
        }
        if snap.credits.is_some() {
            self.rates.credits = snap.credits.clone();
        }
        if snap.plan_type.is_some() {
            self.rates.plan_type = snap.plan_type.clone();
        }
        if snap.rate_limit_reached_type.is_some() {
            self.rates.reached = snap.rate_limit_reached_type.clone();
        }
        if reset_credits.is_some() {
            self.rates.reset_credits = reset_credits;
        }
        let free_plan = matches!(
            self.rates.plan_type,
            Some(p::PlanType::Free) | Some(p::PlanType::Go)
        );
        let window = |id: &str, w: &p::RateLimitWindow, fallback: i64| {
            let minutes = w.window_duration_mins.unwrap_or(fallback);
            RateLimitWindow {
                id: id.to_string(),
                label: window_label(Some(minutes)),
                used_percent: Some(f64::from(w.used_percent.clamp(0, 100))),
                window_minutes: Some(minutes.max(0) as u64),
                resets_at: w.resets_at.and_then(epoch_to_iso),
            }
        };
        let mut windows = Vec::new();
        if let Some(w) = &self.rates.primary {
            windows.push(window(
                "primary",
                w,
                if free_plan { 30 * 24 * 60 } else { 5 * 60 },
            ));
        }
        if let Some(w) = &self.rates.secondary {
            windows.push(window("secondary", w, 7 * 24 * 60));
        }
        let mut credits = Map::new();
        if let Some(c) = &self.rates.credits {
            credits.insert(
                "credits".into(),
                serde_json::to_value(c).unwrap_or(Value::Null),
            );
        }
        if let Some(c) = &self.rates.reset_credits {
            credits.insert("resetCredits".into(), c.clone());
        }
        if let Some(pt) = &self.rates.plan_type {
            credits.insert(
                "planType".into(),
                serde_json::to_value(pt).unwrap_or(Value::Null),
            );
        }
        Some(self.ev(
            None,
            RuntimeEventKind::RateLimitsUpdated(RateLimitsPayload {
                windows,
                credits: if credits.is_empty() {
                    None
                } else {
                    Some(Value::Object(credits))
                },
            }),
        ))
    }

    // ── server requests ─────────────────────────────────────────────────

    pub fn on_server_request(
        &mut self,
        rpc_id: Value,
        method: &str,
        params: &Value,
    ) -> ServerRequestOutcome {
        let provider_turn = str_of(params, "turnId");
        let turn = self.engine_turn(provider_turn.as_deref());
        let t = turn.as_deref();
        let refuse = |code: i64, message: &str| AutoReply::Error {
            code,
            message: message.to_string(),
        };
        let (kind, payload) = match method {
            "item/commandExecution/requestApproval" => {
                let n: p::CommandExecutionRequestApprovalParams =
                    decode(params).unwrap_or_default();
                let cache = self.items.get(&n.item_id).cloned().unwrap_or_default();
                let stdin = matches!(n.kind, Some(p::CommandExecutionApprovalKind::WriteStdin));
                let reason = match (stdin, n.reason.clone()) {
                    (true, Some(r)) => Some(format!("Write to a running terminal: {r}")),
                    (true, None) => Some("Write to a running terminal".into()),
                    (false, r) => r,
                };
                let detail = RequestDetail {
                    tool_name: Some("shell".into()),
                    command: n.command.clone().or(cache.command),
                    cwd: n.cwd.clone().or(cache.cwd),
                    reason,
                    input: n
                        .proposed_execpolicy_amendment
                        .as_ref()
                        .map(|a| json!({ "proposedExecpolicyAmendment": a })),
                    ..Default::default()
                };
                (
                    PendingKind::Command,
                    RequestOpenedPayload {
                        request_type: RequestType::CommandExecutionApproval,
                        detail: Some(detail),
                        app_name: None,
                        options: approval_options(true),
                        args: None,
                    },
                )
            }
            "item/fileChange/requestApproval" => {
                let n: p::FileChangeRequestApprovalParams = decode(params).unwrap_or_default();
                let cache = self.items.get(&n.item_id).cloned().unwrap_or_default();
                let detail = RequestDetail {
                    tool_name: Some("apply_patch".into()),
                    reason: n.reason.clone(),
                    diff: cache.diff,
                    paths: cache.paths,
                    input: n.grant_root.as_ref().map(|g| json!({ "grantRoot": g })),
                    ..Default::default()
                };
                (
                    PendingKind::FileChange,
                    RequestOpenedPayload {
                        request_type: RequestType::FileChangeApproval,
                        detail: Some(detail),
                        app_name: None,
                        options: approval_options(true),
                        args: None,
                    },
                )
            }
            "item/permissions/requestApproval" => {
                let n: p::PermissionsRequestApprovalParams = decode(params).unwrap_or_default();
                let fs = params.get("permissions").and_then(|p| p.get("fileSystem"));
                let mut paths: Vec<String> = Vec::new();
                for key in ["read", "write"] {
                    if let Some(a) = fs.and_then(|f| f.get(key)).and_then(Value::as_array) {
                        paths.extend(
                            a.iter()
                                .filter_map(Value::as_str)
                                .map(|s| format!("{key}: {s}")),
                        );
                    }
                }
                let detail = RequestDetail {
                    tool_name: Some("permissions".into()),
                    cwd: Some(n.cwd.clone()).filter(|c| !c.is_empty()),
                    reason: n.reason.clone(),
                    paths,
                    input: params.get("permissions").cloned(),
                    ..Default::default()
                };
                (
                    PendingKind::Permissions,
                    RequestOpenedPayload {
                        request_type: RequestType::PermissionApproval,
                        detail: Some(detail),
                        app_name: None,
                        options: approval_options(true),
                        args: None,
                    },
                )
            }
            "mcpServer/elicitation/request" => {
                let mode = str_of(params, "mode").unwrap_or_default();
                let app = elicitation_app_name(params);
                let persist = params
                    .get("_meta")
                    .and_then(|m| m.get("persist"))
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                let mut options = vec![opt("accept", "Approve", ApprovalDecision::Accept)];
                if persist.contains("session") {
                    options.push(opt(
                        "acceptForSession",
                        "Always allow this session",
                        ApprovalDecision::AcceptForSession,
                    ));
                }
                if persist.contains("always") {
                    options.push(opt(
                        "acceptAlways",
                        "Always allow",
                        ApprovalDecision::AcceptAlways,
                    ));
                }
                options.push(opt("decline", "Decline", ApprovalDecision::Decline));
                options.push(opt("cancel", "Cancel", ApprovalDecision::Cancel));
                if mode == "url" {
                    // Never open a URL on the server's behalf.
                    options.retain(|o| o.decision != Some(ApprovalDecision::Accept));
                }
                let detail = RequestDetail {
                    tool_name: str_of(params, "serverName"),
                    reason: str_of(params, "message"),
                    input: params.get("requestedSchema").cloned(),
                    preview: str_of(params, "url"),
                    ..Default::default()
                };
                (
                    PendingKind::Elicitation,
                    RequestOpenedPayload {
                        request_type: RequestType::McpElicitationApproval,
                        detail: Some(detail),
                        app_name: app,
                        options,
                        args: None,
                    },
                )
            }
            "item/tool/requestUserInput" => {
                let n: p::ToolRequestUserInputParams = decode(params).unwrap_or_default();
                let questions: Vec<UserInputQuestion> = n
                    .questions
                    .iter()
                    .filter(|q| !q.id.is_empty() && !q.question.is_empty())
                    .map(|q| UserInputQuestion {
                        id: q.id.clone(),
                        header: q.header.clone(),
                        question: q.question.clone(),
                        options: q
                            .options
                            .clone()
                            .unwrap_or_default()
                            .into_iter()
                            .map(|o| UserInputOption {
                                label: o.label,
                                description: Some(o.description).filter(|d| !d.is_empty()),
                                value: None,
                            })
                            .collect(),
                        allow_custom_answer: q.is_other.unwrap_or(true),
                        multi_select: false,
                    })
                    .collect();
                let req = uuid::Uuid::new_v4().to_string();
                self.remember(&req, rpc_id, PendingKind::UserInput, turn.clone(), params);
                let ev = self
                    .ev(
                        t,
                        RuntimeEventKind::UserInputRequested(UserInputRequestedPayload {
                            questions,
                            response_mode: None,
                        }),
                    )
                    .with_request(req);
                return ServerRequestOutcome {
                    events: vec![ev],
                    reply: None,
                };
            }
            "applyPatchApproval" => {
                let n: p::ApplyPatchApprovalParams = decode(params).unwrap_or_default();
                let (diff, paths) = legacy_changes_diff(&n.file_changes);
                (
                    PendingKind::LegacyPatch,
                    RequestOpenedPayload {
                        request_type: RequestType::ApplyPatchApproval,
                        detail: Some(RequestDetail {
                            tool_name: Some("apply_patch".into()),
                            reason: n.reason,
                            diff: Some(diff),
                            paths,
                            ..Default::default()
                        }),
                        app_name: None,
                        options: approval_options(true),
                        args: None,
                    },
                )
            }
            "execCommandApproval" => {
                let n: p::ExecCommandApprovalParams = decode(params).unwrap_or_default();
                (
                    PendingKind::LegacyExec,
                    RequestOpenedPayload {
                        request_type: RequestType::ExecCommandApproval,
                        detail: Some(RequestDetail {
                            tool_name: Some("shell".into()),
                            command: Some(n.command.join(" ")),
                            cwd: Some(n.cwd).filter(|c| !c.is_empty()),
                            reason: n.reason,
                            ..Default::default()
                        }),
                        app_name: None,
                        options: approval_options(true),
                        args: None,
                    },
                )
            }
            "account/chatgptAuthTokens/refresh" => {
                // OmniGet never holds or refreshes a credential.
                return ServerRequestOutcome {
                    events: vec![self.ev(
                        t,
                        RuntimeEventKind::AuthStatus(super::super::AuthStatusPayload {
                            is_authenticating: Some(false),
                            output: Vec::new(),
                            error: Some(
                                "Codex asked for fresh ChatGPT tokens. Run `codex login` in this account's terminal."
                                    .into(),
                            ),
                        }),
                    )],
                    reply: Some(refuse(CODE_METHOD_NOT_FOUND, "the client does not manage ChatGPT tokens")),
                };
            }
            "item/tool/call" => {
                return ServerRequestOutcome {
                    events: vec![self.ev(
                        t,
                        RuntimeEventKind::RuntimeWarning(RuntimeWarningPayload {
                            message: format!(
                                "Codex called a client tool ({}) that OmniGet does not provide.",
                                str_of(params, "tool").unwrap_or_default()
                            ),
                            detail: None,
                        }),
                    )],
                    reply: Some(AutoReply::Result(json!({
                        "contentItems": [{ "type": "inputText", "text": "This tool is not available in this client." }],
                        "success": false
                    }))),
                };
            }
            other => {
                return ServerRequestOutcome {
                    events: Vec::new(),
                    reply: Some(refuse(
                        CODE_METHOD_NOT_FOUND,
                        &format!("{other} is not handled by OmniGet"),
                    )),
                };
            }
        };
        let req = uuid::Uuid::new_v4().to_string();
        self.remember(&req, rpc_id.clone(), kind, turn.clone(), params);
        let mut ev = self
            .ev(t, RuntimeEventKind::RequestOpened(payload))
            .with_request(req);
        ev.provider_refs = Some(ProviderRefs {
            provider_turn_id: provider_turn,
            provider_item_id: str_of(params, "itemId").or_else(|| str_of(params, "callId")),
            provider_request_id: Some(match &rpc_id {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            }),
        });
        ServerRequestOutcome {
            events: vec![ev],
            reply: None,
        }
    }

    fn remember(
        &mut self,
        req: &str,
        rpc_id: Value,
        kind: PendingKind,
        turn: Option<String>,
        params: &Value,
    ) {
        let key = match &rpc_id {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        self.by_rpc.insert(key, req.to_string());
        self.pending.insert(
            req.to_string(),
            PendingRequest {
                rpc_id,
                kind,
                turn_id: turn,
                params: params.clone(),
            },
        );
    }

    fn take_pending(&mut self, request_id: &str) -> Result<PendingRequest, DriverError> {
        let pending = self.pending.remove(request_id).ok_or_else(|| {
            DriverError::new(
                ERR_DRIVER_NO_SESSION,
                format!("request {request_id} is no longer waiting in Codex"),
            )
        })?;
        self.by_rpc.retain(|_, r| r != request_id);
        Ok(pending)
    }

    /// The JSON-RPC answer to an approval, plus `request.resolved`.
    pub fn answer_request(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(Answer, Vec<RuntimeEvent>), DriverError> {
        if self.pending.get(request_id).map(|p| p.kind) == Some(PendingKind::UserInput) {
            // A question answered with a bare decision: no answers.
            return self.answer_user_input(request_id, &json!({}));
        }
        let pending = self.take_pending(request_id)?;
        let result = approval_result(pending.kind, decision, &pending.params);
        let ev = self
            .ev(
                pending.turn_id.as_deref(),
                RuntimeEventKind::RequestResolved(RequestResolvedPayload {
                    request_type: pending.kind.request_type(),
                    decision: Some(decision),
                    resolution: None,
                }),
            )
            .with_request(request_id);
        Ok(((pending.rpc_id, result), vec![ev]))
    }

    pub fn answer_user_input(
        &mut self,
        request_id: &str,
        answers: &Value,
    ) -> Result<(Answer, Vec<RuntimeEvent>), DriverError> {
        let pending = self.take_pending(request_id)?;
        if pending.kind != PendingKind::UserInput {
            // Someone answered an approval as a question: treat it as decline.
            let result = approval_result(pending.kind, ApprovalDecision::Decline, &pending.params);
            return Ok(((pending.rpc_id, result), Vec::new()));
        }
        let result = user_input_result(answers);
        let ev = self
            .ev(
                pending.turn_id.as_deref(),
                RuntimeEventKind::UserInputResolved(UserInputResolvedPayload {
                    answers: answers.clone(),
                }),
            )
            .with_request(request_id);
        Ok(((pending.rpc_id, result), vec![ev]))
    }

    /// Stop settles every parked request first (a parked prompt would
    /// deadlock `turn/interrupt`): approvals get `cancel`, questions an
    /// empty answer.
    pub fn cancel_all_pending(&mut self) -> (Vec<Answer>, Vec<RuntimeEvent>) {
        let ids: Vec<String> = self.pending.keys().cloned().collect();
        let mut answers = Vec::new();
        let mut events = Vec::new();
        for id in ids {
            let Ok(pending) = self.take_pending(&id) else {
                continue;
            };
            let (result, kind) = if pending.kind == PendingKind::UserInput {
                (
                    json!({ "answers": {} }),
                    RuntimeEventKind::UserInputResolved(UserInputResolvedPayload {
                        answers: Value::Object(Map::new()),
                    }),
                )
            } else {
                (
                    approval_result(pending.kind, ApprovalDecision::Cancel, &pending.params),
                    RuntimeEventKind::RequestResolved(RequestResolvedPayload {
                        request_type: pending.kind.request_type(),
                        decision: Some(ApprovalDecision::Cancel),
                        resolution: Some("cancelled".into()),
                    }),
                )
            };
            answers.push((pending.rpc_id, result));
            events.push(self.ev(pending.turn_id.as_deref(), kind).with_request(id));
        }
        (answers, events)
    }

    /// The process died mid-turn: close the turn so the thread does not hang.
    pub fn abort_active(&mut self, message: &str) -> Vec<RuntimeEvent> {
        let mut out = Vec::new();
        self.pending.clear();
        self.by_rpc.clear();
        self.child_turns.clear();
        let Some(turn) = self
            .active_engine_turn
            .take()
            .or_else(|| self.pending_engine_turn.take())
        else {
            return out;
        };
        self.pending_engine_turn = None;
        self.active_provider_turn = None;
        out.push(self.ev(
            Some(&turn),
            RuntimeEventKind::RuntimeError(RuntimeErrorPayload {
                message: message.to_string(),
                class: ErrorClass::TransportError,
                code: Some("ERR_CODEX_EXITED".into()),
                detail: None,
            }),
        ));
        let usage = self.take_turn_usage(None);
        out.push(self.ev(
            Some(&turn),
            RuntimeEventKind::TurnCompleted(TurnCompletedPayload {
                state: TurnEndState::Failed,
                stop_reason: Some("process_exited".into()),
                usage,
                total_cost_usd: None,
                error_message: Some(message.to_string()),
            }),
        ));
        out
    }

    /// `interrupt` with no live Codex turn: the engine still gets its close.
    pub fn aborted(&mut self, turn: &str, reason: &str) -> RuntimeEvent {
        if self.active_engine_turn.as_deref() == Some(turn) {
            self.active_engine_turn = None;
            self.pending_engine_turn = None;
            self.active_provider_turn = None;
        }
        self.ev(
            Some(turn),
            RuntimeEventKind::TurnAborted(TurnAbortedPayload {
                reason: reason.to_string(),
                usage: None,
            }),
        )
    }

    /// `turn/start` failed before Codex took the turn.
    pub fn turn_rejected(&mut self) {
        self.pending_engine_turn = None;
        self.active_engine_turn = None;
        self.active_provider_turn = None;
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn elicitation_app_name(params: &Value) -> Option<String> {
    let meta = params.get("_meta");
    let paths: [&[&str]; 7] = [
        &["_meta", "app_name"],
        &["appName"],
        &["app"],
        &["target", "app"],
        &["target", "name"],
        &["_meta", "tool_params", "app_name"],
        &["_meta", "tool_params", "app"],
    ];
    for path in paths {
        let mut cur = params;
        let mut ok = true;
        for key in path {
            match cur.get(*key) {
                Some(v) => cur = v,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            if let Some(s) = cur.as_str().filter(|s| !s.is_empty()) {
                return Some(s.to_string());
            }
        }
    }
    let _ = meta;
    if let Some(msg) = params.get("message").and_then(Value::as_str) {
        if let Some(rest) = msg.strip_prefix("Allow ChatGPT to use ") {
            if let Some(name) = rest.strip_suffix('?') {
                return Some(name.to_string());
            }
        }
    }
    str_of(params, "serverName")
}

/// The answer body for an approval, per request kind (schema of 0.156):
/// v2 command/file → `{decision: accept|acceptForSession|decline|cancel}`
/// (`acceptAlways` downgraded to the session); permissions → the requested
/// profile granted for the turn or session, or an empty grant; elicitation →
/// `{action, content?}`; legacy v1 → `ReviewDecision`.
pub fn approval_result(kind: PendingKind, decision: ApprovalDecision, params: &Value) -> Value {
    use ApprovalDecision::*;
    match kind {
        PendingKind::Command | PendingKind::FileChange => {
            let d = match decision {
                Accept => "accept",
                AcceptForSession | AcceptAlways => "acceptForSession",
                Decline => "decline",
                Cancel => "cancel",
            };
            json!({ "decision": d })
        }
        PendingKind::Permissions => {
            if decision.allows() {
                let scope = if matches!(decision, AcceptForSession | AcceptAlways) {
                    "session"
                } else {
                    "turn"
                };
                json!({
                    "permissions": params.get("permissions").cloned().unwrap_or_else(|| json!({})),
                    "scope": scope,
                })
            } else {
                json!({ "permissions": {} })
            }
        }
        PendingKind::Elicitation => elicitation_result(decision, params),
        PendingKind::LegacyPatch | PendingKind::LegacyExec => {
            let d = match decision {
                Accept => json!("approved"),
                AcceptForSession | AcceptAlways => json!("approved_for_session"),
                Decline => json!({ "denied": { "rejection": "The user declined this." } }),
                Cancel => json!("abort"),
            };
            json!({ "decision": d })
        }
        PendingKind::UserInput => json!({ "answers": {} }),
    }
}

/// Fills an MCP elicitation form for an accept, or declines when a required
/// field has no sensible default. URL-mode elicitations are never accepted.
pub fn elicitation_result(decision: ApprovalDecision, params: &Value) -> Value {
    use ApprovalDecision::*;
    match decision {
        Decline => return json!({ "action": "decline" }),
        Cancel => return json!({ "action": "cancel" }),
        _ => {}
    }
    if params.get("mode").and_then(Value::as_str) == Some("url") {
        return json!({ "action": "decline" });
    }
    let schema = params
        .get("requestedSchema")
        .cloned()
        .unwrap_or(Value::Null);
    let required: Vec<String> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let persistent = |name: &str, s: &Value| {
        let text = format!(
            "{} {}",
            name,
            s.get("title").and_then(Value::as_str).unwrap_or_default()
        )
        .to_lowercase();
        ["always", "persist", "remember", "forever", "permanent"]
            .iter()
            .any(|w| text.contains(w))
    };
    let mut content = Map::new();
    if let Some(props) = schema.get("properties").and_then(Value::as_object) {
        for (name, s) in props {
            let enum_values: Vec<String> = s
                .get("enum")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .or_else(|| {
                    s.get("oneOf").and_then(Value::as_array).map(|a| {
                        a.iter()
                            .filter_map(|o| {
                                o.get("const").and_then(Value::as_str).map(str::to_string)
                            })
                            .collect()
                    })
                })
                .unwrap_or_default();
            let value = if s.get("type").and_then(Value::as_str) == Some("boolean") {
                if persistent(name, s) {
                    Some(json!(
                        decision == AcceptAlways || decision == AcceptForSession
                    ))
                } else {
                    s.get("default").cloned().or(Some(json!(true)))
                }
            } else if !enum_values.is_empty() {
                let wanted: &[&str] = match decision {
                    AcceptAlways => &["always", "permanent", "forever", "persistent"],
                    AcceptForSession => &["session"],
                    _ => &["once", "accept", "approve", "allow"],
                };
                enum_values
                    .iter()
                    .find(|v| wanted.iter().any(|w| v.to_lowercase().contains(w)))
                    .or_else(|| {
                        enum_values.iter().find(|v| {
                            ["once", "accept", "approve", "allow"]
                                .iter()
                                .any(|w| v.to_lowercase().contains(w))
                        })
                    })
                    .cloned()
                    .map(Value::String)
                    .or_else(|| s.get("default").cloned())
            } else {
                s.get("default").cloned()
            };
            match value {
                Some(v) => {
                    content.insert(name.clone(), v);
                }
                None if required.contains(name) => return json!({ "action": "decline" }),
                None => {}
            }
        }
    }
    let mut out = json!({ "action": "accept", "content": Value::Object(content) });
    let persist = params.get("_meta").and_then(|m| m.get("persist"));
    if persist.is_some() {
        match decision {
            AcceptForSession => out["_meta"] = json!({ "persist": "session" }),
            AcceptAlways => out["_meta"] = json!({ "persist": "always" }),
            _ => {}
        }
    }
    out
}

/// `{answers: {qid: {answers: [..]}}}` from what the UI sends: per question a
/// string, a list of strings, or `{answers: [..]}`.
pub fn user_input_result(answers: &Value) -> Value {
    let mut out = Map::new();
    if let Some(obj) = answers.as_object() {
        for (qid, a) in obj {
            let list: Vec<Value> = match a {
                Value::String(s) => vec![Value::String(s.clone())],
                Value::Array(items) => items
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| Value::String(s.to_string())))
                    .collect(),
                Value::Object(o) => o
                    .get("answers")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|v| v.as_str().map(|s| Value::String(s.to_string())))
                            .collect()
                    })
                    .unwrap_or_default(),
                Value::Null => Vec::new(),
                other => vec![Value::String(other.to_string())],
            };
            out.insert(qid.clone(), json!({ "answers": list }));
        }
    }
    json!({ "answers": Value::Object(out) })
}
