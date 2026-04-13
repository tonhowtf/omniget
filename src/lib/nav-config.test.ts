import { describe, it, expect } from "vitest";
import { CORE_NAV_ITEMS } from "./nav-config";

describe("CORE_NAV_ITEMS", () => {
  it("has the expected 5 core entries", () => {
    expect(CORE_NAV_ITEMS).toHaveLength(5);
  });

  it("Home is first in primary group", () => {
    const home = CORE_NAV_ITEMS.find((i) => i.href === "/");
    expect(home?.group).toBe("primary");
    expect(home?.labelKey).toBe("nav.home");
  });

  it("Downloads carries the downloads badge", () => {
    const downloads = CORE_NAV_ITEMS.find((i) => i.href === "/downloads");
    expect(downloads?.badge).toBe("downloads");
  });

  it("About is in the secondary group and sorted last", () => {
    const about = CORE_NAV_ITEMS.find((i) => i.href === "/about");
    expect(about?.group).toBe("secondary");
    expect(about?.order).toBe(999);
  });

  it("order is strictly increasing among primary items", () => {
    const primaryOrders = CORE_NAV_ITEMS.filter((i) => i.group === "primary")
      .map((i) => i.order ?? 0)
      .sort((a, b) => a - b);
    for (let i = 1; i < primaryOrders.length; i++) {
      expect(primaryOrders[i]).toBeGreaterThan(primaryOrders[i - 1]);
    }
  });
});
