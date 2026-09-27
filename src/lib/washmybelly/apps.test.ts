import { describe, expect, it } from "vitest";
import {
  byConfidence,
  classify,
  defaultLeftovers,
  filterApps,
  isInstalled,
  lastUsed,
  modesFor,
  monogram,
  planBytes,
  planIds,
  removedAppId,
  sortApps,
  sourceInfo,
  sourcesPresent,
} from "./apps";
import { APPS, FIGMA_LEFTOVERS, NOW } from "./apps.fixtures";

const names = (xs: { name: string }[]) => xs.map((x) => x.name);

describe("sources", () => {
  it("lists only the sources present, most apps first", () => {
    expect(sourcesPresent(APPS.apps)).toEqual([
      { source: "app_bundle", count: 4 },
      { source: "homebrew_cask", count: 1 },
      { source: "pkg", count: 1 },
    ]);
  });

  it("has a label key, fallback and colours for known and unknown sources", () => {
    expect(sourceInfo("homebrew_cask")).toMatchObject({ key: "washmybelly.uninstall.source.homebrew_cask", fallback: "Homebrew" });
    const odd = sourceInfo("nix_profile");
    expect(odd.fallback).toBe("nix profile");
    expect(odd.from).toMatch(/^#/);
  });

  it("makes a monogram", () => {
    expect(monogram("Visual Studio Code")).toBe("VS");
    expect(monogram("zoom.us")).toBe("ZU");
    expect(monogram("VLC")).toBe("V");
    expect(monogram("  ")).toBe("?");
  });
});

describe("filterApps", () => {
  it("searches name, publisher and id, ignoring case and accents", () => {
    expect(names(filterApps(APPS.apps, { query: "micro" }))).toEqual(["Visual Studio Code"]);
    expect(names(filterApps(APPS.apps, { query: "emile editeur" }))).toEqual(["Émile Éditeur"]);
    expect(names(filterApps(APPS.apps, { query: "videolan" }))).toEqual(["VLC"]);
    expect(filterApps(APPS.apps, { query: "" })).toHaveLength(6);
  });

  it("filters by source chips; empty set means all", () => {
    expect(names(filterApps(APPS.apps, { sources: new Set(["pkg", "homebrew_cask"]) }))).toEqual(["Figma", "zoom.us"]);
    expect(filterApps(APPS.apps, { sources: new Set() })).toHaveLength(6);
    expect(names(filterApps(APPS.apps, { sources: new Set(["app_bundle"]), query: "v" }))).toEqual(["Visual Studio Code", "VLC"]);
  });
});

describe("sortApps", () => {
  it("by name", () => {
    expect(names(sortApps(APPS.apps, "name"))).toEqual(["Émile Éditeur", "Figma", "Numbers", "Visual Studio Code", "VLC", "zoom.us"]);
  });
  it("by size, unknown last", () => {
    expect(names(sortApps(APPS.apps, "size"))).toEqual(["Numbers", "Visual Studio Code", "Figma", "VLC", "Émile Éditeur", "zoom.us"]);
  });
  it("by last used: longest unused first, never seen last", () => {
    expect(names(sortApps(APPS.apps, "last_used"))).toEqual(["VLC", "Figma", "Émile Éditeur", "Visual Studio Code", "Numbers", "zoom.us"]);
  });
  it("does not mutate the input", () => {
    const before = names(APPS.apps);
    sortApps(APPS.apps, "size");
    expect(names(APPS.apps)).toEqual(before);
  });
});

describe("lastUsed", () => {
  it("picks the unit a person would say", () => {
    expect(lastUsed(NOW - 3_600, NOW)).toEqual({ key: "washmybelly.uninstall.used.today", count: 0, stale: false });
    expect(lastUsed(NOW - 12 * 86_400, NOW)).toEqual({ key: "washmybelly.uninstall.used.days", count: 12, stale: false });
    expect(lastUsed(NOW - 240 * 86_400, NOW)).toEqual({ key: "washmybelly.uninstall.used.months", count: 8, stale: true });
    expect(lastUsed(NOW - 800 * 86_400, NOW)).toEqual({ key: "washmybelly.uninstall.used.years", count: 2, stale: true });
  });
  it("uses a singular key for one", () => {
    expect(lastUsed(NOW - 86_400, NOW)?.key).toBe("washmybelly.uninstall.used.days_one");
    expect(lastUsed(NOW - 40 * 86_400, NOW)?.key).toBe("washmybelly.uninstall.used.months_one");
    expect(lastUsed(NOW - 400 * 86_400, NOW)?.key).toBe("washmybelly.uninstall.used.years_one");
  });
  it("marks 90 days and more as stale", () => {
    expect(lastUsed(NOW - 89 * 86_400, NOW)?.stale).toBe(false);
    expect(lastUsed(NOW - 90 * 86_400, NOW)?.stale).toBe(true);
  });
  it("returns null when unknown and clamps the future to today", () => {
    expect(lastUsed(null, NOW)).toBeNull();
    expect(lastUsed(0, NOW)).toBeNull();
    expect(lastUsed(NOW + 500, NOW)?.key).toBe("washmybelly.uninstall.used.today");
  });
});

describe("leftovers", () => {
  const c = classify(FIGMA_LEFTOVERS);

  it("splits app, zap and leftovers with their evidence", () => {
    expect(c.app.map((i) => i.id)).toEqual(["uninstall:app:/Applications/Figma.app"]);
    expect(c.zap.map((i) => i.id)).toEqual(["uninstall:zap:figma"]);
    expect(c.leftovers.map((l) => [l.group, l.confidence])).toEqual([
      ["support", "high"],
      ["preferences", "high"],
      ["caches", "medium"],
      ["logs", "low"],
    ]);
    expect(c.leftovers[2].evidence).toBe("Team ID T8RA8NE3B7");
  });

  it("matches by id tail when detail is missing, and treats unknown items as low", () => {
    const r = classify({
      items: [
        { id: "uninstall:x:/tmp/a", label: "a", detail: null, group: "support", bytes: 1, risk: "reversible", admin: false },
        { id: "uninstall:y:/tmp/b", label: "b", detail: "/tmp/b", group: "support", bytes: 2, risk: "reversible", admin: false },
      ],
      extra: { leftovers: [{ path: "/tmp/a", bytes: 1, confidence: "high", evidence: "bundle id", kind: "file" }] },
    });
    expect(r.leftovers.map((l) => l.confidence)).toEqual(["high", "low"]);
    expect(r.app).toEqual([]);
  });

  it("groups by confidence high → low and ticks high and medium only", () => {
    const g = byConfidence(c.leftovers);
    expect(g.map((x) => [x.confidence, x.items.length, x.bytes])).toEqual([
      ["high", 2, 210_004_000],
      ["medium", 1, 90_000],
      ["low", 1, 1_200],
    ]);
    const sel = defaultLeftovers(c.leftovers);
    expect(sel.has("uninstall:name:/Users/me/Library/Logs/figma-helper.log")).toBe(false);
    expect(sel.size).toBe(3);
  });

  it("offers uninstall, zap and reset, and builds the plan ids per mode", () => {
    expect(modesFor(c, true)).toEqual(["uninstall", "zap", "reset"]);
    expect(modesFor(c, false)).toEqual(["uninstall", "zap"]);
    const sel = defaultLeftovers(c.leftovers);
    expect(planIds(c, "uninstall", sel)[0]).toBe("uninstall:app:/Applications/Figma.app");
    expect(planIds(c, "uninstall", sel)).toHaveLength(4);
    expect(planIds(c, "zap", sel)).toEqual(["uninstall:zap:figma", ...[...sel]]);
    expect(planIds(c, "reset", sel).some((id) => id.includes("/Applications/"))).toBe(false);
    expect(planBytes(c, "reset", sel)).toBe(210_094_000);
    expect(planBytes(c, "uninstall", new Set())).toBe(480_000_000);
  });

  it("an already removed app offers only its leftovers", () => {
    const r = classify({ items: FIGMA_LEFTOVERS.items.slice(2), extra: FIGMA_LEFTOVERS.extra });
    expect(modesFor(r, false)).toEqual([]);
    expect(planIds(r, "reset", defaultLeftovers(r.leftovers))).toHaveLength(3);
  });
});

describe("removed apps", () => {
  it("builds an id from free text", () => {
    expect(removedAppId("  Sketch ")).toBe("removed:Sketch");
    expect(removedAppId("com.bohemiancoding.sketch3")).toBe("removed:com.bohemiancoding.sketch3");
    expect(removedAppId("app:com.x.y")).toBe("app:com.x.y");
    expect(removedAppId("   ")).toBeNull();
  });
  it("knows whether the app is still there", () => {
    expect(isInstalled(APPS.apps[0])).toBe(true);
    expect(isInstalled({ id: "removed:Sketch", name: "Sketch", source: "removed", removable: false })).toBe(false);
    expect(isInstalled({ id: "app:com.x", name: "com.x", source: "app", removable: false, reason: "not installed anymore" })).toBe(false);
    expect(isInstalled(APPS.apps[2])).toBe(true);
  });
});
