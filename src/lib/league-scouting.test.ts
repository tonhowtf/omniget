import { describe, it, expect } from "vitest";
import { markedPlayers, winrateSquad, type ScoutedPlayer } from "./league-scouting";

const team = (ally: boolean, count: number, offset = 0): ScoutedPlayer[] =>
  Array.from({ length: count }, (_, i) => ({
    puuid: `p${offset + i}`,
    isAlly: ally,
    gameName: `Player${offset + i}`,
  }));

describe("markedPlayers", () => {
  it("splits marked players by side and prefers the display name", () => {
    const players = [...team(true, 2), ...team(false, 2, 10)];
    const marked = markedPlayers(players, { p0: "smurf", p10: "trolled last game" });
    expect(marked).toEqual({ ally: ["Player0"], enemy: ["Player10"] });
  });

  it("treats a blank note as no note", () => {
    const marked = markedPlayers(team(true, 2), { p0: "   ", p1: "" });
    expect(marked).toEqual({ ally: [], enemy: [] });
  });

  it("falls back to a short puuid when the name is missing", () => {
    const marked = markedPlayers([{ puuid: "abcdefghijkl", isAlly: false }], {
      abcdefghijkl: "note",
    });
    expect(marked.enemy).toEqual(["abcdefgh"]);
  });

  it("ignores players without a puuid", () => {
    expect(markedPlayers([{ isAlly: true, gameName: "Ghost" }], {})).toEqual({
      ally: [],
      enemy: [],
    });
  });
});

describe("winrateSquad", () => {
  const five = team(false, 5);
  const allPrivate = Object.fromEntries(five.map((p) => [p.puuid!, { privateProfile: true }]));

  it("flags a five-stack whose profiles are all hidden", () => {
    const premades = [{ label: "A", puuids: five.map((p) => p.puuid!) }];
    expect(winrateSquad(five, premades, allPrivate)).toBe("enemy");
  });

  it("says nothing when one profile is visible", () => {
    const reports = { ...allPrivate, p2: { privateProfile: false } };
    const premades = [{ label: "A", puuids: five.map((p) => p.puuid!) }];
    expect(winrateSquad(five, premades, reports)).toBeNull();
  });

  it("says nothing for a group smaller than five", () => {
    const premades = [{ label: "A", puuids: ["p0", "p1"] }];
    expect(winrateSquad(five, premades, allPrivate)).toBeNull();
  });

  it("reports the ally side when it is the user's own stack", () => {
    const allies = team(true, 5);
    const reports = Object.fromEntries(allies.map((p) => [p.puuid!, { privateProfile: true }]));
    const premades = [{ label: "A", puuids: allies.map((p) => p.puuid!) }];
    expect(winrateSquad(allies, premades, reports)).toBe("ally");
  });

  it("ignores a group that spans both teams or has unknown members", () => {
    const mixed = [...team(true, 3), ...team(false, 2, 3)];
    const reports = Object.fromEntries(mixed.map((p) => [p.puuid!, { privateProfile: true }]));
    const premades = [{ puuids: mixed.map((p) => p.puuid!) }];
    expect(winrateSquad(mixed, premades, reports)).toBeNull();
    expect(winrateSquad([], [{ puuids: ["x", "y", "z", "w", "v"] }], {})).toBeNull();
  });

  it("handles missing or malformed premade payloads", () => {
    expect(winrateSquad(five, [], allPrivate)).toBeNull();
    expect(winrateSquad(five, [{}], allPrivate)).toBeNull();
    expect(winrateSquad(five, [{ puuids: [null, "", "p0", "p1", "p2", "p3", "p4"] }], allPrivate)).toBe(
      "enemy",
    );
  });
});
