import { describe, expect, it } from "vitest";
import { defaultUpdates, managerLabel, managers, updateRows, updatesSummary, versionLine } from "./updates";
import { UPDATES } from "./apps.fixtures";

describe("updates", () => {
  it("writes current → available", () => {
    expect(versionLine({ current: "1.2", available: "1.3" })).toBe("1.2 → 1.3");
    expect(versionLine({ current: null, available: "131.0" })).toBe("131.0");
    expect(versionLine({})).toBe("");
  });

  it("groups rows by manager in a fixed order, names A→Z", () => {
    const rows = updateRows(UPDATES.items);
    expect(rows.map((r) => [r.group, r.label])).toEqual([
      ["winget", "Mozilla Firefox"],
      ["brew", "ffmpeg"],
      ["brew", "node"],
      ["apt", "openssl"],
    ]);
    expect(rows[2].hint).toBe("22.9.0 → 22.10.0");
    expect(rows[2].bytes).toBe(40_000_000);
    expect(rows[1].bytes).toBeUndefined();
  });

  it("ticks everything and sums the selection", () => {
    const sel = defaultUpdates(UPDATES.items);
    expect(sel.size).toBe(4);
    expect(updatesSummary(UPDATES.items, sel)).toEqual({ count: 4, bytes: 42_000_000, admin: 1 });
    sel.delete("packages:apt:openssl");
    expect(updatesSummary(UPDATES.items, sel)).toEqual({ count: 3, bytes: 40_000_000, admin: 0 });
  });

  it("lists managers with counts", () => {
    expect(managers(UPDATES.items)).toEqual([
      { manager: "winget", count: 1 },
      { manager: "brew", count: 2 },
      { manager: "apt", count: 1 },
    ]);
    expect(managerLabel("brew")).toBe("Homebrew");
    expect(managerLabel("apt")).toBe("apt");
  });
});
