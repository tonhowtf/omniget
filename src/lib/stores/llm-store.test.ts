import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { fakeTurnEvents, splitChunks } from "$lib/llm/fake-turn";
import type { TurnEvent } from "$lib/llm/types";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

type LlmStore = typeof import("./llm-store.svelte");

let store: LlmStore;

/** Frames are run by hand so the buffer can be inspected mid-turn. */
let frames: (() => void)[] = [];
function runFrame(): void {
  const next = frames.shift();
  next?.();
}

beforeAll(async () => {
  // The stub backend makes `sendMessage` play a fake turn on a timer; with fake
  // timers it never fires unless a test asks, so each test drives the stream.
  vi.useFakeTimers();
  vi.stubGlobal("$state", <T>(value: T) => value);
  vi.stubGlobal("$derived", <T>(value: T) => value);
  store = await import("./llm-store.svelte");
});

afterEach(() => {
  store.resetLlmStore();
  store.setFrameScheduler(null);
  invoke.mockReset();
  frames = [];
});

afterAll(() => {
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

function manualScheduler(run: () => void): () => void {
  frames.push(run);
  return () => {
    frames = frames.filter((f) => f !== run);
  };
}

async function startTurn(): Promise<string> {
  store.setFrameScheduler(manualScheduler);
  invoke.mockImplementation((cmd: string) => {
    if (cmd === "llm_roster_list") return Promise.reject("ERR_STUB");
    return Promise.reject("ERR_STUB");
  });
  await store.loadRoster();
  const agentId = store.getAgents()[0].id;
  store.selectAgent(agentId);
  await store.sendMessage("hi");
  const turn = store.getActiveTurn();
  expect(turn).not.toBeNull();
  return turn!.requestId;
}

describe("roster", () => {
  it("keeps a real empty roster empty instead of inventing a team", async () => {
    invoke.mockResolvedValue([]);
    await store.loadRoster();
    expect(store.getAgents()).toEqual([]);
    expect(store.isDemoRoster()).toBe(false);
  });
  it("does not substitute demo agents for a failed real read", async () => {
    invoke.mockRejectedValue("ERR_IO");
    await store.loadRoster();
    expect(store.getAgents()).toEqual([]);
    expect(store.isDemoRoster()).toBe(false);
  });
  it("preserves the effective model when the backend refuses a switch", async () => {
    invoke.mockRejectedValue("ERR_STUB");
    await store.loadRoster();
    store.selectAgent(store.getAgents()[0].id);
    const previous = store.getActiveConversation()?.model;
    invoke.mockRejectedValue("ERR_LLM_PROVIDER");
    expect(await store.switchModel({ provider: "new", model: "missing" })).toBe(false);
    expect(store.getActiveConversation()?.model).toEqual(previous);
  });
  it("falls back to the demo roster on ERR_STUB and keeps the section usable", async () => {
    invoke.mockRejectedValue("ERR_STUB");
    await store.loadRoster();
    expect(store.getAgents().length).toBeGreaterThan(0);
    expect(store.isDemoRoster()).toBe(true);
    expect(store.isRosterAvailable()).toBe(false);
  });

  it("uses the backend roster when it answers", async () => {
    const agent = {
      id: "a1",
      name: "Solo",
      role: "coordinator" as const,
      system_prompt: "",
      model: { policy: "fixed" as const, model: { provider: "openai", model: "gpt-5" } },
      runtime: { kind: "native" as const },
    };
    invoke.mockResolvedValue([agent]);
    await store.loadRoster();
    expect(store.getAgents()).toEqual([agent]);
    expect(store.isDemoRoster()).toBe(false);
    expect(store.isRosterAvailable()).toBe(true);
  });

  it("treats the harness `null` answer as unavailable, not as a crash", async () => {
    invoke.mockResolvedValue(null);
    await store.loadRoster();
    expect(store.isDemoRoster()).toBe(true);
    expect(store.getAgents().length).toBeGreaterThan(0);
  });
});

describe("turn launch recovery", () => {
  it("cancels the real server request when stop precedes the startup response", async () => {
    let finishStart!: (value: string) => void;
    const started = new Promise<string>(resolve => { finishStart = resolve; });
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "llm_roster_list") return Promise.reject("ERR_STUB");
      if (cmd === "llm_turn_start") return started;
      return Promise.resolve(null);
    });
    await store.loadRoster();
    store.selectAgent(store.getAgents()[0].id);
    const sending = store.sendMessage("stop during startup");
    await store.cancelTurn();
    finishStart("real-request");
    expect(await sending).toBe(false);
    expect(invoke).toHaveBeenCalledWith("llm_turn_cancel", { requestId: "real-request" });
    expect(store.getActiveTurn()).toBeNull();
  });

  it("preserves retry input without simulated output or duplicate optimistic messages on a real launch failure", async () => {
    const agent = { id: "real", name: "Real", role: "worker", system_prompt: "", model: { policy: "fixed", model: { provider: "openai", model: "gpt-5" } }, runtime: { kind: "native" } };
    invoke.mockImplementation((cmd: string) => cmd === "llm_roster_list" ? Promise.resolve([agent]) : Promise.reject("ERR_NETWORK"));
    await store.loadRoster();
    store.selectAgent("real");
    expect(await store.sendMessage("keep my draft")).toBe(false);
    expect(store.getActiveTurn()).toBeNull();
    expect(store.getMessages()).toEqual([]);
    expect(await store.sendMessage("keep my draft")).toBe(false);
    expect(store.getMessages()).toEqual([]);
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe("turn buffer", () => {
  it("applies deltas in wire order in a single frame", async () => {
    const requestId = await startTurn();
    const events: TurnEvent[] = [
      { type: "started", request_id: requestId },
      { type: "text_delta", text: "Hel" },
      { type: "tool_call_start", id: "c1", name: "downloads.list" },
      { type: "tool_call_delta", id: "c1", input_json_delta: '{"a":' },
      { type: "text_delta", text: "lo " },
      { type: "tool_call_delta", id: "c1", input_json_delta: "1}" },
      { type: "tool_call_end", id: "c1" },
      { type: "text_delta", text: "world" },
    ];
    for (const event of events) store.handleTurnEvent(requestId, event);
    expect(store.pendingEventCount()).toBe(events.length);
    expect(store.scheduledFrameCount()).toBe(1);
    runFrame();
    const turn = store.getActiveTurn()!;
    expect(turn.text).toBe("Hello world");
    expect(turn.starting).toBe(false);
    expect(turn.toolCalls).toEqual([
      { id: "c1", name: "downloads.list", input: '{"a":1}', done: true },
    ]);
    expect(store.pendingEventCount()).toBe(0);
  });

  it("ignores events of another request id", async () => {
    const requestId = await startTurn();
    store.handleTurnEvent("someone-else", { type: "text_delta", text: "nope" });
    expect(store.pendingEventCount()).toBe(0);
    store.handleTurnEvent(requestId, { type: "text_delta", text: "yes" });
    runFrame();
    expect(store.getActiveTurn()!.text).toBe("yes");
  });

  it("schedules no frame at rest and drops it when the turn ends", async () => {
    const requestId = await startTurn();
    expect(store.scheduledFrameCount()).toBe(0);
    store.handleTurnEvent(requestId, { type: "text_delta", text: "x" });
    expect(store.scheduledFrameCount()).toBe(1);
    runFrame();
    expect(store.scheduledFrameCount()).toBe(0);
    store.handleTurnEvent(requestId, { type: "finished", reason: "stop" });
    runFrame();
    expect(store.isTurnRunning()).toBe(false);
    expect(store.scheduledFrameCount()).toBe(0);
    expect(store.pendingEventCount()).toBe(0);
  });

  it("commits the assistant message with usage and the model label", async () => {
    const requestId = await startTurn();
    for (const event of fakeTurnEvents(requestId, { answer: "done", chunks: 2 })) {
      store.handleTurnEvent(requestId, event);
    }
    runFrame();
    const messages = store.getMessages();
    expect(messages.map((m) => m.role)).toEqual(["user", "assistant"]);
    const last = messages[1];
    expect(last.text).toBe("done");
    expect(last.usage?.input_tokens).toBe(412);
    expect(last.modelLabel).toBe("anthropic/claude-opus-5");
    expect(last.finish).toBe("stop");
  });
});

describe("cancellation", () => {
  it("keeps the real stream and permission request when backend actions fail", async () => {
    const agent = { id: "real", name: "Real", role: "worker", system_prompt: "", model: { policy: "fixed", model: { provider: "openai", model: "gpt-5" } }, runtime: { kind: "native" } };
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "llm_roster_list") return Promise.resolve([agent]);
      if (cmd === "llm_turn_start") return Promise.resolve("real-request");
      return Promise.reject("ERR_NETWORK");
    });
    await store.loadRoster();
    store.selectAgent("real");
    expect(await store.sendMessage("work")).toBe(true);
    await expect(store.cancelTurn()).rejects.toBe("ERR_NETWORK");
    expect(store.getActiveTurn()?.requestId).toBe("real-request");
    store.handleToolAsk({ agent: "real", request_id: "real-request", tool_call_id: "permission", tool: "files.write" });
    await expect(store.answerTool("permission", true)).rejects.toBe("ERR_NETWORK");
    expect(store.getToolAsk()?.tool_call_id).toBe("permission");
    invoke.mockRejectedValue("ERR_LLM_NO_TURN");
    await expect(store.cancelTurn()).resolves.toBeUndefined();
    expect(store.getActiveTurn()).toBeNull();
  });
  it("clears turn, buffer and frame, and keeps what streamed so far", async () => {
    const requestId = await startTurn();
    store.handleTurnEvent(requestId, { type: "text_delta", text: "partial" });
    runFrame();
    store.handleTurnEvent(requestId, { type: "text_delta", text: " more" });
    expect(store.pendingEventCount()).toBe(1);
    await store.cancelTurn();
    expect(store.isTurnRunning()).toBe(false);
    expect(store.pendingEventCount()).toBe(0);
    expect(store.scheduledFrameCount()).toBe(0);
    const messages = store.getMessages();
    expect(messages[messages.length - 1].finish).toBe("cancelled");
    expect(messages[messages.length - 1].text).toBe("partial more");
  });

  it("is a no-op with no turn running", async () => {
    await expect(store.cancelTurn()).resolves.toBeUndefined();
    expect(store.scheduledFrameCount()).toBe(0);
  });
});

describe("tool ask (GrantMode::Ask)", () => {
  it("keeps the prompt of the live request and clears it on the answer", async () => {
    const requestId = await startTurn();
    store.handleToolAsk({
      agent: "omni",
      request_id: "another-turn",
      tool_call_id: "c1",
      tool: "downloads.enqueue",
    });
    expect(store.getToolAsk()).toBeNull();
    store.handleToolAsk({
      agent: "omni",
      request_id: requestId,
      tool_call_id: "c1",
      tool: "downloads.enqueue",
    });
    expect(store.getToolAsk()?.tool).toBe("downloads.enqueue");
    await store.answerTool("c1", true);
    expect(store.getToolAsk()).toBeNull();
    expect(invoke).toHaveBeenCalledWith("llm_tool_answer", {
      requestId,
      toolCallId: "c1",
      allow: true,
      always: false,
    });
  });

  it("keeps another turn's question (a mission task) instead of dropping it", async () => {
    await startTurn();
    store.handleToolAsk({ agent: "omni", request_id: "mission-job", tool_call_id: "m1", tool: "fs_write" });
    expect(store.getToolAsk()).toBeNull();
    expect(store.getHeldAsks().map((a) => a.tool_call_id)).toContain("m1");
    store.handleToolResolved("m1");
    expect(store.getHeldAsks().map((a) => a.tool_call_id)).not.toContain("m1");
  });

  it("drops a pending prompt when the turn ends", async () => {
    const requestId = await startTurn();
    store.handleToolAsk({ agent: "omni", request_id: requestId, tool_call_id: "c2", tool: "tools.run" });
    store.handleTurnEvent(requestId, { type: "finished", reason: "stop" });
    runFrame();
    expect(store.getToolAsk()).toBeNull();
    expect(store.isTurnRunning()).toBe(false);
  });
});

describe("conversations", () => {
  it("reuses the agent's conversation and keeps the model override", async () => {
    invoke.mockRejectedValue("ERR_STUB");
    await store.loadRoster();
    const [first, second] = store.getAgents();
    const a = store.selectAgent(first.id);
    store.selectAgent(second.id);
    expect(store.selectAgent(first.id)).toBe(a);
    invoke.mockResolvedValue(null); // The override is shown only after the backend accepts it.
    await store.switchModel({ provider: "openai", model: "gpt-5" });
    expect(store.getActiveConversation()!.model).toEqual({ provider: "openai", model: "gpt-5" });
  });
});

describe("fake turn helpers", () => {
  it("splits without breaking surrogate pairs", () => {
    expect(splitChunks("abcdef", 3)).toEqual(["ab", "cd", "ef"]);
    expect(splitChunks("🙂🙂", 2)).toEqual(["🙂", "🙂"]);
    expect(splitChunks("", 4)).toEqual([""]);
  });

  it("scripts started → deltas → usage → finished", () => {
    const events = fakeTurnEvents("r1", { answer: "hey", chunks: 3 });
    expect(events[0]).toEqual({ type: "started", request_id: "r1" });
    expect(events.filter((e) => e.type === "text_delta")).toHaveLength(3);
    expect(events[events.length - 2].type).toBe("usage");
    expect(events[events.length - 1]).toEqual({ type: "finished", reason: "stop" });
  });
});

describe("roster persistence failures", () => {
  async function existingAgent() {
    invoke.mockRejectedValue("ERR_STUB");
    await store.loadRoster();
    return structuredClone(store.getAgents()[0]);
  }

  it("keeps the saved roster while an update is pending and after failure", async () => {
    const original = await existingAgent();
    let reject!: (reason: unknown) => void;
    invoke.mockImplementation(() => new Promise((_, fail) => { reject = fail; }));
    const pending = store.saveAgent({ ...original, name: "Unsaved edit" });
    expect(store.getAgents().find(a => a.id === original.id)?.name).toBe(original.name);
    reject("disk full");
    expect(await pending).toBe(false);
    expect(store.getAgents().find(a => a.id === original.id)?.name).toBe(original.name);
  });

  it("does not publish a failed create and publishes a successful retry once", async () => {
    const original = await existingAgent();
    const draft = { ...original, id: "new-agent", name: "New agent" };
    invoke.mockRejectedValue("disk full");
    expect(await store.saveAgent(draft)).toBe(false);
    expect(store.getAgents().some(a => a.id === draft.id)).toBe(false);
    invoke.mockResolvedValue(undefined);
    expect(await store.saveAgent(draft)).toBe(true);
    expect(store.getAgents().filter(a => a.id === draft.id)).toEqual([draft]);
  });

  it("retains the agent and conversation when deletion fails", async () => {
    const original = await existingAgent();
    store.selectAgent(original.id);
    const conversations = structuredClone(store.getConversations());
    invoke.mockRejectedValue("disk full");
    expect(await store.deleteAgent(original.id)).toBe(false);
    expect(store.getAgents().some(a => a.id === original.id)).toBe(true);
    expect(store.getConversations()).toEqual(conversations);
  });
});

// ── W5: context, rooms, queue, status ───────────────────────────────────

const realAgent = (id: string, name: string) => ({
  id,
  name,
  role: "worker",
  system_prompt: "",
  model: { policy: "fixed", model: { provider: "openai", model: "gpt-5" } },
  runtime: { kind: "native" },
});

const room = {
  id: "grp-0123456789ab",
  title: "Leitura",
  members: [
    { bot: "curator", role: "Curador" },
    { bot: "researcher", role: "Pesquisador" },
    { bot: "companion", role: "Companheiro" },
  ],
  coordinator: "curator",
  default_bot: null,
  mention_policy: "mentions_or_coordinator",
  limits: {
    max_depth: 2,
    max_delegations_per_round: 4,
    max_turns_per_round: 8,
    max_concurrency: 3,
    max_task_ms: 180000,
    max_tokens_per_round: null,
    unknown_run_tokens: 4000,
  },
  round: 0,
  created_ms: 1,
  updated_ms: 1,
} as const;

describe("mentions (front mirror of the room routing)", async () => {
  const types = await import("$lib/llm/types");
  const names = { curator: "Curador", researcher: "Pesquisador de disponibilidade", companion: "Companheiro" };

  it("routes an explicit mention to that member only", () => {
    expect(types.routeTargets(room as never, "@pesquisador-de-disponibilidade tem legenda?", names)).toEqual([
      "researcher",
    ]);
    expect(types.routeTargets(room as never, "oi @companion e @Curador", names)).toEqual(["companion", "curator"]);
  });

  it("falls back to the coordinator, never to everybody", () => {
    expect(types.routeTargets(room as never, "qual a próxima rodada?", names)).toEqual(["curator"]);
    expect(types.routeTargets({ ...room, coordinator: null } as never, "oi", names)).toEqual(["curator"]);
    expect(types.routeTargets({ ...room, mention_policy: "mentions_only" } as never, "oi", names)).toEqual([]);
  });

  it("ignores e-mails and unknown names, and finds the @query at the caret", () => {
    expect(types.mentionedMembers("mail a@curator.com", [{ id: "curator", name: "Curador" }])).toEqual([]);
    expect(types.mentionedMembers("@ninguem", [{ id: "curator", name: "Curador" }])).toEqual([]);
    expect(types.mentionAt("oi @pes", 7)).toEqual({ start: 3, query: "pes" });
    expect(types.mentionAt("a@b", 3)).toBeNull();
    expect(types.handleOf("Pesquisador de disponibilidade")).toBe("pesquisador-de-disponibilidade");
    expect(types.isRoomSession("grp-0123456789ab~curator")).toBe(true);
    expect(types.isRoomSession("conv-1")).toBe(false);
  });
});

describe("queue while a turn runs", () => {
  it("queues a message sent during a turn, visibly, and sends it when the turn ends", async () => {
    const requestId = await startTurn();
    const conversationId = store.getActiveConversation()!.id;
    expect(await store.sendMessage("and then this")).toBe(true);
    expect(store.getQueue(conversationId).map((q) => q.text)).toEqual(["and then this"]);
    expect(store.getMessages().filter((m) => m.role === "user")).toHaveLength(1);
    store.handleTurnEvent(requestId, { type: "finished", reason: "stop" });
    runFrame();
    await store.drainQueue();
    expect(store.getQueue(conversationId)).toEqual([]);
    expect(store.getMessages().filter((m) => m.role === "user").map((m) => m.text)).toEqual(["hi", "and then this"]);
  });

  it("a queued message can be cancelled and hands its text back", async () => {
    await startTurn();
    await store.sendMessage("never mind");
    const item = store.getQueue()[0];
    expect(store.removeQueued(item.id)).toBe("never mind");
    expect(store.getQueue()).toEqual([]);
  });

  it("a queued message that cannot start stays, marked failed, never lost", async () => {
    invoke.mockImplementation((cmd: string) =>
      cmd === "llm_roster_list" ? Promise.resolve([realAgent("real", "Real")]) : Promise.reject("ERR_NETWORK"),
    );
    await store.loadRoster();
    const conversationId = store.selectAgent("real");
    store.enqueueMessage(conversationId, "keep me");
    await store.drainQueue();
    expect(store.getQueue(conversationId)).toEqual([
      expect.objectContaining({ text: "keep me", state: "failed" }),
    ]);
    expect(store.getMessages()).toEqual([]);
    expect(store.getTurnStatus(conversationId)).toBe("failed");
  });
});

describe("turn status (composer)", () => {
  it("working, waiting for you, then completed only when it really finished", async () => {
    const requestId = await startTurn();
    const id = store.getActiveConversation()!.id;
    expect(store.getTurnStatus(id)).toBe("working");
    store.handleToolAsk({ agent: "a", request_id: requestId, tool_call_id: "c", tool: "shell_exec" });
    expect(store.getTurnStatus(id)).toBe("waiting_you");
    store.handleTurnEvent(requestId, { type: "error", error: { code: "ERR_LLM_X", message: "boom", retryable: false } });
    store.handleTurnEvent(requestId, { type: "finished", reason: "other" });
    runFrame();
    expect(store.getTurnStatus(id)).toBe("failed");
    // The open question did not get an answer: shown as cancelled, not approved.
    expect(store.getLastAsk(id)).toEqual({ tool: "shell_exec", state: "cancelled" });
  });

  it("an interrupted run from the durable record is shown as interrupted", () => {
    store.handleRunUpdate({ run_id: "r1", conversation_id: "conv-x", bot_id: "b", state: "interrupted" });
    expect(store.getTurnStatus("conv-x")).toBe("interrupted");
    store.handleRunUpdate({ run_id: "r2", conversation_id: "grp-0123456789ab~curator", bot_id: "curator", state: "failed" });
    expect(store.getTurnStatus("grp-0123456789ab")).toBe("failed");
    expect(store.getTurnStatus("nothing-here")).toBe("idle");
  });

  it("an expired permission answer drops the buttons and says expired", async () => {
    const requestId = await startTurn();
    store.handleToolAsk({ agent: "a", request_id: requestId, tool_call_id: "c9", tool: "fs_write" });
    invoke.mockImplementation((cmd: string) =>
      cmd === "llm_tool_answer" ? Promise.reject("ERR_PERMISSION_EXPIRED: too late") : Promise.resolve(null),
    );
    await store.answerTool("c9", true);
    expect(store.getToolAsk()).toBeNull();
    expect(store.getLastAsk()).toEqual({ tool: "fs_write", state: "expired" });
  });
});

describe("rooms", () => {
  async function withRoom(send: (cmd: string, args: Record<string, unknown>) => unknown) {
    invoke.mockImplementation((cmd: string, args: Record<string, unknown>) => {
      if (cmd === "llm_roster_list")
        return Promise.resolve([realAgent("curator", "Curador"), realAgent("researcher", "Pesquisador"), realAgent("companion", "Companheiro")]);
      if (cmd === "assist_group_list") return Promise.resolve([room]);
      if (cmd === "assist_group_messages")
        return Promise.resolve({ room, messages: [], runs: [], tasks: [] });
      return send(cmd, args);
    });
    await store.loadRoster();
    await store.loadRooms();
    store.selectConversation(room.id);
  }

  it("a room opens as a group conversation and does not steal the agents' direct chats", async () => {
    await withRoom(() => Promise.resolve(null));
    expect(store.getActiveRoom()?.id).toBe(room.id);
    const direct = store.selectAgent("curator");
    expect(direct).not.toBe(room.id);
  });

  it("sends through the room command, shows authorship and streams only started runs", async () => {
    await withRoom((cmd) => {
      if (cmd === "assist_group_send")
        return Promise.resolve({
          message: { id: "m1", room: room.id, seq: 1, author: "user", bot_id: null, reply_to: null, run_id: null, task_id: null, round: 1, text: "@researcher legenda?", status: "done", created_ms: 1 },
          round: 1,
          started: [{ bot: "researcher", run_id: "run-1" }],
          skipped: [],
        });
      return Promise.resolve(null);
    });
    store.selectConversation(room.id);
    expect(await store.sendMessage("@researcher legenda?")).toBe(true);
    expect(invoke).toHaveBeenCalledWith("assist_group_send", { roomId: room.id, text: "@researcher legenda?" });
    expect(invoke).not.toHaveBeenCalledWith("llm_turn_start", expect.anything());
    expect(store.getRoomLive(room.id)).toEqual([{ runId: "run-1", roomId: room.id, botId: "researcher", text: "" }]);
    store.handleRoomTurnEvent("run-1", { type: "text_delta", text: "Sim" });
    store.handleRoomTurnEvent("other-run", { type: "text_delta", text: "x" });
    expect(store.getRoomLive(room.id)[0].text).toBe("Sim");
    expect(store.getTurnStatus(room.id)).toBe("working");
    expect(store.getMessages()[0].author).toEqual({ kind: "user", botId: null });
  });

  it("a refused room message is rolled back so the draft is the only copy", async () => {
    await withRoom((cmd) => (cmd === "assist_group_send" ? Promise.reject("ERR_GROUP: nope") : Promise.resolve(null)));
    store.selectConversation(room.id);
    expect(await store.sendMessage("keep this")).toBe(false);
    expect(store.getMessages()).toEqual([]);
    expect(store.getTurnStatus(room.id)).toBe("failed");
  });

  it("maps the room transcript with typed authors and keeps delegated results", () => {
    const m = store.roomMessageToChat({
      id: "m2", room: room.id, seq: 2, author: "bot", bot_id: "researcher", reply_to: "m1",
      run_id: "run-1", task_id: "t1", round: 1, text: "Na Max", status: "done", created_ms: 2,
    });
    expect(m).toMatchObject({ role: "assistant", author: { kind: "bot", botId: "researcher" }, replyTo: "m1", taskId: "t1" });
  });
});

describe("saved chats", () => {
  it("old single-agent chats reopen as direct chats with the same id; room sessions are hidden", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "llm_roster_list") return Promise.resolve([realAgent("real", "Real")]);
      if (cmd === "llm_conversation_list")
        return Promise.resolve([
          { id: "conv-old", title: "hello", agent_id: "real", messages: 2, updated_ms: 5 },
          { id: "grp-0123456789ab_curator", title: "x", agent_id: "curator", messages: 2, updated_ms: 6 },
        ]);
      if (cmd === "llm_conversation_get")
        return Promise.resolve({
          conversation_id: "conv-old",
          messages: [
            { role: "user", parts: [{ type: "text", text: "hello" }] },
            { role: "assistant", parts: [{ type: "text", text: "hi!" }, { type: "tool_use", id: "t", name: "fs_read", input: { path: "a" } }] },
            { role: "user", parts: [{ type: "tool_result", tool_use_id: "t", content: "x", is_error: false }] },
          ],
        });
      return Promise.resolve(null);
    });
    await store.loadRoster();
    await store.loadConversations();
    const ids = store.getConversations().map((c) => c.id);
    expect(ids).toContain("conv-old");
    expect(ids).not.toContain("grp-0123456789ab_curator");
    expect(store.selectAgent("real")).toBe("conv-old");
    store.selectConversation("conv-old");
    await vi.waitFor(() => expect(store.getMessages()).toHaveLength(2));
    expect(store.getMessages()[1]).toMatchObject({ text: "hi!", toolCalls: [{ name: "fs_read" }] });
  });
});

describe("a turn whose ending event never arrived", () => {
  const agent = { id: "real", name: "Real", role: "worker", system_prompt: "", model: { policy: "fixed", model: { provider: "claude", model: "default" } }, runtime: { kind: "cli", cli: "claude", account: "" } };

  it("is closed by the durable run record when the run failed before the listener attached", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "llm_roster_list") return Promise.resolve([agent]);
      if (cmd === "llm_turn_start") return Promise.resolve("run-1");
      if (cmd === "assist_runs_list") return Promise.resolve([{ run_id: "run-1", conversation_id: "x", bot_id: "real", state: "failed", error: "ERR_CLI_ACCOUNT: no CLI account ``" }]);
      return Promise.reject("ERR_STUB");
    });
    await store.loadRoster();
    const convId = store.selectAgent("real");
    expect(await store.sendMessage("hi")).toBe(true);
    expect(store.getTurnStatus(convId)).toBe("working");
    // attachListener + reconcile resolve, then the grace for llm://turn runs out.
    await vi.advanceTimersByTimeAsync(1600);
    expect(store.getActiveTurn()).toBeNull();
    expect(store.getTurnStatus(convId)).toBe("failed");
    const last = store.getActiveConversation()?.messages.at(-1);
    expect(last?.role).toBe("assistant");
    expect(last?.error?.code).toBe("ERR_CLI_ACCOUNT");
  });

  it("lets llm://turn deliver its own ending inside the grace, without a second message", async () => {
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "llm_roster_list") return Promise.resolve([agent]);
      if (cmd === "llm_turn_start") return Promise.resolve("run-2");
      if (cmd === "assist_runs_list") return Promise.resolve([]);
      return Promise.reject("ERR_STUB");
    });
    await store.loadRoster();
    const convId = store.selectAgent("real");
    await store.sendMessage("hi");
    store.handleRunUpdate({ run_id: "run-2", conversation_id: convId, bot_id: "real", state: "completed" });
    store.handleTurnEvent("run-2", { type: "text_delta", text: "done" });
    store.handleTurnEvent("run-2", { type: "finished", reason: "stop" });
    store.flushPending();
    await vi.advanceTimersByTimeAsync(1600);
    const answers = store.getActiveConversation()!.messages.filter((m) => m.role === "assistant");
    expect(answers).toHaveLength(1);
    expect(answers[0].text).toBe("done");
    expect(store.getTurnStatus(convId)).toBe("completed");
  });
});

describe("mentions by the name shown in the room", () => {
  it("routes '@Companheiro de leitura' like the backend does", async () => {
    const types = await import("$lib/llm/types");
    const members = [
      { id: "omni", name: "Omni" },
      { id: "bot-companheiro-de-leitura", name: "Companheiro de leitura" },
      { id: "ana", name: "Ana" },
      { id: "ana-maria", name: "Ana Maria" },
      { id: "joao", name: "João Crítico" },
    ];
    expect(types.mentionedMembers("@Companheiro de leitura quais filmes?", members)).toEqual(["bot-companheiro-de-leitura"]);
    expect(types.mentionedMembers("@companheiro-de-leitura oi", members)).toEqual(["bot-companheiro-de-leitura"]);
    expect(types.mentionedMembers("@ana maria, veja", members)).toEqual(["ana-maria"]);
    expect(types.mentionedMembers("@Ana, veja", members)).toEqual(["ana"]);
    expect(types.mentionedMembers("@joao critico opine", members)).toEqual(["joao"]);
    expect(types.mentionedMembers("@Omnipresente", members)).toEqual([]);
    expect(types.mentionedMembers("mail a@omni.com", members)).toEqual([]);
  });
});
