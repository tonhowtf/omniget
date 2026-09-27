/**
 * Wire types of the Central threads engine (Rust: `omniget_core::core::threads`
 * and `core::llm::drivers`). camelCase everywhere, event names as in T3 Code.
 */

import type { PrInfo, SetupSnapshot } from "$lib/central/vcs";

export type AccessMode = "approval-required" | "auto-accept-edits" | "auto" | "full-access";
export type InteractionMode = "default" | "plan";
export type ApprovalDecision = "accept" | "acceptForSession" | "acceptAlways" | "decline" | "cancel";
export type SessionState = "starting" | "ready" | "running" | "waiting" | "stopped" | "error";
export type ThreadStatus = "approval" | "input" | "running" | "error" | "idle";
export type TurnState = "pending" | "running" | "completed" | "failed" | "interrupted" | "cancelled";

export type ItemType =
  | "user_message" | "assistant_message" | "reasoning" | "plan" | "command_execution"
  | "file_change" | "mcp_tool_call" | "dynamic_tool_call" | "collab_agent_tool_call"
  | "web_search" | "image_view" | "review_entered" | "review_exited" | "context_compaction"
  | "error" | "unknown";

export type RequestType =
  | "command_execution_approval" | "file_read_approval" | "file_change_approval"
  | "apply_patch_approval" | "exec_command_approval" | "mcp_elicitation_approval"
  | "permission_approval" | "tool_user_input" | "dynamic_tool_call" | "auth_tokens_refresh"
  | "unknown";

export type TokenUsage = {
  inputTokens: number;
  cachedInputTokens: number;
  cacheWriteTokens: number;
  outputTokens: number;
  reasoningOutputTokens: number;
  usedTokens?: number;
  maxTokens?: number;
  costUsd?: number;
  model?: string;
  durationMs?: number;
  toolUses?: number;
  usageStatus?: string;
};

export type RequestDetail = {
  toolName?: string;
  command?: string;
  cwd?: string;
  reason?: string;
  /** Unified diff / patch of the edit being approved. */
  diff?: string;
  paths?: string[];
  preview?: string;
  input?: unknown;
};

export type ApprovalOption = { id: string; label: string; decision?: ApprovalDecision };

export type UserInputQuestion = {
  id: string;
  header: string;
  question: string;
  options: { label: string; description?: string; value?: string }[];
  allowCustomAnswer: boolean;
  multiSelect: boolean;
};

export type PlanStep = { step: string; status: string };

// ── Read side (projections) ──────────────────────────────────────────────

export type ProjectRow = {
  projectId: string;
  title: string;
  workspaceRoot: string;
  createdAt: string;
  updatedAt: string;
};

/** `pending|preparing|ready|failed|removed` (absent = the thread has no worktree). */
export type WorktreeState = "pending" | "preparing" | "ready" | "failed" | "removed";

/** A read-only thread mirroring a session of a CLI run outside OmniGet. */
export type ExternalLink = {
  tool: string;
  sessionId: string;
  source?: string | null;
  resumeCommand?: string | null;
};

export type ThreadRow = {
  threadId: string;
  projectId: string;
  title: string;
  instanceId: string;
  driver: string;
  model: string | null;
  agentId: string | null;
  runtimeMode: AccessMode;
  interactionMode: InteractionMode;
  branch: string | null;
  worktreePath: string | null;
  forkedFrom: { threadId: string; turnCount: number } | null;
  sessionStatus: SessionState | null;
  lastError: string | null;
  activeTurnId: string | null;
  latestTurnId: string | null;
  turnCount: number;
  pendingApprovalCount: number;
  pendingUserInputCount: number;
  pinnedAt: string | null;
  snoozedUntil: string | null;
  archivedAt: string | null;
  lastVisitedAt: string | null;
  latestUserMessageAt: string | null;
  createdAt: string;
  updatedAt: string;
  baseBranch: string | null;
  worktreeState: WorktreeState | null;
  /** Last worktree setup snapshot (stages with state/percent/tail). */
  worktreeSetup: SetupSnapshot | null;
  /** The PR/MR badge of the thread's branch. */
  pr: PrInfo | null;
  /** PTY session ids opened for this thread. */
  terminals: string[];
  external: ExternalLink | null;
  status: ThreadStatus;
};

export type TurnRow = {
  turnId: string;
  threadId: string;
  ordinal: number;
  messageId: string | null;
  state: TurnState;
  model: string | null;
  requestedAt: string;
  startedAt: string | null;
  completedAt: string | null;
  errorMessage: string | null;
};

export type MessageRow = {
  messageId: string;
  threadId: string;
  turnId: string | null;
  /** `user|assistant|system|reasoning|plan|output`. */
  role: string;
  text: string;
  attachments: unknown[];
  streaming: boolean;
  createdAt: string;
  updatedAt: string;
};

export type ActivityRow = {
  activityId: string;
  threadId: string;
  turnId: string | null;
  /** The runtime event type (`item.completed`, `runtime.error`, `task.progress`…). */
  kind: string;
  tone: "info" | "tool" | "approval" | "error" | "warning";
  summary: string;
  payload: Record<string, unknown>;
  sequence: number | null;
  createdAt: string;
  updatedAt: string;
};

export type ApprovalRow = {
  requestId: string;
  threadId: string;
  turnId: string | null;
  requestType: RequestType;
  detail: RequestDetail | null;
  options: ApprovalOption[];
  status: "pending" | "answered" | "resolved";
  decision: ApprovalDecision | null;
  resolution: string | null;
  createdAt: string;
  resolvedAt: string | null;
};

export type UserInputRow = {
  requestId: string;
  threadId: string;
  turnId: string | null;
  questions: UserInputQuestion[];
  responseMode: string | null;
  status: "pending" | "answered" | "resolved";
  answers: unknown | null;
  resolution: string | null;
  createdAt: string;
  resolvedAt: string | null;
};

export type PlanRow = {
  planId: string;
  threadId: string;
  turnId: string | null;
  explanation: string | null;
  steps: PlanStep[] | null;
  markdown: string | null;
  updatedAt: string;
};

export type UsageRow = {
  turnId: string;
  threadId: string;
  model: string | null;
  inputTokens: number;
  cachedInputTokens: number;
  cacheWriteTokens: number;
  outputTokens: number;
  reasoningOutputTokens: number;
  costUsd: number | null;
  durationMs: number | null;
};

/** One file of a turn's numstat. */
export type CheckpointFile = {
  path: string;
  status: "added" | "deleted" | "modified" | "renamed" | "binary";
  additions: number;
  deletions: number;
};

/** Checkpoint `turnCount` (0 = baseline before the first turn). */
export type CheckpointRow = {
  threadId: string;
  turnCount: number;
  turnId: string | null;
  ref: string | null;
  commit: string | null;
  /** `ready|error`. */
  status: string;
  files: CheckpointFile[];
  additions: number;
  deletions: number;
  createdAt: string;
};

/** What "edit from here" (`threads_revert_to_turn`) gives back. */
export type RevertOutcome = {
  threadId: string;
  turnCount: number;
  /** Files put back (empty when only the conversation was cut). */
  restoredFiles: string[];
  /** Prompt of the first dropped turn, for the composer. */
  prompt: string | null;
  attachments: unknown;
};

export type CommitMessage = { subject: string; body: string; source: "model" | "heuristic"; files: number };

export type LimitWindowView = {
  id: string;
  label: string;
  usedPercent: number | null;
  windowMinutes: number | null;
  resetsAt: string | null;
  source: "live" | "probe";
};

export type InstanceLimitsView = {
  instanceId: string;
  driver: string;
  label: string;
  windows: LimitWindowView[];
  credits: unknown | null;
  liveAt: string | null;
  probeStatus: string | null;
  probeAt: number | null;
};

export type ExternalImport = {
  threadId: string;
  projectId: string;
  turns: number;
  existing: boolean;
  resumeCommand: string | null;
};

/** `mcp_thread_get`: the embedded OmniGet MCP of one thread. */
export type McpThreadState = { threadId: string; enabled: boolean; sessions: number; calls: number };

export type McpCallLogEntry = {
  at: string;
  tool: string;
  caller: "global" | "session";
  threadId: string | null;
  instanceId: string | null;
  ok: boolean;
  ms: number;
  error: string | null;
};

export type ProviderSessionRow = {
  threadId: string;
  instanceId: string;
  driver: string;
  status: SessionState | null;
  resumeCursor: unknown | null;
  providerThreadId: string | null;
  lastError: string | null;
};

export type Snapshot = {
  sequence: number;
  projects: ProjectRow[];
  threads: ThreadRow[];
  pendingApprovals: ApprovalRow[];
  pendingUserInputs: UserInputRow[];
};

export type TurnDetail = {
  turn: TurnRow;
  messages: MessageRow[];
  activities: ActivityRow[];
  approvals: ApprovalRow[];
  userInputs: UserInputRow[];
  plans: PlanRow[];
  usage: UsageRow | null;
  /** Checkpoint taken at the end of this turn, with its file stats. */
  checkpoint?: CheckpointRow | null;
};

export type TurnsPage = {
  threadId: string;
  turns: TurnDetail[];
  looseMessages: MessageRow[];
  looseActivities: ActivityRow[];
  hasMore: boolean;
  beforeTurn: number | null;
  sequence: number;
  threadSequence: number;
  session: ProviderSessionRow | null;
};

// ── Events ───────────────────────────────────────────────────────────────

export type Activity = {
  activityId: string;
  turnId: string | null;
  kind: string;
  tone: ActivityRow["tone"];
  summary: string;
  payload: Record<string, unknown>;
  createdAt: string;
};

/** Persisted domain events: `{type, payload}`. */
export type DomainEvent =
  | { type: "project.created"; payload: { projectId: string; title: string; workspaceRoot: string; createdAt: string } }
  | { type: "project.meta-updated"; payload: { projectId: string; title?: string | null; workspaceRoot?: string | null; updatedAt: string } }
  | { type: "project.deleted"; payload: { projectId: string; deletedAt: string } }
  | {
      type: "thread.created";
      payload: {
        threadId: string; projectId: string; title: string; instanceId: string; driver: string;
        model?: string | null; agentId?: string | null; runtimeMode: AccessMode; interactionMode: InteractionMode;
        branch?: string | null; worktreePath?: string | null;
        forkedFrom?: { threadId: string; turnCount: number } | null;
        /** A worktree was asked for; the host sets it up (`thread.worktree-updated`). */
        worktree?: { baseBranch?: string | null } | null;
        createdAt: string;
      };
    }
  | { type: "thread.deleted"; payload: { threadId: string; deletedAt: string } }
  | { type: "thread.archived"; payload: { threadId: string; archivedAt: string } }
  | { type: "thread.unarchived"; payload: { threadId: string } }
  | { type: "thread.pinned"; payload: { threadId: string; pinnedAt: string } }
  | { type: "thread.unpinned"; payload: { threadId: string } }
  | { type: "thread.snoozed"; payload: { threadId: string; snoozedUntil: string } }
  | { type: "thread.unsnoozed"; payload: { threadId: string } }
  | { type: "thread.meta-updated"; payload: { threadId: string; title?: string | null; updatedAt: string } }
  | { type: "thread.visited"; payload: { threadId: string; visitedAt: string } }
  | { type: "thread.runtime-mode-set"; payload: { threadId: string; runtimeMode: AccessMode } }
  | { type: "thread.interaction-mode-set"; payload: { threadId: string; interactionMode: InteractionMode } }
  | { type: "thread.instance-set"; payload: { threadId: string; instanceId: string; driver: string; model?: string | null; agentId?: string | null } }
  | {
      type: "thread.message-sent";
      payload: {
        threadId: string; messageId: string; role: string; text: string; turnId?: string | null;
        streaming: boolean; attachments?: unknown[]; createdAt: string;
      };
    }
  | {
      type: "thread.turn-start-requested";
      payload: {
        threadId: string; turnId: string; messageId: string; ordinal: number; text: string;
        attachments?: unknown[]; model?: string | null; instanceId: string; driver: string;
        runtimeMode: AccessMode; interactionMode: InteractionMode; requestedAt: string;
      };
    }
  | { type: "thread.turn-started"; payload: { threadId: string; turnId: string; model?: string | null; startedAt: string } }
  | {
      type: "thread.turn-imported";
      payload: {
        threadId: string; turnId: string; ordinal: number; messageId: string; state: TurnState;
        requestedAt: string; completedAt?: string | null;
      };
    }
  | { type: "thread.turn-completed"; payload: { threadId: string; turnId: string; state: TurnState; errorMessage?: string | null; completedAt: string } }
  | { type: "thread.turn-usage"; payload: { threadId: string; turnId: string; usage: TokenUsage } }
  | { type: "thread.turn-interrupt-requested"; payload: { threadId: string; turnId?: string | null } }
  | {
      type: "thread.session-set";
      payload: {
        threadId: string; status?: SessionState | null; lastError?: string | null;
        resumeCursor?: unknown; providerThreadId?: string | null; updatedAt: string;
      };
    }
  | {
      type: "thread.approval-requested";
      payload: {
        threadId: string; turnId?: string | null; requestId: string; requestType: RequestType;
        detail?: RequestDetail | null; options?: ApprovalOption[]; createdAt: string;
      };
    }
  | { type: "thread.approval-response-requested"; payload: { threadId: string; requestId: string; decision: ApprovalDecision } }
  | {
      type: "thread.approval-resolved";
      payload: { threadId: string; requestId: string; decision?: ApprovalDecision | null; resolution?: string | null; resolvedAt: string };
    }
  | {
      type: "thread.user-input-requested";
      payload: {
        threadId: string; turnId?: string | null; requestId: string; questions: UserInputQuestion[];
        responseMode?: string | null; createdAt: string;
      };
    }
  | { type: "thread.user-input-response-requested"; payload: { threadId: string; requestId: string; answers: unknown } }
  | {
      type: "thread.user-input-resolved";
      payload: { threadId: string; requestId: string; answers?: unknown; resolution?: string | null; resolvedAt: string };
    }
  | {
      type: "thread.plan-updated";
      payload: {
        threadId: string; planId: string; turnId?: string | null; explanation?: string | null;
        steps?: PlanStep[] | null; markdown?: string | null; updatedAt: string;
      };
    }
  | { type: "thread.activity-appended"; payload: { threadId: string; activity: Activity } }
  | { type: "thread.reverted"; payload: { threadId: string; turnCount: number } }
  | { type: "thread.session-stop-requested"; payload: { threadId: string } }
  | {
      type: "thread.worktree-updated";
      payload: {
        threadId: string; state: WorktreeState; path?: string | null; branch?: string | null;
        base?: string | null; setup?: SetupSnapshot | null; error?: string | null; updatedAt: string;
      };
    }
  | {
      type: "thread.checkpoint-captured";
      payload: {
        threadId: string; turnCount: number; turnId?: string | null; ref: string; commit: string;
        status: string; files: CheckpointFile[]; additions: number; deletions: number; capturedAt: string;
      };
    }
  | { type: "thread.revert-requested"; payload: { threadId: string; turnCount: number; restoreFiles: boolean; requestedAt: string } }
  | { type: "thread.files-restored"; payload: { threadId: string; turnCount: number; files: string[]; restoredAt: string } }
  | { type: "thread.pr-updated"; payload: { threadId: string; pr?: PrInfo | null; updatedAt: string } }
  | { type: "thread.terminal-attached"; payload: { threadId: string; terminalId: string; cwd?: string | null; attachedAt: string } }
  | { type: "thread.terminal-closed"; payload: { threadId: string; terminalId: string } }
  | {
      type: "thread.external-linked";
      payload: {
        threadId: string; tool: string; sessionId: string; source?: string | null;
        resumeCommand?: string | null; linkedAt: string;
      };
    };

export type StoredEvent = DomainEvent & {
  sequence: number;
  eventId: string;
  aggregateKind: "project" | "thread";
  streamId: string;
  streamVersion: number;
  occurredAt: string;
  commandId: string | null;
  causationId: string | null;
  correlationId: string | null;
  actorKind: "client" | "server" | "provider";
  metadata: Record<string, unknown>;
};

/** `threads://event` payload. */
export type ThreadsEventEnvelope = { sequence: number; event: StoredEvent };

export type EventsPage = { events: StoredEvent[]; head: number; reset: boolean; hasMore: boolean };

export type DispatchResult = {
  commandId: string;
  sequence: number;
  events: StoredEvent[];
  deduplicated: boolean;
};

// ── Commands ─────────────────────────────────────────────────────────────

type C<T extends string, P> = { type: T; commandId?: string } & P;

export type ThreadCommand =
  | C<"project.create", { projectId?: string; title: string; workspaceRoot: string }>
  | C<"project.meta.update", { projectId: string; title?: string; workspaceRoot?: string }>
  | C<"project.delete", { projectId: string }>
  | C<"thread.create", {
      /** `projectId`, or `projectPath` (the project of that folder, created on the spot). */
      threadId?: string; projectId?: string; projectPath?: string; title?: string; instanceId: string; driver: string;
      model?: string; agentId?: string; runtimeMode?: AccessMode; interactionMode?: InteractionMode;
      branch?: string; worktreePath?: string;
      /** Own git worktree (branch `omniget/<hex>` off `baseBranch`), set up by the host. */
      worktree?: boolean; baseBranch?: string;
    }>
  | C<"thread.turn.start", { threadId: string; turnId?: string; messageId?: string; text: string; attachments?: unknown[]; model?: string }>
  | C<"thread.turn.interrupt", { threadId: string; turnId?: string }>
  | C<"thread.approval.respond", { threadId: string; requestId: string; decision: ApprovalDecision }>
  | C<"thread.user-input.respond", { threadId: string; requestId: string; answers: unknown }>
  | C<"thread.archive" | "thread.unarchive" | "thread.pin" | "thread.unpin" | "thread.unsnooze" | "thread.visit" | "thread.delete" | "thread.session.stop", { threadId: string }>
  | C<"thread.snooze", { threadId: string; until: string }>
  | C<"thread.rename", { threadId: string; title: string }>
  | C<"thread.runtime-mode.set", { threadId: string; runtimeMode: AccessMode }>
  | C<"thread.interaction-mode.set", { threadId: string; interactionMode: InteractionMode }>
  | C<"thread.instance.set", { threadId: string; instanceId: string; driver: string; model?: string; agentId?: string }>
  | C<"thread.revert", { threadId: string; turnCount: number }>
  | C<"thread.revert-to-turn", { threadId: string; turn: number; restoreFiles?: boolean }>
  | C<"thread.fork", { threadId: string; newThreadId?: string; turnCount?: number; title?: string }>;

// ── Drivers ──────────────────────────────────────────────────────────────

export type DriverCapabilities = {
  rollback: boolean;
  fork: boolean;
  interrupt: boolean;
  approvals: boolean;
  userInput: boolean;
  modelSwitch: boolean;
  planMode: boolean;
  compaction: boolean;
  continuation: boolean;
  steer: boolean;
  rateLimits: boolean;
};

export type DriverInstance = {
  id: string;
  driver: string;
  label: string;
  color?: string;
  accountId?: string;
  configDir?: string;
  command?: string;
  args?: string[];
  env?: { name: string; value: string; sensitive: boolean; valueRedacted: boolean }[];
  enabled: boolean;
};

export type InstanceView = {
  instance: DriverInstance;
  driverLabel: string;
  available: boolean;
  unavailableReason?: string;
  capabilities: DriverCapabilities;
};

/** A thread's loaded timeline (turn pages merged, oldest first). */
export type ThreadDetail = {
  threadId: string;
  turns: TurnDetail[];
  looseMessages: MessageRow[];
  looseActivities: ActivityRow[];
  hasMore: boolean;
  beforeTurn: number | null;
  session: ProviderSessionRow | null;
  /** Events up to this sequence are already in the page. */
  appliedThrough: number;
  loading: boolean;
};
