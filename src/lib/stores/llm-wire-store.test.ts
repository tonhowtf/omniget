import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));

type WireStore = typeof import("./llm-wire-store.svelte");

let store: WireStore;

const ROUND = {
  provider: "openai",
  model: "gpt-x",
  started_at_ms: 1_700_000_000_000,
  cost_usd: 0.004,
  results: [
    {
      provider: "openai",
      model: "gpt-x",
      param: "temperature",
      sent_a: { model: "gpt-x", temperature: 0 },
      sent_b: { model: "gpt-x", temperature: 1.5 },
      reply_a: "cold",
      reply_b: "hot",
      differs: true,
      cost_usd: 0.002,
      verdict: "proven" as const,
      detail: "temperature",
    },
    {
      provider: "openai",
      model: "gpt-x",
      param: "reasoning_effort",
      sent_a: { model: "gpt-x" },
      sent_b: { model: "gpt-x" },
      reply_a: "3",
      reply_b: "3",
      differs: false,
      cost_usd: 0.002,
      verdict: "ignored" as const,
      detail: "`reasoning_effort` is not in the body sent",
    },
  ],
};

beforeAll(async () => {
  vi.stubGlobal("$state", <T>(value: T) => value);
  vi.stubGlobal("$derived", <T>(value: T) => value);
  store = await import("./llm-wire-store.svelte");
});

afterEach(() => {
  invoke.mockReset();
  store.resetWireStore();
});

afterAll(() => {
  vi.unstubAllGlobals();
});

describe("pure helpers", () => {
  it("maps verdicts to keys and tones", () => {
    expect(store.verdictKey("proven")).toBe("llm.wire.verdict.proven");
    expect(store.verdictTone("proven")).toBe("ok");
    expect(store.verdictTone("sent")).toBe("warn");
    expect(store.verdictTone("ignored")).toBe("bad");
    expect(store.verdictTone("unsupported")).toBe("muted");
  });

  it("maps Rust error codes to i18n keys", () => {
    expect(store.wireErrorKey("ERR_LLM_MODEL: no provider client")).toBe(
      "llm.wire.error.no_provider",
    );
    expect(store.wireErrorKey("ERR_LLM_BUDGET: over 8 requests")).toBe("llm.wire.error.budget");
    expect(store.wireErrorKey("ERR_LLM_STUB: not implemented yet")).toBe("llm.wire.error.stub");
    expect(store.wireErrorKey(new Error("boom"))).toBe("llm.wire.error.generic");
    expect(store.wireErrorKey(undefined)).toBe("llm.wire.error.generic");
  });

  it("formats cost without lying about small numbers", () => {
    expect(store.formatCost(null)).toBe("—");
    expect(store.formatCost(undefined)).toBe("—");
    expect(store.formatCost(0)).toBe("$0.00");
    expect(store.formatCost(0.0004)).toBe("< $0.01");
    expect(store.formatCost(1.239)).toBe("$1.24");
  });

  it("renders a missing body as a dash", () => {
    expect(store.prettyBody(null)).toBe("—");
    expect(store.prettyBody({ a: 1 })).toBe('{\n  "a": 1\n}');
  });

  it("diffs only the lines that changed", () => {
    const lines = store.diffBodies({ model: "x", temperature: 0 }, { model: "x", temperature: 1.5 });
    const changed = lines.filter((l) => l.kind !== "same");
    expect(changed).toHaveLength(2);
    expect(changed[0]).toEqual({ kind: "a", text: '  "temperature": 0' });
    expect(changed[1]).toEqual({ kind: "b", text: '  "temperature": 1.5' });
    expect(lines.filter((l) => l.kind === "same").length).toBeGreaterThan(0);
  });

  it("keeps extra lines from the longer body", () => {
    const lines = store.diffBodies({ a: 1 }, { a: 1, b: 2 });
    expect(lines.some((l) => l.kind === "b" && l.text.includes('"b"'))).toBe(true);
  });

  it("tallies verdicts, empty round included", () => {
    expect(store.tally(null)).toEqual({ proven: 0, sent: 0, ignored: 0, unsupported: 0 });
    expect(store.tally(ROUND)).toEqual({ proven: 1, sent: 0, ignored: 1, unsupported: 0 });
  });

  it("mirrors the Rust budget constants", () => {
    expect(store.MAX_PROBE_REQUESTS).toBe(8);
    expect(store.MAX_OUTPUT_TOKENS).toBe(200);
    expect(store.PROBE_PARAMS).toHaveLength(5);
  });
});

describe("actions", () => {
  it("loads the last round once per session", async () => {
    invoke.mockResolvedValue(ROUND);
    await store.loadLast();
    await store.loadLast();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("llm_wire_probe_last");
    expect(store.getRound()?.model).toBe("gpt-x");
    await store.loadLast(true);
    expect(invoke).toHaveBeenCalledTimes(2);
  });

  it("treats a null last round as no round, not as an error", async () => {
    invoke.mockResolvedValue(null);
    await store.loadLast();
    expect(store.getRound()).toBeNull();
    expect(store.getWireError()).toBeNull();
  });

  it("turns a failed load into an i18n key", async () => {
    invoke.mockRejectedValue("ERR_LLM_STUB");
    await store.loadLast();
    expect(store.getWireError()).toBe("llm.wire.error.stub");
    store.clearWireError();
    expect(store.getWireError()).toBeNull();
  });

  it("estimates without running", async () => {
    invoke.mockResolvedValue({
      cases: 4,
      requests: 8,
      input_tokens: 120,
      output_tokens: 1600,
      cost_usd: null,
    });
    const est = await store.estimateRound("openai", "gpt-x");
    expect(est?.requests).toBe(8);
    expect(invoke).toHaveBeenCalledWith("llm_wire_probe_run", {
      provider: "openai",
      model: "gpt-x",
      cases: null,
      estimateOnly: true,
    });
    expect(store.getRound()).toBeNull();
  });

  it("runs a round and keeps the evidence", async () => {
    invoke.mockResolvedValue(ROUND);
    const ok = await store.runRound("openai", "gpt-x", ["temperature"]);
    expect(ok).toBe(true);
    expect(invoke).toHaveBeenCalledWith("llm_wire_probe_run", {
      provider: "openai",
      model: "gpt-x",
      cases: ["temperature"],
      estimateOnly: false,
    });
    expect(store.getRound()?.results[0].verdict).toBe("proven");
    expect(store.isRunning()).toBe(false);
  });

  it("reports a failed round as a key and keeps the previous evidence", async () => {
    invoke.mockResolvedValueOnce(ROUND);
    await store.runRound("openai", "gpt-x");
    invoke.mockRejectedValueOnce("ERR_LLM_MODEL: no provider client for `openai` yet");
    const ok = await store.runRound("openai", "gpt-x");
    expect(ok).toBe(false);
    expect(store.getWireError()).toBe("llm.wire.error.no_provider");
    expect(store.getRound()?.results).toHaveLength(2);
  });

  it("never runs two rounds at once", async () => {
    let release: (v: unknown) => void = () => {};
    invoke.mockImplementation(() => new Promise((r) => (release = r)));
    const first = store.runRound("openai", "gpt-x");
    const second = await store.runRound("openai", "gpt-x");
    expect(second).toBe(false);
    expect(invoke).toHaveBeenCalledTimes(1);
    release(ROUND);
    expect(await first).toBe(true);
  });
});
