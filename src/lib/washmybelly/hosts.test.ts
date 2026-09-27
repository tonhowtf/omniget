import { describe, expect, it } from "vitest";
import { addDomains, blockLineFlags, normalizeDomain, parseDomains, removeDomain, samePrefs, toggleList } from "./hosts";

describe("hosts", () => {
  it("normalizes domains like the core", () => {
    expect(normalizeDomain("  ADS.Example.COM. ")).toBe("ads.example.com");
    expect(normalizeDomain("https://t.example.net/path")).toBe("t.example.net");
    expect(normalizeDomain("*.cdn.example.org")).toBe("cdn.example.org");
    for (const bad of ["localhost", "192.168.0.1", "a b.com", "-x.com", "x..com", "ab"]) expect(normalizeDomain(bad)).toBeNull();
  });

  it("parses free text and keeps lists sorted and unique", () => {
    expect(parseDomains("a.example.com, b.example.com\nnope a.example.com")).toEqual({ ok: ["a.example.com", "b.example.com"], bad: ["nope"] });
    const r = addDomains(["z.example.com"], "a.example.com z.example.com");
    expect(r.list).toEqual(["a.example.com", "z.example.com"]);
    expect(removeDomain(r.list, "a.example.com")).toEqual(["z.example.com"]);
  });

  it("toggles lists and compares prefs", () => {
    expect(toggleList(["adaway"], "windowsspyblocker", true)).toEqual(["adaway", "windowsspyblocker"]);
    expect(toggleList(["adaway"], "adaway", false)).toEqual([]);
    expect(samePrefs({ lists: ["b", "a"], whitelist: [], extra: [] }, { lists: ["a", "b"], whitelist: [], extra: [] })).toBe(true);
    expect(samePrefs({ lists: ["a"], whitelist: [], extra: [] }, { lists: ["a"], whitelist: ["x.com"], extra: [] })).toBe(false);
  });

  it("flags the lines of our block", () => {
    const lines = ["127.0.0.1 localhost", "# >>> OmniGet block list >>>", "0.0.0.0 a.example.com", "# <<< OmniGet block list <<<", "# after"];
    expect(blockLineFlags(lines)).toEqual([false, true, true, true, false]);
  });
});
