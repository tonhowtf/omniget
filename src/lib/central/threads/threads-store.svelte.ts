/**
 * Threads of the Central: snapshot, then live events from `threads://event`
 * applied incrementally (the same projection the backend does in SQL).
 *
 * Sync protocol (T3 style): the listener is attached before the snapshot is
 * read and buffers; after the snapshot, buffered events with a higher
 * sequence are applied. A gap in `sequence` triggers `threads_events_after`;
 * a replay over budget (`reset`) takes a fresh snapshot. Opened threads load
 * 10 turns, then 20 per `loadOlder`; a page read at head H ignores live
 * events with `sequence <= H` so streamed text is never doubled.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import type {
  ActivityRow,
  ApprovalDecision,
  ApprovalRow,
  DispatchResult,
  EventsPage,
  InstanceView,
  MessageRow,
  McpThreadState,
  ProjectRow,
  RevertOutcome,
  Snapshot,
  StoredEvent,
  ThreadCommand,
  ThreadDetail,
  ThreadRow,
  ThreadStatus,
  ThreadsEventEnvelope,
  TurnDetail,
  TurnsPage,
  UserInputRow,
} from "./types";

export const THREADS_EVENT = "threads://event";
export const FIRST_PAGE = 10;
export const NEXT_PAGE = 20;

function statusOf(t: ThreadRow): ThreadStatus {
  if (t.pendingApprovalCount > 0) return "approval";
  if (t.pendingUserInputCount > 0) return "input";
  if (t.activeTurnId) return "running";
  if (t.sessionStatus === "error") return "error";
  return "idle";
}

function emptyTurn(threadId: string, turnId: string, ordinal: number, requestedAt: string): TurnDetail {
  return {
    turn: {
      turnId,
      threadId,
      ordinal,
      messageId: null,
      state: "pending",
      model: null,
      requestedAt,
      startedAt: null,
      completedAt: null,
      errorMessage: null,
    },
    messages: [],
    activities: [],
    approvals: [],
    userInputs: [],
    plans: [],
    usage: null,
    checkpoint: null,
  };
}

/** Shallow merge of JSON objects, like SQLite `json_patch` for our payloads. */
function patch(a: Record<string, unknown>, b: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = { ...a };
  for (const [k, v] of Object.entries(b)) {
    const prev = out[k];
    if (v && typeof v === "object" && !Array.isArray(v) && prev && typeof prev === "object" && !Array.isArray(prev)) {
      out[k] = patch(prev as Record<string, unknown>, v as Record<string, unknown>);
    } else if (v !== null) {
      out[k] = v;
    } else {
      delete out[k];
    }
  }
  return out;
}

class ThreadsStore {
  sequence = $state(0);
  projects = $state<ProjectRow[]>([]);
  threads = $state<ThreadRow[]>([]);
  pendingApprovals = $state<ApprovalRow[]>([]);
  pendingUserInputs = $state<UserInputRow[]>([]);
  details = $state<Record<string, ThreadDetail>>({});
  instances = $state<InstanceView[]>([]);
  ready = $state(false);
  error = $state<string | null>(null);

  #unlisten: UnlistenFn | null = null;
  #starting: Promise<void> | null = null;
  #buffer: StoredEvent[] | null = null;
  #catchingUp = false;

  /** Idempotent: the first caller subscribes and loads the snapshot. */
  init(): Promise<void> {
    if (!this.#starting) this.#starting = this.#start();
    return this.#starting;
  }

  async #start() {
    this.#buffer = [];
    this.#unlisten = await listen<ThreadsEventEnvelope>(THREADS_EVENT, (e) => {
      const ev = e.payload.event;
      if (this.#buffer) this.#buffer.push(ev);
      else this.#receive(ev);
    });
    try {
      await this.#loadSnapshot();
      const buffered = this.#buffer ?? [];
      this.#buffer = null;
      for (const ev of buffered.sort((a, b) => a.sequence - b.sequence)) this.#receive(ev);
      this.ready = true;
      void this.refreshDrivers();
    } catch (e) {
      this.#buffer = null;
      this.error = String(e);
      this.#starting = null;
    }
  }

  destroy() {
    this.#unlisten?.();
    this.#unlisten = null;
    this.#starting = null;
    this.ready = false;
  }

  async #loadSnapshot() {
    const snap = await invoke<Snapshot>("threads_snapshot");
    this.sequence = snap.sequence;
    this.projects = snap.projects;
    this.threads = snap.threads;
    this.pendingApprovals = snap.pendingApprovals;
    this.pendingUserInputs = snap.pendingUserInputs;
  }

  async refreshDrivers() {
    try {
      this.instances = await invoke<InstanceView[]>("threads_drivers");
    } catch (e) {
      this.error = String(e);
    }
  }

  // ── Live events ──────────────────────────────────────────────────────

  #receive(ev: StoredEvent) {
    if (ev.sequence <= this.sequence) return;
    if (ev.sequence > this.sequence + 1) {
      void this.catchUp();
      return;
    }
    this.apply(ev);
  }

  /** Replays what was missed; a replay over budget reloads everything. */
  async catchUp() {
    if (this.#catchingUp) return;
    this.#catchingUp = true;
    try {
      for (;;) {
        const page = await invoke<EventsPage>("threads_events_after", { afterSequence: this.sequence, limit: 1000 });
        if (page.reset) {
          await this.#loadSnapshot();
          const open = Object.keys(this.details);
          this.details = {};
          for (const id of open) await this.openThread(id);
          break;
        }
        for (const ev of page.events) if (ev.sequence === this.sequence + 1) this.apply(ev);
        if (!page.hasMore || page.events.length === 0) break;
      }
    } catch (e) {
      this.error = String(e);
    } finally {
      this.#catchingUp = false;
    }
  }

  #thread(id: string): ThreadRow | undefined {
    return this.threads.find((t) => t.threadId === id);
  }

  #touch(id: string, f: (t: ThreadRow) => void) {
    const t = this.#thread(id);
    if (!t) return;
    f(t);
    t.status = statusOf(t);
  }

  #detail(threadId: string, seq: number): ThreadDetail | undefined {
    const d = this.details[threadId];
    if (!d || seq <= d.appliedThrough) return undefined;
    return d;
  }

  #turnOf(d: ThreadDetail, turnId: string | null | undefined): TurnDetail | undefined {
    if (!turnId) return undefined;
    return d.turns.find((t) => t.turn.turnId === turnId);
  }

  #upsertMessage(list: MessageRow[], m: MessageRow, streaming: boolean) {
    const i = list.findIndex((x) => x.messageId === m.messageId);
    if (i < 0) {
      list.push(m);
      return;
    }
    const cur = list[i];
    if (streaming) cur.text += m.text;
    else if (m.text !== "") cur.text = m.text;
    cur.streaming = streaming;
    cur.updatedAt = m.updatedAt;
  }

  /** Apply one event in sequence order (exported for tests and tools). */
  apply(ev: StoredEvent) {
    this.sequence = ev.sequence;
    const seq = ev.sequence;
    switch (ev.type) {
      case "project.created": {
        const p = ev.payload;
        this.projects = [
          ...this.projects.filter((x) => x.projectId !== p.projectId),
          { projectId: p.projectId, title: p.title, workspaceRoot: p.workspaceRoot, createdAt: p.createdAt, updatedAt: p.createdAt },
        ];
        break;
      }
      case "project.meta-updated": {
        const p = ev.payload;
        const row = this.projects.find((x) => x.projectId === p.projectId);
        if (row) {
          if (p.title != null) row.title = p.title;
          if (p.workspaceRoot != null) row.workspaceRoot = p.workspaceRoot;
          row.updatedAt = p.updatedAt;
        }
        break;
      }
      case "project.deleted":
        this.projects = this.projects.filter((x) => x.projectId !== ev.payload.projectId);
        break;
      case "thread.created": {
        const p = ev.payload;
        const row: ThreadRow = {
          threadId: p.threadId,
          projectId: p.projectId,
          title: p.title,
          instanceId: p.instanceId,
          driver: p.driver,
          model: p.model ?? null,
          agentId: p.agentId ?? null,
          runtimeMode: p.runtimeMode,
          interactionMode: p.interactionMode,
          branch: p.branch ?? null,
          worktreePath: p.worktreePath ?? null,
          forkedFrom: p.forkedFrom ?? null,
          sessionStatus: null,
          lastError: null,
          activeTurnId: null,
          latestTurnId: null,
          turnCount: 0,
          pendingApprovalCount: 0,
          pendingUserInputCount: 0,
          pinnedAt: null,
          snoozedUntil: null,
          archivedAt: null,
          lastVisitedAt: null,
          latestUserMessageAt: null,
          createdAt: p.createdAt,
          updatedAt: p.createdAt,
          baseBranch: p.worktree?.baseBranch ?? null,
          worktreeState: p.worktree ? "pending" : null,
          worktreeSetup: null,
          pr: null,
          terminals: [],
          external: null,
          status: "idle",
        };
        this.threads = [row, ...this.threads.filter((t) => t.threadId !== p.threadId)];
        break;
      }
      case "thread.deleted": {
        const id = ev.payload.threadId;
        this.threads = this.threads.filter((t) => t.threadId !== id);
        this.pendingApprovals = this.pendingApprovals.filter((a) => a.threadId !== id);
        this.pendingUserInputs = this.pendingUserInputs.filter((a) => a.threadId !== id);
        const { [id]: _gone, ...rest } = this.details;
        this.details = rest;
        break;
      }
      case "thread.archived":
        this.#touch(ev.payload.threadId, (t) => (t.archivedAt = ev.payload.archivedAt));
        break;
      case "thread.unarchived":
        this.#touch(ev.payload.threadId, (t) => (t.archivedAt = null));
        break;
      case "thread.pinned":
        this.#touch(ev.payload.threadId, (t) => (t.pinnedAt = ev.payload.pinnedAt));
        break;
      case "thread.unpinned":
        this.#touch(ev.payload.threadId, (t) => (t.pinnedAt = null));
        break;
      case "thread.snoozed":
        this.#touch(ev.payload.threadId, (t) => (t.snoozedUntil = ev.payload.snoozedUntil));
        break;
      case "thread.unsnoozed":
        this.#touch(ev.payload.threadId, (t) => (t.snoozedUntil = null));
        break;
      case "thread.meta-updated":
        this.#touch(ev.payload.threadId, (t) => {
          if (ev.payload.title != null) t.title = ev.payload.title;
          t.updatedAt = ev.payload.updatedAt;
        });
        break;
      case "thread.visited":
        this.#touch(ev.payload.threadId, (t) => (t.lastVisitedAt = ev.payload.visitedAt));
        break;
      case "thread.runtime-mode-set":
        this.#touch(ev.payload.threadId, (t) => (t.runtimeMode = ev.payload.runtimeMode));
        break;
      case "thread.interaction-mode-set":
        this.#touch(ev.payload.threadId, (t) => (t.interactionMode = ev.payload.interactionMode));
        break;
      case "thread.instance-set":
        this.#touch(ev.payload.threadId, (t) => {
          t.instanceId = ev.payload.instanceId;
          t.driver = ev.payload.driver;
          t.model = ev.payload.model ?? null;
          t.agentId = ev.payload.agentId ?? null;
          t.sessionStatus = null;
          t.lastError = null;
        });
        break;
      case "thread.message-sent": {
        const p = ev.payload;
        this.#touch(p.threadId, (t) => {
          t.updatedAt = ev.occurredAt;
          if (p.role === "user" && (!t.latestUserMessageAt || t.latestUserMessageAt < p.createdAt)) {
            t.latestUserMessageAt = p.createdAt;
          }
        });
        const d = this.#detail(p.threadId, seq);
        if (d) {
          const row: MessageRow = {
            messageId: p.messageId,
            threadId: p.threadId,
            turnId: p.turnId ?? null,
            role: p.role,
            text: p.text,
            attachments: p.attachments ?? [],
            streaming: p.streaming,
            createdAt: p.createdAt,
            updatedAt: p.createdAt,
          };
          const turn = this.#turnOf(d, p.turnId);
          if (turn) this.#upsertMessage(turn.messages, row, p.streaming);
          else if (p.turnId && p.role === "user") {
            // The user message arrives just before its turn row.
            const pending = emptyTurn(p.threadId, p.turnId, (d.turns.at(-1)?.turn.ordinal ?? 0) + 1, p.createdAt);
            pending.messages.push(row);
            d.turns.push(pending);
          } else if (!p.turnId) this.#upsertMessage(d.looseMessages, row, p.streaming);
        }
        break;
      }
      case "thread.turn-start-requested": {
        const p = ev.payload;
        this.#touch(p.threadId, (t) => {
          t.turnCount = Math.max(t.turnCount, p.ordinal);
          t.activeTurnId = p.turnId;
          t.latestTurnId = p.turnId;
          if (p.model) t.model = p.model;
          t.updatedAt = p.requestedAt;
        });
        const d = this.#detail(p.threadId, seq);
        if (d) {
          let turn = this.#turnOf(d, p.turnId);
          if (!turn) {
            turn = emptyTurn(p.threadId, p.turnId, p.ordinal, p.requestedAt);
            d.turns.push(turn);
          }
          turn.turn.ordinal = p.ordinal;
          turn.turn.messageId = p.messageId;
          turn.turn.model = p.model ?? null;
          turn.turn.state = "pending";
        }
        break;
      }
      case "thread.turn-imported": {
        const p = ev.payload;
        this.#touch(p.threadId, (t) => {
          if (p.ordinal >= t.turnCount) t.latestTurnId = p.turnId;
          t.turnCount = Math.max(t.turnCount, p.ordinal);
        });
        const d = this.#detail(p.threadId, seq);
        if (d && !this.#turnOf(d, p.turnId)) {
          const turn = emptyTurn(p.threadId, p.turnId, p.ordinal, p.requestedAt);
          turn.turn.state = p.state;
          turn.turn.messageId = p.messageId;
          turn.turn.completedAt = p.completedAt ?? null;
          d.turns.push(turn);
          d.turns.sort((a, b) => a.turn.ordinal - b.turn.ordinal);
        }
        break;
      }
      case "thread.turn-started": {
        const p = ev.payload;
        this.#touch(p.threadId, (t) => (t.updatedAt = p.startedAt));
        const d = this.#detail(p.threadId, seq);
        const turn = d && this.#turnOf(d, p.turnId);
        if (turn && (turn.turn.state === "pending" || turn.turn.state === "running")) {
          turn.turn.state = "running";
          turn.turn.startedAt = p.startedAt;
          if (p.model) turn.turn.model = p.model;
        }
        break;
      }
      case "thread.turn-completed": {
        const p = ev.payload;
        this.#touch(p.threadId, (t) => {
          if (t.activeTurnId === p.turnId) t.activeTurnId = null;
          t.updatedAt = p.completedAt;
        });
        const d = this.#detail(p.threadId, seq);
        const turn = d && this.#turnOf(d, p.turnId);
        if (turn) {
          if (turn.turn.state === "pending" || turn.turn.state === "running") {
            turn.turn.state = p.state;
            turn.turn.completedAt = p.completedAt;
            if (p.errorMessage) turn.turn.errorMessage = p.errorMessage;
          }
          for (const m of turn.messages) m.streaming = false;
        }
        break;
      }
      case "thread.turn-usage": {
        const p = ev.payload;
        const d = this.#detail(p.threadId, seq);
        const turn = d && this.#turnOf(d, p.turnId);
        if (turn) {
          turn.usage = {
            turnId: p.turnId,
            threadId: p.threadId,
            model: p.usage.model ?? turn.usage?.model ?? null,
            inputTokens: p.usage.inputTokens,
            cachedInputTokens: p.usage.cachedInputTokens,
            cacheWriteTokens: p.usage.cacheWriteTokens,
            outputTokens: p.usage.outputTokens,
            reasoningOutputTokens: p.usage.reasoningOutputTokens,
            costUsd: p.usage.costUsd ?? turn.usage?.costUsd ?? null,
            durationMs: p.usage.durationMs ?? turn.usage?.durationMs ?? null,
          };
        }
        break;
      }
      case "thread.turn-interrupt-requested":
      case "thread.session-stop-requested":
        break;
      case "thread.session-set": {
        const p = ev.payload;
        this.#touch(p.threadId, (t) => {
          if (p.status) {
            t.sessionStatus = p.status;
            t.lastError = p.lastError ?? null;
          }
          t.updatedAt = p.updatedAt;
        });
        const d = this.#detail(p.threadId, seq);
        if (d?.session) {
          if (p.status) {
            d.session.status = p.status;
            d.session.lastError = p.lastError ?? null;
          }
          if (p.resumeCursor != null) d.session.resumeCursor = p.resumeCursor;
          if (p.providerThreadId) d.session.providerThreadId = p.providerThreadId;
        }
        break;
      }
      case "thread.approval-requested": {
        const p = ev.payload;
        const row: ApprovalRow = {
          requestId: p.requestId,
          threadId: p.threadId,
          turnId: p.turnId ?? null,
          requestType: p.requestType,
          detail: p.detail ?? null,
          options: p.options ?? [],
          status: "pending",
          decision: null,
          resolution: null,
          createdAt: p.createdAt,
          resolvedAt: null,
        };
        if (!this.pendingApprovals.some((a) => a.threadId === p.threadId && a.requestId === p.requestId)) {
          this.pendingApprovals = [...this.pendingApprovals, row];
        }
        this.#recount(p.threadId);
        const d = this.#detail(p.threadId, seq);
        const turn = d && this.#turnOf(d, p.turnId);
        if (turn && !turn.approvals.some((a) => a.requestId === p.requestId)) turn.approvals.push({ ...row });
        break;
      }
      case "thread.approval-response-requested":
      case "thread.approval-resolved": {
        const p = ev.payload;
        const resolved = ev.type === "thread.approval-resolved";
        this.pendingApprovals = this.pendingApprovals.filter(
          (a) => !(a.threadId === p.threadId && a.requestId === p.requestId),
        );
        this.#recount(p.threadId);
        const d = this.#detail(p.threadId, seq);
        if (d) {
          for (const turn of d.turns) {
            const a = turn.approvals.find((x) => x.requestId === p.requestId);
            if (!a) continue;
            if (resolved && ev.type === "thread.approval-resolved") {
              a.status = "resolved";
              a.decision = ev.payload.decision ?? a.decision;
              a.resolution = ev.payload.resolution ?? null;
              a.resolvedAt = ev.payload.resolvedAt;
            } else if (ev.type === "thread.approval-response-requested" && a.status === "pending") {
              a.status = "answered";
              a.decision = ev.payload.decision;
            }
          }
        }
        break;
      }
      case "thread.user-input-requested": {
        const p = ev.payload;
        const row: UserInputRow = {
          requestId: p.requestId,
          threadId: p.threadId,
          turnId: p.turnId ?? null,
          questions: p.questions,
          responseMode: p.responseMode ?? null,
          status: "pending",
          answers: null,
          resolution: null,
          createdAt: p.createdAt,
          resolvedAt: null,
        };
        if (!this.pendingUserInputs.some((a) => a.threadId === p.threadId && a.requestId === p.requestId)) {
          this.pendingUserInputs = [...this.pendingUserInputs, row];
        }
        this.#recount(p.threadId);
        const d = this.#detail(p.threadId, seq);
        const turn = d && this.#turnOf(d, p.turnId);
        if (turn && !turn.userInputs.some((a) => a.requestId === p.requestId)) turn.userInputs.push({ ...row });
        break;
      }
      case "thread.user-input-response-requested":
      case "thread.user-input-resolved": {
        const p = ev.payload;
        this.pendingUserInputs = this.pendingUserInputs.filter(
          (a) => !(a.threadId === p.threadId && a.requestId === p.requestId),
        );
        this.#recount(p.threadId);
        const d = this.#detail(p.threadId, seq);
        if (d) {
          for (const turn of d.turns) {
            const u = turn.userInputs.find((x) => x.requestId === p.requestId);
            if (!u) continue;
            if (ev.type === "thread.user-input-resolved") {
              u.status = "resolved";
              if (ev.payload.answers !== undefined) u.answers = ev.payload.answers;
              u.resolution = ev.payload.resolution ?? null;
              u.resolvedAt = ev.payload.resolvedAt;
            } else if (u.status === "pending") {
              u.status = "answered";
              u.answers = ev.payload.answers;
            }
          }
        }
        break;
      }
      case "thread.plan-updated": {
        const p = ev.payload;
        const d = this.#detail(p.threadId, seq);
        const turn = d && this.#turnOf(d, p.turnId);
        if (turn) {
          const cur = turn.plans.find((x) => x.planId === p.planId);
          if (cur) {
            cur.explanation = p.explanation ?? cur.explanation;
            cur.steps = p.steps ?? cur.steps;
            cur.markdown = p.markdown ?? cur.markdown;
            cur.updatedAt = p.updatedAt;
          } else {
            turn.plans.push({
              planId: p.planId,
              threadId: p.threadId,
              turnId: p.turnId ?? null,
              explanation: p.explanation ?? null,
              steps: p.steps ?? null,
              markdown: p.markdown ?? null,
              updatedAt: p.updatedAt,
            });
          }
        }
        break;
      }
      case "thread.activity-appended": {
        const p = ev.payload;
        const d = this.#detail(p.threadId, seq);
        if (d) {
          const a = p.activity;
          const list: ActivityRow[] | undefined = a.turnId ? this.#turnOf(d, a.turnId)?.activities : d.looseActivities;
          if (list) {
            const cur = list.find((x) => x.activityId === a.activityId);
            if (cur) {
              cur.kind = a.kind;
              cur.tone = a.tone;
              if (a.summary) cur.summary = a.summary;
              cur.payload = patch(cur.payload ?? {}, a.payload ?? {});
              cur.sequence = seq;
              cur.updatedAt = a.createdAt;
            } else {
              list.push({
                activityId: a.activityId,
                threadId: p.threadId,
                turnId: a.turnId,
                kind: a.kind,
                tone: a.tone,
                summary: a.summary,
                payload: a.payload ?? {},
                sequence: seq,
                createdAt: a.createdAt,
                updatedAt: a.createdAt,
              });
            }
          }
        }
        break;
      }
      case "thread.reverted": {
        const p = ev.payload;
        const d = this.#detail(p.threadId, seq);
        const gone = new Set(
          (d?.turns ?? []).filter((x) => x.turn.ordinal > p.turnCount).map((x) => x.turn.turnId),
        );
        if (d) d.turns = d.turns.filter((x) => x.turn.ordinal <= p.turnCount);
        this.pendingApprovals = this.pendingApprovals.filter(
          (a) => !(a.threadId === p.threadId && a.turnId && gone.has(a.turnId)),
        );
        this.pendingUserInputs = this.pendingUserInputs.filter(
          (a) => !(a.threadId === p.threadId && a.turnId && gone.has(a.turnId)),
        );
        this.#touch(p.threadId, (t) => {
          t.turnCount = p.turnCount;
          t.activeTurnId = null;
          t.latestTurnId = d?.turns.at(-1)?.turn.turnId ?? t.latestTurnId;
        });
        this.#recount(p.threadId);
        break;
      }
      case "thread.worktree-updated": {
        const p = ev.payload;
        this.#touch(p.threadId, (t) => {
          t.worktreeState = p.state;
          if (p.path != null) t.worktreePath = p.path;
          if (p.branch != null) t.branch = p.branch;
          if (p.base != null) t.baseBranch = p.base;
          if (p.setup != null) t.worktreeSetup = p.setup;
          if (p.error != null) t.lastError = p.error;
          t.updatedAt = p.updatedAt;
        });
        break;
      }
      case "thread.checkpoint-captured": {
        const p = ev.payload;
        const d = this.#detail(p.threadId, seq);
        if (!d || p.turnCount === 0) break;
        const turn = this.#turnOf(d, p.turnId) ?? d.turns.find((x) => x.turn.ordinal === p.turnCount);
        if (turn) {
          turn.checkpoint = {
            threadId: p.threadId,
            turnCount: p.turnCount,
            turnId: p.turnId ?? turn.turn.turnId,
            ref: p.ref,
            commit: p.commit,
            status: p.status,
            files: p.files,
            additions: p.additions,
            deletions: p.deletions,
            createdAt: p.capturedAt,
          };
        }
        break;
      }
      case "thread.pr-updated":
        this.#touch(ev.payload.threadId, (t) => {
          t.pr = ev.payload.pr ?? null;
          t.updatedAt = ev.payload.updatedAt;
        });
        break;
      case "thread.terminal-attached":
        this.#touch(ev.payload.threadId, (t) => {
          if (!t.terminals.includes(ev.payload.terminalId)) t.terminals = [...t.terminals, ev.payload.terminalId];
        });
        break;
      case "thread.terminal-closed":
        this.#touch(ev.payload.threadId, (t) => {
          t.terminals = t.terminals.filter((x) => x !== ev.payload.terminalId);
        });
        break;
      case "thread.external-linked": {
        const p = ev.payload;
        this.#touch(p.threadId, (t) => {
          t.external = { tool: p.tool, sessionId: p.sessionId, source: p.source ?? null, resumeCommand: p.resumeCommand ?? null };
        });
        break;
      }
      case "thread.revert-requested":
      case "thread.files-restored":
        // Progress of "edit from here"; the cut itself is `thread.reverted`.
        break;
    }
  }

  #recount(threadId: string) {
    this.#touch(threadId, (t) => {
      t.pendingApprovalCount = this.pendingApprovals.filter((a) => a.threadId === threadId).length;
      t.pendingUserInputCount = this.pendingUserInputs.filter((a) => a.threadId === threadId).length;
    });
  }

  // ── Thread detail (turn pages) ───────────────────────────────────────

  /** Loads the newest 10 turns of a thread (again, if already open). */
  async openThread(threadId: string): Promise<ThreadDetail | undefined> {
    try {
      const page = await invoke<TurnsPage>("threads_turns_page", { threadId, beforeTurn: null, limit: FIRST_PAGE });
      const detail: ThreadDetail = {
        threadId,
        turns: page.turns,
        looseMessages: page.looseMessages,
        looseActivities: page.looseActivities,
        hasMore: page.hasMore,
        beforeTurn: page.beforeTurn,
        session: page.session,
        appliedThrough: page.sequence,
        loading: false,
      };
      this.details = { ...this.details, [threadId]: detail };
      // Live events between our cursor and the page's head are already in it.
      if (page.sequence > this.sequence) void this.catchUp();
      return this.details[threadId];
    } catch (e) {
      this.error = String(e);
      return undefined;
    }
  }

  /** The previous 20 turns. */
  async loadOlder(threadId: string) {
    const d = this.details[threadId];
    if (!d || !d.hasMore || d.loading) return;
    d.loading = true;
    try {
      const page = await invoke<TurnsPage>("threads_turns_page", {
        threadId,
        beforeTurn: d.beforeTurn,
        limit: NEXT_PAGE,
      });
      const known = new Set(d.turns.map((t) => t.turn.turnId));
      d.turns = [...page.turns.filter((t) => !known.has(t.turn.turnId)), ...d.turns];
      d.hasMore = page.hasMore;
      d.beforeTurn = page.beforeTurn;
      if (!page.hasMore) {
        d.looseMessages = page.looseMessages;
        d.looseActivities = page.looseActivities;
      }
    } catch (e) {
      this.error = String(e);
    } finally {
      d.loading = false;
    }
  }

  closeThread(threadId: string) {
    const { [threadId]: _closed, ...rest } = this.details;
    this.details = rest;
  }

  // ── Commands ─────────────────────────────────────────────────────────

  async dispatch(command: ThreadCommand): Promise<DispatchResult> {
    return invoke<DispatchResult>("threads_dispatch", { commandJson: command });
  }

  createProject(title: string, workspaceRoot: string) {
    return this.dispatch({ type: "project.create", title, workspaceRoot });
  }

  async createThread(input: {
    /** Or `projectPath`: the project of that folder, created on the spot. */
    projectId?: string;
    projectPath?: string;
    instanceId: string;
    driver: string;
    title?: string;
    model?: string;
    agentId?: string;
    runtimeMode?: ThreadRow["runtimeMode"];
    /** Own git worktree set up by the host. */
    worktree?: boolean;
    baseBranch?: string;
  }): Promise<string | undefined> {
    const threadId = `thr_${crypto.randomUUID().replaceAll("-", "").slice(0, 16)}`;
    await this.dispatch({ type: "thread.create", threadId, ...input });
    return threadId;
  }

  send(threadId: string, text: string, model?: string) {
    return this.dispatch({ type: "thread.turn.start", threadId, text, model });
  }

  interrupt(threadId: string) {
    return this.dispatch({ type: "thread.turn.interrupt", threadId });
  }

  respondApproval(threadId: string, requestId: string, decision: ApprovalDecision) {
    return this.dispatch({ type: "thread.approval.respond", threadId, requestId, decision });
  }

  respondUserInput(threadId: string, requestId: string, answers: unknown) {
    return this.dispatch({ type: "thread.user-input.respond", threadId, requestId, answers });
  }

  revert(threadId: string, turnCount: number) {
    return this.dispatch({ type: "thread.revert", threadId, turnCount });
  }

  /** "Edit from here" on the host: files (optional) + conversation + driver rollback. */
  revertToTurn(threadId: string, turn: number, restoreFiles: boolean): Promise<RevertOutcome> {
    return invoke<RevertOutcome>("threads_revert_to_turn", { threadId, turn, restoreFiles });
  }

  /** Embedded OmniGet MCP of one thread (on by default). */
  mcpState(threadId: string): Promise<McpThreadState> {
    return invoke<McpThreadState>("mcp_thread_get", { threadId });
  }

  /** Off revokes the thread's session tokens at once; the next session follows. */
  setMcpEnabled(threadId: string, enabled: boolean): Promise<boolean> {
    return invoke<boolean>("mcp_thread_set", { threadId, enabled });
  }

  async fork(threadId: string, turnCount?: number): Promise<string> {
    const newThreadId = `thr_${crypto.randomUUID().replaceAll("-", "").slice(0, 16)}`;
    await this.dispatch({ type: "thread.fork", threadId, newThreadId, turnCount });
    return newThreadId;
  }

  // ── Derived helpers ──────────────────────────────────────────────────

  threadsOf(projectId: string): ThreadRow[] {
    return this.threads
      .filter((t) => t.projectId === projectId && !t.archivedAt)
      .sort((a, b) => {
        if (!!a.pinnedAt !== !!b.pinnedAt) return a.pinnedAt ? -1 : 1;
        return b.updatedAt.localeCompare(a.updatedAt);
      });
  }

  /** Threads waiting on the user, oldest request first. */
  get inbox(): ThreadRow[] {
    return this.threads.filter((t) => t.status === "approval" || t.status === "input");
  }
}

export const threadsStore = new ThreadsStore();
