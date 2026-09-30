import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";

// Fake provider: every IPC is answered by hand, nothing reaches a real model.
const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

type LlmStore = typeof import("./llm-store.svelte");
let store: LlmStore;

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

const agent = (id: string) => ({
  id, name: id, role: "worker", system_prompt: "",
  model: { policy: "fixed", model: { provider: "openai", model: "fake" } },
  runtime: { kind: "native" },
});

function calls(command: string) {
  return invoke.mock.calls.filter(([cmd]) => cmd === command);
}

beforeAll(async () => {
  vi.stubGlobal("$state", <T>(value: T) => value);
  vi.stubGlobal("$derived", <T>(value: T) => value);
  store = await import("./llm-store.svelte");
});

afterEach(() => {
  store.resetLlmStore();
  invoke.mockReset();
});

afterAll(() => vi.unstubAllGlobals());

async function twoConversations(turnStart: () => Promise<unknown>) {
  invoke.mockImplementation((cmd: string) => {
    if (cmd === "llm_roster_list") return Promise.resolve([agent("a"), agent("b")]);
    if (cmd === "llm_turn_start") return turnStart();
    if (cmd === "llm_turn_cancel") return Promise.resolve(null);
    return Promise.reject(new Error(`unexpected IPC ${cmd}`));
  });
  await store.loadRoster();
  const first = store.newConversation("a");
  const second = store.newConversation("b");
  store.selectConversation(first);
  return { first, second };
}

describe("drafts keep their conversation identity across awaits", () => {
  it("an accepted send clears the origin draft even after navigating away", async () => {
    const start = deferred<string>();
    const { first, second } = await twoConversations(() => start.promise);
    store.setConversationDraft(first, "question for A");
    const sending = store.sendConversationDraft(first, "question for A");
    store.selectConversation(second);
    store.setConversationDraft(second, "unrelated B draft");
    start.resolve("req-1");
    expect(await sending).toBe(true);
    expect(store.getConversationDraft(first)).toBe("");
    expect(store.getConversationDraft(second)).toBe("unrelated B draft");
    expect(calls("llm_turn_start")).toHaveLength(1);
    expect(calls("llm_turn_start")[0][1]).toMatchObject({ conversationId: first, agentId: "a", input: "question for A" });
    // The user bubble lands in A, never in the chat on screen.
    expect(store.getConversations().find((c) => c.id === first)?.messages.map((m) => m.text)).toEqual(["question for A"]);
    expect(store.getConversations().find((c) => c.id === second)?.messages).toEqual([]);
  });

  it("a send refused while away keeps the origin draft intact", async () => {
    const { first, second } = await twoConversations(() => Promise.reject("ERR_LLM_NET"));
    store.setConversationDraft(first, "keep me");
    const sending = store.sendConversationDraft(first, "keep me");
    store.selectConversation(second);
    expect(await sending).toBe(false);
    expect(store.getConversationDraft(first)).toBe("keep me");
    expect(calls("llm_turn_start")).toHaveLength(1);
  });

  it("does not clear an origin draft that was edited again during the send", async () => {
    const start = deferred<string>();
    const { first } = await twoConversations(() => start.promise);
    store.setConversationDraft(first, "sent text");
    const sending = store.sendConversationDraft(first, "sent text");
    store.setConversationDraft(first, "newer thought");
    start.resolve("req-2");
    expect(await sending).toBe(true);
    expect(store.getConversationDraft(first)).toBe("newer thought");
  });

  it("stop restores the interrupted text into the origin only, and only when empty", async () => {
    const start = deferred<string>();
    const { first, second } = await twoConversations(() => start.promise);
    const sending = store.sendConversationDraft(first, "long task");
    start.resolve("req-3");
    await sending;
    expect(store.getConversationDraft(first)).toBe("");
    const stopping = store.stopConversationTurn(first);
    store.selectConversation(second);
    await stopping;
    expect(calls("llm_turn_cancel")).toEqual([["llm_turn_cancel", { requestId: "req-3" }]]);
    expect(store.getConversationDraft(first)).toBe("long task");
    expect(store.getConversationDraft(second)).toBe("");
  });

  it("stop never replaces a newer draft in the origin", async () => {
    const cancel = deferred<null>();
    const start = deferred<string>();
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "llm_roster_list") return Promise.resolve([agent("a")]);
      if (cmd === "llm_turn_start") return start.promise;
      if (cmd === "llm_turn_cancel") return cancel.promise;
      return Promise.reject(new Error(`unexpected IPC ${cmd}`));
    });
    await store.loadRoster();
    const id = store.newConversation("a");
    const sending = store.sendConversationDraft(id, "first try");
    start.resolve("req-4");
    await sending;
    const stopping = store.stopConversationTurn(id);
    store.setConversationDraft(id, "rewritten while stopping");
    cancel.resolve(null);
    await stopping;
    expect(store.getConversationDraft(id)).toBe("rewritten while stopping");
  });

  it("stop for another conversation does not cancel the running turn", async () => {
    const start = deferred<string>();
    const { first, second } = await twoConversations(() => start.promise);
    const sending = store.sendConversationDraft(first, "work in A");
    start.resolve("req-5");
    await sending;
    await store.stopConversationTurn(second);
    expect(calls("llm_turn_cancel")).toHaveLength(0);
    expect(store.getActiveTurn()?.conversationId).toBe(first);
  });

  it("a late saved list keeps a placeholder that holds a draft, but drops an empty one", async () => {
    const list = deferred<unknown[]>();
    invoke.mockImplementation((cmd: string) => {
      if (cmd === "llm_conversation_list") return list.promise;
      if (cmd === "llm_conversation_get") return Promise.resolve({ messages: [] });
      return Promise.reject(new Error(`unexpected IPC ${cmd}`));
    });
    const loading = store.loadConversations();
    const empty = store.newConversation("x");
    const drafted = store.newConversation("y");
    store.setConversationDraft(drafted, "unsent");
    list.resolve([
      { id: "saved-x", agent_id: "x", title: "X", messages: 1, updated_ms: 1 },
      { id: "saved-y", agent_id: "y", title: "Y", messages: 1, updated_ms: 2 },
    ]);
    await loading;
    const ids = store.getConversations().map((c) => c.id);
    expect(ids).toContain(drafted);
    expect(ids).not.toContain(empty);
    expect(store.getActiveConversation()?.id).toBe(drafted);
    expect(store.getConversationDraft(drafted)).toBe("unsent");
    expect(calls("llm_turn_start")).toHaveLength(0);
  });
});
