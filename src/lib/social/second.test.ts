import { describe, expect, it } from "vitest";
import { drawArgs, hasExtras, isoDay, lastDays, periodArgs, recParams, savable, seriesPath, signed, weighted, type SeriesPoint } from "./second";

describe("second wave helpers", () => {
  it("offers the extras only where the engines exist", () => {
    expect(["x", "instagram", "tiktok", "reddit", "pinterest", "github"].filter((p) => hasExtras(p as never))).toEqual(["x", "instagram", "tiktok"]);
  });

  it("sizes words by how many followers use them", () => {
    const w = weighted([
      { word: "rust", accounts: 100 },
      { word: "design", accounts: 25 },
      { word: "music", accounts: 4 },
    ]);
    expect(w[0].size).toBe(1.9);
    expect(w[2].size).toBe(0.85);
    expect(w[1].size).toBeGreaterThan(0.85);
    expect(w[1].size).toBeLessThan(1.9);
    expect(weighted([{ accounts: 3 }, { accounts: 3 }]).map((x) => x.size)).toEqual([1, 1]);
    expect(weighted([])).toEqual([]);
  });

  it("builds local days and the default period", () => {
    expect(isoDay(new Date(2026, 0, 5))).toBe("2026-01-05");
    expect(lastDays(new Date(2026, 10, 11, 23, 30), 7)).toEqual({ from: "2026-11-05", to: "2026-11-11" });
    expect(lastDays(new Date(2026, 2, 2), 7).from).toBe("2026-02-24");
  });

  it("only sends periods the tool accepts", () => {
    const base = { from: "2026-11-05", to: "2026-11-11", auto: true, compareFrom: "", compareTo: "" };
    expect(periodArgs(base)).toEqual({ from: "2026-11-05", to: "2026-11-11" });
    expect(periodArgs({ ...base, from: "2026-11-12" })).toBeNull();
    expect(periodArgs({ ...base, to: "" })).toBeNull();
    expect(periodArgs({ ...base, auto: false })).toBeNull();
    expect(periodArgs({ ...base, auto: false, compareFrom: "2026-10-01", compareTo: "2026-10-07" })).toEqual({ from: "2026-11-05", to: "2026-11-11", compare_from: "2026-10-01", compare_to: "2026-10-07" });
    expect(periodArgs({ ...base, auto: false, compareFrom: "2026-11-01", compareTo: "2026-11-05" })).toBeNull();
    expect(periodArgs({ ...base, auto: false, compareFrom: "2026-10-07", compareTo: "2026-10-01" })).toBeNull();
  });

  it("shows differences with their sign", () => {
    expect(signed(12.346)).toBe("+12.35");
    expect(signed(-3)).toBe("-3");
    expect(signed(0)).toBe("0");
    expect(signed(null)).toBe("–");
  });

  it("draws a series only when two readings have the number", () => {
    const point = (at: number, likes: number | null): SeriesPoint => ({ at, likes, replies: null, reposts: null, saves: null, views: null });
    expect(seriesPath([point(0, 10)], "likes", 100, 40)).toBe("");
    expect(seriesPath([point(0, 10), point(50, null), point(100, 30)], "likes", 100, 40)).toBe("M0.0 40.0 L100.0 0.0");
    expect(seriesPath([point(0, 10), point(100, 30)], "views", 100, 40)).toBe("");
  });

  it("saves only filters that have a field", () => {
    expect(savable({})).toBeNull();
    expect(savable({ q: "", verified: null })).toBeNull();
    expect(savable({ verified: false, min_followers: 10 })).toEqual({ verified: false, min_followers: 10 });
  });

  it("builds the draw call from the form", () => {
    const form = { post: "123", source: "reposts" as const, winners: 2, runnerUps: 1, mustFollow: true, mustRepost: false, exclude: "@ana, bob\n  carl", seed: " launch " };
    expect(drawArgs(form)).toEqual({ platform: "x", post: "123", source: "reposts", winners: 2, runner_ups: 1, must_follow: true, exclude: ["ana", "bob", "carl"], seed: "launch" });
    const bare = drawArgs({ ...form, winners: 0, runnerUps: -4, mustFollow: false, exclude: "", seed: "" }, { export: true });
    expect(bare).toEqual({ platform: "x", post: "123", source: "reposts", winners: 1, runner_ups: 0, export: true });
    expect(drawArgs({ ...form, winners: 500 }).winners).toBe(100);
  });

  it("turns the numbers of a recommendation into message values", () => {
    const rec = { id: "best_time", message: "", numbers: { weekday: 1, hour: 9, cell_median: 31, posts: null }, act_with: { tool: "x", args: {} } };
    expect(recParams(rec, (i) => ["Mon", "Tue"][i])).toEqual({ weekday: 1, hour: "09", cell_median: 31, day: "Tue" });
  });
});
