import { describe, expect, it } from "vitest";
import { EMPTY_FILTERS, activeFilters, changeBars, errorCode, filterArgs, fromRepoSummary, fromRepoTrend, gaps, lastSync, linePath, pendingSync, progress, repoName, type ListState, type Summary, type Trend, type TrendDay } from "./audience";

const list = (kind: ListState["kind"], state: ListState["state"], fetched: number, expected: number | null): ListState => ({ kind, state, fetched, expected, started: 1, finished: null });

describe("audience filters", () => {
  it("sends nothing for the default filters", () => {
    expect(filterArgs(EMPTY_FILTERS)).toEqual({});
    expect(activeFilters(EMPTY_FILTERS)).toBe(0);
  });

  it("turns the form into tool arguments", () => {
    const f = { ...EMPTY_FILTERS, q: "  ana ", sort: "ratio", verified: "yes" as const, has_avatar: "no" as const, min_followers: "100", max_ratio: "0,5", max_followers: "abc", min_ratio: "-1" };
    expect(filterArgs(f)).toEqual({ q: "ana", sort: "ratio", verified: true, has_avatar: false, min_followers: 100, max_ratio: 0.5 });
    expect(activeFilters(f)).toBe(5);
  });
});

describe("repository stargazers", () => {
  it("reads a repository from a name or a link", () => {
    expect(repoName(" Ana/Tool ")).toBe("ana/tool");
    expect(repoName("https://github.com/ana/tool.rs/stargazers")).toBe("ana/tool.rs");
    expect(repoName("https://github.com/ana/tool.git")).toBe("ana/tool");
    expect(repoName("ana")).toBeNull();
    expect(repoName("ana/to ol")).toBeNull();
  });

  it("reads a repository summary and trend with the follower fields the tab draws", () => {
    const s = { account: "", counts: { stargazers: 3, new_stargazers: 1, unstarred: 1 }, history: { snapshots: 2, started: 5, too_short: false }, lists: [], synced_at: { stargazers: 9 } } as unknown as Summary;
    const out = fromRepoSummary(s);
    expect(out.counts.followers).toBe(3);
    expect(out.counts.unstarred).toBe(1);
    expect(lastSync(out.synced_at)).toBe(9);
    const t = { days: [{ day: "2026-11-11", stargazers: 3, gained: 1, lost: 0 }], retention: { buckets: [], since_before_history: 0, history_started: null, history_days: null }, too_short: false } as unknown as Trend;
    expect(fromRepoTrend(t).days[0]).toMatchObject({ followers: 3, following: null, ratio: null, gained: 1 });
  });
});

describe("sync state", () => {
  it("uses the older list as the last sync", () => {
    expect(lastSync({ followers: 20, following: 10 })).toBe(10);
    expect(lastSync({ followers: 20, following: null })).toBe(20);
    expect(lastSync({ followers: null, following: null })).toBeNull();
    expect(lastSync(undefined)).toBeNull();
  });

  it("knows when a sync stopped half way", () => {
    expect(pendingSync([])).toBe(false);
    expect(pendingSync([list("followers", "complete", 5, 5), list("following", "complete", 5, 5)])).toBe(false);
    expect(pendingSync([list("followers", "partial", 5, 50), list("following", "pending", 0, 9)])).toBe(true);
    expect(pendingSync([list("followers", "complete", 5, 5), list("following", "pending", 0, 9)])).toBe(true);
    expect(pendingSync([list("followers", "pending", 0, 5), list("following", "pending", 0, 9)])).toBe(false);
  });

  it("adds up progress and hides the total when a count is unknown", () => {
    expect(progress([list("followers", "partial", 30, 100), list("following", "pending", 0, 50)])).toEqual({ done: 30, total: 150 });
    expect(progress([list("followers", "partial", 30, null)])).toEqual({ done: 30, total: null });
  });
});

describe("list completeness", () => {
  it("reports each site list that is below the profile count, following first", () => {
    const side = (fetched: number, expected: number | null, source: "site" | "export" = "site") => ({ fetched, expected, source, at: 10 });
    expect(gaps(undefined)).toEqual([]);
    expect(gaps({ followers: null, following: null })).toEqual([]);
    expect(gaps({ followers: side(222, 222), following: side(222, 222) })).toEqual([]);
    expect(gaps({ followers: side(221, 222), following: side(198, 222) })).toEqual([
      { kind: "following", fetched: 198, expected: 222 },
      { kind: "followers", fetched: 221, expected: 222 },
    ]);
    expect(gaps({ followers: side(201, null, "export"), following: side(225, 225, "export") })).toEqual([]);
    expect(gaps({ followers: side(230, 222), following: side(5, null) })).toEqual([]);
  });
});

describe("trend chart", () => {
  it("draws a line and lifts the pen over missing days", () => {
    expect(linePath([], 100, 40)).toBe("");
    expect(linePath([null, null], 100, 40)).toBe("");
    expect(linePath([10, 20], 100, 40, 0)).toBe("M0.0 40.0L100.0 0.0");
    expect(linePath([10, null, 20], 100, 40, 0)).toBe("M0.0 40.0M100.0 0.0");
    expect(linePath([7], 100, 40, 0)).toBe("M50.0 40.0");
  });

  it("scales gained and lost bars to the busiest day", () => {
    const day = (gained: number, lost: number): TrendDay => ({ day: "d", followers: null, following: null, ratio: null, gained, lost });
    const bars = changeBars([day(4, 0), day(2, 1)], 100, 20, 0);
    expect(bars.map((b) => [b.up, b.down])).toEqual([[20, 0], [10, 5]]);
    expect(bars[1].x).toBeGreaterThan(bars[0].x);
    expect(changeBars([], 100, 20)).toEqual([]);
  });
});

describe("errors", () => {
  it("reads the code before the colon", () => {
    expect(errorCode('BUSY: {"message":"x"}')).toBe("BUSY");
    expect(errorCode({ message: "RATE_LIMITED: wait" })).toBe("RATE_LIMITED");
  });
});
