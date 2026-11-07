import { describe, expect, it } from "vitest";
import { resolveSettingsLocation } from "$lib/settings-navigation";
import { advancedNav } from "$lib/settings-nav/advanced";
import { NAV_PARTS } from "$lib/settings-nav";

function go(path: string) {
  const { tab, section } = resolveSettingsLocation(new URL(path, "http://app.local"));
  return [tab, section];
}

describe("advanced settings links", () => {
  it.each([
    ["/settings?tab=advanced&section=network", "connection"],
    ["/settings?tab=advanced&section=expert", "connection"],
    ["/settings?tab=advanced&section=performance", "speed"],
    ["/settings?tab=advanced&section=torrent", "torrent"],
    ["/settings?tab=advanced&section=storage", "dependencies"],
    ["/settings?tab=advanced&section=debug", "troubleshooting"],
    ["/settings?tab=advanced&section=reset", "troubleshooting"],
    ["/settings?tab=network&section=proxy", "connection"],
    ["/settings?tab=dependencies", "dependencies"],
  ])("%s opens %s", (path, section) => {
    expect(go(path)).toEqual(["advanced", section]);
  });

  it("sends the old browser cookies setting to Downloads › Cookies", () => {
    expect(go("/settings?setting=settings.advanced.cookies_from_browser")).toEqual(["downloads", "cookies"]);
  });

  it("every alias points at a section that exists", () => {
    for (const alias of Object.values(advancedNav.aliases)) {
      if ("href" in alias || alias.tab !== "advanced" || alias.section === null) continue;
      expect(advancedNav.sections).toContain(alias.section);
    }
  });

  it("every link into Advanced from any category lands on a section that exists", () => {
    for (const [category, part] of Object.entries(NAV_PARTS)) {
      for (const [key, [tab, section]] of Object.entries(part.legacy)) {
        if (tab === "advanced" && section !== null) expect(advancedNav.sections, `${category} legacy ${key}`).toContain(section);
      }
      for (const [key, alias] of Object.entries(part.aliases)) {
        if ("href" in alias || alias.tab !== "advanced" || alias.section === null) continue;
        expect(advancedNav.sections, `${category} alias ${key}`).toContain(alias.section);
      }
    }
  });
});
