import { describe, expect, it } from "vitest";
import en from "$lib/i18n/en.json";
import { CATEGORIES, SUPERPOWERS, TAGS } from "./catalog";

function lookup(key: string): unknown {
  return key.split(".").reduce<unknown>((node, part) => (node as Record<string, unknown> | undefined)?.[part], en);
}

describe("superpowers catalog", () => {
  it("has unique ids and routes", () => {
    expect(new Set(SUPERPOWERS.map((entry) => entry.id)).size).toBe(SUPERPOWERS.length);
    expect(new Set(SUPERPOWERS.map((entry) => entry.href)).size).toBe(SUPERPOWERS.length);
  });

  it("gives every entry a title, a description and a known category", () => {
    const categories = new Set(CATEGORIES.map((category) => category.id));
    for (const entry of SUPERPOWERS) {
      expect(entry.title ?? lookup(entry.titleKey ?? ""), entry.id).toEqual(expect.any(String));
      expect(lookup(entry.descKey), entry.id).toEqual(expect.any(String));
      expect(categories.has(entry.category), entry.id).toBe(true);
    }
  });

  it("has no empty category and labels every category and tag", () => {
    for (const category of CATEGORIES) {
      expect(SUPERPOWERS.some((entry) => entry.category === category.id), category.id).toBe(true);
      expect(lookup(category.labelKey), category.id).toEqual(expect.any(String));
    }
    for (const tag of Object.values(TAGS)) {
      expect(tag.label ?? lookup(tag.labelKey ?? "")).toEqual(expect.any(String));
    }
  });

  it("groups the entries as the hub shows them", () => {
    const ids = (category: string) => SUPERPOWERS.filter((entry) => entry.category === category).map((entry) => entry.id);
    expect(ids("dev")).toEqual(["devtools", "pack", "scrape", "api"]);
    expect(ids("download")).toEqual(["global", "telegram"]);
    expect(SUPERPOWERS.find((entry) => entry.id === "global")?.tags).toContain("browser");
    expect(SUPERPOWERS.find((entry) => entry.id === "scrape")?.icon).toBe("scraper");
    expect(SUPERPOWERS.find((entry) => entry.id === "scrape")?.tags).toEqual(["mcp"]);
  });
});
