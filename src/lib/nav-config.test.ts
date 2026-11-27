import { describe, expect, it } from "vitest";
import { activeNavHref, CORE_NAV_ITEMS, NAV_ALIASES, type NavItem, visibleCoreNav } from "./nav-config";

const pinned: NavItem = { href: "/music", labelKey: "music.title", icon: "music", group: "app", pinned: true };

describe("activeNavHref", () => {
  it("matches the home route only exactly", () => {
    expect(activeNavHref(CORE_NAV_ITEMS, "/")).toBe("/");
    expect(activeNavHref(CORE_NAV_ITEMS, "/downloads")).toBe("/downloads");
    expect(activeNavHref(CORE_NAV_ITEMS, "/global")).toBe("/global");
  });

  it("matches nested routes but not routes that only share a prefix", () => {
    expect(activeNavHref(CORE_NAV_ITEMS, "/settings/about")).toBe("/settings");
    expect(activeNavHref(CORE_NAV_ITEMS, "/settingsx")).toBeNull();
  });

  it("lights Superpowers on a superpower route that is not pinned", () => {
    expect(activeNavHref(CORE_NAV_ITEMS, "/music")).toBe("/superpowers");
    expect(activeNavHref(CORE_NAV_ITEMS, "/shortpost")).toBe("/superpowers");
  });

  it("lights the pinned item instead of Superpowers on its own route", () => {
    expect(activeNavHref([...CORE_NAV_ITEMS, pinned], "/music")).toBe("/music");
    expect(activeNavHref([...CORE_NAV_ITEMS, pinned], "/music/album")).toBe("/music");
    expect(activeNavHref([...CORE_NAV_ITEMS, pinned], "/video")).toBe("/superpowers");
  });

  it("lists every superpower route, old and new, as a Superpowers alias", () => {
    expect(NAV_ALIASES["/superpowers"]).toEqual(expect.arrayContaining(["/league", "/youtube", "/shortpost", "/devtools", "/pack", "/scrape", "/api"]));
  });
});

describe("visibleCoreNav", () => {
  it("keeps everything by default", () => {
    expect(visibleCoreNav({}).map((i) => i.href)).toEqual(CORE_NAV_ITEMS.map((i) => i.href));
  });

  it("downloads only hides the tools and the canvas and nothing else", () => {
    expect(visibleCoreNav({ downloadsOnly: true }).map((i) => i.href)).toEqual(["/", "/downloads", "/global", "/settings", "/community"]);
  });

  it("a disabled canvas stays hidden either way", () => {
    expect(visibleCoreNav({ worldEnabled: false }).some((i) => i.href === "/world")).toBe(false);
  });
});
