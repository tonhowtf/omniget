import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { companionVolume, fileName, findSet, isActive, mergeJob, swapInQueue, syncPlan, toolFromParams, type StemJob, type StemSet } from "./stems";

function set(over: Partial<StemSet> = {}): StemSet {
  return {
    source: "/m/song.mp3",
    source_id: 1,
    vocals: "/m/song (Vocals).flac",
    vocals_id: 2,
    instrumental: "/m/song (Instrumental).flac",
    instrumental_id: 3,
    model: "spleeter-2stems",
    created_at: 0,
    ...over,
  };
}

function job(over: Partial<StemJob> = {}): StemJob {
  return { id: 1, track_id: 1, source: "/m/song.mp3", title: "Song", outputs: [], state: "queued", progress: 0, error: null, ...over };
}

describe("stem sets", () => {
  it("knows which part of a set a track is", () => {
    const sets = [set()];
    expect(findSet(sets, 1)?.role).toBe("source");
    expect(findSet(sets, 2)?.role).toBe("vocals");
    expect(findSet(sets, 3)?.role).toBe("instrumental");
    expect(findSet(sets, 9)).toBeNull();
    expect(findSet(sets, null)).toBeNull();
  });

  it("prefers the instrumental match over an older set of the same source", () => {
    const old = set({ instrumental_id: 30, vocals_id: 20 });
    const fresh = set();
    expect(findSet([old, fresh], 3)?.set).toBe(fresh);
    expect(findSet([old, fresh], 1)?.set).toBe(old);
  });

  it("does not offer karaoke for a source whose instrumental left the library", () => {
    expect(findSet([set({ instrumental_id: null })], 1)).toBeNull();
  });
});

describe("vocal companion", () => {
  it("follows the player volume curve and mute", () => {
    expect(companionVolume(0.5, 1, false)).toBeCloseTo(0.5);
    expect(companionVolume(1, 0.5, false)).toBeCloseTo(0.25);
    expect(companionVolume(1, 1, true)).toBe(0);
    expect(companionVolume(3, 1, false)).toBe(1);
    expect(companionVolume(-1, 1, false)).toBe(0);
  });

  it("starts and stops with the main track", () => {
    expect(syncPlan({ time: 10, paused: false, rate: 1 }, { time: 10, paused: true, rate: 1 }).play).toBe(true);
    expect(syncPlan({ time: 10, paused: true, rate: 1 }, { time: 10, paused: false, rate: 1 }).play).toBe(false);
    expect(syncPlan({ time: 10, paused: false, rate: 1 }, { time: 10.05, paused: false, rate: 1 }).play).toBeNull();
  });

  it("only jumps when it drifts too far", () => {
    expect(syncPlan({ time: 10, paused: false, rate: 1 }, { time: 10.05, paused: false, rate: 1 }).seek).toBeNull();
    expect(syncPlan({ time: 42, paused: false, rate: 1 }, { time: 10, paused: false, rate: 1 }).seek).toBe(42);
    expect(syncPlan({ time: 10, paused: true, rate: 1 }, { time: 10.05, paused: true, rate: 1 }).seek).toBe(10);
  });

  it("copies the playback speed", () => {
    expect(syncPlan({ time: 0, paused: false, rate: 1.25 }, { time: 0, paused: false, rate: 1 }).rate).toBe(1.25);
    expect(syncPlan({ time: 0, paused: false, rate: 1 }, { time: 0, paused: false, rate: 1 }).rate).toBeNull();
  });
});

describe("jobs and helpers", () => {
  it("merges progress events by id", () => {
    const a = job({ id: 1 });
    const b = job({ id: 2 });
    const next = mergeJob([a, b], job({ id: 2, state: "running", progress: 0.4 }));
    expect(next).toHaveLength(2);
    expect(next[1].progress).toBe(0.4);
    expect(mergeJob([a], b)).toHaveLength(2);
  });

  it("treats queued, preparing and running as active", () => {
    expect(isActive(job({ state: "preparing" }))).toBe(true);
    expect(isActive(job({ state: "running" }))).toBe(true);
    expect(isActive(job({ state: "done" }))).toBe(false);
    expect(isActive(job({ state: "cancelled" }))).toBe(false);
  });

  it("swaps only the current queue slot", () => {
    expect(swapInQueue([5, 1, 5], 2, 9)).toEqual([5, 1, 9]);
  });

  it("reads file names on any platform", () => {
    expect(fileName("/a/b/c.flac")).toBe("c.flac");
    expect(fileName("C:\\Music\\x.mp3")).toBe("x.mp3");
  });

  it("opens a music tool from the address", () => {
    expect(toolFromParams(new URLSearchParams("tool=stems"))).toBe("stems");
    expect(toolFromParams(new URLSearchParams("tool=nope"))).toBeNull();
    expect(toolFromParams(new URLSearchParams(""))).toBeNull();
  });
});
