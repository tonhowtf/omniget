import { describe, expect, it } from "vitest";
import { categoryParent, categoryTitle, counts, filterItems, groupByCategory, isActionable, presetIds, pruneSelection, reasonKey, type PrivacyItem } from "./privacy";

function item(p: Partial<PrivacyItem>): PrivacyItem {
  return {
    id: p.id ?? `privacy:${p.name}`,
    category: "Telemetry",
    name: p.name ?? "x",
    recommend: null,
    risk: "reversible",
    state: "not_applied",
    admin: false,
    restart: false,
    ...p,
  };
}

const LIST = [
  item({ id: "s1", recommend: "standard" }),
  item({ id: "s2", recommend: "standard", state: "applied" }),
  item({ id: "t1", recommend: "strict" }),
  item({ id: "t2", recommend: "strict", state: "unknown", reason: "needs_admin" }),
  item({ id: "x1", recommend: "standard", risk: "extreme", category: "Privacy over security" }),
  item({ id: "c1", recommend: "standard", code_only: true, code: "defaults write x y" }),
  item({ id: "a1", category: "Cleanup / Browsers" }),
];

describe("privacy catalog", () => {
  it("presets never take extreme, applied, unknown or code-only items", () => {
    expect(presetIds(LIST, "standard")).toEqual(["s1"]);
    expect(presetIds(LIST, "strict")).toEqual(["s1", "t1"]);
  });

  it("hides extreme items until asked and filters recommended", () => {
    expect(filterItems(LIST, {}).some((i) => i.id === "x1")).toBe(false);
    expect(filterItems(LIST, { showExtreme: true }).some((i) => i.id === "x1")).toBe(true);
    expect(filterItems(LIST, { recommendedOnly: true }).map((i) => i.id)).toEqual(["s1", "s2", "t1", "t2", "c1"]);
    expect(filterItems(LIST, { query: "browsers" }).map((i) => i.id)).toEqual(["a1"]);
  });

  it("groups in catalog order with counts", () => {
    const g = groupByCategory(filterItems(LIST, {}));
    expect(g.map((x) => x.category)).toEqual(["Telemetry", "Cleanup / Browsers"]);
    expect(g[0].applied).toBe(1);
    expect(g[0].pending).toBe(2);
  });

  it("knows what can be ticked and prunes the rest", () => {
    expect(isActionable(LIST[3])).toBe(false);
    expect(isActionable(LIST[5])).toBe(false);
    expect([...pruneSelection(new Set(["s1", "s2", "gone"]), LIST)]).toEqual(["s1"]);
  });

  it("counts and titles", () => {
    expect(counts(LIST)).toEqual({ applied: 1, pending: 3, unknown: 1, recommendedPending: 2 });
    expect(categoryTitle("Cleanup / Browsers")).toBe("Browsers");
    expect(categoryParent("Cleanup / Browsers")).toBe("Cleanup");
    expect(categoryParent("Telemetry")).toBeNull();
    expect(reasonKey("needs_admin")).toBe("washmybelly.privacy.reason.needs_admin");
    expect(reasonKey("Some text")).toBeNull();
  });
});
