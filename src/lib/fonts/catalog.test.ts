import { describe, expect, it } from "vitest";
import { CURATED_IDS, categoryIndex, categoryKey, curatedFallback, filterCatalog, filterInstalled, hiddenCount, installedCategory, installedNames, isInstalled, mergeCatalog, normalize } from "./catalog";
import { classifyLicense, licenseName, licenseUrl } from "./license";
import type { CatalogFont, InstalledFamily } from "./types";

function font(id: string, family: string, category = "sans-serif", license = "OFL-1.1"): CatalogFont {
  return { id, family, category, subsets: ["latin"], default_subset: "latin", weights: [400], styles: ["normal"], variable: false, license, license_class: classifyLicense(license), source: "google" };
}

function fam(family: string, mono = false): InstalledFamily {
  return { family, styles: [], weights: [400], has_italic: false, is_variable: false, axes: [], is_monospace: mono, files: [], user_installed: false };
}

const list = [font("roboto", "Roboto"), font("inter", "Inter"), font("inter-tight", "Inter Tight"), font("ubuntu", "Ubuntu", "sans-serif", "UFL-1.0"), font("geist-mono", "Geist Mono", "monospace"), font("lora", "Lora", "serif"), font("winter", "Winter Sans")];

describe("license classification", () => {
  it("accepts OFL, Apache and MIT only", () => {
    expect(classifyLicense("OFL-1.1")).toBe("open");
    expect(classifyLicense("Apache-2.0")).toBe("open");
    expect(classifyLicense("mit")).toBe("open");
    expect(classifyLicense("UFL-1.0")).toBe("other");
    expect(classifyLicense("CC0-1.0")).toBe("other");
    expect(classifyLicense("")).toBe("other");
    expect(classifyLicense(undefined)).toBe("other");
    expect(classifyLicense("Mitra")).toBe("other");
  });

  it("names licenses", () => {
    expect(licenseName("OFL-1.1")).toBe("SIL Open Font License 1.1");
    expect(licenseName("Apache-2.0")).toBe("Apache License 2.0");
    expect(licenseName("Weird-1")).toBe("Weird-1");
    expect(licenseUrl("mit")).toBe("https://opensource.org/license/mit");
    expect(licenseUrl("Weird")).toBeNull();
  });
});

describe("catalog search", () => {
  it("normalizes accents and punctuation", () => {
    expect(normalize("  Fïra—Code ")).toBe("fira code");
  });

  it("ranks exact, prefix, word, then substring", () => {
    const out = filterCatalog(list, { query: "inter", category: null, includeOther: false }).map((f) => f.id);
    expect(out).toEqual(["inter", "inter-tight", "winter"]);
  });

  it("hides other licenses unless asked", () => {
    const filter = { query: "", category: null, includeOther: false };
    expect(filterCatalog(list, filter).some((f) => f.id === "ubuntu")).toBe(false);
    expect(hiddenCount(list, filter)).toBe(1);
    expect(filterCatalog(list, { ...filter, includeOther: true }).some((f) => f.id === "ubuntu")).toBe(true);
    expect(hiddenCount(list, { ...filter, includeOther: true })).toBe(0);
  });

  it("filters by category", () => {
    expect(filterCatalog(list, { query: "", category: "serif", includeOther: true }).map((f) => f.id)).toEqual(["lora"]);
  });

  it("puts the curated fonts first when there is no query", () => {
    const out = filterCatalog(list, { query: "", category: null, includeOther: false }).map((f) => f.id);
    expect(out.slice(0, 2)).toEqual(["geist-mono", "inter"]);
  });

  it("keeps the curated list when the API is out of reach", () => {
    expect(curatedFallback().map((f) => f.id)).toEqual(CURATED_IDS);
    expect(curatedFallback().every((f) => f.license_class === "open")).toBe(true);
    const merged = mergeCatalog([font("inter", "Inter")]);
    expect(merged.filter((f) => f.id === "inter")).toHaveLength(1);
    expect(merged.length).toBe(CURATED_IDS.length);
  });

  it("maps categories to label keys", () => {
    expect(categoryKey("monospace")).toBe("fonts.cat_mono");
    expect(categoryKey("handwriting")).toBe("fonts.cat_script");
    expect(categoryKey("weird")).toBe("fonts.cat_other");
  });
});

describe("installed families", () => {
  const installed = [fam("Inter"), fam("Menlo", true), fam("Lora"), fam("Helvetica Neue")];

  it("knows which catalog fonts are already installed", () => {
    const names = installedNames(installed);
    expect(isInstalled(font("inter", "INTER"), names)).toBe(true);
    expect(isInstalled(font("roboto", "Roboto"), names)).toBe(false);
  });

  it("guesses a category from the catalog or from monospace", () => {
    expect(installedCategory(fam("Lora"), categoryIndex(list))).toBe("serif");
    expect(installedCategory(fam("Menlo", true), categoryIndex(list))).toBe("monospace");
    expect(installedCategory(fam("Helvetica Neue"), categoryIndex(list))).toBe("");
  });

  it("filters by name and category", () => {
    expect(filterInstalled(installed, "neue", null, categoryIndex(list)).map((f) => f.family)).toEqual(["Helvetica Neue"]);
    expect(filterInstalled(installed, "", "monospace", categoryIndex(list)).map((f) => f.family)).toEqual(["Menlo"]);
    expect(filterInstalled(installed, "", "serif", categoryIndex(list)).map((f) => f.family)).toEqual(["Lora"]);
  });
});
