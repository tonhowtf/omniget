import { describe, expect, it } from "vitest";
import { hasFullFiles, pickSubset, plannedCount, subsetKey, subsetOptions } from "./install";
import type { FontFile } from "./types";

const file = (subset: string, weight: number, style = "normal", ttf = true): FontFile => ({ weight, style, subset, ttf: ttf ? `https://x/${subset}-${weight}-${style}.ttf` : null, woff2: null });

const jp = {
  subsets: ["cyrillic", "japanese", "latin", "latin-ext", "vietnamese"],
  default_subset: "japanese",
  source: "other",
  files: [file("0", 400), file("1", 400), file("latin", 400), file("latin", 700), file("cyrillic", 400), file("vietnamese", 400, "normal", false)],
};

describe("install planning", () => {
  it("offers only scripts that have TrueType files", () => {
    expect(subsetOptions(jp)).toEqual(["cyrillic", "latin"]);
    expect(subsetOptions(null)).toEqual([]);
  });

  it("falls back to latin when the default script has no files", () => {
    expect(pickSubset(jp)).toBe("latin");
    expect(pickSubset({ ...jp, default_subset: "cyrillic" })).toBe("cyrillic");
    expect(pickSubset({ ...jp, files: [file("cyrillic", 400)] })).toBe("cyrillic");
  });

  it("counts one file per style for full Google files", () => {
    expect(plannedCount(jp, "latin")).toBe(2);
    expect(plannedCount(jp, "japanese")).toBe(0);
    const google = { ...jp, source: "google", files: [file("latin", 400), file("cyrillic", 400), file("latin", 400, "italic"), file("latin", 700)] };
    expect(hasFullFiles(google)).toBe(true);
    expect(plannedCount(google, "anything")).toBe(3);
    expect(hasFullFiles(jp)).toBe(false);
  });

  it("names known scripts and leaves unknown ones raw", () => {
    expect(subsetKey("latin-ext")).toBe("fonts.subset_latin_ext");
    expect(subsetKey("chinese-simplified")).toBe("fonts.subset_chinese_simplified");
    expect(subsetKey("17")).toBeNull();
  });
});
