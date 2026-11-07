import { describe, expect, it } from "vitest";
import { categoryOf, groupSets, installCommand, licenseKind, orderRecommended, relatedQuery, splitId } from "./sets";
import type { IconSet } from "./types";

const set = (prefix: string, category: string | null, total = 10): IconSet => ({
  prefix,
  name: prefix.toUpperCase(),
  total,
  author: null,
  author_url: null,
  license: null,
  license_spdx: "MIT",
  license_url: null,
  category,
  palette: false,
  height: 24,
  samples: [],
  tags: [],
});

describe("licenses", () => {
  it("classifies licenses", () => {
    expect(licenseKind("MIT")).toBe("open");
    expect(licenseKind("ISC")).toBe("open");
    expect(licenseKind("CC-BY-4.0")).toBe("attribution");
    expect(licenseKind("CC-BY-SA-4.0")).toBe("attribution");
    expect(licenseKind("CC-BY-NC-4.0")).toBe("restricted");
    expect(licenseKind("GPL-3.0-or-later")).toBe("copyleft");
    expect(licenseKind(null)).toBe("unknown");
  });
});

describe("categories", () => {
  it("maps iconify categories to our names", () => {
    expect(categoryOf("UI 24px")).toBe("interface");
    expect(categoryOf("Logos")).toBe("brands");
    expect(categoryOf("Something new")).toBe("other");
    expect(categoryOf(null)).toBe("other");
  });

  it("groups in our order, biggest first, with filter", () => {
    const groups = groupSets([set("a", "Logos", 5), set("b", "UI 24px", 1), set("c", "UI 24px", 9)]);
    expect(groups.map((g) => g.key)).toEqual(["interface", "brands"]);
    expect(groups[0].sets.map((s) => s.prefix)).toEqual(["c", "b"]);
    expect(groupSets([set("lucide", "UI 24px")], "luc")).toHaveLength(1);
    expect(groupSets([set("lucide", "UI 24px")], "zzz")).toHaveLength(0);
  });

  it("orders recommended sets", () => {
    expect(orderRecommended([set("ri", null), set("x", null), set("lucide", null)]).map((s) => s.prefix)).toEqual(["lucide", "ri"]);
  });
});

describe("helpers", () => {
  it("install commands", () => {
    expect(installCommand("lucide")).toBe("npm install lucide-svelte");
    expect(installCommand("ph")).toBe("npm install phosphor-svelte");
    expect(installCommand("nope")).toBeNull();
  });

  it("splits ids and builds related queries", () => {
    expect(splitId("lucide:arrow-left")).toEqual({ prefix: "lucide", name: "arrow-left" });
    expect(splitId("bad")).toBeNull();
    expect(relatedQuery("arrow-left")).toBe("arrow");
    expect(relatedQuery("ph:x".split(":")[1])).toBe("x");
    expect(relatedQuery("24-solid-bell")).toBe("bell");
  });
});
