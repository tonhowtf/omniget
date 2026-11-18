import { describe, expect, it } from "vitest";
import { proxyState, type PoolProxy } from "./proxy-pool";

const base: PoolProxy = { id: "px_1", proxy: "http://10.0.0.1:80", current: false, fails: 0, restingSecs: 0 };
const checked = { ms: 120, ip: "203.0.113.9", country: "BR", status: 200, error: null };

describe("proxyState", () => {
  it("a proxy nobody tested is untested", () => {
    expect(proxyState(base)).toBe("untested");
  });

  it("a passed test is ok", () => {
    expect(proxyState({ ...base, checked: { ...checked, ok: true } })).toBe("ok");
  });

  it("a failed test wins over the rest it causes", () => {
    expect(proxyState({ ...base, restingSecs: 30, fails: 1, checked: { ...checked, ok: false, error: "no answer" } })).toBe("failed");
  });

  it("a proxy refused by a site rests even when its last test passed", () => {
    expect(proxyState({ ...base, restingSecs: 30, fails: 1, checked: { ...checked, ok: true } })).toBe("resting");
    expect(proxyState({ ...base, restingSecs: 30, fails: 1 })).toBe("resting");
  });
});
