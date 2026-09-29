/**
 * The "needs you" count follows the backend's events (3.9 aceite: visible on
 * any /llm route ≤ 1 s after the event), with the real jobs store behind the
 * work store and no timer of the badge's own.
 */
import { beforeAll, describe, expect, it, vi } from "vitest";

const handlers = new Map<string, (ev: { payload: unknown }) => void>();
let missions: unknown[] = [];
const invoke = vi.fn(async (cmd: string) => {
  if (cmd === "assist_mission_list") return missions;
  if (cmd === "llm_triggers_list") return { triggers: [], bridge: null };
  if (cmd === "llm_jobs_state") return null;
  return [];
});
vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string) => invoke(cmd) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (event: string, cb: (ev: { payload: unknown }) => void) => {
    handlers.set(event, cb);
    return () => {};
  },
}));
vi.mock("$lib/stores/toast-store.svelte", () => ({ showToast: () => {} }));
vi.mock("$lib/stores/debug-store.svelte", () => ({ addLog: () => {} }));
vi.mock("$lib/i18n", async () => {
  const { readable } = await import("svelte/store");
  return { t: readable((key: string) => key) };
});
vi.mock("$lib/central/threads/threads-store.svelte", () => ({
  threadsStore: { threads: [], pendingApprovals: [], pendingUserInputs: [], error: null, init: async () => {} },
}));

type WorkStore = typeof import("./llm-work-store.svelte");
let work: WorkStore;

beforeAll(async () => {
  vi.stubGlobal("$state", <T>(value: T) => value);
  vi.stubGlobal("$derived", <T>(value: T) => value);
  work = await import("./llm-work-store.svelte");
  work.initWorkStore();
  await vi.waitFor(() => expect(handlers.has("llm://job")).toBe(true));
});

function job(state: string, seq: number) {
  return {
    id: "j1", kind: "run", agent_id: "a", conversation_id: "c1", prompt: "Fix the build", workspace: null,
    state, loop_id: null, trigger_id: null, request_id: "r1", created_ms: Date.now(), started_ms: Date.now(),
    finished_ms: null, result: null, error: null, log: "", state_seq: seq,
  };
}

describe("needs you, live", () => {
  it("counts a job's question as soon as `llm://job` says it waits, and drops it when it moves on", () => {
    expect(work.getWorkSummary().needsYou).toBe(0);
    const t0 = performance.now();
    handlers.get("llm://job")!({ payload: job("waiting_approval", 2) });
    expect(work.getWorkSummary().needsYou).toBe(1);
    expect(performance.now() - t0).toBeLessThan(1000);
    // A stale event (lower state_seq) does not bring it back.
    handlers.get("llm://job")!({ payload: job("running", 3) });
    handlers.get("llm://job")!({ payload: job("waiting_approval", 2) });
    expect(work.getWorkSummary().needsYou).toBe(0);
    expect(work.getWorkSummary().running).toBe(1);
  });

  it("re-reads the missions on `assist://mission` within a second", async () => {
    vi.useFakeTimers();
    missions = [{ id: "m1", conversation_id: "mc", objective: "Ship it", state: "blocked", created_ms: 1, updated_ms: 1, finished_ms: null, block: { code: "budget" } }];
    handlers.get("assist://mission")!({ payload: { mission_id: "m1", state: "blocked", revision: 2 } });
    await vi.advanceTimersByTimeAsync(1000);
    vi.useRealTimers();
    expect(work.getWorkItems().find((i) => i.key === "mission:m1")?.pill).toBe("decision");
  });
});
