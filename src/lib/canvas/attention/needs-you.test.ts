import { describe, expect, it } from "vitest";
import { orderInbox, reasonKey, splitVisible, type InboxItem } from "./needs-you";

const item = (nodeId: string, reason: string, sinceMs = 0): InboxItem => ({
  nodeId,
  name: nodeId,
  workspaceId: "w",
  floorId: "ground",
  reason,
  sinceMs,
});

describe("needs-you list", () => {
  it("puts what blocks an agent before what only waits for review", () => {
    const out = orderInbox([item("done", "review", 1), item("perm", "permission", 9), item("gate", "trust", 5)]);
    expect(out.map((i) => i.nodeId)).toEqual(["gate", "perm", "done"]);
  });

  it("puts the longest wait first inside a group", () => {
    const out = orderInbox([item("new", "blocked", 50), item("old", "stuck", 10)]);
    expect(out.map((i) => i.nodeId)).toEqual(["old", "new"]);
  });

  it("drops rows without a node or a workspace and repeated nodes", () => {
    const broken = { ...item("x", "blocked"), workspaceId: "" };
    const out = orderInbox([broken, item("a", "blocked"), item("a", "review")]);
    expect(out.map((i) => i.nodeId)).toEqual(["a"]);
  });

  it("names every known reason and falls back for an unknown one", () => {
    expect(reasonKey("permission")).toBe("permission");
    expect(reasonKey("chrome_prompt")).toBe("chrome_prompt");
    expect(reasonKey("something_new")).toBe("other");
  });

  it("shows at most five rows and counts the rest", () => {
    const many = Array.from({ length: 8 }, (_, i) => item(`n${i}`, "blocked", i));
    const { shown, more } = splitVisible(many);
    expect(shown.length).toBe(5);
    expect(more).toBe(3);
  });
});
