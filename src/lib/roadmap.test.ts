import { describe, expect, it } from "vitest";
import { roadmapColumns, roadmapErrorText, type Roadmap, type RoadmapTicket } from "./roadmap";

function ticket(key: string, status: string, updated: string): RoadmapTicket {
  return { key, title: key, status, type: "bug", priority: "normal", area: "all", created: updated, updated, url: `https://getomniget.com/contact#t-${key}`, issue: null };
}

const base: Roadmap = {
  updated: "2026-11-19",
  next_version: "1.0.1",
  board: "https://getomniget.com/contact#roadmap",
  columns: ["backlog", "todo", "progress", "done", "wontfix"],
  items: [ticket("A", "done", "2026-11-01"), ticket("B", "progress", "2026-11-02"), ticket("C", "progress", "2026-11-10"), ticket("D", "wontfix", "2026-10-01")],
};

describe("roadmapColumns", () => {
  it("puts work in progress first and drops empty columns", () => {
    expect(roadmapColumns(base).map((c) => c.id)).toEqual(["progress", "done", "wontfix"]);
  });

  it("shows the most recently updated ticket first", () => {
    expect(roadmapColumns(base)[0].items.map((t) => t.key)).toEqual(["C", "B"]);
  });

  it("keeps a column the app does not know, after the known ones", () => {
    const r = { ...base, columns: [...base.columns, "review"], items: [...base.items, ticket("E", "review", "2026-11-11")] };
    expect(roadmapColumns(r).map((c) => c.id)).toEqual(["progress", "done", "wontfix", "review"]);
  });
});

describe("roadmapErrorText", () => {
  it("reads the message of a structured error", () => {
    expect(roadmapErrorText('{"code":"ROADMAP_HTTP","message":"the roadmap answered HTTP 503"}')).toBe("the roadmap answered HTTP 503");
  });

  it("keeps plain text as it is", () => {
    expect(roadmapErrorText(new Error("offline"))).toBe("offline");
  });
});
