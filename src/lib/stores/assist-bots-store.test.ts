import { beforeAll, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

type Store = typeof import("./assist-bots-store.svelte");
let store: Store;

beforeAll(async () => {
  vi.stubGlobal("$state", <T>(value: T) => value);
  vi.stubGlobal("$derived", <T>(value: T) => value);
  store = await import("./assist-bots-store.svelte");
});

const base = {
  name: "Companheiro de leitura", purpose: "", instructions: "", capabilities: ["memory"] as const,
  memory_policy: "remember" as const, skills: [], connection_agent_id: "connection-1",
};

describe("assist-bots-store", () => {
  it("a new bot never starts with code or shell", () => {
    expect(store.defaultCapabilities()).not.toContain("project_code");
  });

  it("validates the create form before any call", () => {
    expect(store.validateNewBot({ ...base, capabilities: [...base.capabilities] })).toBeNull();
    expect(store.validateNewBot({ ...base, capabilities: [], name: "  " })).toBe("assist.bots.create.err_name");
    expect(store.validateNewBot({ ...base, capabilities: [], name: "x".repeat(49) })).toBe("assist.bots.create.err_name_long");
    expect(store.validateNewBot({ ...base, capabilities: [], connection_agent_id: "" })).toBe("assist.bots.create.err_connection");
  });

  it("a ticked box is not an available capability", () => {
    const ticked: import("./assist-bots-store.svelte").CapabilityStatus = { id: "web", requested: true, state: "missing", tools: [], missing_tools: ["web_search"], reason: "tool_not_registered" };
    expect(store.isReallyAvailable(ticked)).toBe(false);
    expect(store.isReallyAvailable({ ...ticked, missing_tools: [], state: "available", tools: ["web_search"] })).toBe(true);
    expect(store.capStateKey({ ...ticked, missing_tools: [], state: "available", via: "builtin" })).toBe("assist.bots.state.builtin");
  });

  it("maps reasons and fixes to keys and actions", () => {
    expect(store.reasonKey("projectless")).toBe("assist.bots.reason.projectless");
    expect(store.reasonKey("something_new")).toBe("assist.bots.reason.other");
    expect(store.reasonKey(undefined)).toBeNull();
    expect(store.fixPlan({ action: "accept_version", target: "x" }).kind).toBe("command");
    expect(store.fixPlan({ action: "add_mcp", target: "files" })).toMatchObject({ kind: "route", route: "/llm/mcp" });
    expect(store.fixPlan({ action: "enable_capability", capability: "web" }).kind).toBe("local");
    expect(store.bindingTone("ready")).toBe("good");
    expect(store.bindingTone("drift")).toBe("warn");
    expect(store.bindingTone("removed")).toBe("bad");
  });

  it("offers only roster agents that carry a connection", () => {
    const agents = [
      { id: "a", name: "A", role: "worker", system_prompt: "", model: { policy: "fixed", model: { provider: "openai", model: "gpt" } }, runtime: { kind: "native" } },
      { id: "b", name: "B", role: "worker", system_prompt: "", model: { policy: "fixed", model: { provider: "openai", model: "" } }, runtime: { kind: "native" } },
      { id: "c", name: "C", role: "worker", system_prompt: "", model: { policy: "fixed", model: { provider: "claude", model: "default" } }, runtime: { kind: "cli", cli: "claude", account: "x" } },
    ] as never;
    expect(store.connectionAgents(agents).map((a) => a.id)).toEqual(["a", "c"]);
  });

  it("sends the capability query with the conversation", async () => {
    invoke.mockResolvedValueOnce({ capabilities: [] });
    await store.capabilities("bot-1", "conv-1");
    expect(invoke).toHaveBeenCalledWith("assist_bot_capabilities", { botId: "bot-1", conversationId: "conv-1" });
  });

  it("labels a version by metadata and hash", () => {
    expect(store.versionLabel({ version: "1.2", hash: "abcdef0123456789" })).toBe("1.2 · abcdef01");
    expect(store.versionLabel({ version: null, hash: "", installed_hash: "0011223344556677" })).toBe("00112233");
  });
});
