import { describe, expect, it } from "vitest";
import { CORE_NAV_ITEMS } from "./nav-config";
import { SUPERPOWERS } from "./superpowers/catalog";
import { paletteNavEntries } from "./command-palette-nav";

describe("paletteNavEntries", () => {
  it("lists every core page and every superpower once", () => {
    const entries = paletteNavEntries(CORE_NAV_ITEMS, SUPERPOWERS, {});
    const hrefs = entries.map((entry) => entry.href);
    const coreHrefs = new Set(CORE_NAV_ITEMS.map((item) => item.href));
    expect(hrefs).toEqual([
      ...CORE_NAV_ITEMS.map((item) => item.href),
      ...SUPERPOWERS.filter((entry) => !coreHrefs.has(entry.href)).map((entry) => entry.href),
    ]);
    expect(hrefs).toEqual(expect.arrayContaining(["/world", "/telegram", "/global", "/league"]));
    expect(new Set(entries.map((entry) => entry.id)).size).toBe(entries.length);
  });

  it("leaves out superpowers that are turned off", () => {
    const hrefs = paletteNavEntries(CORE_NAV_ITEMS, SUPERPOWERS, { telegram: false, league: true }).map((entry) => entry.href);
    expect(hrefs).not.toContain("/telegram");
    expect(hrefs).toContain("/league");
  });

  it("only lists the core pages it is given", () => {
    const core = CORE_NAV_ITEMS.filter((item) => item.href !== "/world");
    expect(paletteNavEntries(core, [], {}).map((entry) => entry.id)).toEqual(["nav-home", "nav-downloads", "nav-global", "nav-superpowers", "nav-settings", "nav-community"]);
  });

  it("carries the label source and the keywords", () => {
    const entries = paletteNavEntries(CORE_NAV_ITEMS, SUPERPOWERS, {});
    expect(entries.find((entry) => entry.id === "nav-telegram")).toMatchObject({ label: "Telegram", href: "/telegram" });
    expect(entries.find((entry) => entry.id === "nav-music")).toMatchObject({ labelKey: "music.title" });
    expect(entries.find((entry) => entry.id === "nav-downloads")?.keywords).toContain("queue");
  });
});
