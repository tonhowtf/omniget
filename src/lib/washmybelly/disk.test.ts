import { describe, expect, it } from "vitest";
import { arcPath, crumbs, labelTier, squarify, sunburst, volumeSegments, type UsageNode } from "./disk";

const n = (name: string, bytes: number, children?: UsageNode[], kind: UsageNode["kind"] = "dir"): UsageNode => ({ name, bytes, files: 1, kind, children });

describe("squarify", () => {
  it("fills the box with areas proportional to bytes", () => {
    const tiles = squarify([n("a", 60), n("b", 30), n("c", 10), n("zero", 0)], 0, 0, 100, 50);
    expect(tiles).toHaveLength(3);
    const area = tiles.reduce((s, t) => s + t.w * t.h, 0);
    expect(area).toBeCloseTo(5000, 3);
    const a = tiles.find((t) => t.node.name === "a")!;
    expect(a.w * a.h).toBeCloseTo(3000, 3);
    for (const t of tiles) {
      expect(t.x).toBeGreaterThanOrEqual(-1e-9);
      expect(t.y).toBeGreaterThanOrEqual(-1e-9);
      expect(t.x + t.w).toBeLessThanOrEqual(100 + 1e-6);
      expect(t.y + t.h).toBeLessThanOrEqual(50 + 1e-6);
    }
  });

  it("keeps aspect ratios sane", () => {
    const tiles = squarify(Array.from({ length: 12 }, (_, i) => n(`f${i}`, 12 - i)), 0, 0, 600, 400);
    const worst = Math.max(...tiles.map((t) => Math.max(t.w / t.h, t.h / t.w)));
    expect(worst).toBeLessThan(4);
  });

  it("empty input gives nothing", () => {
    expect(squarify([], 0, 0, 10, 10)).toEqual([]);
    expect(squarify([n("a", 5)], 0, 0, 0, 10)).toEqual([]);
  });
});

describe("sunburst", () => {
  it("rings per depth and angles by bytes of the root", () => {
    const root = n("root", 100, [n("a", 50, [n("a1", 25), n("a2", 25)]), n("b", 40), n("other", 10, undefined, "other")]);
    const arcs = sunburst(root, 2, 100, 20);
    expect(arcs.filter((a) => a.depth === 1)).toHaveLength(3);
    expect(arcs.filter((a) => a.depth === 2)).toHaveLength(2);
    const a = arcs.find((x) => x.node.name === "a")!;
    expect(a.a1 - a.a0).toBeCloseTo(Math.PI, 6);
    const a1 = arcs.find((x) => x.node.name === "a1")!;
    expect(a1.color).toBe(a.color);
    expect(a1.r0).toBeCloseTo(a.r1, 6);
    expect(arcs.find((x) => x.node.name === "other")!.color).toBe("#8E8E93");
  });

  it("arc path is closed", () => {
    expect(arcPath(0, Math.PI / 2, 10, 20)).toMatch(/^M.*Z$/);
    expect(arcPath(0, Math.PI * 2, 10, 20)).toContain("A20,20 0 1 1");
  });
});

describe("volume bar and crumbs", () => {
  it("purgeable is carved out of used", () => {
    const s = volumeSegments({ total: 100, free: 30, purgeable: 10 });
    expect(s.map((x) => [x.key, x.bytes])).toEqual([["used", 60], ["purgeable", 10], ["free", 30]]);
    expect(volumeSegments({ total: 100, free: 40, purgeable: null }).map((x) => x.key)).toEqual(["used", "free"]);
  });

  it("crumbs from root to path", () => {
    expect(crumbs("/Users/me", "/Users/me/Library/Caches", "me").map((c) => c.path)).toEqual(["/Users/me", "/Users/me/Library", "/Users/me/Library/Caches"]);
    expect(crumbs("C:\\Users\\me", "C:\\Users\\me\\AppData", "me").map((c) => c.path)).toEqual(["C:\\Users\\me", "C:\\Users\\me\\AppData"]);
    expect(crumbs("/", "/opt", "/").map((c) => c.path)).toEqual(["/", "/opt"]);
  });

  it("label tiers", () => {
    expect(labelTier(100, 50)).toBe("full");
    expect(labelTier(50, 22)).toBe("name");
    expect(labelTier(10, 10)).toBe("none");
  });
});
