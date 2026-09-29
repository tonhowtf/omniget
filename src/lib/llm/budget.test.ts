import { describe, expect, it } from "vitest";
import { formatAmount, scopeLabel, suggestedRaise, usedShare } from "./budget";

describe("budget formatting", () => {
  it("prints each metric in its own unit", () => {
    expect(formatAmount("usd", 0.0512)).toBe("US$ 0.0512");
    expect(formatAmount("usd", 12.5)).toBe("US$ 12.50");
    expect(formatAmount("weighted_tokens", 2_000_000)).toBe("2.00 M");
    expect(formatAmount("weighted_tokens", 15_300)).toBe("15.3 k");
    expect(formatAmount("quota_pct", 5)).toBe("5.0%");
  });

  it("reads a scope and fills the bar without passing 100", () => {
    expect(scopeLabel("loop:l1")).toBe("loop l1");
    expect(scopeLabel("global")).toBe("global");
    expect(usedShare(0.06, 0.05)).toBe(100);
    expect(usedShare(null, 5)).toBe(0);
    expect(usedShare(2.5, 5)).toBe(50);
  });

  it("suggests a raise above what is used, within the quota window", () => {
    expect(suggestedRaise({ metric: "usd", observed: 0.06, limit: 0.05 })).toBeGreaterThan(0.06);
    expect(suggestedRaise({ metric: "quota_pct", observed: 90, limit: 80 })).toBe(100);
    expect(suggestedRaise({ metric: "weighted_tokens", observed: 210_000, limit: 200_000 }) % 1000).toBe(0);
  });
});
