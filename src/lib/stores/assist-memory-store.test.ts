import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

type Store = typeof import("./assist-memory-store.svelte");
let store: Store;

beforeAll(async () => {
  vi.stubGlobal("$state", <T>(value: T) => value);
  vi.stubGlobal("$derived", <T>(value: T) => value);
  store = await import("./assist-memory-store.svelte");
});

afterEach(() => {
  store.resetMemoryStore();
  invoke.mockReset();
});

afterAll(() => {
  vi.unstubAllGlobals();
});

function item(over: Partial<import("./assist-memory-store.svelte").MemoryItem>) {
  return {
    id: "m1",
    scope: { kind: "user" },
    scope_key: "user",
    category: "declared",
    content: "No spoilers",
    source: { kind: "user", author: "user", at_ms: 1 },
    source_line: "(source: stated on the memory screen, 2026-09-24)",
    confidence: 1,
    version: 1,
    chain: "m1",
    status: "active",
    created_ms: 1,
    updated_ms: 1,
    ...over,
  } as import("./assist-memory-store.svelte").MemoryItem;
}

describe("helpers", () => {
  it("builds scope keys like the backend", () => {
    expect(store.scopeKey({ kind: "user" })).toBe("user");
    expect(store.scopeKey({ kind: "bot", bot: "reader" })).toBe("bot:reader");
    expect(store.scopeKey({ kind: "room", conversation: "r1" })).toBe("room:r1");
    expect(store.botScopes("reader")).toEqual(["user", "bot:reader"]);
  });

  it("groups active items by kind in display order and drops inactive ones", () => {
    const groups = store.groupByKind([
      item({ id: "a", category: "observation" }),
      item({ id: "b", category: "declared" }),
      item({ id: "c", category: "inference" }),
      item({ id: "d", category: "declared", status: "superseded" }),
    ]);
    expect(groups.map((g) => g.kind)).toEqual(["declared", "inference", "observation"]);
    expect(groups[0].items.map((i) => i.id)).toEqual(["b"]);
  });

  it("maps backend error codes to i18n keys", () => {
    expect(store.memoryErrorKey("ERR_MEMORY_FORGOTTEN: the person asked")).toBe("assist.memory.err_forgotten");
    expect(store.memoryErrorKey(new Error("ERR_MEMORY_NOT_FOUND: no such memory"))).toBe("assist.memory.err_not_found");
    expect(store.memoryErrorKey("boom")).toBe("assist.memory.err_generic");
  });

  it("reads the bot author", () => {
    expect(store.authorBot(item({ source: { kind: "message", author: "bot:reader", at_ms: 1 } }))).toBe("reader");
    expect(store.authorBot(item({}))).toBeNull();
  });
});

describe("commands", () => {
  it("loads scopes with the backend argument names", async () => {
    invoke.mockResolvedValueOnce([item({})]).mockResolvedValueOnce([]);
    await store.loadScopes(["user", "bot:reader"]);
    expect(invoke).toHaveBeenCalledWith("assist_memory_list", { scope: "user", includeInactive: false });
    expect(invoke).toHaveBeenCalledWith("assist_memory_list", { scope: "bot:reader", includeInactive: false });
    expect(store.getScopeItems("user")).toHaveLength(1);
    expect(store.getScopeItems("bot:reader")).toEqual([]);
  });

  it("a failed write returns the error key and changes nothing", async () => {
    invoke.mockResolvedValueOnce([item({})]);
    await store.loadScopes(["user"]);
    invoke.mockRejectedValueOnce("ERR_MEMORY_INVALID: the memory is empty");
    const out = await store.createMemory("user", "declared", "   ");
    expect(out).toEqual({ ok: false, errorKey: "assist.memory.err_invalid" });
    expect(store.getScopeItems("user")).toHaveLength(1);
  });

  it("forgetting refreshes the scope", async () => {
    invoke.mockResolvedValueOnce(2).mockResolvedValueOnce([]);
    const out = await store.forgetMemory(item({}));
    expect(out).toEqual({ ok: true, value: 2 });
    expect(invoke).toHaveBeenNthCalledWith(1, "assist_memory_forget", { id: "m1" });
    expect(invoke).toHaveBeenNthCalledWith(2, "assist_memory_list", { scope: "user", includeInactive: false });
  });
});
