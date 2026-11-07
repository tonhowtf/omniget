import { describe, expect, it } from "vitest";
import { resolveSettingsLocation } from "$lib/settings-navigation";
import { integrationsNav } from "$lib/settings-nav/integrations";
import { POWER_IDS, powerEnabled, powerPatch } from "./superpowers";
import { petCornerOrDefault } from "./pet-corners";
import { RPC_DEFAULTS, rpcIdentityChanged, rpcTestOutcome } from "./rpc-defaults";

function resolve(search: string) {
  return resolveSettingsLocation(new URL(`http://localhost/settings${search}`));
}

describe("integrations settings links", () => {
  it.each(POWER_IDS)("sends the old %s section to the superpowers card", (id) => {
    const loc = resolve(`?tab=integrations&section=${id}`);
    expect(loc.tab).toBe("integrations");
    expect(loc.section).toBe("superpowers");
  });

  it("keeps the old advanced tab links", () => {
    expect(resolve("?tab=advanced&section=league")).toMatchObject({ tab: "integrations", section: "superpowers" });
  });

  it("sends the canvas switch to AI", () => {
    expect(resolve("?tab=integrations&section=world")).toMatchObject({ tab: "ai", section: "canvas" });
    expect(resolve("?tab=integrations&section=canvas")).toMatchObject({ tab: "ai", section: "canvas" });
    expect(resolve("?tab=world")).toMatchObject({ tab: "ai", section: "canvas" });
    expect(resolve("?setting=world.settings.enabled")).toMatchObject({ tab: "ai", section: "canvas" });
  });

  it("keeps the pet and extension links", () => {
    expect(resolve("?tab=world&section=pet")).toMatchObject({ tab: "integrations", section: "pet" });
    expect(resolve("?tab=integrations&section=extension-install")).toMatchObject({ tab: "integrations", section: "extension" });
    expect(resolve("?tab=dependencies&section=extension")).toMatchObject({ tab: "integrations", section: "extension" });
  });

  it("sends feature settings that moved out to their pages", () => {
    expect(resolve("?setting=washmybelly.settings_quarantine_days").canonicalHref).toBe("/washmybelly?m=receipts");
    expect(resolve("?setting=washmybelly.settings_auto_hold").canonicalHref).toBe("/washmybelly?m=power");
    expect(resolve("?setting=league.auto_accept").canonicalHref).toBe("/league?tab=automation");
  });

  it("points every alias inside integrations at a real section", () => {
    for (const alias of Object.values(integrationsNav.aliases)) {
      if ("href" in alias || alias.tab !== "integrations") continue;
      expect(integrationsNav.sections).toContain(alias.section);
    }
    for (const [tab, section] of Object.values(integrationsNav.legacy)) {
      if (tab === "integrations") expect(integrationsNav.sections).toContain(section);
    }
  });

  it("labels every section", () => {
    for (const section of integrationsNav.sections) expect(integrationsNav.sectionLabels[section]).toBeTruthy();
  });
});

describe("superpowers toggles", () => {
  it("treats a missing value as on", () => {
    expect(powerEnabled(null, "music")).toBe(true);
    expect(powerEnabled({ music: { enabled: false } }, "music")).toBe(false);
  });

  it("keeps the other OmniTube fields in the patch", () => {
    const settings = { tube: { enabled: true, sponsorblock: true } as { enabled: boolean } };
    expect(powerPatch(settings, "tube", false)).toEqual({ tube: { enabled: false, sponsorblock: true } });
    expect(powerPatch(settings, "league", false)).toEqual({ league: { enabled: false } });
  });
});

describe("pet corner", () => {
  it("falls back to bottom right", () => {
    expect(petCornerOrDefault("top-left")).toBe("top-left");
    expect(petCornerOrDefault("middle")).toBe("bottom-right");
    expect(petCornerOrDefault(undefined)).toBe("bottom-right");
  });
});

describe("discord test result", () => {
  it("names the missing app id", () => {
    expect(rpcTestOutcome({ ok: false, reason: "missing_app_id" })).toBe("missing_app_id");
    expect(rpcTestOutcome({ ok: false })).toBe("fail");
    expect(rpcTestOutcome({ ok: true })).toBe("ok");
    expect(rpcTestOutcome(null)).toBe("fail");
  });

  it("offers a reset only when the identity was changed", () => {
    expect(rpcIdentityChanged({ ...RPC_DEFAULTS })).toBe(false);
    expect(rpcIdentityChanged({ ...RPC_DEFAULTS, app_id: "" })).toBe(true);
    expect(rpcIdentityChanged({ ...RPC_DEFAULTS, large_image_key: "x" })).toBe(true);
    expect(rpcIdentityChanged(null)).toBe(false);
  });
});
