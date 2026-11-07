import { describe, expect, it } from "vitest";
import { proxyModeStep } from "./proxy-mode";

describe("proxyModeStep", () => {
  it("switching to direct saves only when a proxy was on", () => {
    expect(proxyModeStep("direct", true, "10.0.0.1")).toEqual({ enabled: false, save: true, discard: true, focusHost: false });
    expect(proxyModeStep("direct", false, "")).toEqual({ enabled: false, save: false, discard: true, focusHost: false });
  });

  it("switching to a proxy with no host keeps a draft and asks for the host", () => {
    expect(proxyModeStep("proxy", false, "  ")).toEqual({ enabled: true, save: false, discard: false, focusHost: true });
  });

  it("switching to a proxy with a saved host saves at once", () => {
    expect(proxyModeStep("proxy", false, "127.0.0.1")).toEqual({ enabled: true, save: true, discard: false, focusHost: false });
  });

  it("choosing the proxy again when it is already on does nothing", () => {
    expect(proxyModeStep("proxy", true, "127.0.0.1").save).toBe(false);
  });
});

