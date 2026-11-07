import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Autosaver, hasUnsaved, initialSaveState, reduceSave, retryDelay, type SaveState } from "./autosave";
import { ago, filterBoards, formatSize, sortBoards, upsert, visibleBoards } from "./library";
import { excalidrawLang } from "./locale";
import { checkName, fileStem, sanitizeName, stripExtension } from "./names";
import type { BoardEntry } from "./types";

function board(name: string, modified_ms: number): BoardEntry {
  return { name, path: `/b/${name}.excalidraw`, modified_ms, size: 100, has_thumb: false };
}

describe("names", () => {
  it("accepts normal names and strips the extension", () => {
    expect(checkName("  Plano de voo  ")).toEqual({ ok: true, name: "Plano de voo" });
    expect(checkName("idea.excalidraw")).toEqual({ ok: true, name: "idea" });
    expect(stripExtension("A.EXCALIDRAW")).toBe("A");
  });

  it("rejects separators, traversal, reserved and long names", () => {
    for (const bad of ["a/b", "a\\b", "..", ".hidden", "x:y", "tab\there", "CON", "trail."]) {
      expect(checkName(bad).ok, bad).toBe(false);
    }
    expect(checkName("   ")).toEqual({ ok: false, error: "empty" });
    expect(checkName("a".repeat(121))).toEqual({ ok: false, error: "too_long" });
  });

  it("detects collisions case-insensitively but allows the current name", () => {
    expect(checkName("map", ["Map", "Other"])).toEqual({ ok: false, error: "exists" });
    expect(checkName("MAP", ["Map"], "Map")).toEqual({ ok: true, name: "MAP" });
    expect(checkName("MAP", [], "Map")).toEqual({ ok: true, name: "MAP" });
    expect(checkName("MAP", ["MAP"], "Map")).toEqual({ ok: false, error: "exists" });
    expect(checkName("maP", ["Map", "MAP"], "Map")).toEqual({ ok: false, error: "exists" });
  });

  it("sanitizes into something saveable", () => {
    expect(sanitizeName("a/b:c")).toBe("a b c");
    expect(sanitizeName("...")).toBe("board");
    expect(sanitizeName("..x..", "f")).toBe("x");
    expect(fileStem("/Users/me/Plan v2.excalidraw")).toBe("Plan v2");
    expect(fileStem("C:\\x\\y.excalidraw")).toBe("y");
  });
});

describe("library", () => {
  const list = [board("beta", 10), board("Alpha 10", 30), board("alpha 2", 20), board("Ação", 5)];

  it("sorts by recent first and by natural name", () => {
    expect(sortBoards(list, "recent").map((b) => b.name)).toEqual(["Alpha 10", "alpha 2", "beta", "Ação"]);
    expect(sortBoards(list, "name").map((b) => b.name)).toEqual(["Ação", "alpha 2", "Alpha 10", "beta"]);
  });

  it("filters by every word, ignoring case and accents", () => {
    expect(filterBoards(list, "acao").map((b) => b.name)).toEqual(["Ação"]);
    expect(filterBoards(list, "ALPHA 1").map((b) => b.name)).toEqual(["Alpha 10"]);
    expect(filterBoards(list, "  ").length).toBe(4);
    expect(visibleBoards(list, "alpha", "name").map((b) => b.name)).toEqual(["alpha 2", "Alpha 10"]);
  });

  it("upserts to the top and replaces renamed paths", () => {
    const renamed = board("gamma", 40);
    const next = upsert(list, renamed, "/b/beta.excalidraw");
    expect(next[0]).toBe(renamed);
    expect(next.some((b) => b.name === "beta")).toBe(false);
    expect(next.length).toBe(4);
  });

  it("formats sizes and relative times", () => {
    expect(formatSize(512)).toBe("512 B");
    expect(formatSize(1536)).toBe("1.5 KB");
    expect(formatSize(50 * 1024 * 1024)).toBe("50 MB");
    expect(ago(0, 30_000)).toEqual({ unit: "now", value: 0 });
    expect(ago(0, 5 * 60_000)).toEqual({ unit: "minute", value: 5 });
    expect(ago(0, 3 * 3_600_000)).toEqual({ unit: "hour", value: 3 });
    expect(ago(0, 2 * 86_400_000)).toEqual({ unit: "day", value: 2 });
    expect(ago(1, 30 * 86_400_000).unit).toBe("date");
  });
});

describe("locale", () => {
  it("maps app locales to Excalidraw language codes", () => {
    expect(excalidrawLang("pt")).toBe("pt-BR");
    expect(excalidrawLang("zh")).toBe("zh-CN");
    expect(excalidrawLang("zh-TW")).toBe("zh-TW");
    expect(excalidrawLang("zh_tw")).toBe("zh-TW");
    expect(excalidrawLang("fa")).toBe("fa-IR");
    expect(excalidrawLang("el")).toBe("el-GR");
    expect(excalidrawLang("es-MX")).toBe("es-ES");
    expect(excalidrawLang("lo")).toBe("en");
    expect(excalidrawLang(null)).toBe("en");
  });
});

describe("save reducer", () => {
  it("tracks revisions through a save", () => {
    let s = initialSaveState();
    s = reduceSave(s, { type: "change" });
    expect(s.status.kind).toBe("dirty");
    s = reduceSave(s, { type: "start", revision: 1 });
    s = reduceSave(s, { type: "change" });
    expect(s.status.kind).toBe("saving");
    s = reduceSave(s, { type: "ok", revision: 1, at: 5 });
    expect(s.status.kind).toBe("dirty");
    expect(hasUnsaved(s)).toBe(true);
    s = reduceSave(s, { type: "ok", revision: 2, at: 6 });
    expect(s.status).toEqual({ kind: "saved", at: 6 });
  });

  it("keeps the error visible while edits continue, and clears on success", () => {
    let s: SaveState = reduceSave(initialSaveState(), { type: "change" });
    s = reduceSave(s, { type: "fail", revision: 1, reason: "disk full", retryInMs: 2000 });
    s = reduceSave(s, { type: "change" });
    expect(s.status).toMatchObject({ kind: "error", reason: "disk full", failures: 1 });
    s = reduceSave(s, { type: "ok", revision: 2, at: 9 });
    expect(s.status.kind).toBe("saved");
    expect(s.failures).toBe(0);
  });

  it("backs off retries and caps the delay", () => {
    expect(retryDelay(1)).toBe(2000);
    expect(retryDelay(2)).toBe(5000);
    expect(retryDelay(99)).toBe(30000);
  });
});

describe("Autosaver", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("debounces changes into a single save", async () => {
    const save = vi.fn(async () => {});
    const saver = new Autosaver({ delayMs: 1500, save, onState: () => {}, now: () => 1 });
    saver.change();
    await vi.advanceTimersByTimeAsync(1000);
    saver.change();
    await vi.advanceTimersByTimeAsync(1000);
    expect(save).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(600);
    expect(save).toHaveBeenCalledTimes(1);
    expect(saver.current.status).toEqual({ kind: "saved", at: 1 });
  });

  it("retries after a failure without losing the change", async () => {
    let fail = true;
    const save = vi.fn(async () => {
      if (fail) throw new Error("locked");
    });
    const states: string[] = [];
    const saver = new Autosaver({ delayMs: 100, save, onState: (s) => states.push(s.status.kind), describe: (e) => (e as Error).message, retryDelays: [500] });
    saver.change();
    await vi.advanceTimersByTimeAsync(100);
    expect(saver.current.status).toMatchObject({ kind: "error", reason: "locked" });
    fail = false;
    await vi.advanceTimersByTimeAsync(500);
    expect(save).toHaveBeenCalledTimes(2);
    expect(saver.current.status.kind).toBe("saved");
    expect(states).toContain("error");
  });

  it("flush saves immediately and reports whether everything is on disk", async () => {
    const save = vi.fn(async () => {});
    const saver = new Autosaver({ delayMs: 10_000, save, onState: () => {} });
    expect(await saver.flush()).toBe(true);
    expect(save).not.toHaveBeenCalled();
    saver.change();
    expect(await saver.flush()).toBe(true);
    expect(save).toHaveBeenCalledTimes(1);
    const failing = new Autosaver({ delayMs: 10, save: async () => Promise.reject(new Error("x")), onState: () => {} });
    failing.change();
    expect(await failing.flush()).toBe(false);
    failing.dispose();
  });

  it("idle waits for the save in flight without starting a new one", async () => {
    let release: () => void = () => {};
    const save = vi.fn(() => new Promise<void>((r) => (release = r)));
    const saver = new Autosaver({ delayMs: 50, save, onState: () => {} });
    await saver.idle();
    saver.change();
    await vi.advanceTimersByTimeAsync(50);
    let settled = false;
    const waiting = saver.idle().then(() => (settled = true));
    await vi.advanceTimersByTimeAsync(0);
    expect(settled).toBe(false);
    release();
    await waiting;
    expect(settled).toBe(true);
    expect(save).toHaveBeenCalledTimes(1);
  });

  it("saves again when edits land during a save", async () => {
    let release: () => void = () => {};
    const save = vi.fn(() => new Promise<void>((r) => (release = r)));
    const saver = new Autosaver({ delayMs: 50, save, onState: () => {} });
    saver.change();
    await vi.advanceTimersByTimeAsync(50);
    saver.change();
    release();
    await vi.advanceTimersByTimeAsync(0);
    expect(saver.current.status.kind).toBe("dirty");
    await vi.advanceTimersByTimeAsync(50);
    release();
    await vi.advanceTimersByTimeAsync(0);
    expect(save).toHaveBeenCalledTimes(2);
    expect(saver.current.status.kind).toBe("saved");
  });
});
