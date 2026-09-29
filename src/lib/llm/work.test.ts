import { describe, expect, it } from "vitest";
import type { Job, LoopDef, PendingAsk, PermissionRequest, RunView } from "$lib/stores/llm-jobs-store.svelte";
import type { MissionRow } from "$lib/stores/assist-missions-store.svelte";
import type { ApprovalRow, ThreadRow } from "$lib/central/threads/types";
import {
  deriveWork,
  freshNotifications,
  groupBySection,
  notificationKey,
  rememberSeen,
  summarize,
  type WorkInput,
} from "./work";

const NOW = 1_800_000_000_000;
const HOUR = 3_600_000;

function job(id: string, state: Job["state"], extra: Partial<Job> = {}): Job {
  return {
    id,
    kind: "run",
    agent_id: "a",
    conversation_id: `conv-${id}`,
    prompt: `Task ${id}\nmore`,
    workspace: null,
    state,
    loop_id: null,
    trigger_id: null,
    request_id: null,
    created_ms: NOW - 2 * HOUR,
    started_ms: NOW - 2 * HOUR,
    finished_ms: null,
    result: null,
    error: null,
    log: "",
    state_seq: 1,
    ...extra,
  };
}

function loop(id: string, state: LoopDef["state"], extra: Partial<LoopDef> = {}): LoopDef {
  return {
    id,
    name: `Loop ${id}`,
    agent_id: "a",
    prompt: "p",
    workspace: null,
    max_rounds: 5,
    max_minutes: null,
    check_command: null,
    state,
    rounds_done: 1,
    conversation_id: `loop-conv-${id}`,
    created_ms: NOW - HOUR,
    finished_ms: null,
    last_check: null,
    stop_reason: null,
    ...extra,
  };
}

function mission(id: string, state: MissionRow["state"], extra: Partial<MissionRow> = {}): MissionRow {
  return {
    id,
    bot_id: null,
    room_id: null,
    conversation_id: `mission-conv-${id}`,
    auth_origin: "user",
    objective: `Objective ${id}`,
    state,
    criteria_version: 1,
    budget: {} as MissionRow["budget"],
    spent: {} as MissionRow["spent"],
    workspace: null,
    context_kind: "none",
    pins: null,
    allow_replay: false,
    block: null,
    result: null,
    revision: 1,
    created_ms: NOW - HOUR,
    updated_ms: NOW - HOUR,
    started_ms: null,
    finished_ms: null,
    tasks_total: 1,
    tasks_done: 0,
    verdict: "",
    completion: "auto",
    driving: false,
    ...extra,
  };
}

function thread(id: string, status: ThreadRow["status"], extra: Partial<ThreadRow> = {}): ThreadRow {
  return {
    threadId: id,
    projectId: "p",
    title: `Thread ${id}`,
    status,
    activeTurnId: status === "running" ? "turn-2" : null,
    latestTurnId: "turn-2",
    turnCount: 2,
    archivedAt: null,
    snoozedUntil: null,
    updatedAt: new Date(NOW - HOUR).toISOString(),
    lastError: null,
    ...extra,
  } as ThreadRow;
}

function ask(requestId: string, toolCallId: string, tool = "shell_exec"): PendingAsk {
  return { agent: "a", request_id: requestId, tool_call_id: toolCallId, tool, preview: "ls" };
}

function input(extra: Partial<WorkInput> = {}): WorkInput {
  return {
    jobs: [],
    loops: [],
    asks: [],
    runs: [],
    permissions: [],
    missions: [],
    threads: [],
    threadApprovals: [],
    threadInputs: [],
    now: NOW,
    ...extra,
  };
}

describe("deriveWork", () => {
  it("counts every kind of work that waits for a person once, and links each to its page", () => {
    const items = deriveWork(
      input({
        jobs: [job("j1", "waiting_approval", { request_id: "r1" }), job("j2", "interrupted"), job("j3", "running")],
        asks: [ask("r1", "tc1")],
        threads: [thread("t1", "approval"), thread("t2", "input")],
        threadApprovals: [{ requestId: "tr1", threadId: "t1", status: "pending", requestType: "command" } as unknown as ApprovalRow],
        missions: [mission("m1", "blocked")],
      }),
    );
    const s = summarize(items);
    expect(s.needsYou).toBe(5); // j1 approval, j2 decision, t1, t2, m1
    expect(s.running).toBe(1);
    expect(s.top).toBe("approval");
    const byKey = Object.fromEntries(items.map((i) => [i.key, i]));
    expect(byKey["job:j1"].href).toBe("/llm/jobs?job=j1");
    expect(byKey["job:j1"].detail).toBe("shell_exec");
    expect(byKey["thread:t1"].href).toBe("/llm/threads?thread=t1");
    expect(byKey["thread:t2"].pill).toBe("input");
    expect(byKey["mission:m1"].href).toBe("/llm/missions?id=m1");
    expect(items[0].section).toBe("needs_you");
  });

  it("puts each item in one of the five sections", () => {
    const items = deriveWork(
      input({
        jobs: [
          job("run", "running"),
          job("q", "queued"),
          job("w", "waiting_workspace"),
          job("rev", "review", { finished_ms: NOW - HOUR }),
          job("ok", "done", { finished_ms: NOW - HOUR }),
          job("bad", "failed", { finished_ms: NOW - HOUR, failure_reason: "stalled" }),
        ],
      }),
    );
    const sections = Object.fromEntries(groupBySection(items).map((g) => [g.section, g.items.map((i) => i.id)]));
    expect(sections.needs_you).toEqual([]);
    expect(sections.running).toEqual(["run"]);
    expect([...sections.queued].sort()).toEqual(["q", "w"]);
    expect(sections.review).toEqual(["rev"]);
    expect([...sections.done].sort()).toEqual(["bad", "ok"]);
    // Failed ranks before ready inside "done".
    expect(sections.done[0]).toBe("bad");
  });

  it("folds a loop's round and a mission's turn into their parent instead of listing them twice", () => {
    const items = deriveWork(
      input({
        loops: [loop("l1", "running")],
        missions: [mission("m1", "running")],
        jobs: [
          job("round", "waiting_approval", { kind: "loop", loop_id: "l1", request_id: "r-round" }),
          job("mturn", "running", { conversation_id: "mission-conv-m1-bot" }),
          job("old-round", "done", { kind: "loop", loop_id: "l1", finished_ms: NOW - HOUR }),
        ],
        asks: [ask("r-round", "tc-round")],
      }),
    );
    expect(items.map((i) => i.key).sort()).toEqual(["loop:l1", "mission:m1"]);
    const l1 = items.find((i) => i.key === "loop:l1")!;
    expect(l1.pill).toBe("approval");
    expect(l1.href).toBe("/llm/loops?loop=l1");
    expect(summarize(items).needsYou).toBe(1);
  });

  it("counts an ask once across the job, the durable run and the thread that also carry it", () => {
    const run: RunView = {
      id: "run1",
      conversation_id: "conv-j1",
      state: "waiting_user",
      pending_permissions: 1,
      resolution: null,
      actions: [],
      input_preview: "chat",
      updated_ms: NOW,
    } as unknown as RunView;
    const perm = { id: "p1", run_id: "run1", tool_call_id: "tc1", action: "shell_exec", state: "pending" } as PermissionRequest;
    const items = deriveWork(
      input({
        jobs: [job("j1", "waiting_approval", { kind: "chat", request_id: "r1" })],
        asks: [ask("r1", "tc1"), ask("thread-req", "tr1")],
        runs: [run],
        permissions: [perm],
        threads: [thread("t1", "approval")],
        threadApprovals: [{ requestId: "tr1", threadId: "t1", status: "pending", requestType: "command" } as unknown as ApprovalRow],
      }),
    );
    // j1 (its ask + the run's permission) and t1 (the thread + the broker's copy).
    expect(summarize(items).needsYou).toBe(2);
    expect(items.find((i) => i.key === "job:j1")?.href).toBe("/llm?conversation=conv-j1");
  });

  it("lists an orphan question and an interrupted chat run so nothing waits unseen", () => {
    const run = {
      id: "run9",
      conversation_id: "c9",
      state: "interrupted",
      pending_permissions: 0,
      resolution: null,
      actions: ["continue", "discard"],
      input_preview: "fix it",
      updated_ms: NOW - HOUR,
    } as unknown as RunView;
    const items = deriveWork(input({ asks: [ask("r-x", "tc-x")], runs: [run] }));
    expect(items.map((i) => `${i.key}:${i.pill}`).sort()).toEqual(["ask:tc-x:approval", "run:run9:decision"]);
    expect(items.find((i) => i.key === "run:run9")?.href).toBe("/llm/jobs?run=run9");
  });

  it("keeps finished chat turns, old items and snoozed or archived threads out", () => {
    const items = deriveWork(
      input({
        jobs: [
          job("chat-done", "done", { kind: "chat", finished_ms: NOW - HOUR }),
          job("ancient", "done", { finished_ms: NOW - 10 * 24 * HOUR }),
        ],
        threads: [
          thread("snoozed", "idle", { snoozedUntil: new Date(NOW + HOUR).toISOString() }),
          thread("archived", "running", { archivedAt: new Date(NOW).toISOString() }),
          thread("fresh", "idle", { turnCount: 0 }),
        ],
      }),
    );
    expect(items).toEqual([]);
  });

  it("caps the done section", () => {
    const jobs = Array.from({ length: 30 }, (_, i) => job(`d${i}`, "done", { finished_ms: NOW - i * 1000 }));
    const items = deriveWork(input({ jobs, doneLimit: 5 }));
    expect(items.map((i) => i.id)).toEqual(["d0", "d1", "d2", "d3", "d4"]);
  });
});

describe("notifications", () => {
  const since = NOW - 10 * 60_000;

  it("announces each (item, turn, status) once, and nothing again after a reopen", () => {
    const first = deriveWork(input({ jobs: [job("j1", "waiting_approval", { request_id: "r1" })], asks: [ask("r1", "tc1")] }));
    const fresh = freshNotifications(first, new Set(), since);
    expect(fresh.map((i) => i.key)).toEqual(["job:j1"]);
    // What the store persists.
    const stored = rememberSeen([], fresh.map(notificationKey));
    // Same state after reopening: the same derivation, the persisted keys.
    expect(freshNotifications(first, new Set(stored), NOW)).toEqual([]);
    // A second question on the same job is a new turn: announced.
    const second = deriveWork(input({ jobs: [job("j1", "waiting_approval", { request_id: "r1", state_seq: 3 })], asks: [ask("r1", "tc2")] }));
    expect(freshNotifications(second, new Set(stored), since).map(notificationKey)).toEqual(["job:j1|tc2|approval"]);
  });

  it("announces finishes and failures of background work only when they happened this session", () => {
    const items = deriveWork(
      input({
        jobs: [
          job("new-fail", "failed", { finished_ms: NOW - 60_000 }),
          job("old-done", "done", { finished_ms: NOW - HOUR }),
          job("running", "running"),
        ],
        threads: [thread("t-idle", "idle", { updatedAt: new Date(NOW).toISOString() })],
      }),
    );
    expect(freshNotifications(items, new Set(), since).map((i) => i.key)).toEqual(["job:new-fail"]);
  });

  it("keeps only the newest seen keys", () => {
    const seen = rememberSeen(["a", "b", "c"], ["b", "d"], 3);
    expect(seen).toEqual(["c", "b", "d"]);
  });
});
