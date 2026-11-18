import { describe, expect, it } from "vitest";
import { buildOptions, cleanHandle, hasReplies, heatGrid, heatLevel, monitorQueryKey, forumsOptions, reportsFor, sortRows, sparkPoints, splitList, toolError, whoKey, type Cell } from "./insights";
import { NET } from "./api";

const cell = (weekday: number, hour: number, mean: number, posts = 3): Cell => ({ weekday, hour, mean, posts, engagement: mean * posts, thin: posts < 3 });

describe("networks", () => {
  it("offers each network the reports it can answer", () => {
    expect(reportsFor(NET.codehub)).toEqual(["summary", "posts", "times", "trend", "repos", "compare"]);
    expect(reportsFor(NET.forums)).not.toContain("repos");
    expect(reportsFor(NET.shortpost)).toHaveLength(9);
    expect(hasReplies(NET.forums)).toBe(true);
    expect(hasReplies(NET.pinboard)).toBe(false);
    expect(whoKey(NET.codehub)).toBe("username");
  });

  it("builds Forums search options without defaults", () => {
    expect(forumsOptions({ sort: "new", time: "", subreddit: " " })).toEqual({});
    expect(forumsOptions({ sort: "top", time: "week", subreddit: " r/rust " })).toEqual({ sort: "top", time: "week", subreddit: "r/rust" });
  });

  it("labels the query field by network and kind", () => {
    expect(monitorQueryKey(NET.shortpost, "search")).toBe("query_shortpost");
    expect(monitorQueryKey(NET.photofeed, "profile")).toBe("query_profile");
    expect(monitorQueryKey(NET.forums, "search")).toBe("query_clipfeed");
    expect(monitorQueryKey(NET.forums, "subreddit")).toBe("query_community");
    expect(monitorQueryKey(NET.pinboard, "search")).toBe("query_clipfeed");
    expect(monitorQueryKey(NET.codehub, "repositories")).toBe("query_gh_repositories");
    expect(monitorQueryKey(NET.codehub, "issues")).toBe("query_gh_issues");
    expect(monitorQueryKey(NET.codehub, "releases")).toBe("query_repo");
    expect(monitorQueryKey(NET.codehub, "repo_issues")).toBe("query_repo");
  });
});

describe("toolError", () => {
  it("reads the JSON body after the code", () => {
    expect(toolError('EMPTY_POOL: {"message":"nobody left","retryable":false,"next_step":"Check the post."}')).toEqual({
      code: "EMPTY_POOL",
      message: "nobody left",
      next_step: "Check the post.",
      retryable: false,
    });
  });

  it("keeps the plain codes of the network clients", () => {
    expect(toolError("RATE_LIMITED: X is limiting requests")).toEqual({ code: "RATE_LIMITED", message: "X is limiting requests", next_step: null, retryable: true });
    expect(toolError(new Error("boom")).code).toBe("UNKNOWN");
    expect(toolError("NOT_FOUND: {broken").message).toBe("{broken");
  });
});

describe("handles and lists", () => {
  it("uses the argument name of each network", () => {
    expect(whoKey(NET.shortpost)).toBe("handle");
    expect(whoKey(NET.photofeed)).toBe("username");
  });

  it("cleans @ and profile links", () => {
    expect(cleanHandle(" @ana ")).toBe("ana");
    expect(cleanHandle("https://www.tiktok.com/@ana?lang=en")).toBe("ana");
    expect(cleanHandle("https://x.com/ana/status/1")).toBe("ana");
  });

  it("splits comma and line separated values", () => {
    expect(splitList("a, b\n\n c ,")).toEqual(["a", "b", "c"]);
  });
});

describe("heat grid", () => {
  it("places cells by weekday and hour and finds the highest mean", () => {
    const grid = heatGrid([cell(0, 9, 10), cell(6, 23, 40), cell(9, 1, 999)]);
    expect(grid.max).toBe(40);
    expect(grid.rows).toHaveLength(7);
    expect(grid.rows[0][9]?.mean).toBe(10);
    expect(grid.rows[6][23]?.mean).toBe(40);
    expect(grid.rows[3][3]).toBeNull();
  });

  it("maps a mean to one of four levels", () => {
    expect(heatLevel(0, 40)).toBe(0);
    expect(heatLevel(1, 40)).toBe(1);
    expect(heatLevel(20, 40)).toBe(2);
    expect(heatLevel(40, 40)).toBe(4);
    expect(heatLevel(5, 0)).toBe(0);
  });
});

describe("sparkPoints", () => {
  it("scales the series into the box", () => {
    expect(sparkPoints([0, 5, 10], 100, 20)).toBe("0,20 50,10 100,0");
    expect(sparkPoints([4], 100, 20)).toBe("50,0");
    expect(sparkPoints([], 100, 20)).toBe("");
  });
});

describe("sortRows", () => {
  it("sorts numbers and text and keeps missing values last", () => {
    const rows = [{ k: "b", n: 2 }, { k: "a", n: null }, { k: "c", n: 9 }];
    expect(sortRows(rows, "n", false).map((r) => r.k)).toEqual(["c", "b", "a"]);
    expect(sortRows(rows, "n", true).map((r) => r.k)).toEqual(["b", "c", "a"]);
    expect(sortRows(rows, "k", true).map((r) => r.k)).toEqual(["a", "b", "c"]);
    expect(rows[0].k).toBe("b");
  });
});

describe("buildOptions", () => {
  const empty = { phrase: "", any: "", exclude: "", hashtags: "", from: "", to: "", mentions: "", lang: "", minLikes: "", minReposts: "", excludeReplies: false, excludeReposts: false };

  it("sends only what was filled in", () => {
    expect(buildOptions(empty)).toEqual({});
    expect(buildOptions({ ...empty, phrase: " open source ", any: "a, b", lang: "pt", minLikes: "10", minReposts: "0", excludeReplies: true })).toEqual({
      phrase: "open source",
      any: ["a", "b"],
      lang: "pt",
      min_likes: 10,
      exclude_replies: true,
    });
  });
});
