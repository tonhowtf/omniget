import { describe, expect, it } from "vitest";
import { pickMeter, raiseTo, type PolicyStatus } from "./budget";

const policy = (over: Partial<PolicyStatus>): PolicyStatus => ({
  id: "p",
  metric: "usd",
  window: "day",
  amount: 5,
  enabled: true,
  observed: 1,
  incident: null,
  ...over,
});

describe("budget meter", () => {
  it("shows nothing without an enabled limit that has a reading", () => {
    expect(pickMeter([])).toBeNull();
    expect(pickMeter([policy({ enabled: false })])).toBeNull();
    expect(pickMeter([policy({ observed: null })])).toBeNull();
  });

  it("picks the limit closest to its ceiling", () => {
    const m = pickMeter([policy({ id: "loose", observed: 1 }), policy({ id: "tight", observed: 4 })]);
    expect(m?.policyId).toBe("tight");
    expect(m?.ratio).toBeCloseTo(0.8);
  });

  it("warns from the policy's own threshold, or 80% when it has none", () => {
    expect(pickMeter([policy({ observed: 3, warn_pct: 50 })])?.tone).toBe("warn");
    expect(pickMeter([policy({ observed: 3 })])?.tone).toBe("ok");
    expect(pickMeter([policy({ observed: 4.5 })])?.tone).toBe("warn");
  });

  it("a paused limit wins over any other and carries its incident", () => {
    const m = pickMeter([
      policy({ id: "near", observed: 4.9 }),
      policy({ id: "hit", observed: 2, incident: { id: "bi_1", state: "paused", observed: 5, limit: 5 } }),
    ]);
    expect(m?.tone).toBe("paused");
    expect(m?.incidentId).toBe("bi_1");
    expect(m?.ratio).toBe(1);
  });

  it("offers a raise only above what is already spent", () => {
    expect(raiseTo({ metric: "usd", limit: 5, spent: 5 })).toBe(10);
    expect(raiseTo({ metric: "quota_pct", limit: 80, spent: 80 })).toBe(100);
    expect(raiseTo({ metric: "quota_pct", limit: 100, spent: 100 })).toBeNull();
    expect(raiseTo({ metric: "usd", limit: 5, spent: 12 })).toBe(18);
    expect(raiseTo({ metric: "weighted_tokens", limit: 1000, spent: 2500.2 })).toBe(3751);
  });
});
