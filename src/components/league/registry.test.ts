import { describe, it, expect } from "vitest";
import {
  availability,
  featureById,
  LEAGUE_FEATURES,
  needsBadge,
  type Feature,
} from "./registry";

const online = { platform: "macos" as const, clientConnected: true, inGame: true };

describe("availability", () => {
  const feature: Feature = {
    id: "x",
    labelKey: "k",
    state: "stable",
    platforms: ["windows", "macos"],
    requiresClient: true,
    requiresGame: true,
  };

  it("clears a feature whose requirements are all met", () => {
    expect(availability(feature, online)).toEqual({ available: true });
  });

  it("reports the platform first, because nothing else can fix it", () => {
    const result = availability(feature, { ...online, platform: "linux", clientConnected: false });
    expect(result).toEqual({ available: false, reasonKey: "league.unavailable_platform" });
  });

  it("reports the client before the game", () => {
    expect(availability(feature, { ...online, clientConnected: false, inGame: false })).toEqual({
      available: false,
      reasonKey: "league.unavailable_client",
    });
  });

  it("reports the game when only that is missing", () => {
    expect(availability(feature, { ...online, inGame: false })).toEqual({
      available: false,
      reasonKey: "league.unavailable_game",
    });
  });

  it("lets a feature with no requirements run anywhere it is supported", () => {
    const anywhere: Feature = { id: "y", labelKey: "k", state: "stable", platforms: ["linux"] };
    expect(availability(anywhere, { platform: "linux", clientConnected: false, inGame: false })).toEqual({
      available: true,
    });
  });
});

describe("the registry itself", () => {
  it("has unique ids", () => {
    const ids = LEAGUE_FEATURES.map((f) => f.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("looks features up by id", () => {
    expect(featureById("auto-accept")?.flag).toBe("auto_accept");
    expect(featureById("nope")).toBeUndefined();
  });

  it("marks only beta and experimental features", () => {
    expect(needsBadge({ id: "a", labelKey: "k", state: "beta", platforms: [] })).toBe(true);
    expect(needsBadge({ id: "a", labelKey: "k", state: "experimental", platforms: [] })).toBe(true);
    expect(needsBadge({ id: "a", labelKey: "k", state: "stable", platforms: [] })).toBe(false);
  });

  it("never claims a client-only feature works without the client", () => {
    for (const feature of LEAGUE_FEATURES.filter((f) => f.requiresGame)) {
      expect(feature.requiresGame).toBe(true);
      const result = availability(feature, { ...online, inGame: false });
      expect(result.available).toBe(false);
    }
  });
});
