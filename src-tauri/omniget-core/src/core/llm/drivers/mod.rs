//! Runtime drivers of the Central (plan §3.2): one normalized event stream
//! ([`RuntimeEvent`], a port of T3 Code's 49 `ProviderRuntimeEvent`s plus
//! `raw`), one trait every harness implements ([`Driver`]), and a registry
//! that maps a driver kind (`native`, `claude`, `codex`, `acp`, `opencode`)
//! to a factory. A driver is the *type*; a [`DriverInstance`] is one
//! configured account of it (config dir, label, color, redacted env).
//!
//! Wire shape (JSON, camelCase like T3): a runtime event is
//! `{eventId, driver, instanceId, threadId, createdAt, turnId?, itemId?,
//! requestId?, providerRefs?, raw?, type: "content.delta", payload: {...}}`.
//!
//! How a driver plugs in (the workers of round 2 do exactly this):
//! ```ignore
//! use omniget_core::core::llm::drivers::*;
//! register_driver(DriverRegistration {
//!     kind: "claude".into(),
//!     label: "Claude Code".into(),
//!     capabilities: DriverCapabilities { rollback: true, ..Default::default() },
//!     factory: std::sync::Arc::new(|instance, sink| {
//!         Ok(std::sync::Arc::new(MyClaudeDriver::new(instance, sink)) as std::sync::Arc<dyn Driver>)
//!     }),
//! });
//! ```
//! The threads host builds one driver object per instance on first use,
//! hands it a [`RuntimeSink`] and calls the trait from its reactors. The
//! driver pushes every event through the sink; the host coalesces deltas
//! (50 ms), persists them as thread events and fans them out.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

pub mod claude;
pub mod acp;
pub mod coalesce;
pub mod opencode;
pub mod codex;

// ── Access modes and decisions (plan §3.3) ──────────────────────────────

/// How much a thread's agent may do without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccessMode {
    /// Every write and command asks.
    #[default]
    ApprovalRequired,
    /// File edits go through; commands still ask.
    AutoAcceptEdits,
    /// The driver's own "auto" (Codex `on-request`, Claude `auto`).
    Auto,
    /// Nothing asks.
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

    /// Unknown strings fall back to the safe mode: a typo never grants more.
    pub fn parse(s: &str) -> AccessMode {
        match s.trim() {
            "auto-accept-edits" => AccessMode::AutoAcceptEdits,
            "auto" => AccessMode::Auto,
            "full-access" => AccessMode::FullAccess,
            _ => AccessMode::ApprovalRequired,
        }
    }
}

/// Plan mode vs normal turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InteractionMode {
    #[default]
    Default,
    Plan,
}

/// The answer to a `request.opened`.
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

// ── Canonical item / request / stream kinds ─────────────────────────────

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

// ── Payloads ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStartedPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Opaque resume cursor (Claude session id, Codex thread id, …).
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
    /// `graceful` or `error`.
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
    /// `active|idle|archived|closed|compacted|error`.
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

/// Token accounting of one turn (or of the thread, for the snapshot event).
/// `input_tokens` excludes cache reads/writes, which are counted apart.
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
    /// `complete|partial|unavailable`.
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
    /// `pending|inProgress|completed`.
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

/// `item.started|updated|completed`. `data` carries the tool input/output
/// (`{input, output}`), clipped by the driver.
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

/// What an approval is about. Every field optional: a command approval sets
/// `command`/`cwd`, an edit sets `diff` and `paths`, a native tool sets
/// `toolName`/`input`/`preview`.
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
    /// Unified diff / patch of the edit being approved.
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
    /// Free text: `stale`, `answered-elsewhere`, `timeout`, …
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
    /// `"message"` for async questions answered by a normal user message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserInputResolvedPayload {
    pub answers: Value,
}

/// Sub-agents and background tasks (`task.*`). One shape for the four events.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPayload {
    pub task_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// `pending|running|waiting|idle|completed|failed|cancelled|interrupted|stopped`.
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
    /// `success|error|cancelled` on `hook.completed`.
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

/// One subscription window (5 h, weekly, credits…), normalized at the driver.
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
    /// ISO-8601.
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
    /// `claude.stream-json`, `codex.app-server.notification`, `acp.jsonrpc`, …
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    pub payload: Value,
}

/// The 49 T3 event types + `raw`. Serialized adjacently tagged:
/// `{"type": "content.delta", "payload": {...}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum RuntimeEventKind {
    // session.* (4)
    #[serde(rename = "session.started")]
    SessionStarted(SessionStartedPayload),
    #[serde(rename = "session.configured")]
    SessionConfigured(ValuePayload),
    #[serde(rename = "session.state.changed")]
    SessionStateChanged(SessionStatePayload),
    #[serde(rename = "session.exited")]
    SessionExited(SessionExitedPayload),
    // thread.* (9)
    #[serde(rename = "thread.started")]
    ThreadStarted(ThreadStartedPayload),
    #[serde(rename = "thread.state.changed")]
    ThreadStateChanged(ThreadStatePayload),
    #[serde(rename = "thread.metadata.updated")]
    ThreadMetadataUpdated(ThreadMetadataPayload),
    /// T3 `thread.token-usage.updated`; the plan calls it `usage.updated`.
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
    // turn.* (7)
    #[serde(rename = "turn.started")]
    TurnStarted(TurnStartedPayload),
    #[serde(rename = "turn.completed")]
    TurnCompleted(TurnCompletedPayload),
    #[serde(rename = "turn.aborted")]
    TurnAborted(TurnAbortedPayload),
    /// The plan calls it `plan.updated`.
    #[serde(rename = "turn.plan.updated", alias = "plan.updated")]
    PlanUpdated(PlanUpdatedPayload),
    #[serde(rename = "turn.proposed.delta")]
    ProposedDelta(DeltaPayload),
    #[serde(rename = "turn.proposed.completed")]
    ProposedCompleted(ProposedCompletedPayload),
    #[serde(rename = "turn.diff.updated")]
    DiffUpdated(DiffUpdatedPayload),
    // item.* + content (4)
    #[serde(rename = "item.started")]
    ItemStarted(ItemPayload),
    #[serde(rename = "item.updated")]
    ItemUpdated(ItemPayload),
    #[serde(rename = "item.completed")]
    ItemCompleted(ItemPayload),
    #[serde(rename = "content.delta")]
    ContentDelta(ContentDeltaPayload),
    // requests (4)
    #[serde(rename = "request.opened")]
    RequestOpened(RequestOpenedPayload),
    #[serde(rename = "request.resolved")]
    RequestResolved(RequestResolvedPayload),
    #[serde(rename = "user-input.requested")]
    UserInputRequested(UserInputRequestedPayload),
    #[serde(rename = "user-input.resolved")]
    UserInputResolved(UserInputResolvedPayload),
    // task.* (4)
    #[serde(rename = "task.started")]
    TaskStarted(TaskPayload),
    #[serde(rename = "task.progress")]
    TaskProgress(TaskPayload),
    #[serde(rename = "task.updated")]
    TaskUpdated(TaskPayload),
    #[serde(rename = "task.completed")]
    TaskCompleted(TaskPayload),
    // hook.* (3)
    #[serde(rename = "hook.started")]
    HookStarted(HookPayload),
    #[serde(rename = "hook.progress")]
    HookProgress(HookPayload),
    #[serde(rename = "hook.completed")]
    HookCompleted(HookPayload),
    // tool.* (3)
    #[serde(rename = "tool.progress")]
    ToolProgress(ToolProgressPayload),
    #[serde(rename = "tool.summary")]
    ToolSummary(ToolSummaryPayload),
    #[serde(rename = "tool.denied")]
    ToolDenied(ToolDeniedPayload),
    // account / auth (3)
    #[serde(rename = "auth.status")]
    AuthStatus(AuthStatusPayload),
    #[serde(rename = "account.updated")]
    AccountUpdated(ValuePayload),
    #[serde(rename = "account.rate-limits.updated")]
    RateLimitsUpdated(RateLimitsPayload),
    // mcp (2)
    #[serde(rename = "mcp.status.updated")]
    McpStatusUpdated(ValuePayload),
    #[serde(rename = "mcp.oauth.completed")]
    McpOauthCompleted(McpOauthPayload),
    // misc (6)
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
    // passthrough (the 50th)
    #[serde(rename = "raw")]
    Raw(RawPayload),
}

impl RuntimeEventKind {
    /// The wire name (`"content.delta"`).
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

/// Native ids of the harness, kept for correlation and debugging.
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

/// One normalized event of a harness.
///
/// `turn_id` is the id the threads engine minted in `start_turn` (never the
/// harness's own; that goes in `provider_refs`). `item_id` is stable across
/// `item.started/updated/completed` and `content.delta` of the same item, and
/// `request_id` is stable across `request.opened/resolved`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeEvent {
    pub event_id: String,
    pub driver: String,
    pub instance_id: String,
    pub thread_id: String,
    /// ISO-8601 (RFC 3339).
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
    /// A fresh event with a new id and `now` as `created_at`.
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

// ── Driver trait ────────────────────────────────────────────────────────

/// What a driver can do; the UI hides what is `false`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverCapabilities {
    /// `rollback(thread, turn_count)` cuts the harness's own history.
    pub rollback: bool,
    /// `fork` copies a thread's harness state into a new thread.
    pub fork: bool,
    pub interrupt: bool,
    /// Emits `request.opened` and accepts `respond_request`.
    pub approvals: bool,
    /// Emits `user-input.requested` and accepts `respond_user_input`.
    pub user_input: bool,
    /// The model can change between turns of one thread.
    pub model_switch: bool,
    pub plan_mode: bool,
    pub compaction: bool,
    /// A turn can be continued without a new prompt.
    pub continuation: bool,
    /// A new turn may start while one is running (steering).
    pub steer: bool,
    /// Reports `account.rate-limits.updated`.
    pub rate_limits: bool,
}

/// One environment variable of an instance. `sensitive` values never leave
/// the host: [`DriverInstance::redacted`] blanks them.
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

/// One configured account of a driver.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverInstance {
    /// Slug, `[a-zA-Z][a-zA-Z0-9_-]*`, ≤ 64. The default instance of a driver
    /// has the driver's own id (`native`, `claude`).
    pub id: String,
    /// Driver kind: `native|claude|codex|acp|opencode|…`.
    pub driver: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// The `accounts.json` id when the instance is a CLI account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// `CLAUDE_CONFIG_DIR` / `CODEX_HOME` of the account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<PathBuf>,
    /// Extra command/args for generic drivers (`acp`: the agent command).
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

    /// Copy safe to send to the webview: sensitive env values blanked.
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

/// Error of a driver call. `code` is a stable `ERR_*` string.
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

/// `start_session` input. One session per thread; the driver keeps it until
/// `stop`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStart {
    pub thread_id: String,
    pub instance_id: String,
    /// Workspace (worktree or project root).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The roster agent (native driver only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub access_mode: AccessMode,
    pub interaction_mode: InteractionMode,
    /// The last cursor the driver reported (`session.started.resume` or
    /// `thread.started.providerThreadId`), to resume the harness's session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_cursor: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStart {
    pub thread_id: String,
    /// Minted by the engine; every event of the turn carries it.
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

/// One harness behind the Central. Every method is called by the threads
/// host's reactors; events go out through the [`RuntimeSink`] the factory
/// received, never as return values (a turn streams for minutes).
#[async_trait]
pub trait Driver: Send + Sync {
    fn kind(&self) -> &str;
    fn capabilities(&self) -> DriverCapabilities;
    /// Open (or resume) the harness session of one thread. Idempotent: a
    /// second call for a live session only updates cwd/model/mode.
    async fn start_session(&self, input: SessionStart) -> Result<(), DriverError>;
    /// Start a turn. Returns as soon as the harness accepted it; the turn's
    /// events (`turn.started` … `turn.completed`) come through the sink.
    async fn start_turn(&self, input: TurnStart) -> Result<TurnStartResult, DriverError>;
    async fn interrupt(&self, thread_id: &str, turn_id: Option<&str>) -> Result<(), DriverError>;
    /// Answer a `request.opened`. The driver emits `request.resolved`.
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
    /// Keep only the first `turn_count` turns of the harness's history.
    async fn rollback(&self, thread_id: &str, turn_count: u32) -> Result<(), DriverError>;
    /// Copy `source`'s harness state up to `turn_count` turns into `target`.
    async fn fork(
        &self,
        _source_thread_id: &str,
        _target_thread_id: &str,
        _turn_count: u32,
    ) -> Result<(), DriverError> {
        Err(DriverError::unsupported("fork"))
    }
    /// Access mode changed mid-thread.
    async fn set_access_mode(
        &self,
        _thread_id: &str,
        _mode: AccessMode,
    ) -> Result<(), DriverError> {
        Ok(())
    }
    async fn stop(&self, thread_id: &str) -> Result<(), DriverError>;
}

/// Where a driver pushes its events. Unbounded on purpose: a driver never
/// awaits the host; the host coalesces and persists at its own pace.
pub type RuntimeSink = mpsc::UnboundedSender<RuntimeEvent>;

/// Builds the driver object of one instance.
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

/// Register (or replace) a driver kind. Call it once at startup (the host
/// does it for `native` when the threads engine opens; a round-2 driver adds
/// one line next to it).
pub fn register_driver(reg: DriverRegistration) {
    let mut guard = REGISTRY.write().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(HashMap::new)
        .insert(reg.kind.clone(), reg);
}

pub fn unregister_driver(kind: &str) -> bool {
    let mut guard = REGISTRY.write().unwrap_or_else(|e| e.into_inner());
    guard
        .as_mut()
        .map(|m| m.remove(kind).is_some())
        .unwrap_or(false)
}

pub fn driver_registration(kind: &str) -> Option<DriverRegistration> {
    REGISTRY
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|m| m.get(kind).cloned())
}

/// Registered kinds, sorted.
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

/// Build the driver of an instance; an unknown kind is `ERR_DRIVER_UNAVAILABLE`
/// (the UI shows the instance as "unavailable" instead of breaking).
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

// ── Durable asks (plan §3.3) ────────────────────────────────────────────

static DURABLE: RwLock<Option<HashSet<String>>> = RwLock::new(None);

/// Mark a conversation (a native thread id) as durable: the tool broker's
/// `Ask` then waits for an answer with no timeout, because the question is a
/// persisted `request.opened` the user can answer hours later. Jobs and the
/// old chat are never marked and keep the 120 s timeout.
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn runtime_event_wire_shape_is_flat_with_type_and_payload() {
        let ev = RuntimeEvent::new(
            "native",
            "native",
            "t1",
            Some("turn1"),
            RuntimeEventKind::ContentDelta(ContentDeltaPayload {
                stream_kind: StreamKind::AssistantText,
                delta: "hi".into(),
                content_index: None,
                summary_index: None,
            }),
        )
        .with_item("i1");
        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(v["type"], "content.delta");
        assert_eq!(v["payload"]["streamKind"], "assistant_text");
        assert_eq!(v["threadId"], "t1");
        assert_eq!(v["itemId"], "i1");
        let back: RuntimeEvent = serde_json::from_value(v).unwrap();
        assert_eq!(back, ev);
    }

    #[test]
    fn plan_aliases_decode() {
        let v = json!({
            "eventId": "e", "driver": "codex", "instanceId": "codex", "threadId": "t",
            "createdAt": "2026-01-01T00:00:00Z",
            "type": "usage.updated", "payload": {"usage": {"inputTokens": 3}}
        });
        let ev: RuntimeEvent = serde_json::from_value(v).unwrap();
        assert_eq!(ev.type_name(), "thread.token-usage.updated");
        let v = json!({
            "eventId": "e", "driver": "codex", "instanceId": "codex", "threadId": "t",
            "createdAt": "2026-01-01T00:00:00Z",
            "type": "request.opened",
            "payload": {"requestType": "something_new", "detail": {"paths": ["a.rs"], "diff": "@@"}}
        });
        let ev: RuntimeEvent = serde_json::from_value(v).unwrap();
        match ev.kind {
            RuntimeEventKind::RequestOpened(p) => {
                assert_eq!(p.request_type, RequestType::Unknown);
                assert_eq!(p.detail.unwrap().paths, vec!["a.rs".to_string()]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn redaction_blanks_sensitive_env_only() {
        let mut inst = DriverInstance::new("c1", "claude", "Claude");
        inst.env = vec![
            EnvVar {
                name: "A".into(),
                value: "x".into(),
                sensitive: false,
                value_redacted: false,
            },
            EnvVar {
                name: "TOKEN".into(),
                value: "secret".into(),
                sensitive: true,
                value_redacted: false,
            },
        ];
        let r = inst.redacted();
        assert_eq!(r.env[0].value, "x");
        assert_eq!(r.env[1].value, "");
        assert!(r.env[1].value_redacted);
    }

    #[test]
    fn access_mode_parse_is_safe_by_default() {
        assert_eq!(AccessMode::parse("full-access"), AccessMode::FullAccess);
        assert_eq!(AccessMode::parse("typo"), AccessMode::ApprovalRequired);
    }

    #[test]
    fn durable_registry_round_trip() {
        set_durable_conversation("thr-x", true);
        assert!(is_durable_conversation("thr-x"));
        set_durable_conversation("thr-x", false);
        assert!(!is_durable_conversation("thr-x"));
    }
}
