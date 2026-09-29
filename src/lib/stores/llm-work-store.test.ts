import { afterAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkItem } from "$lib/llm/work";

const toasts: { type: string; message: string }[] = [];
vi.mock("@tauri-apps/api/core", () => ({ invoke: async () => [] }));
vi.mock("@tauri-apps/api/event", () => ({ listen: async () => () => {} }));
vi.mock("$lib/stores/toast-store.svelte", () => ({
  showToast: (type: string, message: string) => toasts.push({ type, message }),
}));
vi.mock("$lib/stores/debug-store.svelte", () => ({ addLog: () => {} }));
vi.mock("$lib/i18n", async () => {
  const { readable } = await import("svelte/store");
  return {
    t: readable((key: string, params: Record<string, unknown> = {}) => `${key}|${JSON.stringify(params)}`),
  };
});
vi.mock("$lib/stores/llm-jobs-store.svelte", () => ({
  getAsks: () => [],
  getJobs: () => [],
  getJobsReadError: () => null,
  getLoops: () => [],
  getPermissions: () => [],
  getRuns: () => [],
  initJobsStore: () => {},
  pollAsks: async () => {},
}));
vi.mock("$lib/central/threads/threads-store.svelte", () => ({
  threadsStore: { threads: [], pendingApprovals: [], pendingUserInputs: [], error: null, init: async () => {} },
}));

/** A browser-like storage that survives a module reload (an app reopen). */
const storage = new Map<string, string>();
vi.stubGlobal("localStorage", {
  getItem: (k: string) => storage.get(k) ?? null,
  setItem: (k: string, v: string) => void storage.set(k, v),
  removeItem: (k: string) => void storage.delete(k),
});
vi.stubGlobal("$state", <T>(value: T) => value);

async function freshStore() {
  vi.resetModules();
  return await import("./llm-work-store.svelte");
}

function item(extra: Partial<WorkItem>): WorkItem {
  return {
    key: "job:j1",
    source: "job",
    id: "j1",
    title: "Update the changelog",
    pill: "approval",
    section: "needs_you",
    href: "/llm/jobs?job=j1",
    at: Date.now(),
    turn: "tc1",
    detail: "shell_exec",
    ...extra,
  };
}

beforeEach(() => {
  toasts.length = 0;
});

afterAll(() => {
  vi.unstubAllGlobals();
});

describe("announce", () => {
  it("tells about a question once, and not again after the app reopens", async () => {
    storage.clear();
    let store = await freshStore();
    expect(store.announce([item({})], "/llm/threads")).toHaveLength(1);
    expect(toasts).toHaveLength(1);
    expect(toasts[0].message).toContain("llm.work.notify.approval");
    // The same state again, same session: nothing.
    expect(store.announce([item({})], "/llm/threads")).toHaveLength(0);
    // Reopen: a new module instance, the persisted keys.
    store = await freshStore();
    expect(store.announce([item({})], "/llm/threads")).toHaveLength(0);
    expect(toasts).toHaveLength(1);
    // A new question on the same job is news.
    expect(store.announce([item({ turn: "tc2" })], "/llm/threads")).toHaveLength(1);
    expect(toasts).toHaveLength(2);
  });

  it("records without a toast what is already on screen", async () => {
    storage.clear();
    const store = await freshStore();
    expect(store.announce([item({})], "/llm/jobs")).toHaveLength(0);
    expect(toasts).toHaveLength(0);
    // And it is not announced later from another page either.
    expect(store.announce([item({})], "/llm/threads")).toHaveLength(0);
  });

  it("never announces work that finished before this session", async () => {
    storage.clear();
    const store = await freshStore();
    const old = item({ key: "job:old", id: "old", pill: "ready", section: "done", at: Date.now() - 60_000, turn: "4" });
    expect(store.announce([old], "/llm")).toHaveLength(0);
  });

  it("groups a burst into three toasts and a count", async () => {
    storage.clear();
    const store = await freshStore();
    const burst = Array.from({ length: 5 }, (_, i) => item({ key: `job:b${i}`, id: `b${i}`, turn: `t${i}` }));
    expect(store.announce(burst, "/llm")).toHaveLength(5);
    expect(toasts).toHaveLength(4);
    expect(toasts[3].message).toContain("llm.work.notify.more");
  });

  it("survives a storage that throws", async () => {
    storage.clear();
    const store = await freshStore();
    const broken = vi.spyOn(globalThis.localStorage, "getItem").mockImplementation(() => {
      throw new Error("blocked");
    });
    expect(store.announce([item({ key: "job:z", turn: "z" })], "/llm")).toHaveLength(1);
    broken.mockRestore();
  });
});
