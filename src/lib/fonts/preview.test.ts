import { describe, expect, it } from "vitest";
import { aliasFor, cdnUrl, fontStack, fromCatalog, fromInstalled, nearestWeight, previewSubsets, snapWeight, toggleCompare, variableUrl } from "./preview";
import { pluralKey } from "./text";
import { clampTo, presetDefaults, trackingLabel } from "./sample";
import type { CatalogFont, InstalledFamily } from "./types";

const geist: CatalogFont = { id: "geist", family: "Geist", category: "sans-serif", subsets: ["cyrillic", "latin", "latin-ext"], default_subset: "latin", weights: [700, 100, 400], styles: ["normal", "italic"], variable: true, license: "OFL-1.1", license_class: "open", source: "google" };

describe("preview urls", () => {
  it("builds Fontsource CDN urls", () => {
    expect(cdnUrl("geist", "latin", 400, "normal")).toBe("https://cdn.jsdelivr.net/fontsource/fonts/geist@latest/latin-400-normal.woff2");
    expect(cdnUrl("geist", "latin", 700, "italic", "ttf")).toBe("https://cdn.jsdelivr.net/fontsource/fonts/geist@latest/latin-700-italic.ttf");
    expect(variableUrl("inter", "latin", "normal")).toBe("https://cdn.jsdelivr.net/fontsource/fonts/inter:vf@latest/latin-wght-normal.woff2");
  });

  it("picks the subsets the current language needs", () => {
    expect(previewSubsets(geist, "en")).toEqual(["latin"]);
    expect(previewSubsets(geist, "ru")).toEqual(["latin", "cyrillic"]);
    expect(previewSubsets(geist, "pt")).toEqual(["latin", "latin-ext"]);
    expect(previewSubsets({ subsets: ["japanese"], default_subset: "latin" }, "en")).toEqual(["japanese"]);
  });
});

describe("weights", () => {
  it("snaps to the nearest weight, heavier on ties", () => {
    expect(nearestWeight([100, 400, 700], 520)).toBe(400);
    expect(nearestWeight([400, 600], 500)).toBe(600);
    expect(nearestWeight([], 500)).toBe(400);
  });

  it("keeps variable fonts continuous inside their range", () => {
    const f = fromCatalog(geist);
    expect(f.weights).toEqual([100, 400, 700]);
    expect(f.wghtRange).toEqual([100, 700]);
    expect(snapWeight(f, 555)).toBe(555);
    expect(snapWeight(f, 950)).toBe(700);
    expect(snapWeight({ ...f, wghtRange: null }, 520)).toBe(400);
  });
});

describe("font refs", () => {
  it("uses a private alias for web previews and the real name for installed fonts", () => {
    expect(fromCatalog(geist).cssFamily).toBe(aliasFor("geist"));
    const installed: InstalledFamily = { family: "Menlo", styles: [], weights: [400, 700], has_italic: true, is_variable: false, axes: [], is_monospace: true, files: [], user_installed: false };
    const ref = fromInstalled(installed, "");
    expect(ref.category).toBe("monospace");
    expect(fontStack(ref)).toBe('"Menlo", ui-monospace, monospace');
    expect(fontStack(null)).toBe("var(--font-body)");
  });

  it("limits the comparison to three fonts", () => {
    let list: string[] = [];
    for (const k of ["a", "b", "c", "d"]) list = toggleCompare(list, k);
    expect(list).toEqual(["a", "b", "c"]);
    expect(toggleCompare(list, "b")).toEqual(["a", "c"]);
  });
});

describe("sample controls", () => {
  it("clamps and labels values", () => {
    expect(clampTo(500, { min: 8, max: 160 })).toBe(160);
    expect(clampTo(Number.NaN, { min: 8, max: 160 })).toBe(8);
    expect(trackingLabel(0.0251)).toBe("+0.025em");
    expect(trackingLabel(-0.02)).toBe("-0.02em");
    expect(trackingLabel(0)).toBe("0em");
  });

  it("gives each preset a readable size", () => {
    expect(presetDefaults("pangram").size).toBeGreaterThan(presetDefaults("paragraph").size);
    expect(presetDefaults("code").lineHeight).toBe(1.6);
  });
});

describe("plural keys", () => {
  it("uses the _one key only for exactly one", () => {
    expect(pluralKey("fonts.results", 1)).toBe("fonts.results_one");
    expect(pluralKey("fonts.results", 0)).toBe("fonts.results");
    expect(pluralKey("fonts.results", 12)).toBe("fonts.results");
  });
});
