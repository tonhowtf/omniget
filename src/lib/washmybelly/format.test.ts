import { describe, expect, it } from "vitest";
import { dayBucket, explainDetail, formatBytes, formatPercent, middle, shortenPath, splitBytes, tildify } from "./format";

describe("formatBytes", () => {
  it("uses decimal units like Finder", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(-4)).toBe("0 B");
    expect(formatBytes(null)).toBe("0 B");
    expect(formatBytes(999)).toBe("999 B");
    expect(formatBytes(1000)).toBe("1.0 KB");
    expect(formatBytes(1_530_000)).toBe("1.5 MB");
    expect(formatBytes(250_000_000_000)).toBe("250 GB");
  });
  it("splits value and unit", () => {
    expect(splitBytes(2_400_000_000)).toEqual({ value: "2.4", unit: "GB" });
  });
});

describe("paths", () => {
  it("replaces the home folder with ~", () => {
    expect(tildify("/Users/ana/Library/Caches")).toBe("~/Library/Caches");
    expect(tildify("/home/bob/.cache")).toBe("~/.cache");
    expect(tildify("C:\\Users\\Eve\\AppData")).toBe("~\\AppData");
    expect(tildify("/Users/ana")).toBe("~");
    expect(tildify("/var/log")).toBe("/var/log");
  });

  it("keeps short paths whole", () => {
    expect(shortenPath("/Users/ana/Library/Caches", 40)).toBe("~/Library/Caches");
  });

  it("shortens in the middle, keeping the start and the file name", () => {
    const p = "/Users/ana/Library/Application Support/Code/User/workspaceStorage/abc123/state.vscdb";
    const s = shortenPath(p, 44);
    expect(s.length).toBeLessThanOrEqual(44);
    expect(s.startsWith("~/Library/…/")).toBe(true);
    expect(s.endsWith("state.vscdb")).toBe(true);
  });

  it("works with Windows paths and absolute roots", () => {
    const s = shortenPath("C:\\Users\\Eve\\AppData\\Local\\Temp\\some\\deep\\folder\\file.tmp", 30);
    expect(s.startsWith("~\\AppData\\…\\")).toBe(true);
    expect(s.endsWith("file.tmp")).toBe(true);
    const r = shortenPath("/var/folders/xy/abcdefghijklmnop/T/com.apple.thing/cache.db", 34);
    expect(r.startsWith("/var/…/")).toBe(true);
    expect(r.length).toBeLessThanOrEqual(34);
  });

  it("falls back to a middle ellipsis for one long segment", () => {
    const s = shortenPath("/" + "x".repeat(100), 20);
    expect(s.length).toBe(20);
    expect(s).toContain("…");
    expect(middle("abcdefghij", 5)).toBe("ab…ij");
  });
});

describe("dates and numbers", () => {
  const now = new Date(2026, 8, 27, 15, 0, 0);
  const secs = (d: Date) => Math.floor(d.getTime() / 1000);
  it("buckets by day", () => {
    expect(dayBucket(secs(new Date(2026, 8, 27, 1, 0)), now).bucket).toBe("today");
    expect(dayBucket(secs(new Date(2026, 8, 26, 23, 59)), now).bucket).toBe("yesterday");
    const e = dayBucket(secs(new Date(2026, 8, 3, 9, 0)), now);
    expect(e.bucket).toBe("earlier");
    expect(e.key).toBe("2026-09-03");
  });
  it("formats percent", () => {
    expect(formatPercent(0.42)).toBe("42%");
    expect(formatPercent(73.5, 1)).toBe("73.5%");
    expect(formatPercent(null)).toBe("–");
  });
});

describe("explainDetail", () => {
  it("maps known codes to a key", () => {
    expect(explainDetail("CHANGED: the file was modified after the scan")).toEqual({
      code: "CHANGED",
      key: "washmybelly.ui.reason.changed",
      text: "the file was modified after the scan",
    });
  });
  it("keeps unknown codes and plain text", () => {
    expect(explainDetail("WEIRD_THING: x").key).toBeNull();
    expect(explainDetail("permission denied")).toEqual({ code: null, key: null, text: "permission denied" });
    expect(explainDetail(undefined).text).toBe("");
  });
});
