import { describe, expect, it } from "vitest";
import {
  areaPath,
  confirmKey,
  execSteps,
  flattenRows,
  groupByDay,
  groupItems,
  groupRows,
  kindModule,
  linePath,
  niceMax,
  pruneSelection,
  riskRows,
  rowOffsets,
  selectionState,
  setMany,
  splitByStatus,
  sumSelected,
  toggleOne,
  visibleRange,
  type SelectItem,
} from "./ui";

const items: SelectItem[] = [
  { id: "a", label: "A", group: "Browsers", bytes: 100 },
  { id: "b", label: "B", group: "Logs", bytes: 50 },
  { id: "c", label: "C", group: "Browsers", bytes: 25 },
  { id: "d", label: "D", group: "Logs", bytes: 5, disabled: true },
  { id: "e", label: "E", bytes: 1 },
];

describe("grouping", () => {
  it("keeps first-appearance order and sums bytes", () => {
    const g = groupItems(items);
    expect(g.map((x) => x.key)).toEqual(["Browsers", "Logs", ""]);
    expect(g[0]).toMatchObject({ count: 2, bytes: 125 });
    expect(g[1]).toMatchObject({ count: 2, bytes: 55 });
  });
});

describe("selection", () => {
  it("sums only selected, enabled items", () => {
    expect(sumSelected(items, new Set(["a", "d", "e"]))).toEqual({ count: 2, bytes: 101 });
  });
  it("computes the tri-state ignoring disabled rows", () => {
    const logs = items.filter((i) => i.group === "Logs");
    expect(selectionState(logs, new Set())).toBe("none");
    expect(selectionState(logs, new Set(["b"]))).toBe("all");
    const browsers = items.filter((i) => i.group === "Browsers");
    expect(selectionState(browsers, new Set(["a"]))).toBe("some");
  });
  it("selects and clears a group without touching others or disabled rows", () => {
    const logs = items.filter((i) => i.group === "Logs");
    const s = setMany(new Set(["a"]), logs, true);
    expect([...s].sort()).toEqual(["a", "b"]);
    expect([...setMany(s, logs, false)]).toEqual(["a"]);
  });
  it("toggles and prunes", () => {
    expect([...toggleOne(new Set(["a"]), "a")]).toEqual([]);
    expect([...toggleOne(new Set(), "a")]).toEqual(["a"]);
    expect([...pruneSelection(new Set(["a", "zz"]), items)]).toEqual(["a"]);
  });
});

describe("virtual rows", () => {
  it("adds headers only when groups have names or there are several", () => {
    const rows = flattenRows(groupItems(items));
    expect(rows.filter((r) => r.kind === "header")).toHaveLength(3);
    const single = flattenRows(groupItems([{ id: "x", label: "x" }]));
    expect(single.map((r) => r.kind)).toEqual(["item"]);
  });
  it("hides the rows of a collapsed group", () => {
    const rows = flattenRows(groupItems(items), new Set(["Browsers"]));
    expect(rows.map((r) => r.key)).toEqual(["h:Browsers", "h:Logs", "b", "d", "h:", "e"]);
  });
  it("renders only the window plus overscan for thousands of rows", () => {
    const heights = Array.from({ length: 5000 }, () => 40);
    const off = rowOffsets(heights);
    expect(off[5000]).toBe(200_000);
    const r = visibleRange(off, 40_000, 400, 5);
    expect(r.start).toBe(995);
    expect(r.end).toBe(1015);
    expect(r.padTop).toBe(995 * 40);
    expect(r.padTop + (r.end - r.start) * 40 + r.padBottom).toBe(200_000);
  });
  it("handles mixed heights and the edges", () => {
    const off = rowOffsets([30, 50, 50, 30, 50]);
    expect(visibleRange(off, 0, 60, 0)).toMatchObject({ start: 0, end: 2 });
    expect(visibleRange(off, 1000, 60, 0).end).toBe(5);
    expect(visibleRange(rowOffsets([]), 0, 100)).toEqual({ start: 0, end: 0, padTop: 0, padBottom: 0 });
  });
});

describe("plans", () => {
  it("orders risks and groups", () => {
    const byRisk = { destructive: { count: 1, bytes: 9 }, read: { count: 0, bytes: 0 }, reversible: { count: 2, bytes: 3 } };
    expect(riskRows(byRisk).map((r) => r.risk)).toEqual(["reversible", "destructive"]);
    const byGroup = { Logs: { count: 1, bytes: 10 }, Caches: { count: 4, bytes: 99 } };
    expect(groupRows(byGroup).map((g) => g.group)).toEqual(["Caches", "Logs"]);
  });
  it("picks the button label by kind", () => {
    expect(confirmKey("cleanup")).toBe("washmybelly.plan.confirm");
    expect(confirmKey("uninstall")).toBe("washmybelly.plan.confirm_uninstall");
    expect(confirmKey("tweaks")).toBe("washmybelly.plan.confirm_apply");
    expect(confirmKey("???")).toBe("washmybelly.plan.confirm_generic");
  });
  it("walks the execution steps", () => {
    expect(execSteps(null, false).map((s) => s.state)).toEqual(["active", "pending", "pending"]);
    expect(execSteps("run", true).map((s) => s.state)).toEqual(["done", "active", "pending", "pending"]);
    expect(execSteps("admin", true, true).map((s) => s.state)).toEqual(["done", "done", "failed", "pending"]);
    expect(execSteps("done", true).every((s) => s.state === "done")).toBe(true);
  });
});

describe("receipts", () => {
  const now = new Date(2026, 8, 27, 12);
  const at = (d: Date) => Math.floor(d.getTime() / 1000);
  it("groups by day, newest first", () => {
    const g = groupByDay(
      [
        { receipt_id: "old", created: at(new Date(2026, 8, 20, 9)) },
        { receipt_id: "t1", created: at(new Date(2026, 8, 27, 8)) },
        { receipt_id: "y", created: at(new Date(2026, 8, 26, 22)) },
        { receipt_id: "t2", created: at(new Date(2026, 8, 27, 11)) },
      ],
      now,
    );
    expect(g.map((x) => x.bucket)).toEqual(["today", "yesterday", "earlier"]);
    expect(g[0].items.map((r) => r.receipt_id)).toEqual(["t2", "t1"]);
  });
  it("splits items by status", () => {
    const s = splitByStatus([
      { status: "done" as const },
      { status: "failed" as const },
      { status: "skipped" as const },
      { status: "done" as const },
    ]);
    expect([s.failed.length, s.skipped.length, s.done.length]).toEqual([1, 1, 2]);
  });
});

describe("charts", () => {
  it("rounds axes to nice numbers", () => {
    expect(niceMax(0)).toBe(1);
    expect(niceMax(73)).toBe(100);
    expect(niceMax(180)).toBe(200);
    expect(niceMax(2300)).toBe(2500);
  });
  it("draws right-aligned series", () => {
    expect(linePath([], 100, 10, 1)).toBe("");
    expect(linePath([0, 1], 100, 10, 1)).toBe("M0.0,10.0 L100.0,0.0");
    expect(linePath([1], 100, 10, 1, 3)).toBe("M100.0,0.0");
    expect(areaPath([0, 1], 100, 10, 1)).toBe("M0.0,10.0 L100.0,0.0 L100.0,10 L0.0,10 Z");
  });
});

describe("kindModule", () => {
  it("maps plan kinds to the module that owns them", () => {
    expect(kindModule("cleanup")).toBe("clean");
    expect(kindModule("duplicates")).toBe("disk");
    expect(kindModule("hosts")).toBe("privacy");
    expect(kindModule("tweaks")).toBe("tweaks");
    expect(kindModule("mystery")).toBe("receipts");
  });
});
