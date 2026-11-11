import { describe, expect, it } from "vitest";
import { MAX_RAIL_PINS, pinnedNavItems, togglePin } from "./pins";

const catalog = [
  { id: "music", icon: "music", titleKey: "music.title", href: "/music" },
  { id: "league", icon: "league", title: "League of Legends", href: "/league" },
  { id: "scrape", icon: "scraper", titleKey: "scrape.title", href: "/scrape" },
];

describe("togglePin", () => {
  it("adds a new id at the end", () => {
    expect(togglePin(["music"], "league")).toEqual(["music", "league"]);
  });

  it("removes an id that is already pinned and keeps the order of the rest", () => {
    expect(togglePin(["music", "league", "scrape"], "league")).toEqual(["music", "scrape"]);
  });

  it("does not change the list it receives", () => {
    const pinned = ["music"];
    togglePin(pinned, "league");
    expect(pinned).toEqual(["music"]);
  });
});

describe("pinnedNavItems", () => {
  it("returns nav items in the pinned order, right after Superpowers", () => {
    const items = pinnedNavItems(catalog, ["scrape", "music"], {});
    expect(items.map((item) => item.href)).toEqual(["/scrape", "/music"]);
    expect(items[0]).toMatchObject({ icon: "scraper", labelKey: "scrape.title", group: "app", pinned: true });
    expect(items.every((item) => (item.order ?? 0) > 30 && (item.order ?? 0) < 40)).toBe(true);
    expect(items[0].order).toBeLessThan(items[1].order ?? 0);
  });

  it("uses the fixed title when the entry has no title key", () => {
    const [item] = pinnedNavItems(catalog, ["league"], {});
    expect(item.label).toBe("League of Legends");
    expect(item.labelKey).toBeUndefined();
  });

  it("drops unknown ids, repeated ids and superpowers that are turned off", () => {
    const items = pinnedNavItems(catalog, ["ghost", "music", "music", "league"], { league: false });
    expect(items.map((item) => item.href)).toEqual(["/music"]);
  });

  it("caps the rail at six items", () => {
    const many = Array.from({ length: 9 }, (_, i) => ({ id: `p${i}`, icon: "music", title: `P${i}`, href: `/p${i}` }));
    const items = pinnedNavItems(many, many.map((entry) => entry.id), {});
    expect(MAX_RAIL_PINS).toBe(6);
    expect(items.map((item) => item.href)).toEqual(["/p0", "/p1", "/p2", "/p3", "/p4", "/p5"]);
  });
});
