import { describe, expect, it } from "vitest";
import { resolveSettingsLocation } from "$lib/settings-navigation";
import { aiNav } from "$lib/settings-nav/ai";

function resolve(search: string, hash = "") {
  return resolveSettingsLocation(new URL(`http://localhost/settings${search}${hash}`));
}

describe("ai settings links", () => {
  it.each(["accounts", "connections", "limits", "connect", "mcp", "canvas"])("keeps the %s section", (section) => {
    expect(resolve(`?tab=ai&section=${section}`)).toMatchObject({ tab: "ai", section });
  });

  it("keeps the old aliases", () => {
    expect(resolve("?tab=ai&section=managers")).toMatchObject({ tab: "ai", section: "accounts" });
    expect(resolve("?tab=ai&section=execution")).toMatchObject({ tab: "ai", section: "connections" });
    expect(resolve("?tab=integrations&section=mcp-server")).toMatchObject({ tab: "ai", section: "connect" });
  });

  it("sends old canvas sub-section hashes to their new group", () => {
    expect(resolve("?tab=ai", "#settings-section-canvas-terminal")).toMatchObject({ tab: "ai", section: "canvas" });
    expect(resolve("?tab=ai", "#settings-section-canvas-presets")).toMatchObject({ tab: "ai", section: "presets" });
    expect(resolve("?tab=ai", "#settings-section-canvas-defaults")).toMatchObject({ tab: "ai", section: "agents-canvas" });
    expect(resolve("?tab=ai", "#settings-section-canvas-report")).toMatchObject({ tab: "advanced", section: "troubleshooting" });
  });

  it("points the app AI fields at the connections group", () => {
    for (const key of ["settings.ai.provider", "settings.ai.model", "settings.ai.connection_key", "settings.ai.endpoint_url"]) {
      expect(resolve(`?setting=${key}`)).toMatchObject({ tab: "ai", section: "connections" });
    }
  });

  it("only aliases to sections that exist", () => {
    for (const alias of Object.values(aiNav.aliases)) {
      if ("tab" in alias && alias.tab === "ai" && alias.section) expect(aiNav.sections).toContain(alias.section);
    }
  });

  it("labels every section", () => {
    for (const section of aiNav.sections) expect(aiNav.sectionLabels[section]).toBeTruthy();
  });
});
