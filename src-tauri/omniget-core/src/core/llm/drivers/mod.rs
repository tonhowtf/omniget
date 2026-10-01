use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

pub mod acp;
pub mod claude;
pub mod coalesce;
pub mod codex;
pub mod opencode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccessMode {
    #[default]
    ApprovalRequired,
    AutoAcceptEdits,
    Auto,
    FullAccess,
}

impl AccessMode {
    pub fn as_str(self) -> &'static str {
        match self {
            AccessMode::ApprovalRequired => "approval-required",
            AccessMode::AutoAcceptEdits => "auto-accept-edits",
            AccessMode::Auto => "auto",
            AccessMode::FullAccess => "full-access",
        }
    }

    pub fn parse(s: &str) -> AccessMode {
        match s.trim() {
            "auto-accept-edits" => AccessMode::AutoAcceptEdits,
            "auto" => AccessMode::Auto,
            "full-access" => AccessMode::FullAccess,
            _ => AccessMode::ApprovalRequired,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InteractionMode {
    #[default]
    Default,
    Plan,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApprovalDecision {
    Accept,
    AcceptForSession,
    AcceptAlways,
    Decline,
    Cancel,
}

impl ApprovalDecision {
    pub fn allows(self) -> bool {
        matches!(
            self,
            ApprovalDecision::Accept
                | ApprovalDecision::AcceptForSession
                | ApprovalDecision::AcceptAlways
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemType {
    UserMessage,
    AssistantMessage,
    Reasoning,
    Plan,
    CommandExecution,
    FileChange,
    McpToolCall,
    DynamicToolCall,
    CollabAgentToolCall,
    WebSearch,
    ImageView,
    ReviewEntered,
    ReviewExited,
    ContextCompaction,
    Error,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemStatus {
    InProgress,
    Completed,
    Failed,
    Declined,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    AssistantText,
    ReasoningText,
    ReasoningSummaryText,
    PlanText,
    CommandOutput,
    FileChangeOutput,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestType {
    CommandExecutionApproval,
    FileReadApproval,
    FileChangeApproval,
    ApplyPatchApproval,
    ExecCommandApproval,
    McpElicitationApproval,
    PermissionApproval,
    ToolUserInput,
    DynamicToolCall,
    AuthTokensRefresh,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    ProviderError,
    TransportError,
    PermissionError,
    ValidationError,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Starting,
    Ready,
    Running,
    Waiting,
    Stopped,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnEndState {
    Completed,
    Failed,
    Interrupted,
    Cancelled,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStartedPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStatePayload {
    pub state: SessionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionExitedPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recoverable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_kind: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStartedPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_thread_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadStatePayload {
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadMetadataPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub cached_input_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub reasoning_output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_uses: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_status: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageUpdatedPayload {
    pub usage: TokenUsage,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartedPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnCompletedPayload {
    pub state: TurnEndState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnAbortedPayload {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub step: String,
    pub status: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanUpdatedPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
    #[serde(default)]
    pub plan: Vec<PlanStep>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeltaPayload {
    pub delta: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProposedCompletedPayload {
    pub plan_markdown: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffUpdatedPayload {
    pub unified_diff: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemPayload {
    pub item_type: ItemType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ItemStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tool_use_id: Option<String>,
}

impl ItemPayload {
    pub fn new(item_type: ItemType) -> Self {
        Self {
            item_type,
            status: None,
            title: None,
            detail: None,
            tool_name: None,
            data: None,
            agent_id: None,
            parent_tool_use_id: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentDeltaPayload {
    pub stream_kind: StreamKind,
    pub delta: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_index: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestDetail {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalOption {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<ApprovalDecision>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestOpenedPayload {
    pub request_type: RequestType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<RequestDetail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<ApprovalOption>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestResolvedPayload {
    pub request_type: RequestType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<ApprovalDecision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInputOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInputQuestion {
    pub id: String,
    #[serde(default)]
    pub header: String,
    pub question: String,
    #[serde(default)]
    pub options: Vec<UserInputOption>,
    #[serde(default = "yes")]
    pub allow_custom_answer: bool,
    #[serde(default)]
    pub multi_select: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInputRequestedPayload {
    pub questions: Vec<UserInputQuestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInputResolvedPayload {
    pub answers: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPayload {
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_backgrounded: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookPayload {
    pub hook_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolProgressPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elapsed_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_tool_use_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSummaryPayload {
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preceding_tool_use_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDeniedPayload {
    pub tool_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatusPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_authenticating: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValuePayload {
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitWindow {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_minutes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitsPayload {
    #[serde(default)]
    pub windows: Vec<RateLimitWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credits: Option<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOauthPayload {
    pub success: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelReroutedPayload {
    pub from_model: String,
    pub to_model: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoticePayload {
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedFile {
    pub filename: String,
    pub file_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilesPersistedPayload {
    #[serde(default)]
    pub files: Vec<PersistedFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeWarningPayload {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeErrorPayload {
    pub message: String,
    pub class: ErrorClass,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawPayload {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum RuntimeEventKind {
    #[serde(rename = "session.started")]
    SessionStarted(SessionStartedPayload),
    #[serde(rename = "session.configured")]
    SessionConfigured(ValuePayload),
    #[serde(rename = "session.state.changed")]
    SessionStateChanged(SessionStatePayload),
    #[serde(rename = "session.exited")]
    SessionExited(SessionExitedPayload),
    #[serde(rename = "thread.started")]
    ThreadStarted(ThreadStartedPayload),
    #[serde(rename = "thread.state.changed")]
    ThreadStateChanged(ThreadStatePayload),
    #[serde(rename = "thread.metadata.updated")]
    ThreadMetadataUpdated(ThreadMetadataPayload),
    #[serde(rename = "thread.token-usage.updated", alias = "usage.updated")]
    UsageUpdated(UsageUpdatedPayload),
    #[serde(rename = "thread.realtime.started")]
    RealtimeStarted(ValuePayload),
    #[serde(rename = "thread.realtime.item-added")]
    RealtimeItemAdded(ValuePayload),
    #[serde(rename = "thread.realtime.audio.delta")]
    RealtimeAudioDelta(ValuePayload),
    #[serde(rename = "thread.realtime.error")]
    RealtimeError(RuntimeWarningPayload),
    #[serde(rename = "thread.realtime.closed")]
    RealtimeClosed(ValuePayload),
    #[serde(rename = "turn.started")]
    TurnStarted(TurnStartedPayload),
    #[serde(rename = "turn.completed")]
    TurnCompleted(TurnCompletedPayload),
    #[serde(rename = "turn.aborted")]
    TurnAborted(TurnAbortedPayload),
    #[serde(rename = "turn.plan.updated", alias = "plan.updated")]
    PlanUpdated(PlanUpdatedPayload),
    #[serde(rename = "turn.proposed.delta")]
    ProposedDelta(DeltaPayload),
    #[serde(rename = "turn.proposed.completed")]
    ProposedCompleted(ProposedCompletedPayload),
    #[serde(rename = "turn.diff.updated")]
    DiffUpdated(DiffUpdatedPayload),
    #[serde(rename = "item.started")]
    ItemStarted(ItemPayload),
    #[serde(rename = "item.updated")]
    ItemUpdated(ItemPayload),
    #[serde(rename = "item.completed")]
    ItemCompleted(ItemPayload),
    #[serde(rename = "content.delta")]
    ContentDelta(ContentDeltaPayload),
    #[serde(rename = "request.opened")]
    RequestOpened(RequestOpenedPayload),
    #[serde(rename = "request.resolved")]
    RequestResolved(RequestResolvedPayload),
    #[serde(rename = "user-input.requested")]
    UserInputRequested(UserInputRequestedPayload),
    #[serde(rename = "user-input.resolved")]
    UserInputResolved(UserInputResolvedPayload),
    #[serde(rename = "task.started")]
    TaskStarted(TaskPayload),
    #[serde(rename = "task.progress")]
    TaskProgress(TaskPayload),
    #[serde(rename = "task.updated")]
    TaskUpdated(TaskPayload),
    #[serde(rename = "task.completed")]
    TaskCompleted(TaskPayload),
    #[serde(rename = "hook.started")]
    HookStarted(HookPayload),
    #[serde(rename = "hook.progress")]
    HookProgress(HookPayload),
    #[serde(rename = "hook.completed")]
    HookCompleted(HookPayload),
    #[serde(rename = "tool.progress")]
    ToolProgress(ToolProgressPayload),
    #[serde(rename = "tool.summary")]
    ToolSummary(ToolSummaryPayload),
    #[serde(rename = "tool.denied")]
    ToolDenied(ToolDeniedPayload),
    #[serde(rename = "auth.status")]
    AuthStatus(AuthStatusPayload),
    #[serde(rename = "account.updated")]
    AccountUpdated(ValuePayload),
    #[serde(rename = "account.rate-limits.updated")]
    RateLimitsUpdated(RateLimitsPayload),
    #[serde(rename = "mcp.status.updated")]
    McpStatusUpdated(ValuePayload),
    #[serde(rename = "mcp.oauth.completed")]
    McpOauthCompleted(McpOauthPayload),
    #[serde(rename = "model.rerouted")]
    ModelRerouted(ModelReroutedPayload),
    #[serde(rename = "config.warning")]
    ConfigWarning(NoticePayload),
    #[serde(rename = "deprecation.notice")]
    DeprecationNotice(NoticePayload),
    #[serde(rename = "files.persisted")]
    FilesPersisted(FilesPersistedPayload),
    #[serde(rename = "runtime.warning")]
    RuntimeWarning(RuntimeWarningPayload),
    #[serde(rename = "runtime.error")]
    RuntimeError(RuntimeErrorPayload),
    #[serde(rename = "raw")]
    Raw(RawPayload),
}

impl RuntimeEventKind {
    pub fn type_name(&self) -> &'static str {
        use RuntimeEventKind::*;
        match self {
            SessionStarted(_) => "session.started",
            SessionConfigured(_) => "session.configured",
            SessionStateChanged(_) => "session.state.changed",
            SessionExited(_) => "session.exited",
            ThreadStarted(_) => "thread.started",
            ThreadStateChanged(_) => "thread.state.changed",
            ThreadMetadataUpdated(_) => "thread.metadata.updated",
            UsageUpdated(_) => "thread.token-usage.updated",
            RealtimeStarted(_) => "thread.realtime.started",
            RealtimeItemAdded(_) => "thread.realtime.item-added",
            RealtimeAudioDelta(_) => "thread.realtime.audio.delta",
            RealtimeError(_) => "thread.realtime.error",
            RealtimeClosed(_) => "thread.realtime.closed",
            TurnStarted(_) => "turn.started",
            TurnCompleted(_) => "turn.completed",
            TurnAborted(_) => "turn.aborted",
            PlanUpdated(_) => "turn.plan.updated",
            ProposedDelta(_) => "turn.proposed.delta",
            ProposedCompleted(_) => "turn.proposed.completed",
            DiffUpdated(_) => "turn.diff.updated",
            ItemStarted(_) => "item.started",
            ItemUpdated(_) => "item.updated",
            ItemCompleted(_) => "item.completed",
            ContentDelta(_) => "content.delta",
            RequestOpened(_) => "request.opened",
            RequestResolved(_) => "request.resolved",
            UserInputRequested(_) => "user-input.requested",
            UserInputResolved(_) => "user-input.resolved",
            TaskStarted(_) => "task.started",
            TaskProgress(_) => "task.progress",
            TaskUpdated(_) => "task.updated",
            TaskCompleted(_) => "task.completed",
            HookStarted(_) => "hook.started",
            HookProgress(_) => "hook.progress",
            HookCompleted(_) => "hook.completed",
            ToolProgress(_) => "tool.progress",
            ToolSummary(_) => "tool.summary",
            ToolDenied(_) => "tool.denied",
            AuthStatus(_) => "auth.status",
            AccountUpdated(_) => "account.updated",
            RateLimitsUpdated(_) => "account.rate-limits.updated",
            McpStatusUpdated(_) => "mcp.status.updated",
            McpOauthCompleted(_) => "mcp.oauth.completed",
            ModelRerouted(_) => "model.rerouted",
            ConfigWarning(_) => "config.warning",
            DeprecationNotice(_) => "deprecation.notice",
            FilesPersisted(_) => "files.persisted",
            RuntimeWarning(_) => "runtime.warning",
            RuntimeError(_) => "runtime.error",
            Raw(_) => "raw",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRefs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_request_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeEvent {
    pub event_id: String,
    pub driver: String,
    pub instance_id: String,
    pub thread_id: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_refs: Option<ProviderRefs>,
    #[serde(flatten)]
    pub kind: RuntimeEventKind,
}

impl RuntimeEvent {
    pub fn new(
        driver: &str,
        instance_id: &str,
        thread_id: &str,
        turn_id: Option<&str>,
        kind: RuntimeEventKind,
    ) -> Self {
        Self {
            event_id: uuid::Uuid::new_v4().to_string(),
            driver: driver.to_string(),
            instance_id: instance_id.to_string(),
            thread_id: thread_id.to_string(),
            created_at: now_iso(),
            turn_id: turn_id.map(str::to_string),
            item_id: None,
            request_id: None,
            provider_refs: None,
            kind,
        }
    }

    pub fn with_item(mut self, item_id: impl Into<String>) -> Self {
        self.item_id = Some(item_id.into());
        self
    }

    pub fn with_request(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }

    pub fn type_name(&self) -> &'static str {
        self.kind.type_name()
    }
}

pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverCapabilities {
    pub rollback: bool,
    pub fork: bool,
    pub interrupt: bool,
    pub approvals: bool,
    pub user_input: bool,
    pub model_switch: bool,
    pub plan_mode: bool,
    pub compaction: bool,
    pub continuation: bool,
    pub steer: bool,
    pub rate_limits: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvVar {
    pub name: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub sensitive: bool,
    #[serde(default)]
    pub value_redacted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverInstance {
    pub id: String,
    pub driver: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<EnvVar>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

impl DriverInstance {
    pub fn new(id: &str, driver: &str, label: &str) -> Self {
        Self {
            id: id.to_string(),
            driver: driver.to_string(),
            label: label.to_string(),
            color: None,
            account_id: None,
            config_dir: None,
            command: None,
            args: Vec::new(),
            env: Vec::new(),
            enabled: true,
        }
    }

    pub fn redacted(&self) -> Self {
        let mut out = self.clone();
        for var in &mut out.env {
            if var.sensitive {
                var.value = String::new();
                var.value_redacted = true;
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriverError {
    pub code: String,
    pub message: String,
}

impl DriverError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Self {
            code: code.to_string(),
            message: message.into(),
        }
    }

    pub fn unsupported(what: &str) -> Self {
        Self::new(ERR_DRIVER_UNSUPPORTED, format!("{what} is not supported"))
    }
}

impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for DriverError {}

pub const ERR_DRIVER_UNSUPPORTED: &str = "ERR_DRIVER_UNSUPPORTED";
pub const ERR_DRIVER_UNAVAILABLE: &str = "ERR_DRIVER_UNAVAILABLE";
pub const ERR_DRIVER_NO_SESSION: &str = "ERR_DRIVER_NO_SESSION";
pub const ERR_DRIVER_FAILED: &str = "ERR_DRIVER_FAILED";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStart {
    pub thread_id: String,
    pub instance_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub access_mode: AccessMode,
    pub interaction_mode: InteractionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_cursor: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStart {
    pub thread_id: String,
    pub turn_id: String,
    pub message_id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub access_mode: AccessMode,
    pub interaction_mode: InteractionMode,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_cursor: Option<Value>,
}

#[async_trait]
pub trait Driver: Send + Sync {
    fn kind(&self) -> &str;
    fn capabilities(&self) -> DriverCapabilities;
    async fn start_session(&self, input: SessionStart) -> Result<(), DriverError>;
    async fn start_turn(&self, input: TurnStart) -> Result<TurnStartResult, DriverError>;
    async fn interrupt(&self, thread_id: &str, turn_id: Option<&str>) -> Result<(), DriverError>;
    async fn respond_request(
        &self,
        thread_id: &str,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<(), DriverError>;
    async fn respond_user_input(
        &self,
        thread_id: &str,
        request_id: &str,
        answers: Value,
    ) -> Result<(), DriverError>;
    async fn rollback(&self, thread_id: &str, turn_count: u32) -> Result<(), DriverError>;
    async fn fork(
        &self,
        _source_thread_id: &str,
        _target_thread_id: &str,
        _turn_count: u32,
    ) -> Result<(), DriverError> {
        Err(DriverError::unsupported("fork"))
    }
    async fn set_access_mode(
        &self,
        _thread_id: &str,
        _mode: AccessMode,
    ) -> Result<(), DriverError> {
        Ok(())
    }
    async fn stop(&self, thread_id: &str) -> Result<(), DriverError>;
}

pub type RuntimeSink = mpsc::UnboundedSender<RuntimeEvent>;

pub type DriverFactory =
    Arc<dyn Fn(DriverInstance, RuntimeSink) -> Result<Arc<dyn Driver>, DriverError> + Send + Sync>;

#[derive(Clone)]
pub struct DriverRegistration {
    pub kind: String,
    pub label: String,
    pub capabilities: DriverCapabilities,
    pub factory: DriverFactory,
}

impl std::fmt::Debug for DriverRegistration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriverRegistration")
            .field("kind", &self.kind)
            .field("label", &self.label)
            .finish()
    }
}

static REGISTRY: RwLock<Option<HashMap<String, DriverRegistration>>> = RwLock::new(None);

pub fn register_driver(reg: DriverRegistration) {
    let mut guard = REGISTRY.write().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(HashMap::new)
        .insert(reg.kind.clone(), reg);
}

pub fn driver_registration(kind: &str) -> Option<DriverRegistration> {
    REGISTRY
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(kind).cloned())
}

pub fn registered_drivers() -> Vec<String> {
    let mut out: Vec<String> = REGISTRY
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    out.sort();
    out
}

pub fn build_driver(
    instance: DriverInstance,
    sink: RuntimeSink,
) -> Result<Arc<dyn Driver>, DriverError> {
    let reg = driver_registration(&instance.driver).ok_or_else(|| {
        DriverError::new(
            ERR_DRIVER_UNAVAILABLE,
            format!("no driver registered for `{}`", instance.driver),
        )
    })?;
    (reg.factory)(instance, sink)
}

static DURABLE: RwLock<Option<HashSet<String>>> = RwLock::new(None);

pub fn set_durable_conversation(conversation_id: &str, durable: bool) {
    let mut guard = DURABLE.write().unwrap_or_else(|e| e.into_inner());
    let set = guard.get_or_insert_with(HashSet::new);
    if durable {
        set.insert(conversation_id.to_string());
    } else {
        set.remove(conversation_id);
    }
}

pub fn is_durable_conversation(conversation_id: &str) -> bool {
    DURABLE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .map(|s| s.contains(conversation_id))
        .unwrap_or(false)
}
