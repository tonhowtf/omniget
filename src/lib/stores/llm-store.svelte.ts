/**
 * State of the `/llm` section: roster, conversations and the live turn.
 *
 * Three rules shape this file:
 *
 * 1. **Nothing runs at rest.** The `llm://turn` listener is attached when a
 *    turn starts and dropped when it ends; the delta buffer only schedules a
 *    frame while there are events waiting. With no turn there is no listener,
 *    no timer and no `requestAnimationFrame`.
 * 2. **Deltas are applied in wire order.** Events land in a FIFO buffer and a
 *    single frame drains it; a `text_delta` never overtakes a `tool_call_end`.
 * 3. **`ERR_STUB` is not a failure.** While the Rust side is a stub every
 *    command answers `ERR_STUB` (and the screenshot harness answers `null`):
 *    the store then shows the demo roster from `$lib/llm/fake-turn` and runs
 *    the turn locally, so the UI is exercisable before the backend exists.
 */
import { invoke } from "@tauri-apps/api/core";
import {
  DEMO_ANSWER,
  DEMO_ROSTER,
  fakeTurnEvents,
  runFakeTurn,
  type CancelFake,
} from "$lib/llm/fake-turn";
import type {
  AgentDef,
  ChatMessage,
  ContentPart,
  Conversation,
  LlmError,
  Message,
  ModelRef,
  QueuedMessage,
  Room,
  RoomDraft,
  RoomMessageWire,
  RoomRun,
  RoomSendOutcome,
  RoomTask,
  RunUpdate,
  ToolAsk,
  TurnEvent,
  TurnEventEnvelope,
  TurnState,
  TurnStatus,
} from "$lib/llm/types";
import { effectiveModelLabel, isRoomId, isRoomSession } from "$lib/llm/types";
import { getJobs, reloadJobs } from "$lib/stores/llm-jobs-store.svelte";

// ── Frame scheduler (seam: vitest runs in node, with no rAF) ────────────

export type FrameScheduler = (run: () => void) => () => void;

function defaultScheduler(run: () => void): () => void {
  if (typeof requestAnimationFrame === "function") {
    // A hidden or covered window gets no frames; the timer keeps the turn
    // moving (and finishing) while the user is in another app.
    let done = false;
    const once = () => {
      if (done) return;
      done = true;
      cancelAnimationFrame(frame);
      clearTimeout(timer);
      run();
    };
    const frame = requestAnimationFrame(once);
    const timer = setTimeout(once, 250);
    return () => {
      done = true;
      cancelAnimationFrame(frame);
      clearTimeout(timer);
    };
  }
  const id = setTimeout(run, 16);
  return () => clearTimeout(id);
}

let scheduler: FrameScheduler = defaultScheduler;

/** Test seam. `null` restores the real one. */
export function setFrameScheduler(next: FrameScheduler | null): void {
  scheduler = next ?? defaultScheduler;
}

// ── State ───────────────────────────────────────────────────────────────

let agents = $state<AgentDef[]>([]);
let rosterLoading = $state(false);
let rosterAvailable = $state(true);
let demoRoster = $state(false);
let conversations = $state<Conversation[]>([]);
let activeConversationId = $state<string | null>(null);
let turn = $state<TurnState | null>(null);
let toolAsk = $state<ToolAsk | null>(null);
/**
 * Questions of turns this chat does not drive (mission tasks, background
 * jobs). They are kept, not dropped: the conversation that owns the job and
 * the Missions page show them. Each expires with the broker's deadline.
 */
let heldAsks = $state<(ToolAsk & { receivedMs: number })[]>([]);
const HELD_ASK_TTL_MS = 125_000;
let errorKey = $state<string | null>(null);
/** How the last permission request of a conversation ended when it did not get an answer. */
let lastAsk = $state<{ conversationId: string; tool: string; state: "expired" | "cancelled" } | null>(null);
/** Messages sent while a turn was running: they wait, visible and cancelable. */
let queue = $state<QueuedMessage[]>([]);
/** Last known end of the work of each conversation (never invented). */
let outcomes = $state<Record<string, TurnStatus>>({});
/** Latest `assist://run` update per conversation id. */
let runUpdates = $state<Record<string, RunUpdate>>({});
let rooms = $state<Room[]>([]);
/** Live member turns of rooms, by run (request) id. */
let roomLive = $state<Record<string, { runId: string; roomId: string; botId: string; text: string }>>({});
let roomActivity = $state<Record<string, { runs: RoomRun[]; tasks: RoomTask[] }>>({});

let rosterLoadedOnce = false;
let rosterInFlight: Promise<void> | null = null;
let pending: TurnEvent[] = [];
let frameCancel: (() => void) | null = null;
let cancelFake: CancelFake | null = null;
let unlisten: (() => void) | null = null;
let idCounter = 0;
let startingRequestId: string | null = null;
let roomUnlisten: (() => void) | null = null;
let draining = false;
/** Optimistic room messages not yet confirmed by the backend. */
const pendingRoomMessages = new Set<string>();

function nextId(prefix: string): string {
  idCounter += 1;
  return `${prefix}-${Date.now().toString(36)}-${idCounter}`;
}

// ── Roster ──────────────────────────────────────────────────────────────

export function getAgents(): AgentDef[] {
  return agents;
}

export function getAgent(id: string | null | undefined): AgentDef | null {
  if (!id) return null;
  return agents.find((a) => a.id === id) ?? null;
}

export function isRosterLoading(): boolean {
  return rosterLoading;
}

/** False once a command answered `ERR_STUB`. */
export function isRosterAvailable(): boolean {
  return rosterAvailable;
}

/** True when the rail is showing `DEMO_ROSTER` instead of saved agents. */
export function isDemoRoster(): boolean {
  return demoRoster;
}

export function getErrorKey(): string | null {
  return errorKey;
}

export function isUnavailable(err: unknown): boolean {
  return String(err ?? "").includes("ERR_STUB");
}

/** Maps a backend error to an i18n key. */
export function llmErrorKey(err: unknown): string {
  const code = String(err ?? "");
  if (code.includes("ERR_STUB")) return "llm.err.unavailable";
  if (code.includes("ERR_LLM_BUDGET")) return "llm.err.budget";
  if (code.includes("ERR_LLM")) return "llm.err.turn_failed";
  return "llm.err.turn_failed";
}

/** Loads once per session unless `force`. Never throws. */
export function loadRoster(force = false): Promise<void> {
  if (rosterInFlight) return rosterInFlight;
  if (rosterLoadedOnce && !force) return Promise.resolve();
  rosterLoading = true;
  errorKey = null;
  rosterInFlight = invoke<AgentDef[] | null>("llm_roster_list")
    .then((list) => {
      if (Array.isArray(list)) {
        agents = list;
        rosterAvailable = true;
        demoRoster = false;
        return;
      }
      // `null` is what the screenshot harness answers for an unknown command.
      agents = DEMO_ROSTER;
      demoRoster = true;
      rosterAvailable = Array.isArray(list);
    })
    .catch((err) => {
      if (isUnavailable(err)) { agents = DEMO_ROSTER; demoRoster = true; }
      else if (demoRoster) { agents = []; demoRoster = false; }
      rosterAvailable = false;
      if (!isUnavailable(err)) errorKey = llmErrorKey(err);
    })
    .finally(() => {
      rosterLoading = false;
      rosterLoadedOnce = true;
      rosterInFlight = null;
      if (!activeConversationId && agents.length > 0) selectAgent(agents[0].id);
    });
  return rosterInFlight;
}

/**
 * Puts one canned exchange in front of the user while the backend is a stub,
 * so the chat shows what it will look like instead of an empty column. Called
 * by the page, never by `loadRoster`, and only in demo mode.
 */
export function seedDemoConversation(): void {
  if (!demoRoster || agents.length === 0) return;
  if (conversations.some((c) => c.messages.length > 0)) return;
  const agent = agents[0];
  const id = selectAgent(agent.id);
  const conversation = conversations.find((c) => c.id === id);
  if (!conversation) return;
  conversation.title = "Retry the failed downloads";
  conversation.messages = [
    { id: nextId("msg"), role: "user", text: "Retry the failed downloads, please." },
    {
      id: nextId("msg"),
      role: "assistant",
      text: DEMO_ANSWER,
      toolCalls: [
        { id: "call-demo", name: "downloads.list", input: '{"status":"failed"}', done: true },
      ],
      usage: {
        input_tokens: 412,
        output_tokens: 96,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        first_token_ms: 180,
        total_ms: 1420,
        cost_usd: 0.0021,
      },
      modelLabel: effectiveModelLabel(conversation, agent),
      finish: "stop",
    },
  ];
}

/** Creates or updates one agent. Returns false when the backend refused. */
export async function saveAgent(agent: AgentDef): Promise<boolean> {
  const existing = agents.some((a) => a.id === agent.id);
  try {
    await invoke(existing ? "llm_roster_update" : "llm_roster_create", { agent });
    agents = existing
      ? agents.map((a) => (a.id === agent.id ? agent : a))
      : [...agents, agent];
    rosterAvailable = true;
    return true;
  } catch (err) {
    rosterAvailable = !isUnavailable(err);
    return false;
  }
}

export async function deleteAgent(agentId: string): Promise<boolean> {
  try {
    await invoke("llm_roster_delete", { agentId });
    agents = agents.filter((a) => a.id !== agentId);
    conversations = conversations.filter((c) => c.agentId !== agentId || c.kind === "group");
    if (!conversations.some((c) => c.id === activeConversationId)) {
      activeConversationId = conversations[0]?.id ?? null;
    }
    return true;
  } catch (err) {
    rosterAvailable = !isUnavailable(err);
    return false;
  }
}

export async function applyTemplate(template: string): Promise<boolean> {
  try {
    const list = await invoke<AgentDef[] | null>("llm_roster_apply_template", { template });
    if (Array.isArray(list)) {
      agents = list;
      rosterAvailable = true;
      demoRoster = false;
      return true;
    }
    return false;
  } catch (err) {
    rosterAvailable = !isUnavailable(err);
    return false;
  }
}

// ── Conversations ───────────────────────────────────────────────────────

export function getConversations(): Conversation[] {
  return conversations;
}

export function getActiveConversation(): Conversation | null {
  if (!activeConversationId) return null;
  return conversations.find((c) => c.id === activeConversationId) ?? null;
}

export function getActiveAgentId(): string | null {
  return getActiveConversation()?.agentId ?? null;
}

export function getMessages(): ChatMessage[] {
  return getActiveConversation()?.messages ?? [];
}

/** Last assistant line of an agent, for the rail preview. */
export function lastSpokenBy(agentId: string): string {
  for (let i = conversations.length - 1; i >= 0; i--) {
    const conversation = conversations[i];
    if (conversation.agentId !== agentId || conversation.kind === "group") continue;
    for (let j = conversation.messages.length - 1; j >= 0; j--) {
      const message = conversation.messages[j];
      if (message.text) return message.text;
    }
  }
  return "";
}

export function newConversation(agentId: string): string {
  const conversation: Conversation = {
    id: nextId("conv"),
    agentId,
    title: "",
    messages: [],
    model: null,
    updatedAtMs: Date.now(),
  };
  conversations = [...conversations, conversation];
  activeConversationId = conversation.id;
  return conversation.id;
}

/** Opens the agent's newest conversation, creating one when there is none. */
export function selectAgent(agentId: string): string {
  const existing = [...conversations].reverse().find((c) => c.agentId === agentId && c.kind !== "group");
  if (existing) {
    activeConversationId = existing.id;
    return existing.id;
  }
  return newConversation(agentId);
}

export function selectConversation(id: string): void {
  const conversation = conversations.find((c) => c.id === id);
  if (!conversation) return;
  activeConversationId = id;
  if (conversation.kind === "group") void loadRoom(id);
  else if (conversation.loaded === false) void loadHistory(id);
}

/**
 * Undo took a turn back on disk: drop its bubbles (and everything after them)
 * here too. `userMessagesKept` is how many user messages the backend kept.
 */
export function rewindConversation(id: string, userMessagesKept: number): void {
  const conversation = conversations.find((c) => c.id === id);
  if (!conversation) return;
  let seen = 0;
  const cut = conversation.messages.findIndex((m) => m.role === "user" && ++seen > userMessagesKept);
  if (cut < 0) return;
  conversation.messages = conversation.messages.slice(0, cut);
  conversation.updatedAtMs = Date.now();
}

export function deleteConversation(id: string): void {
  conversations = conversations.filter((c) => c.id !== id);
  queue = queue.filter((q) => q.conversationId !== id);
  if (activeConversationId === id) activeConversationId = conversations[0]?.id ?? null;
}

/** Per-conversation model override (`llm_switch_model`). */
export async function switchModel(model: ModelRef): Promise<boolean> {
  const conversation = getActiveConversation();
  if (!conversation) return false;
  try {
    await invoke("llm_switch_model", { conversationId: conversation.id, modelRef: model });
    conversation.model = model;
    conversation.updatedAtMs = Date.now();
    return true;
  } catch (err) {
    rosterAvailable = !isUnavailable(err);
    return false;
  }
}

// ── Turn ────────────────────────────────────────────────────────────────

export function getActiveTurn(): TurnState | null {
  return turn;
}

/**
 * Pending `GrantMode::Ask` prompt, or `null`. With no turn of its own, the
 * open conversation also shows a question raised by a job running in it (a
 * mission made from this chat).
 */
export function getToolAsk(): ToolAsk | null {
  if (toolAsk) return toolAsk;
  if (turn || !activeConversationId) return null;
  return conversationAsk(activeConversationId);
}

/** Questions kept for turns the chat does not drive (see `heldAsks`). */
export function getHeldAsks(): ToolAsk[] {
  const now = Date.now();
  return heldAsks.filter((a) => now - a.receivedMs < HELD_ASK_TTL_MS);
}

/** The oldest held question raised by a job of this conversation (or its per-bot sub-conversation). */
function conversationAsk(conversationId: string): ToolAsk | null {
  const jobs = getJobs() ?? [];
  for (const ask of getHeldAsks()) {
    const job = jobs.find((j) => j.request_id === ask.request_id);
    if (job && (job.conversation_id === conversationId || job.conversation_id.startsWith(`${conversationId}-`))) return ask;
  }
  return null;
}

/** Entry point for `llm://tool-ask`; exported so tests can feed it. */
export function handleToolAsk(ask: ToolAsk): void {
  if (turn && turn.requestId === ask.request_id) {
    toolAsk = ask;
    return;
  }
  // Another turn's question (a mission task, a background job): keep it.
  const now = Date.now();
  heldAsks = [
    ...heldAsks.filter((a) => a.tool_call_id !== ask.tool_call_id && now - a.receivedMs < HELD_ASK_TTL_MS),
    { ...ask, receivedMs: now },
  ].slice(-20);
  if (!(getJobs() ?? []).some((j) => j.request_id === ask.request_id)) void reloadJobs();
}

/** Someone answered a question (here, the pet, Activity, Missions): drop it everywhere. */
export function handleToolResolved(toolCallId: string): void {
  heldAsks = heldAsks.filter((a) => a.tool_call_id !== toolCallId);
  if (toolAsk?.tool_call_id === toolCallId) toolAsk = null;
}

export function isTurnRunning(): boolean {
  return turn !== null;
}

/** Events waiting for the next frame. 0 at rest. */
export function pendingEventCount(): number {
  return pending.length;
}

/** 1 while a frame is scheduled, 0 at rest. The budget says 0 with no turn. */
export function scheduledFrameCount(): number {
  return frameCancel ? 1 : 0;
}

function enqueue(event: TurnEvent): void {
  pending.push(event);
  if (!frameCancel) frameCancel = scheduler(flushPending);
}

/** Drains the buffer in arrival order. Exported so tests can step the clock. */
export function flushPending(): void {
  frameCancel = null;
  if (pending.length === 0) return;
  const batch = pending;
  pending = [];
  for (const event of batch) applyEvent(event);
}

/** Entry point for both the Tauri event and the fake turn. */
export function handleTurnEvent(requestId: string, event: TurnEvent): void {
  if (!turn || turn.requestId !== requestId) return;
  enqueue(event);
}

function applyEvent(event: TurnEvent): void {
  const state = turn;
  if (!state) return;
  switch (event.type) {
    case "started":
      state.starting = false;
      break;
    case "text_delta":
      state.text += event.text;
      break;
    case "thinking_delta":
      state.thinking += event.text;
      break;
    case "tool_call_start":
      state.toolCalls = [...state.toolCalls, { id: event.id, name: event.name, input: "", done: false }];
      break;
    case "tool_call_delta": {
      const call = state.toolCalls.find((c) => c.id === event.id);
      if (call) call.input += event.input_json_delta;
      break;
    }
    case "tool_call_end": {
      const call = state.toolCalls.find((c) => c.id === event.id);
      if (call) call.done = true;
      break;
    }
    case "usage":
      state.usage = event.usage;
      break;
    case "error":
      state.error = event.error;
      break;
    case "finished":
      commitTurn(event.reason === "cancelled" ? "cancelled" : event.reason, state.error);
      break;
  }
}

function commitTurn(finish: ChatMessage["finish"], error: LlmError | null): void {
  const state = turn;
  if (!state) return;
  outcomes = {
    ...outcomes,
    [state.conversationId]: error ? "failed" : finish === "cancelled" ? "cancelled" : "completed",
  };
  const conversation = conversations.find((c) => c.id === state.conversationId);
  if (conversation && (state.text || state.toolCalls.length > 0 || error)) {
    const agent = getAgent(state.agentId);
    conversation.messages = [
      ...conversation.messages,
      {
        id: nextId("msg"),
        role: "assistant",
        text: state.text,
        toolCalls: state.toolCalls.length > 0 ? state.toolCalls : undefined,
        thinking: state.thinking || undefined,
        usage: state.usage,
        modelLabel: effectiveModelLabel(conversation, agent),
        error,
        finish,
      },
    ];
    conversation.updatedAtMs = Date.now();
  }
  stopTurn();
}

/** Clears every resource a turn owns: buffer, frame, timer and listener. */
function stopTurn(): void {
  const ended = turn;
  if (ended && toolAsk && toolAsk.request_id === ended.requestId) {
    // The turn ended with a question still open: it was not answered, say so.
    lastAsk = { conversationId: ended.conversationId, tool: toolAsk.tool, state: "cancelled" };
  }
  turn = null;
  toolAsk = null;
  pending = [];
  if (frameCancel) frameCancel();
  frameCancel = null;
  cancelFake = null;
  if (unlisten) unlisten();
  unlisten = null;
  if (ended) scheduleDrain();
}

async function attachListener(): Promise<void> {
  if (unlisten) return;
  try {
    const { listen } = await import("@tauri-apps/api/event");
    const stopTurnEvents = await listen<TurnEventEnvelope>("llm://turn", (message) => {
      const payload = message.payload;
      if (payload?.request_id && payload.event) handleTurnEvent(payload.request_id, payload.event);
    });
    // `GrantMode::Ask`: the coordinator re-emits `BusEvent::ToolAsk` here.
    const stopAsks = await listen<ToolAsk>("llm://tool-ask", (message) => {
      if (message.payload?.tool_call_id) handleToolAsk(message.payload);
    });
    const stopRuns = await listen<RunUpdate>("assist://run", (message) => {
      if (message.payload?.run_id) handleRunUpdate(message.payload);
    });
    const stopResolved = await listen<{ tool_call_id?: string }>("omni://tool-resolved", (message) => {
      if (message.payload?.tool_call_id) handleToolResolved(message.payload.tool_call_id);
    });
    const stop = () => {
      stopTurnEvents();
      stopAsks();
      stopRuns();
      stopResolved();
    };
    // The turn may have ended while `listen` was in flight.
    if (turn) unlisten = stop;
    else stop();
  } catch {
    // No Tauri event bridge (browser, tests): the fake turn feeds the store.
  }
}

/**
 * Starts a real turn and reports whether the draft was accepted. Only a demo
 * roster may play a stub response; real startup failures keep input recoverable.
 */
export async function sendMessage(text: string): Promise<boolean> {
  const trimmed = text.trim();
  if (!trimmed) return false;
  let conversation = getActiveConversation();
  if (!conversation) {
    const agentId = agents[0]?.id;
    if (!agentId) return false;
    selectAgent(agentId);
    conversation = getActiveConversation();
    if (!conversation) return false;
  }
  if (conversation.kind === "group") return sendRoomMessage(conversation.id, trimmed);
  // A turn is running: the message waits in a visible queue instead of being
  // refused in silence (or sent concurrently into a session that cannot take it).
  if (turn || startingRequestId) {
    enqueueMessage(conversation.id, trimmed);
    return true;
  }
  return startDirectTurn(conversation, trimmed);
}

async function startDirectTurn(conversation: Conversation, trimmed: string): Promise<boolean> {
  if (turn || startingRequestId) return false;
  const sentMessageId = nextId("msg");
  conversation.messages = [
    ...conversation.messages,
    { id: sentMessageId, role: "user", text: trimmed },
  ];
  if (!conversation.title) conversation.title = trimmed.slice(0, 48);
  conversation.updatedAtMs = Date.now();
  errorKey = null;

  const localRequestId = nextId("req");
  turn = {
    requestId: localRequestId,
    conversationId: conversation.id,
    agentId: conversation.agentId,
    text: "",
    thinking: "",
    toolCalls: [],
    usage: null,
    error: null,
    startedAtMs: Date.now(),
    starting: true,
  };

  const requestedTurn = turn;
  startingRequestId = localRequestId;
  let requestId: string | null = null;
  let canDemo = demoRoster;
  try {
    const answer = await invoke<string | { request_id?: string } | null>("llm_turn_start", {
      conversationId: conversation.id,
      agentId: conversation.agentId,
      input: trimmed,
    });
    if (typeof answer === "string") requestId = answer;
    else if (answer && typeof answer === "object" && answer.request_id) requestId = answer.request_id;
  } catch (err) {
    canDemo = demoRoster && isUnavailable(err);
    if (!isUnavailable(err)) {
      errorKey = llmErrorKey(err);
      rosterAvailable = false;
      outcomes = { ...outcomes, [conversation.id]: "failed" };
    }
  }

  if (!turn || turn.requestId !== localRequestId) {
    // Stop may arrive before startup returns its server id. Cancel that late
    // server request too, without touching a newer conversation's active turn.
    if (requestId) {
      try { await invoke("llm_turn_cancel", { requestId }); }
      catch (err) {
        if (!String(err).includes("ERR_LLM_NO_TURN")) {
          errorKey = llmErrorKey(err);
          turn = { ...requestedTurn, requestId };
          void attachListener();
        }
      }
    }
    startingRequestId = null;
    return false;
  }
  startingRequestId = null;
  if (requestId) {
    turn.requestId = requestId;
    const startedId = requestId;
    void attachListener().then(() => reconcileStartedTurn(conversation.id, startedId));
    return true;
  }
  if (!canDemo) {
    // A failed launch must not fabricate a successful assistant response.
    // Roll back only this optimistic message so explicit retry cannot duplicate it.
    conversation.messages = conversation.messages.filter(message => message.id !== sentMessageId);
    stopTurn();
    return false;
  }
  // Explicit demo roster: local playback through the same event path.
  const events = fakeTurnEvents(localRequestId, {
    answer: DEMO_ANSWER,
    chunks: 40,
    thinking: "Checking the queue…",
    tool: { id: "call-1", name: "downloads.list", input: '{"status":"failed"}' },
  });
  cancelFake = runFakeTurn(events, (event) => handleTurnEvent(localRequestId, event), 24);
  return true;
}

/** Stops the running turn: local timer first, then the backend. */
export async function cancelTurn(): Promise<void> {
  errorKey = null;
  const state = turn;
  if (!state) return;
  if (startingRequestId === state.requestId) {
    stopTurn(); // sendMessage cancels the server id as soon as startup returns.
    return;
  }
  const cancel = cancelFake;
  cancelFake = null;
  if (cancel) {
    cancel(); // emits `finished { cancelled }` through the buffer
    flushPending();
  } else {
    try {
      await invoke("llm_turn_cancel", { requestId: state.requestId });
    } catch (err) {
      if (!(demoRoster && isUnavailable(err)) && !String(err).includes("ERR_LLM_NO_TURN")) { errorKey = llmErrorKey(err); throw err; }
    }
    if (turn === state) {
      applyEvent({ type: "finished", reason: "cancelled" });
      if (turn === state) stopTurn();
    }
  }
}

/** Answers a tool permission prompt (`GrantMode::Ask`). */
export async function answerTool(toolCallId: string, allow: boolean, always = false): Promise<void> {
  errorKey = null;
  const held = heldAsks.find((a) => a.tool_call_id === toolCallId);
  if (!turn && held) {
    // A job's question shown in its conversation (no chat turn of its own).
    try {
      await invoke("llm_tool_answer", { requestId: held.request_id, toolCallId, allow, always });
    } catch (err) {
      const code = String(err ?? "");
      if (!code.includes("ERR_PERMISSION_EXPIRED") && !code.includes("ERR_PERMISSION_RESOLVED")) {
        errorKey = llmErrorKey(err);
        throw err;
      }
    }
    handleToolResolved(toolCallId);
    return;
  }
  const state = turn;
  if (!state) return;
  try {
    await invoke("llm_tool_answer", { requestId: state.requestId, toolCallId, allow, always });
  } catch (err) {
    const code = String(err ?? "");
    // The request is gone: expired, cancelled or already answered. Say which,
    // drop the buttons, and never pretend the answer applied.
    if (code.includes("ERR_PERMISSION_EXPIRED") || code.includes("ERR_PERMISSION_RESOLVED")) {
      if (toolAsk?.tool_call_id === toolCallId) {
        lastAsk = {
          conversationId: state.conversationId,
          tool: toolAsk.tool,
          state: code.includes("ERR_PERMISSION_EXPIRED") ? "expired" : "cancelled",
        };
        toolAsk = null;
      }
      return;
    }
    if (!(demoRoster && isUnavailable(err))) { errorKey = llmErrorKey(err); throw err; }
  }
  if (toolAsk?.tool_call_id === toolCallId) toolAsk = null;
}

/** Test seam: drops every bit of state and every resource. */
export function resetLlmStore(): void {
  stopTurn();
  startingRequestId = null;
  agents = [];
  rosterLoading = false;
  rosterAvailable = true;
  demoRoster = false;
  conversations = [];
  activeConversationId = null;
  toolAsk = null;
  heldAsks = [];
  errorKey = null;
  lastAsk = null;
  queue = [];
  outcomes = {};
  runUpdates = {};
  rooms = [];
  roomLive = {};
  roomActivity = {};
  draining = false;
  if (roomUnlisten) roomUnlisten();
  roomUnlisten = null;
  rosterLoadedOnce = false;
  rosterInFlight = null;
  idCounter = 0;
}

// ── Saved chats (backend JSONL, lazily loaded) ──────────────────────────

interface ConversationInfoWire {
  id: string;
  title: string;
  agent_id: string | null;
  messages: number;
  updated_ms: number;
}

/** Chats the backend kept (one agent each): they reopen as direct chats, same ids. */
export async function loadConversations(): Promise<void> {
  let list: ConversationInfoWire[] | null = null;
  try {
    list = await invoke<ConversationInfoWire[] | null>("llm_conversation_list");
  } catch {
    return;
  }
  if (!Array.isArray(list)) return;
  const known = new Set(conversations.map((c) => c.id));
  const saved: Conversation[] = [];
  for (const info of list) {
    // Member sessions of rooms, bridge/setup scratch chats are not chats of their own.
    if (known.has(info.id) || isRoomSession(info.id) || isRoomId(info.id)) continue;
    if (/^(bridge|setup|help)-/.test(info.id) || !info.agent_id) continue;
    saved.push({
      id: info.id,
      agentId: info.agent_id,
      kind: "direct",
      loaded: false,
      title: info.title === "…" ? "" : info.title,
      messages: [],
      model: null,
      updatedAtMs: info.updated_ms,
    });
  }
  if (saved.length === 0) return;
  // An empty chat opened before the saved ones arrived gives way to them.
  const savedAgents = new Set(saved.map((c) => c.agentId));
  const placeholders = new Set(
    conversations
      .filter((c) => c.kind !== "group" && c.loaded === undefined && c.messages.length === 0 && savedAgents.has(c.agentId))
      .map((c) => c.id),
  );
  const activeAgent = placeholders.has(activeConversationId ?? "")
    ? conversations.find((c) => c.id === activeConversationId)?.agentId
    : null;
  // Oldest first, so `selectAgent` (newest = last) keeps its meaning.
  conversations = [
    ...saved.sort((a, b) => a.updatedAtMs - b.updatedAtMs),
    ...conversations.filter((c) => !placeholders.has(c.id)),
  ];
  if (activeAgent) selectConversation(selectAgent(activeAgent));
}

function partsText(parts: ContentPart[]): string {
  return parts
    .filter((p): p is Extract<ContentPart, { type: "text" }> => p.type === "text")
    .map((p) => p.text)
    .join("");
}

/** Backend `Message`s → bubbles. Tool results fold into the call before them. */
export function historyToChat(messages: Message[], prefix: string): ChatMessage[] {
  const out: ChatMessage[] = [];
  messages.forEach((message, i) => {
    if (message.role === "system" || message.role === "tool") return;
    const calls = message.parts
      .filter((p): p is Extract<ContentPart, { type: "tool_use" }> => p.type === "tool_use")
      .map((p) => ({ id: p.id, name: p.name, input: JSON.stringify(p.input ?? {}), done: true }));
    const text = partsText(message.parts);
    if (message.role === "user" && !text) return; // tool results travel as user turns
    out.push({
      id: `${prefix}-${i}`,
      role: message.role,
      text,
      toolCalls: calls.length > 0 ? calls : undefined,
      author: { kind: message.role === "user" ? "user" : "bot" },
    });
  });
  return out;
}

async function loadHistory(id: string): Promise<void> {
  const conversation = conversations.find((c) => c.id === id);
  if (!conversation || conversation.loaded !== false) return;
  try {
    const answer = await invoke<{ messages?: Message[] } | null>("llm_conversation_get", { conversationId: id });
    const target = conversations.find((c) => c.id === id);
    if (!target) return;
    const history = Array.isArray(answer?.messages) ? historyToChat(answer.messages, id) : [];
    // Anything typed while the history loaded stays after it.
    target.messages = [...history, ...target.messages];
    target.loaded = true;
  } catch {
    // Leave it unloaded: the next open tries again; nothing is invented.
  }
}

// ── Queue (messages sent while a turn runs) ─────────────────────────────

export function getQueue(conversationId?: string | null): QueuedMessage[] {
  return conversationId ? queue.filter((q) => q.conversationId === conversationId) : queue;
}

export function enqueueMessage(conversationId: string, text: string): QueuedMessage {
  const item: QueuedMessage = { id: nextId("queued"), conversationId, text, state: "waiting" };
  queue = [...queue, item];
  return item;
}

/** Cancels a waiting message; returns its text so the caller can put it back in the draft. */
export function removeQueued(id: string): string | null {
  const item = queue.find((q) => q.id === id);
  if (!item) return null;
  queue = queue.filter((q) => q.id !== id);
  return item.text;
}

/** A failed queued message is tried again (it waits again if a turn is running). */
export function retryQueued(id: string): void {
  queue = queue.map((q) => (q.id === id ? { ...q, state: "waiting" } : q));
  scheduleDrain();
}

function scheduleDrain(): void {
  if (draining || !queue.some((q) => q.state === "waiting")) return;
  queueMicrotask(() => void drainQueue());
}

/** Sends the oldest waiting message once no turn is running. Exported for tests. */
export async function drainQueue(): Promise<void> {
  if (draining || turn || startingRequestId) return;
  const next = queue.find((q) => q.state === "waiting");
  if (!next) return;
  const conversation = conversations.find((c) => c.id === next.conversationId);
  draining = true;
  try {
    if (!conversation) {
      queue = queue.filter((q) => q.id !== next.id);
      return;
    }
    queue = queue.filter((q) => q.id !== next.id);
    const ok = await startDirectTurn(conversation, next.text);
    if (!ok) {
      // Keep it, visible and recoverable: never lost, never sent twice.
      queue = [...queue, { ...next, state: "failed" }];
    }
  } finally {
    draining = false;
  }
}

// ── Status of the work (composer) ───────────────────────────────────────

/** Entry point of `assist://run`; exported so tests can feed it. */
export function handleRunUpdate(update: RunUpdate): void {
  const conversationId = update.conversation_id;
  runUpdates = { ...runUpdates, [conversationId]: update };
  const terminal: Partial<Record<RunUpdate["state"], TurnStatus>> = {
    completed: "completed",
    failed: "failed",
    cancelled: "cancelled",
    interrupted: "interrupted",
    unknown: "interrupted",
  };
  const end = terminal[update.state];
  if (end) outcomes = { ...outcomes, [conversationId]: end };
  // A room member's session reports under the room.
  const room = /^(grp-[0-9a-f]{12})[~_]/.exec(conversationId)?.[1];
  if (room && end) outcomes = { ...outcomes, [room]: end };
  if (end) settleFromRun(update);
}

/** How long `llm://turn` gets to deliver its own ending after the durable
 * record says the run ended (its text is coalesced at 30 Hz, so it can lag). */
const RUN_END_GRACE_MS = 1500;

/**
 * The durable run record is written before any event is emitted, so it is the
 * authority when `llm://turn` never delivers an ending: a turn that failed
 * before the listener attached, or a lost event. After a short grace, a turn
 * still open for a run that already ended is closed with that run's outcome.
 */
function settleFromRun(update: RunUpdate): void {
  const requestId = update.run_id;
  if (!turn || turn.requestId !== requestId) return;
  setTimeout(() => {
    if (!turn || turn.requestId !== requestId) return;
    if (pending.length > 0) flushPending();
    if (!turn || turn.requestId !== requestId) return;
    if (update.state === "completed") {
      commitTurn("stop", turn.error);
      return;
    }
    const raw = update.error ?? update.state;
    const code = /^(ERR_[A-Z0-9_]+)/.exec(raw)?.[1] ?? (update.state === "cancelled" ? "ERR_LLM_CANCELLED" : "ERR_LLM_RUN_ENDED");
    const error = turn.error ?? { code, message: raw, retryable: update.state !== "cancelled" };
    commitTurn(update.state === "cancelled" ? "cancelled" : "other", update.state === "cancelled" ? turn.error : error);
  }, RUN_END_GRACE_MS);
}

/** After the listener attached: a run that ended before it did is settled now. */
async function reconcileStartedTurn(conversationId: string, requestId: string): Promise<void> {
  try {
    const list = await invoke<RunUpdate[] | { runs?: RunUpdate[] } | null>("assist_runs_list", { conversationId });
    const runs = Array.isArray(list) ? list : Array.isArray(list?.runs) ? list.runs : [];
    const run = runs.find((r) => r.run_id === requestId);
    if (run?.state) handleRunUpdate({ ...run, conversation_id: run.conversation_id ?? conversationId });
  } catch {
    // Older backend without the run record: `llm://turn` stays the only source.
  }
}

/**
 * What the composer shows: working, waiting for you, interrupted, failed,
 * cancelled, completed or nothing. Only from real events: never a success
 * that did not happen.
 */
export function getTurnStatus(conversationId?: string | null): TurnStatus {
  const id = conversationId ?? activeConversationId;
  if (!id) return "idle";
  if (turn && turn.conversationId === id) {
    if (toolAsk && toolAsk.request_id === turn.requestId) return "waiting_you";
    const live = runUpdates[id];
    if (live && live.run_id === turn.requestId && live.state === "waiting_user") return "waiting_you";
    return "working";
  }
  if (Object.values(roomLive).some((r) => r.roomId === id)) return "working";
  return outcomes[id] ?? "idle";
}

/** How the last unanswered permission request of this conversation ended. */
export function getLastAsk(conversationId?: string | null): { tool: string; state: "expired" | "cancelled" } | null {
  const id = conversationId ?? activeConversationId;
  return lastAsk && lastAsk.conversationId === id ? { tool: lastAsk.tool, state: lastAsk.state } : null;
}

/** Latest persisted run state of a conversation (after a restart: interrupted). */
export async function refreshRunState(conversationId: string): Promise<void> {
  try {
    const list = await invoke<RunUpdate[] | { runs?: RunUpdate[] } | null>("assist_runs_list", { conversationId });
    const runs = Array.isArray(list) ? list : Array.isArray(list?.runs) ? list.runs : [];
    const latest = runs[0];
    if (latest?.state && latest.run_id && !(turn && turn.conversationId === conversationId)) {
      handleRunUpdate({ ...latest, conversation_id: latest.conversation_id ?? conversationId });
    }
  } catch {
    // Older backend without the run record: nothing to show, nothing invented.
  }
}

// ── Rooms ───────────────────────────────────────────────────────────────

export function getRooms(): Room[] {
  return rooms;
}

export function getRoom(id: string | null | undefined): Room | null {
  if (!id) return null;
  return rooms.find((r) => r.id === id) ?? null;
}

export function getActiveRoom(): Room | null {
  const conversation = getActiveConversation();
  return conversation?.kind === "group" ? getRoom(conversation.id) : null;
}

export function getRoomActivity(id: string | null | undefined): { runs: RoomRun[]; tasks: RoomTask[] } {
  return (id && roomActivity[id]) || { runs: [], tasks: [] };
}

/** Live (streaming) member turns of a room. */
export function getRoomLive(id: string | null | undefined): { runId: string; botId: string; text: string }[] {
  if (!id) return [];
  return Object.values(roomLive).filter((r) => r.roomId === id);
}

function roomAnswerer(room: Room): string {
  return room.coordinator ?? room.default_bot ?? room.members[0]?.bot ?? "";
}

function upsertRoomConversation(room: Room): void {
  const existing = conversations.find((c) => c.id === room.id);
  if (existing) {
    existing.title = room.title;
    existing.agentId = roomAnswerer(room);
    existing.updatedAtMs = room.updated_ms;
    return;
  }
  conversations = [
    ...conversations,
    {
      id: room.id,
      agentId: roomAnswerer(room),
      kind: "group",
      loaded: false,
      title: room.title,
      messages: [],
      model: null,
      updatedAtMs: room.updated_ms,
    },
  ];
}

export async function loadRooms(): Promise<void> {
  try {
    const list = await invoke<Room[] | null>("assist_group_list");
    if (!Array.isArray(list)) return;
    rooms = list;
    for (const room of list) upsertRoomConversation(room);
  } catch {
    // No rooms backend: the rail simply shows none.
  }
}

export function roomMessageToChat(m: RoomMessageWire): ChatMessage {
  return {
    id: m.id,
    role: m.author === "user" ? "user" : "assistant",
    text: m.text,
    author: { kind: m.author, botId: m.bot_id },
    replyTo: m.reply_to,
    runId: m.run_id,
    taskId: m.task_id,
    status: m.status,
    error:
      m.status === "failed"
        ? { code: /^(ERR_[A-Z0-9_]+)/.exec(m.text)?.[1] ?? "ERR_GROUP_RUN", message: m.text, retryable: true }
        : null,
    finish: m.status === "cancelled" ? "cancelled" : null,
  };
}

export async function loadRoom(id: string): Promise<void> {
  try {
    const answer = await invoke<{
      room: Room;
      messages: RoomMessageWire[];
      runs: RoomRun[];
      tasks: RoomTask[];
    } | null>("assist_group_messages", { roomId: id });
    if (!answer?.room) return;
    rooms = rooms.some((r) => r.id === id)
      ? rooms.map((r) => (r.id === id ? answer.room : r))
      : [...rooms, answer.room];
    upsertRoomConversation(answer.room);
    const conversation = conversations.find((c) => c.id === id);
    if (conversation) {
      // A reload that started before a send must not wipe the message being sent.
      const pending = conversation.messages.filter((m) => pendingRoomMessages.has(m.id));
      conversation.messages = [...(answer.messages ?? []).map(roomMessageToChat), ...pending];
      conversation.loaded = true;
    }
    roomActivity = { ...roomActivity, [id]: { runs: answer.runs ?? [], tasks: answer.tasks ?? [] } };
    // Runs the backend says are over stop streaming here.
    const running = new Set((answer.runs ?? []).filter((r) => r.state === "running").map((r) => r.run_id));
    const next = { ...roomLive };
    for (const [runId, live] of Object.entries(next)) {
      if (live.roomId === id && !running.has(runId)) delete next[runId];
    }
    roomLive = next;
    const last = [...(answer.runs ?? [])].reverse().find((r) => r.state !== "running");
    if (last && !Object.values(roomLive).some((r) => r.roomId === id)) {
      const map: Record<string, TurnStatus> = {
        completed: "completed",
        failed: "failed",
        cancelled: "cancelled",
        interrupted: "interrupted",
      };
      if (map[last.state]) outcomes = { ...outcomes, [id]: map[last.state] };
    }
    if (Object.keys(roomLive).length === 0) detachRoomListener();
  } catch {
    // Keep what is on screen.
  }
}

export async function createRoom(draft: RoomDraft): Promise<Room | null> {
  try {
    const room = await invoke<Room>("assist_group_create", { draft });
    if (!room?.id) return null;
    rooms = [...rooms, room];
    upsertRoomConversation(room);
    activeConversationId = room.id;
    const conversation = conversations.find((c) => c.id === room.id);
    if (conversation) conversation.loaded = true;
    return room;
  } catch (err) {
    errorKey = String(err ?? "").includes("ERR_GROUP") ? "assist.groups.err_save" : llmErrorKey(err);
    return null;
  }
}

export async function updateRoom(id: string, draft: RoomDraft): Promise<Room | null> {
  try {
    const room = await invoke<Room>("assist_group_update", { roomId: id, draft });
    if (!room?.id) return null;
    rooms = rooms.map((r) => (r.id === id ? room : r));
    upsertRoomConversation(room);
    return room;
  } catch {
    return null;
  }
}

export async function deleteRoom(id: string): Promise<boolean> {
  try {
    await invoke("assist_group_delete", { roomId: id });
    rooms = rooms.filter((r) => r.id !== id);
    deleteConversation(id);
    return true;
  } catch {
    return false;
  }
}

/**
 * Sends a user message into a room. The backend routes it (mention → that
 * member only; none → coordinator or default bot); here the message shows
 * at once and is rolled back if the backend refused it, so the draft stays.
 */
export async function sendRoomMessage(roomId: string, text: string): Promise<boolean> {
  const conversation = conversations.find((c) => c.id === roomId);
  if (!conversation) return false;
  const optimisticId = nextId("msg");
  conversation.messages = [
    ...conversation.messages,
    { id: optimisticId, role: "user", text, author: { kind: "user" }, status: "done" },
  ];
  pendingRoomMessages.add(optimisticId);
  errorKey = null;
  let out: RoomSendOutcome | null = null;
  try {
    out = await invoke<RoomSendOutcome>("assist_group_send", { roomId, text });
  } catch (err) {
    errorKey = llmErrorKey(err);
  }
  pendingRoomMessages.delete(optimisticId);
  const target = conversations.find((c) => c.id === roomId);
  if (!out?.message) {
    if (target) target.messages = target.messages.filter((m) => m.id !== optimisticId);
    outcomes = { ...outcomes, [roomId]: "failed" };
    return false;
  }
  if (target) {
    target.messages = target.messages.map((m) => (m.id === optimisticId ? roomMessageToChat(out!.message) : m));
    target.updatedAtMs = Date.now();
    if (!target.title) target.title = text.slice(0, 48);
    // Who could not start says why, in the room, instead of silence.
    for (const skip of out.skipped ?? []) {
      target.messages = [
        ...target.messages,
        {
          id: nextId("note"),
          role: "assistant",
          text: skip.reason,
          author: { kind: "system", botId: skip.bot },
          status: "skipped",
        },
      ];
    }
  }
  const live = { ...roomLive };
  for (const started of out.started ?? []) {
    live[started.run_id] = { runId: started.run_id, roomId, botId: started.bot, text: "" };
  }
  roomLive = live;
  if ((out.started ?? []).length > 0) void attachRoomListener();
  return true;
}

/** Stops everything still running in the room; finished answers stay. */
export async function cancelRoom(roomId: string): Promise<boolean> {
  try {
    await invoke("assist_group_cancel", { roomId });
    const next = { ...roomLive };
    for (const [runId, live] of Object.entries(next)) if (live.roomId === roomId) delete next[runId];
    roomLive = next;
    outcomes = { ...outcomes, [roomId]: "cancelled" };
    await loadRoom(roomId);
    return true;
  } catch (err) {
    errorKey = llmErrorKey(err);
    return false;
  }
}

/** A streaming event of a room member's turn; exported for tests. */
export function handleRoomTurnEvent(requestId: string, event: TurnEvent): void {
  const live = roomLive[requestId];
  if (!live) return;
  if (event.type === "text_delta") {
    roomLive = { ...roomLive, [requestId]: { ...live, text: live.text + event.text } };
  } else if (event.type === "finished") {
    // The backend writes the member's message; reload to show it with its author.
    void loadRoom(live.roomId);
  }
}

/** `assist://group` payload; exported for tests. */
export function handleRoomEvent(payload: { room_id?: string; kind?: string }): void {
  const id = payload?.room_id;
  if (!id) return;
  if (payload.kind === "deleted") {
    rooms = rooms.filter((r) => r.id !== id);
    return;
  }
  if (activeConversationId === id || Object.values(roomLive).some((r) => r.roomId === id)) void loadRoom(id);
}

async function attachRoomListener(): Promise<void> {
  if (roomUnlisten) return;
  try {
    const { listen } = await import("@tauri-apps/api/event");
    const a = await listen<TurnEventEnvelope>("llm://turn", (message) => {
      const payload = message.payload;
      if (payload?.request_id && payload.event) handleRoomTurnEvent(payload.request_id, payload.event);
    });
    const b = await listen<{ room_id?: string; kind?: string }>("assist://group", (message) => {
      handleRoomEvent(message.payload ?? {});
    });
    const c = await listen<RunUpdate>("assist://run", (message) => {
      if (message.payload?.run_id) handleRunUpdate(message.payload);
    });
    const stop = () => { a(); b(); c(); };
    if (Object.keys(roomLive).length > 0) roomUnlisten = stop;
    else stop();
  } catch {
    // No event bridge (tests, browser).
  }
}

function detachRoomListener(): void {
  if (roomUnlisten) roomUnlisten();
  roomUnlisten = null;
}

/** Scopes the user shared into a room (memory reaches a room only this way). */
export interface RoomShare {
  id: string;
  room: string;
  scope: string;
  record_id: string | null;
  note: string;
  created_ms: number;
}

export async function loadShares(roomId: string): Promise<RoomShare[]> {
  try {
    const list = await invoke<RoomShare[] | null>("assist_group_shares", { roomId });
    return Array.isArray(list) ? list : [];
  } catch {
    return [];
  }
}

export async function shareIntoRoom(roomId: string, scope: string, note = ""): Promise<boolean> {
  try {
    await invoke("assist_group_share", { roomId, scope, recordId: null, note });
    return true;
  } catch {
    return false;
  }
}

export async function unshareFromRoom(roomId: string, shareId: string): Promise<boolean> {
  try {
    await invoke("assist_group_unshare", { roomId, shareId });
    return true;
  } catch {
    return false;
  }
}
