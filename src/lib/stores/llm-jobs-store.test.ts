import { beforeAll, describe, expect, it, vi } from "vitest";

const invoke = vi.fn(async (_cmd: string, _args?: unknown) => [] as unknown);
const handlers = new Map<string, (ev: { payload: unknown }) => void>();
const listen = vi.fn(async (event: string, cb: (ev: { payload: unknown }) => void) => {
  handlers.set(event, cb);
  return () => {};
});

vi.mock("@tauri-apps/api/core", () => ({ invoke: (cmd: string, args?: unknown) => invoke(cmd, args) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, cb: (ev: { payload: unknown }) => void) => listen(event, cb),
}));
vi.mock("$lib/stores/toast-store.svelte", () => ({ showToast: () => {} }));

type Store = typeof import("./llm-jobs-store.svelte");
let store: Store;

beforeAll(async () => {
  vi.stubGlobal("$state", <T>(value: T) => value);
  vi.stubGlobal("$derived", <T>(value: T) => value);
  store = await import("./llm-jobs-store.svelte");
});

function job(state: string, seq: number, log = "") {
  return {
    id: "j1",
    kind: "run",
    agent_id: "a",
    conversation_id: "c",
    prompt: "p",
    workspace: null,
    state,
    state_seq: seq,
    loop_id: null,
    trigger_id: null,
    request_id: "r1",
    created_ms: 1,
    started_ms: 1,
    finished_ms: null,
    result: null,
    error: null,
    log,
  } as unknown as import("./llm-jobs-store.svelte").Job;
}

describe("mergeJobEvent", () => {
  it("never lets an older state overwrite a newer one", () => {
    let list = store.mergeJobEvent([], job("running", 2));
    list = store.mergeJobEvent(list, job("waiting_approval", 3));
    // A late event of the earlier state arrives after the newer one.
    list = store.mergeJobEvent(list, job("running", 2));
    expect(list[0].state).toBe("waiting_approval");
    // Same state, same seq: a log update still applies.
    list = store.mergeJobEvent(list, job("waiting_approval", 3, "→ Bash\n"));
    expect(list[0].log).toBe("→ Bash\n");
    list = store.mergeJobEvent(list, job("running", 4));
    expect(list[0].state).toBe("running");
  });

  it("accepts rows from before the sequence existed", () => {
    const old = { ...job("done", 0) } as Record<string, unknown>;
    delete old.state_seq;
    const list = store.mergeJobEvent([job("running", 0)], old as never);
    expect(list[0].state).toBe("done");
  });
});

describe("asks follow events, not a timer (A5)", () => {
  it("fetches once when a job starts or stops waiting, and drops a resolved ask", async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === "llm_tool_asks_pending")
        return [{ agent: "a", request_id: "r1", tool_call_id: "c1", tool: "Bash", preview: "ls" }];
      return [];
    });
    store.initJobsStore();
    await Promise.resolve();
    const onJob = handlers.get("llm://job");
    expect(onJob).toBeDefined();
    invoke.mockClear();
    onJob!({ payload: job("waiting_approval", 5) });
    await new Promise((r) => setTimeout(r, 0));
    expect(invoke).toHaveBeenCalledWith("llm_tool_asks_pending", undefined);
    expect(store.getAsks().map((a) => a.tool_call_id)).toEqual(["c1"]);
    handlers.get("omni://tool-resolved")!({ payload: { tool_call_id: "c1", outcome: "expired" } });
    expect(store.getAsks()).toEqual([]);
    // No interval anywhere: nothing polls by itself.
    invoke.mockClear();
    await new Promise((r) => setTimeout(r, 50));
    expect(invoke).not.toHaveBeenCalled();
  });
});

describe("one door for every answer (2.2)", () => {
  it("answers a durable ask by its approval id, and a stale one is dismissed", async () => {
    invoke.mockReset();
    invoke.mockImplementation(async () => ({}) as unknown);
    await store.answerAsk({ id: "apr_1", agent: "a", request_id: "r1", tool_call_id: "c1", tool: "Bash", preview: "ls" }, true, false);
    expect(invoke).toHaveBeenCalledWith("llm_approval_respond", { id: "apr_1", decision: "accept" });
    await store.answerAsk({ id: "apr_2", agent: "a", request_id: "r1", tool_call_id: "c2", tool: "Bash", preview: "ls" }, false, false);
    expect(invoke).toHaveBeenCalledWith("llm_approval_respond", { id: "apr_2", decision: "decline" });
    await store.respondApproval("apr_3", "cancel");
    expect(invoke).toHaveBeenCalledWith("llm_approval_respond", { id: "apr_3", decision: "cancel" });
    // A row without an id (no run record) still answers by its tool call.
    await store.answerAsk({ agent: "a", request_id: "r1", tool_call_id: "c4", tool: "Bash", preview: "" }, true, true);
    expect(invoke).toHaveBeenCalledWith("llm_tool_answer", { requestId: "r1", toolCallId: "c4", allow: true, always: true });
  });
});

describe("the queue is explained (1.6)", () => {
  const state = {
    running: [{ id: "j1", agent_id: "a", kind: "run", state: "running", workspace: "/w/repo" }],
    blocked: [],
    queued: [
      { id: "j2", position: 1, reason: "workspace", holder: "j1", folder: "/w/repo", detail: "folder busy by j1", agent_id: "a", kind: "run", state: "waiting_workspace", workspace: "/w/repo" },
      { id: "j3", position: 2, reason: "agent_cap", detail: "agent a at cap", agent_id: "a", kind: "run", state: "queued", workspace: null },
      { id: "j4", position: 3, reason: "slot", detail: "all busy", agent_id: "b", kind: "mission", state: "queued", workspace: null },
    ],
    limits: { max_parallel: 2, max_per_agent: 1, in_use: 2, available: 0, stagger_ms: 500 },
  } as unknown as import("./llm-jobs-store.svelte").QueueState;

  it("gives every queued job its place and a translated reason", () => {
    const [w, a, s] = state.queued.map((q) => store.queueLines(q, state));
    expect(w.position).toEqual({ key: "llm.jobs.queue.position", params: { position: 1, total: 3 } });
    expect(w.reason).toEqual({ key: "llm.jobs.queue.reason_workspace", params: { holder: "j1" } });
    expect(a.reason).toEqual({ key: "llm.jobs.queue.reason_agent_cap", params: { cap: 1 } });
    expect(s.reason).toEqual({ key: "llm.jobs.queue.reason_slot", params: { limit: 2 } });
  });

  it("reads the reason stored on the row when the queue has not arrived yet", () => {
    expect(store.reasonLine("workspace:j9", state)).toEqual({ key: "llm.jobs.queue.reason_workspace", params: { holder: "j9" } });
    expect(store.reasonLine("slot", null)).toEqual({ key: "llm.jobs.queue.reason_slot", params: { limit: "?" } });
    expect(store.reasonLine(null, state)).toBeNull();
  });

  it("follows llm://queue without asking", async () => {
    store.initJobsStore();
    await Promise.resolve();
    const onQueue = handlers.get("llm://queue");
    expect(onQueue).toBeDefined();
    onQueue!({ payload: state });
    expect(store.getQueue()?.queued.map((q) => q.id)).toEqual(["j2", "j3", "j4"]);
    expect(store.queuedEntry("j3")?.position).toBe(2);
  });
});

describe("routines keep every field when saved (1.4)", () => {
  const tr = {
    id: "t1",
    name: "daily",
    kind: "cron" as const,
    cron: "0 9 * * *",
    agent_id: "a",
    prompt: "p",
    workspace: "/w",
    enabled: true,
    last_fired_ms: null,
    fire_count: 3,
    created_ms: 1,
    muted: true,
    mission: { criteria: [] },
    deadline_min: 20,
    catch_up: "all_capped" as const,
    concurrency: "queue" as const,
    tz: "+02:00",
    missed: 4,
    backlog: 1,
  };

  it("carries the policies, the zone, the mission and the silence", () => {
    const input = store.triggerInput(tr, { enabled: false });
    expect(input).toMatchObject({
      id: "t1",
      enabled: false,
      muted: true,
      mission: { criteria: [] },
      deadline_min: 20,
      catch_up: "all_capped",
      concurrency: "queue",
      tz: "+02:00",
    });
    expect(store.triggerInput(tr, { catch_up: "skip" }).catch_up).toBe("skip");
  });

  it("defaults a routine from before 1.4 to once / skip_if_active / local", () => {
    const old = { ...tr, catch_up: undefined, concurrency: undefined, tz: undefined };
    const input = store.triggerInput(old, {});
    expect([input.catch_up, input.concurrency, input.tz]).toEqual(["once", "skip_if_active", "local"]);
  });

  it("follows llm://trigger without asking", async () => {
    store.initJobsStore();
    await Promise.resolve();
    handlers.get("llm://trigger")?.({ payload: { ...tr, missed: 5 } });
    expect(store.getTriggers().find((x) => x.id === "t1")?.missed).toBe(5);
  });
});

describe("a job's diff (1.7)", () => {
  it("translates why a diff is unavailable, never as 'no changes'", () => {
    expect(store.diffReasonKey("no_snapshot")).toBe("llm.jobs.diff.unavailable_no_snapshot");
    expect(store.diffReasonKey("no_workspace")).toBe("llm.jobs.diff.unavailable_no_workspace");
    expect(store.diffReasonKey("something_new")).toBe("llm.jobs.diff.unavailable_error");
    expect(store.diffReasonKey(null)).toBe("llm.jobs.diff.unavailable_no_snapshot");
  });

  it("fetches the diff with the job id", async () => {
    invoke.mockResolvedValueOnce({ job_id: "j1", available: true, files: [], patch: "" });
    const d = await store.fetchJobDiff("j1");
    expect(invoke).toHaveBeenCalledWith("llm_job_diff", { id: "j1" });
    expect(d?.available).toBe(true);
  });
});
