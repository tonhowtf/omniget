import { describe, expect, it, vi } from "vitest";

// Any command reaching Tauri from a navigation helper is a bug: count them.
const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { parseMcpTarget, parseMusicTarget, parseRosterTarget } from "./settings-navigation";
import { applyMusicTarget, mcpModeHref, mcpSectionSelector, rosterSectionSelector } from "./destination-targets";

const params = (q: string) => new URLSearchParams(q);

describe("mcpModeHref", () => {
  it("writes mode=server and keeps unrelated params and hash", () => {
    const url = new URL("https://app.local/llm/mcp?x=1#top");
    expect(mcpModeHref(url, "server")).toBe("/llm/mcp?x=1&mode=server#top");
  });

  it("removes mode and the server-only section when back on the client", () => {
    const url = new URL("https://app.local/llm/mcp?mode=server&section=media&x=1");
    expect(mcpModeHref(url, "client")).toBe("/llm/mcp?x=1");
  });

  it("returns null when the URL already matches (no duplicate history)", () => {
    expect(mcpModeHref(new URL("https://app.local/llm/mcp"), "client")).toBeNull();
    expect(mcpModeHref(new URL("https://app.local/llm/mcp?mode=server&section=permissions"), "server")).toBeNull();
  });

  it("normalizes an invalid mode to the client default", () => {
    const url = new URL("https://app.local/llm/mcp?mode=bogus");
    expect(parseMcpTarget(url.searchParams).mode).toBe("client");
    expect(mcpModeHref(url, parseMcpTarget(url.searchParams).mode)).toBe("/llm/mcp");
  });
});

describe("section selectors", () => {
  it("maps only allowlisted sections", () => {
    expect(mcpSectionSelector(parseMcpTarget(params("mode=server&section=permissions")).section)).toBe("#mcp-section-permissions");
    expect(mcpSectionSelector(parseMcpTarget(params("mode=server&section=media")).section)).toBe("#mcp-section-media");
    expect(mcpSectionSelector(parseMcpTarget(params("section=media")).section)).toBeNull();
    expect(rosterSectionSelector(parseRosterTarget(params("bot=a&section=skills")).section)).toBe("#bot-section-skills");
    expect(rosterSectionSelector(parseRosterTarget(params("bot=a&section=learning")).section)).toBe("#bot-section-learning");
    expect(rosterSectionSelector(parseRosterTarget(params("section=skills")).section)).toBeNull();
    expect(rosterSectionSelector(parseRosterTarget(params("bot=a&section=grants")).section)).toBeNull();
  });
});

describe("applyMusicTarget", () => {
  function deps(currentKind = "home") {
    return { currentKind, go: vi.fn(), showEq: vi.fn() };
  }

  it("reveals the settings view and the equalizer through the store navigation only", () => {
    const d = deps();
    expect(applyMusicTarget(parseMusicTarget(params("view=settings&panel=eq")), d)).toBe("#music-eq-panel");
    expect(d.go).toHaveBeenCalledWith({ kind: "settings" });
    expect(d.showEq).toHaveBeenCalledTimes(1);
  });

  it("does not push the same view twice", () => {
    const d = deps("settings");
    expect(applyMusicTarget(parseMusicTarget(params("view=settings")), d)).toBe("#music-settings-view");
    expect(d.go).not.toHaveBeenCalled();
  });

  it("ignores unknown values", () => {
    const d = deps();
    expect(applyMusicTarget(parseMusicTarget(params("view=delete&panel=scan")), d)).toBeNull();
    expect(d.go).not.toHaveBeenCalled();
    expect(d.showEq).not.toHaveBeenCalled();
  });

  it("never reaches a command", () => {
    applyMusicTarget(parseMusicTarget(params("view=settings&panel=eq")), deps());
    mcpModeHref(new URL("https://app.local/llm/mcp?mode=server"), "client");
    expect(invoke).not.toHaveBeenCalled();
  });
});
