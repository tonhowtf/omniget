import { describe, expect, it } from "vitest";
import { divergentTouched, estimateSummary, type ArenaCompare, type ArenaEstimate } from "./arena";

describe("arena estimate", () => {
  it("says rough and counts unknown columns instead of pricing them at zero", () => {
    const e: ArenaEstimate = {
      entries: [
        { driver: "claude", model: "sonnet", usd: 0.12, source: "history", runs: 3, avgDurationMs: 1000 },
        { driver: "codex", model: "gpt-5", usd: 0.1, source: "price", runs: 0, avgDurationMs: null },
        { driver: "gemini", model: null, usd: null, source: "unknown", runs: 0, avgDurationMs: null },
      ],
      totalUsd: 0.22,
      unknown: 1,
    };
    expect(estimateSummary(e)).toEqual({ total: "$0.22", rough: true, unknown: 1 });
    expect(estimateSummary({ entries: [e.entries[2]], totalUsd: null, unknown: 1 })).toEqual({ total: "—", rough: false, unknown: 1 });
    expect(estimateSummary(null)).toBeNull();
  });
});

describe("arena compare", () => {
  it("counts the divergent files a column touched", () => {
    const c: ArenaCompare = {
      arenaId: "a",
      entries: ["x", "y"],
      pending: [],
      divergent: 1,
      agreed: 1,
      files: [
        { path: "NOTES.md", divergent: true, touchedBy: 1, variants: 2, cells: [
          { entryId: "x", status: "=", additions: null, deletions: null, variant: 0 },
          { entryId: "y", status: "A", additions: 3, deletions: 0, variant: 1 },
        ] },
        { path: "a.py", divergent: false, touchedBy: 2, variants: 1, cells: [
          { entryId: "x", status: "M", additions: 1, deletions: 1, variant: 1 },
          { entryId: "y", status: "M", additions: 1, deletions: 1, variant: 1 },
        ] },
      ],
    };
    expect(divergentTouched(c, "x")).toBe(0);
    expect(divergentTouched(c, "y")).toBe(1);
    expect(divergentTouched(null, "y")).toBe(0);
  });
});
