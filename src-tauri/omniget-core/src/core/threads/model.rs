//! Commands, persisted events and the light in-memory read model the decider
//! works on. Wire shape is camelCase (T3-compatible), tags are T3's names.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::core::llm::drivers::{
    AccessMode, ApprovalDecision, ApprovalOption, InteractionMode, PlanStep, RequestDetail,
    RequestType, RuntimeEvent, SessionState, TokenUsage, UserInputQuestion,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateKind {
    Project,
    Thread,
}

impl AggregateKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AggregateKind::Project => "project",
            AggregateKind::Thread => "thread",
        }
    }
    pub fn parse(s: &str) -> AggregateKind {
        if s == "project" {
            AggregateKind::Project
        } else {
            AggregateKind::Thread
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorKind {
    Client,
    Server,
    Provider,
}

impl ActorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ActorKind::Client => "client",
            ActorKind::Server => "server",
            ActorKind::Provider => "provider",
        }
    }
    pub fn parse(s: &str) -> ActorKind {
        match s {
            "server" => ActorKind::Server,
            "provider" => ActorKind::Provider,
            _ => ActorKind::Client,
        }
    }
    /// T3 `inferActorKind`: the command id prefix decides.
    pub fn of_command(command_id: &str) -> ActorKind {
        if command_id.starts_with("provider:") {
            ActorKind::Provider
        } else if command_id.starts_with("server:") || command_id.starts_with("migrate:") {
            ActorKind::Server
        } else {
            ActorKind::Client
        }
    }
}

// ── Commands ────────────────────────────────────────────────────────────

/// One message of an imported history (C-5 migration, fork).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportMessage {
    #[serde(default)]
    pub message_id: Option<String>,
    /// `user|assistant|system|reasoning`.
    pub role: String,
    pub text: String,
    #[serde(default)]
    pub created_at: Option<String>,
    /// Tool calls done by the assistant in this message, as activities
    /// (`{name, input, output, isError}`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Value>,
}

/// What a client (or the host) asks for. `commandId` is the idempotency key:
/// dispatching the same id twice returns the first result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandEnvelope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
    #[serde(flatten)]
    pub command: Command,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Command {
    #[serde(rename = "project.create", rename_all = "camelCase")]
    ProjectCreate {
        #[serde(default)]
        project_id: Option<String>,
        title: String,
        #[serde(default)]
        workspace_root: String,
    },
    #[serde(rename = "project.meta.update", rename_all = "camelCase")]
    ProjectUpdate {
        project_id: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        workspace_root: Option<String>,
    },
    #[serde(rename = "project.delete", rename_all = "camelCase")]
    ProjectDelete { project_id: String },
    /// `projectId`, or `projectPath` (the project of that folder, created on
    /// the spot when missing). `worktree: true` gives the thread its own git
    /// worktree (branch `omniget/<hex>` off `baseBranch`), set up by the host.
    #[serde(rename = "thread.create", rename_all = "camelCase")]
    ThreadCreate {
        #[serde(default)]
        thread_id: Option<String>,
        #[serde(default)]
        project_id: String,
        #[serde(default, alias = "project_path")]
        project_path: Option<String>,
        #[serde(default)]
        worktree: bool,
        #[serde(default, alias = "base_branch")]
        base_branch: Option<String>,
        #[serde(default)]
        title: Option<String>,
        instance_id: String,
        driver: String,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        agent_id: Option<String>,
        #[serde(default)]
        runtime_mode: AccessMode,
        #[serde(default)]
        interaction_mode: InteractionMode,
        #[serde(default)]
        branch: Option<String>,
        #[serde(default)]
        worktree_path: Option<String>,
    },
    #[serde(rename = "thread.turn.start", rename_all = "camelCase")]
    TurnStart {
        thread_id: String,
        #[serde(default)]
        turn_id: Option<String>,
        #[serde(default)]
        message_id: Option<String>,
        text: String,
        #[serde(default)]
        attachments: Vec<Value>,
        #[serde(default)]
        model: Option<String>,
    },
    #[serde(rename = "thread.turn.interrupt", rename_all = "camelCase")]
    TurnInterrupt {
        thread_id: String,
        #[serde(default)]
        turn_id: Option<String>,
    },
    #[serde(rename = "thread.approval.respond", rename_all = "camelCase")]
    ApprovalRespond {
        thread_id: String,
        request_id: String,
        decision: ApprovalDecision,
    },
    #[serde(rename = "thread.user-input.respond", rename_all = "camelCase")]
    UserInputRespond {
        thread_id: String,
        request_id: String,
        answers: Value,
    },
    #[serde(rename = "thread.archive", rename_all = "camelCase")]
    Archive { thread_id: String },
    #[serde(rename = "thread.unarchive", rename_all = "camelCase")]
    Unarchive { thread_id: String },
    #[serde(rename = "thread.pin", rename_all = "camelCase")]
    Pin { thread_id: String },
    #[serde(rename = "thread.unpin", rename_all = "camelCase")]
    Unpin { thread_id: String },
    #[serde(rename = "thread.snooze", rename_all = "camelCase")]
    Snooze { thread_id: String, until: String },
    #[serde(rename = "thread.unsnooze", rename_all = "camelCase")]
    Unsnooze { thread_id: String },
    #[serde(rename = "thread.rename", rename_all = "camelCase")]
    Rename { thread_id: String, title: String },
    #[serde(rename = "thread.visit", rename_all = "camelCase")]
    Visit { thread_id: String },
    #[serde(rename = "thread.delete", rename_all = "camelCase")]
    ThreadDelete { thread_id: String },
    #[serde(rename = "thread.runtime-mode.set", rename_all = "camelCase")]
    SetRuntimeMode {
        thread_id: String,
        runtime_mode: AccessMode,
    },
    #[serde(rename = "thread.interaction-mode.set", rename_all = "camelCase")]
    SetInteractionMode {
        thread_id: String,
        interaction_mode: InteractionMode,
    },
    #[serde(rename = "thread.instance.set", rename_all = "camelCase")]
    SetInstance {
        thread_id: String,
        instance_id: String,
        driver: String,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        agent_id: Option<String>,
    },
    /// Keep the first `turnCount` turns (conversation revert; files are T2).
    #[serde(rename = "thread.revert", rename_all = "camelCase")]
    Revert { thread_id: String, turn_count: u32 },
    /// New thread with the history of `threadId` up to `turnCount` turns
    /// (all when absent).
    #[serde(rename = "thread.fork", rename_all = "camelCase")]
    Fork {
        thread_id: String,
        #[serde(default)]
        new_thread_id: Option<String>,
        #[serde(default)]
        turn_count: Option<u32>,
        #[serde(default)]
        title: Option<String>,
    },
    #[serde(rename = "thread.session.stop", rename_all = "camelCase")]
    SessionStop { thread_id: String },
    /// "Edit from here": keep the first `turn` turns; with `restoreFiles` the
    /// host also puts the files back to checkpoint `turn` before cutting the
    /// conversation (and the driver rolls back).
    #[serde(
        rename = "thread.revert-to-turn",
        alias = "thread.revert_to_turn",
        rename_all = "camelCase"
    )]
    RevertToTurn {
        thread_id: String,
        turn: u32,
        #[serde(default, alias = "restore_files")]
        restore_files: bool,
    },
    /// Internal: a fact the host observed (worktree setup, checkpoint, PR,
    /// terminal, cost, external link). Only the kinds in
    /// `decider::HOST_RECORDABLE` pass.
    #[serde(rename = "thread.host.record", rename_all = "camelCase")]
    HostRecord { event: DomainEvent },
    /// Internal: one normalized runtime event of a driver.
    #[serde(rename = "thread.runtime.append", rename_all = "camelCase")]
    RuntimeAppend { event: RuntimeEvent },
    /// Internal: a whole history at once (C-5 migration).
    #[serde(rename = "thread.history.import", rename_all = "camelCase")]
    HistoryImport {
        thread_id: String,
        project_id: String,
        title: String,
        instance_id: String,
        driver: String,
        #[serde(default)]
        agent_id: Option<String>,
        #[serde(default)]
        created_at: Option<String>,
        messages: Vec<ImportMessage>,
    },
}

impl Command {
    pub fn type_name(&self) -> &'static str {
        use Command::*;
        match self {
            ProjectCreate { .. } => "project.create",
            ProjectUpdate { .. } => "project.meta.update",
            ProjectDelete { .. } => "project.delete",
            ThreadCreate { .. } => "thread.create",
            TurnStart { .. } => "thread.turn.start",
            TurnInterrupt { .. } => "thread.turn.interrupt",
            ApprovalRespond { .. } => "thread.approval.respond",
            UserInputRespond { .. } => "thread.user-input.respond",
            Archive { .. } => "thread.archive",
            Unarchive { .. } => "thread.unarchive",
            Pin { .. } => "thread.pin",
            Unpin { .. } => "thread.unpin",
            Snooze { .. } => "thread.snooze",
            Unsnooze { .. } => "thread.unsnooze",
            Rename { .. } => "thread.rename",
            Visit { .. } => "thread.visit",
            ThreadDelete { .. } => "thread.delete",
            SetRuntimeMode { .. } => "thread.runtime-mode.set",
            SetInteractionMode { .. } => "thread.interaction-mode.set",
            SetInstance { .. } => "thread.instance.set",
            Revert { .. } => "thread.revert",
            Fork { .. } => "thread.fork",
            SessionStop { .. } => "thread.session.stop",
            RevertToTurn { .. } => "thread.revert-to-turn",
            HostRecord { .. } => "thread.host.record",
            RuntimeAppend { .. } => "thread.runtime.append",
            HistoryImport { .. } => "thread.history.import",
        }
    }

    /// The aggregate the receipt is filed under.
    pub fn aggregate(&self) -> (AggregateKind, String) {
        use Command::*;
        match self {
            ProjectCreate { project_id, .. } => (
                AggregateKind::Project,
                project_id.clone().unwrap_or_default(),
            ),
            ProjectUpdate { project_id, .. } | ProjectDelete { project_id } => {
                (AggregateKind::Project, project_id.clone())
            }
            ThreadCreate { thread_id, .. } => {
                (AggregateKind::Thread, thread_id.clone().unwrap_or_default())
            }
            Fork { new_thread_id, .. } => (
                AggregateKind::Thread,
                new_thread_id.clone().unwrap_or_default(),
            ),
            RuntimeAppend { event } => (AggregateKind::Thread, event.thread_id.clone()),
            HostRecord { event } => {
                let (k, id) = event.aggregate();
                (k, id.to_string())
            }
            TurnStart { thread_id, .. }
            | TurnInterrupt { thread_id, .. }
            | ApprovalRespond { thread_id, .. }
            | UserInputRespond { thread_id, .. }
            | Archive { thread_id }
            | Unarchive { thread_id }
            | Pin { thread_id }
            | Unpin { thread_id }
            | Snooze { thread_id, .. }
            | Unsnooze { thread_id }
            | Rename { thread_id, .. }
            | Visit { thread_id }
            | ThreadDelete { thread_id }
            | SetRuntimeMode { thread_id, .. }
            | SetInteractionMode { thread_id, .. }
            | SetInstance { thread_id, .. }
            | Revert { thread_id, .. }
            | RevertToTurn { thread_id, .. }
            | SessionStop { thread_id }
            | HistoryImport { thread_id, .. } => (AggregateKind::Thread, thread_id.clone()),
        }
    }
}

// ── Events ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkedFrom {
    pub thread_id: String,
    pub turn_count: u32,
}

/// One entry of the timeline besides messages: tool calls, tasks, hooks,
/// errors, reroutes… `activityId` is stable across the lifecycle of an item
/// so later events replace the row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Activity {
    pub activity_id: String,
    #[serde(default)]
    pub turn_id: Option<String>,
    pub kind: String,
    /// `info|tool|approval|error|warning`.
    pub tone: String,
    pub summary: String,
    pub payload: Value,
    pub created_at: String,
}

/// The worktree a `thread.create` asked for.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<String>,
}

/// One file of a turn's diff (numstat).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckpointFile {
    pub path: String,
    /// `added|deleted|modified|renamed|binary`.
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum DomainEvent {
    #[serde(rename = "project.created", rename_all = "camelCase")]
    ProjectCreated {
        project_id: String,
        title: String,
        workspace_root: String,
        created_at: String,
    },
    #[serde(rename = "project.meta-updated", rename_all = "camelCase")]
    ProjectMetaUpdated {
        project_id: String,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        workspace_root: Option<String>,
        updated_at: String,
    },
    #[serde(rename = "project.deleted", rename_all = "camelCase")]
    ProjectDeleted {
        project_id: String,
        deleted_at: String,
    },
    #[serde(rename = "thread.created", rename_all = "camelCase")]
    ThreadCreated {
        thread_id: String,
        project_id: String,
        title: String,
        instance_id: String,
        driver: String,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        agent_id: Option<String>,
        runtime_mode: AccessMode,
        interaction_mode: InteractionMode,
        #[serde(default)]
        branch: Option<String>,
        #[serde(default)]
        worktree_path: Option<String>,
        #[serde(default)]
        forked_from: Option<ForkedFrom>,
        /// A worktree was asked for; the host sets it up
        /// (`thread.worktree-updated`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worktree: Option<WorktreeSpec>,
        created_at: String,
    },
    #[serde(rename = "thread.deleted", rename_all = "camelCase")]
    ThreadDeleted {
        thread_id: String,
        deleted_at: String,
    },
    #[serde(rename = "thread.archived", rename_all = "camelCase")]
    ThreadArchived {
        thread_id: String,
        archived_at: String,
    },
    #[serde(rename = "thread.unarchived", rename_all = "camelCase")]
    ThreadUnarchived { thread_id: String },
    #[serde(rename = "thread.pinned", rename_all = "camelCase")]
    ThreadPinned {
        thread_id: String,
        pinned_at: String,
    },
    #[serde(rename = "thread.unpinned", rename_all = "camelCase")]
    ThreadUnpinned { thread_id: String },
    #[serde(rename = "thread.snoozed", rename_all = "camelCase")]
    ThreadSnoozed {
        thread_id: String,
        snoozed_until: String,
    },
    #[serde(rename = "thread.unsnoozed", rename_all = "camelCase")]
    ThreadUnsnoozed { thread_id: String },
    #[serde(rename = "thread.meta-updated", rename_all = "camelCase")]
    ThreadMetaUpdated {
        thread_id: String,
        #[serde(default)]
        title: Option<String>,
        updated_at: String,
    },
    #[serde(rename = "thread.visited", rename_all = "camelCase")]
    ThreadVisited {
        thread_id: String,
        visited_at: String,
    },
    #[serde(rename = "thread.runtime-mode-set", rename_all = "camelCase")]
    RuntimeModeSet {
        thread_id: String,
        runtime_mode: AccessMode,
    },
    #[serde(rename = "thread.interaction-mode-set", rename_all = "camelCase")]
    InteractionModeSet {
        thread_id: String,
        interaction_mode: InteractionMode,
    },
    #[serde(rename = "thread.instance-set", rename_all = "camelCase")]
    InstanceSet {
        thread_id: String,
        instance_id: String,
        driver: String,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        agent_id: Option<String>,
    },
    /// `streaming: true` appends `text`; `false` with a non-empty `text`
    /// replaces it; `false` with an empty `text` only closes the stream.
    #[serde(rename = "thread.message-sent", rename_all = "camelCase")]
    MessageSent {
        thread_id: String,
        message_id: String,
        role: String,
        text: String,
        #[serde(default)]
        turn_id: Option<String>,
        streaming: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<Value>,
        created_at: String,
    },
    #[serde(rename = "thread.turn-start-requested", rename_all = "camelCase")]
    TurnStartRequested {
        thread_id: String,
        turn_id: String,
        message_id: String,
        ordinal: u32,
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<Value>,
        #[serde(default)]
        model: Option<String>,
        instance_id: String,
        driver: String,
        runtime_mode: AccessMode,
        interaction_mode: InteractionMode,
        requested_at: String,
    },
    #[serde(rename = "thread.turn-started", rename_all = "camelCase")]
    TurnStarted {
        thread_id: String,
        turn_id: String,
        #[serde(default)]
        model: Option<String>,
        started_at: String,
    },
    /// A finished turn copied from elsewhere (fork, C-5 migration). Reactors
    /// never act on it.
    #[serde(rename = "thread.turn-imported", rename_all = "camelCase")]
    TurnImported {
        thread_id: String,
        turn_id: String,
        ordinal: u32,
        message_id: String,
        state: String,
        requested_at: String,
        #[serde(default)]
        completed_at: Option<String>,
    },
    /// `state`: `completed|failed|interrupted|cancelled`.
    #[serde(rename = "thread.turn-completed", rename_all = "camelCase")]
    TurnCompleted {
        thread_id: String,
        turn_id: String,
        state: String,
        #[serde(default)]
        error_message: Option<String>,
        completed_at: String,
    },
    #[serde(rename = "thread.turn-usage", rename_all = "camelCase")]
    TurnUsage {
        thread_id: String,
        turn_id: String,
        usage: TokenUsage,
    },
    #[serde(rename = "thread.turn-interrupt-requested", rename_all = "camelCase")]
    TurnInterruptRequested {
        thread_id: String,
        #[serde(default)]
        turn_id: Option<String>,
    },
    #[serde(rename = "thread.session-set", rename_all = "camelCase")]
    SessionSet {
        thread_id: String,
        #[serde(default)]
        status: Option<SessionState>,
        #[serde(default)]
        last_error: Option<String>,
        #[serde(default)]
        resume_cursor: Option<Value>,
        #[serde(default)]
        provider_thread_id: Option<String>,
        updated_at: String,
    },
    #[serde(rename = "thread.approval-requested", rename_all = "camelCase")]
    ApprovalRequested {
        thread_id: String,
        #[serde(default)]
        turn_id: Option<String>,
        request_id: String,
        request_type: RequestType,
        #[serde(default)]
        detail: Option<RequestDetail>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        options: Vec<ApprovalOption>,
        created_at: String,
    },
    #[serde(
        rename = "thread.approval-response-requested",
        rename_all = "camelCase"
    )]
    ApprovalResponseRequested {
        thread_id: String,
        request_id: String,
        decision: ApprovalDecision,
    },
    #[serde(rename = "thread.approval-resolved", rename_all = "camelCase")]
    ApprovalResolved {
        thread_id: String,
        request_id: String,
        #[serde(default)]
        decision: Option<ApprovalDecision>,
        #[serde(default)]
        resolution: Option<String>,
        resolved_at: String,
    },
    #[serde(rename = "thread.user-input-requested", rename_all = "camelCase")]
    UserInputRequested {
        thread_id: String,
        #[serde(default)]
        turn_id: Option<String>,
        request_id: String,
        questions: Vec<UserInputQuestion>,
        #[serde(default)]
        response_mode: Option<String>,
        created_at: String,
    },
    #[serde(
        rename = "thread.user-input-response-requested",
        rename_all = "camelCase"
    )]
    UserInputResponseRequested {
        thread_id: String,
        request_id: String,
        answers: Value,
    },
    #[serde(rename = "thread.user-input-resolved", rename_all = "camelCase")]
    UserInputResolved {
        thread_id: String,
        request_id: String,
        #[serde(default)]
        answers: Option<Value>,
        #[serde(default)]
        resolution: Option<String>,
        resolved_at: String,
    },
    #[serde(rename = "thread.plan-updated", rename_all = "camelCase")]
    PlanUpdated {
        thread_id: String,
        plan_id: String,
        #[serde(default)]
        turn_id: Option<String>,
        #[serde(default)]
        explanation: Option<String>,
        #[serde(default)]
        steps: Option<Vec<PlanStep>>,
        #[serde(default)]
        markdown: Option<String>,
        updated_at: String,
    },
    #[serde(rename = "thread.activity-appended", rename_all = "camelCase")]
    ActivityAppended {
        thread_id: String,
        activity: Activity,
    },
    #[serde(rename = "thread.reverted", rename_all = "camelCase")]
    Reverted { thread_id: String, turn_count: u32 },
    #[serde(rename = "thread.session-stop-requested", rename_all = "camelCase")]
    SessionStopRequested { thread_id: String },
    /// `state`: `pending|preparing|ready|failed|removed`. `setup` is the
    /// vcs `SetupSnapshot` (stages fetch/checkout/submodules/setup-script).
    #[serde(rename = "thread.worktree-updated", rename_all = "camelCase")]
    WorktreeUpdated {
        thread_id: String,
        state: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        branch: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        setup: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
        updated_at: String,
    },
    /// Checkpoint `turnCount` (0 = baseline before the first turn) with the
    /// numstat of the turn (checkpoint N-1 → N).
    #[serde(rename = "thread.checkpoint-captured", rename_all = "camelCase")]
    CheckpointCaptured {
        thread_id: String,
        turn_count: u32,
        #[serde(default)]
        turn_id: Option<String>,
        #[serde(rename = "ref")]
        ref_name: String,
        commit: String,
        /// `ready|error`.
        status: String,
        #[serde(default)]
        files: Vec<CheckpointFile>,
        #[serde(default)]
        additions: u32,
        #[serde(default)]
        deletions: u32,
        captured_at: String,
    },
    #[serde(rename = "thread.revert-requested", rename_all = "camelCase")]
    RevertRequested {
        thread_id: String,
        turn_count: u32,
        restore_files: bool,
        requested_at: String,
    },
    #[serde(rename = "thread.files-restored", rename_all = "camelCase")]
    FilesRestored {
        thread_id: String,
        turn_count: u32,
        files: Vec<String>,
        restored_at: String,
    },
    /// The PR/MR of the thread's branch (`null` clears the badge).
    #[serde(rename = "thread.pr-updated", rename_all = "camelCase")]
    PrUpdated {
        thread_id: String,
        #[serde(default)]
        pr: Option<Value>,
        updated_at: String,
    },
    #[serde(rename = "thread.terminal-attached", rename_all = "camelCase")]
    TerminalAttached {
        thread_id: String,
        terminal_id: String,
        #[serde(default)]
        cwd: Option<String>,
        attached_at: String,
    },
    #[serde(rename = "thread.terminal-closed", rename_all = "camelCase")]
    TerminalClosed {
        thread_id: String,
        terminal_id: String,
    },
    /// A read-only thread mirroring a session of a CLI run outside OmniGet.
    #[serde(rename = "thread.external-linked", rename_all = "camelCase")]
    ExternalLinked {
        thread_id: String,
        tool: String,
        session_id: String,
        #[serde(default)]
        source: Option<String>,
        #[serde(default)]
        resume_command: Option<String>,
        linked_at: String,
    },
}

impl DomainEvent {
    pub fn type_name(&self) -> &'static str {
        use DomainEvent::*;
        match self {
            ProjectCreated { .. } => "project.created",
            ProjectMetaUpdated { .. } => "project.meta-updated",
            ProjectDeleted { .. } => "project.deleted",
            ThreadCreated { .. } => "thread.created",
            ThreadDeleted { .. } => "thread.deleted",
            ThreadArchived { .. } => "thread.archived",
            ThreadUnarchived { .. } => "thread.unarchived",
            ThreadPinned { .. } => "thread.pinned",
            ThreadUnpinned { .. } => "thread.unpinned",
            ThreadSnoozed { .. } => "thread.snoozed",
            ThreadUnsnoozed { .. } => "thread.unsnoozed",
            ThreadMetaUpdated { .. } => "thread.meta-updated",
            ThreadVisited { .. } => "thread.visited",
            RuntimeModeSet { .. } => "thread.runtime-mode-set",
            InteractionModeSet { .. } => "thread.interaction-mode-set",
            InstanceSet { .. } => "thread.instance-set",
            MessageSent { .. } => "thread.message-sent",
            TurnStartRequested { .. } => "thread.turn-start-requested",
            TurnStarted { .. } => "thread.turn-started",
            TurnImported { .. } => "thread.turn-imported",
            TurnCompleted { .. } => "thread.turn-completed",
            TurnUsage { .. } => "thread.turn-usage",
            TurnInterruptRequested { .. } => "thread.turn-interrupt-requested",
            SessionSet { .. } => "thread.session-set",
            ApprovalRequested { .. } => "thread.approval-requested",
            ApprovalResponseRequested { .. } => "thread.approval-response-requested",
            ApprovalResolved { .. } => "thread.approval-resolved",
            UserInputRequested { .. } => "thread.user-input-requested",
            UserInputResponseRequested { .. } => "thread.user-input-response-requested",
            UserInputResolved { .. } => "thread.user-input-resolved",
            PlanUpdated { .. } => "thread.plan-updated",
            ActivityAppended { .. } => "thread.activity-appended",
            Reverted { .. } => "thread.reverted",
            SessionStopRequested { .. } => "thread.session-stop-requested",
            WorktreeUpdated { .. } => "thread.worktree-updated",
            CheckpointCaptured { .. } => "thread.checkpoint-captured",
            RevertRequested { .. } => "thread.revert-requested",
            FilesRestored { .. } => "thread.files-restored",
            PrUpdated { .. } => "thread.pr-updated",
            TerminalAttached { .. } => "thread.terminal-attached",
            TerminalClosed { .. } => "thread.terminal-closed",
            ExternalLinked { .. } => "thread.external-linked",
        }
    }

    pub fn aggregate(&self) -> (AggregateKind, &str) {
        use DomainEvent::*;
        match self {
            ProjectCreated { project_id, .. }
            | ProjectMetaUpdated { project_id, .. }
            | ProjectDeleted { project_id, .. } => (AggregateKind::Project, project_id),
            ThreadCreated { thread_id, .. }
            | ThreadDeleted { thread_id, .. }
            | ThreadArchived { thread_id, .. }
            | ThreadUnarchived { thread_id }
            | ThreadPinned { thread_id, .. }
            | ThreadUnpinned { thread_id }
            | ThreadSnoozed { thread_id, .. }
            | ThreadUnsnoozed { thread_id }
            | ThreadMetaUpdated { thread_id, .. }
            | ThreadVisited { thread_id, .. }
            | RuntimeModeSet { thread_id, .. }
            | InteractionModeSet { thread_id, .. }
            | InstanceSet { thread_id, .. }
            | MessageSent { thread_id, .. }
            | TurnStartRequested { thread_id, .. }
            | TurnStarted { thread_id, .. }
            | TurnImported { thread_id, .. }
            | TurnCompleted { thread_id, .. }
            | TurnUsage { thread_id, .. }
            | TurnInterruptRequested { thread_id, .. }
            | SessionSet { thread_id, .. }
            | ApprovalRequested { thread_id, .. }
            | ApprovalResponseRequested { thread_id, .. }
            | ApprovalResolved { thread_id, .. }
            | UserInputRequested { thread_id, .. }
            | UserInputResponseRequested { thread_id, .. }
            | UserInputResolved { thread_id, .. }
            | PlanUpdated { thread_id, .. }
            | ActivityAppended { thread_id, .. }
            | Reverted { thread_id, .. }
            | SessionStopRequested { thread_id }
            | WorktreeUpdated { thread_id, .. }
            | CheckpointCaptured { thread_id, .. }
            | RevertRequested { thread_id, .. }
            | FilesRestored { thread_id, .. }
            | PrUpdated { thread_id, .. }
            | TerminalAttached { thread_id, .. }
            | TerminalClosed { thread_id, .. }
            | ExternalLinked { thread_id, .. } => (AggregateKind::Thread, thread_id),
        }
    }

    /// Split into `(type, payload)` for storage.
    pub fn to_parts(&self) -> (String, Value) {
        let v = serde_json::to_value(self).unwrap_or(Value::Null);
        let payload = v.get("payload").cloned().unwrap_or(Value::Null);
        (self.type_name().to_string(), payload)
    }

    pub fn from_parts(event_type: &str, payload: Value) -> Result<Self, serde_json::Error> {
        serde_json::from_value(serde_json::json!({ "type": event_type, "payload": payload }))
    }
}

/// An event as stored and as sent to clients (`threads://event`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredEvent {
    pub sequence: i64,
    pub event_id: String,
    pub aggregate_kind: AggregateKind,
    pub stream_id: String,
    pub stream_version: i64,
    pub occurred_at: String,
    #[serde(default)]
    pub command_id: Option<String>,
    #[serde(default)]
    pub causation_id: Option<String>,
    #[serde(default)]
    pub correlation_id: Option<String>,
    pub actor_kind: ActorKind,
    #[serde(flatten)]
    pub event: DomainEvent,
    #[serde(default)]
    pub metadata: Value,
}

// ── Light read model (decider state) ────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectState {
    pub id: String,
    pub title: String,
    pub workspace_root: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenRequest {
    pub turn_id: Option<String>,
    pub request_type: Option<RequestType>,
    /// A response was already sent to the driver; waiting for `resolved`.
    pub answered: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThreadState {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub instance_id: String,
    pub driver: String,
    pub model: Option<String>,
    pub agent_id: Option<String>,
    pub runtime_mode: AccessMode,
    pub interaction_mode: InteractionMode,
    pub archived: bool,
    pub deleted: bool,
    pub pinned: bool,
    pub snoozed_until: Option<String>,
    /// Ordinal of the last turn (= number of turns).
    pub turn_count: u32,
    /// The pending or running turn.
    pub active_turn: Option<String>,
    pub session_status: Option<SessionState>,
    pub open_approvals: HashMap<String, OpenRequest>,
    pub open_inputs: HashMap<String, OpenRequest>,
    /// Message ids that exist (to tell a first delta from a later one).
    pub has_user_message: bool,
    /// `pending|preparing|ready|failed|removed`; `None` = no worktree.
    pub worktree_state: Option<String>,
    /// Mirror of a session run outside OmniGet: read-only.
    pub external: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ReadModel {
    pub sequence: i64,
    pub projects: HashMap<String, ProjectState>,
    pub threads: HashMap<String, ThreadState>,
}

impl ReadModel {
    /// The in-memory projector: pure, used by the engine after each commit
    /// and by the tests to replay a log.
    pub fn apply(&mut self, stored: &StoredEvent) {
        self.sequence = self.sequence.max(stored.sequence);
        self.apply_event(&stored.event);
    }

    pub fn apply_event(&mut self, event: &DomainEvent) {
        use DomainEvent::*;
        match event {
            ProjectCreated {
                project_id,
                title,
                workspace_root,
                ..
            } => {
                self.projects.insert(
                    project_id.clone(),
                    ProjectState {
                        id: project_id.clone(),
                        title: title.clone(),
                        workspace_root: workspace_root.clone(),
                        deleted: false,
                    },
                );
            }
            ProjectMetaUpdated {
                project_id,
                title,
                workspace_root,
                ..
            } => {
                if let Some(p) = self.projects.get_mut(project_id) {
                    if let Some(t) = title {
                        p.title = t.clone();
                    }
                    if let Some(w) = workspace_root {
                        p.workspace_root = w.clone();
                    }
                }
            }
            ProjectDeleted { project_id, .. } => {
                if let Some(p) = self.projects.get_mut(project_id) {
                    p.deleted = true;
                }
            }
            ThreadCreated {
                thread_id,
                project_id,
                title,
                instance_id,
                driver,
                model,
                agent_id,
                runtime_mode,
                interaction_mode,
                worktree,
                ..
            } => {
                self.threads.insert(
                    thread_id.clone(),
                    ThreadState {
                        id: thread_id.clone(),
                        project_id: project_id.clone(),
                        title: title.clone(),
                        instance_id: instance_id.clone(),
                        driver: driver.clone(),
                        model: model.clone(),
                        agent_id: agent_id.clone(),
                        runtime_mode: *runtime_mode,
                        interaction_mode: *interaction_mode,
                        archived: false,
                        deleted: false,
                        pinned: false,
                        snoozed_until: None,
                        turn_count: 0,
                        active_turn: None,
                        session_status: None,
                        open_approvals: HashMap::new(),
                        open_inputs: HashMap::new(),
                        has_user_message: false,
                        worktree_state: worktree.as_ref().map(|_| "pending".to_string()),
                        external: false,
                    },
                );
            }
            other => {
                let (_, id) = other.aggregate();
                let id = id.to_string();
                let Some(t) = self.threads.get_mut(&id) else {
                    return;
                };
                apply_thread(t, other);
            }
        }
    }
}

fn apply_thread(t: &mut ThreadState, event: &DomainEvent) {
    use DomainEvent::*;
    match event {
        ThreadDeleted { .. } => t.deleted = true,
        ThreadArchived { .. } => t.archived = true,
        ThreadUnarchived { .. } => t.archived = false,
        ThreadPinned { .. } => t.pinned = true,
        ThreadUnpinned { .. } => t.pinned = false,
        ThreadSnoozed { snoozed_until, .. } => t.snoozed_until = Some(snoozed_until.clone()),
        ThreadUnsnoozed { .. } => t.snoozed_until = None,
        ThreadMetaUpdated { title, .. } => {
            if let Some(title) = title {
                t.title = title.clone();
            }
        }
        RuntimeModeSet { runtime_mode, .. } => t.runtime_mode = *runtime_mode,
        InteractionModeSet {
            interaction_mode, ..
        } => t.interaction_mode = *interaction_mode,
        InstanceSet {
            instance_id,
            driver,
            model,
            agent_id,
            ..
        } => {
            t.instance_id = instance_id.clone();
            t.driver = driver.clone();
            t.model = model.clone();
            t.agent_id = agent_id.clone();
        }
        MessageSent { role, .. } => {
            if role == "user" {
                t.has_user_message = true;
            }
        }
        TurnStartRequested {
            turn_id,
            ordinal,
            model,
            ..
        } => {
            t.turn_count = t.turn_count.max(*ordinal);
            t.active_turn = Some(turn_id.clone());
            if model.is_some() {
                t.model = model.clone();
            }
        }
        TurnImported { ordinal, .. } => {
            t.turn_count = t.turn_count.max(*ordinal);
        }
        TurnCompleted { turn_id, .. } => {
            if t.active_turn.as_deref() == Some(turn_id.as_str()) {
                t.active_turn = None;
            }
        }
        SessionSet { status, .. } => {
            if let Some(s) = status {
                t.session_status = Some(*s);
            }
        }
        ApprovalRequested {
            request_id,
            turn_id,
            request_type,
            ..
        } => {
            t.open_approvals.insert(
                request_id.clone(),
                OpenRequest {
                    turn_id: turn_id.clone(),
                    request_type: Some(*request_type),
                    answered: false,
                },
            );
        }
        ApprovalResponseRequested { request_id, .. } => {
            if let Some(r) = t.open_approvals.get_mut(request_id) {
                r.answered = true;
            }
        }
        ApprovalResolved { request_id, .. } => {
            t.open_approvals.remove(request_id);
        }
        UserInputResponseRequested { request_id, .. } => {
            if let Some(r) = t.open_inputs.get_mut(request_id) {
                r.answered = true;
            }
        }
        UserInputRequested {
            request_id,
            turn_id,
            ..
        } => {
            t.open_inputs.insert(
                request_id.clone(),
                OpenRequest {
                    turn_id: turn_id.clone(),
                    request_type: None,
                    answered: false,
                },
            );
        }
        UserInputResolved { request_id, .. } => {
            t.open_inputs.remove(request_id);
        }
        WorktreeUpdated { state, .. } => t.worktree_state = Some(state.clone()),
        ExternalLinked { .. } => t.external = true,
        Reverted { turn_count, .. } => {
            t.turn_count = *turn_count;
            t.active_turn = None;
            t.open_approvals.clear();
            t.open_inputs.clear();
        }
        _ => {}
    }
}
