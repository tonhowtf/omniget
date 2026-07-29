import { describe, it, expect } from "vitest";
import { findTeam, teamBans, teamObjectives } from "./league-match-detail";

const blue = {
  teamId: 100,
  towerKills: 9,
  inhibitorKills: 2,
  baronKills: 1,
  dragonKills: 3,
  riftHeraldKills: 1,
  bans: [
    { championId: 55, pickTurn: 3 },
    { championId: 22, pickTurn: 1 },
    { championId: -1, pickTurn: 5 },
  ],
};

describe("teamObjectives", () => {
  it("reads every objective count", () => {
    expect(teamObjectives(blue)).toEqual({
      teamId: 100,
      towers: 9,
      inhibitors: 2,
      barons: 1,
      dragons: 3,
      heralds: 1,
    });
  });

  it("treats missing counts as zero rather than blank", () => {
    expect(teamObjectives({ teamId: 200 })).toEqual({
      teamId: 200,
      towers: 0,
      inhibitors: 0,
      barons: 0,
      dragons: 0,
      heralds: 0,
    });
  });

  it("returns nothing for a team without an id", () => {
    expect(teamObjectives(undefined)).toBeNull();
    expect(teamObjectives({})).toBeNull();
  });
});

describe("teamBans", () => {
  it("orders by pick turn and drops empty bans", () => {
    expect(teamBans(blue)).toEqual([22, 55]);
  });

  it("handles a team that banned nothing", () => {
    expect(teamBans({ teamId: 100 })).toEqual([]);
    expect(teamBans({ teamId: 100, bans: [] })).toEqual([]);
    expect(teamBans(undefined)).toEqual([]);
  });

  it("does not mutate the source array", () => {
    const team = { teamId: 100, bans: [{ championId: 2, pickTurn: 2 }, { championId: 1, pickTurn: 1 }] };
    teamBans(team);
    expect(team.bans[0].championId).toBe(2);
  });
});

describe("findTeam", () => {
  it("finds by id and tolerates missing data", () => {
    expect(findTeam([blue], 100)).toBe(blue);
    expect(findTeam([blue], 200)).toBeUndefined();
    expect(findTeam(undefined, 100)).toBeUndefined();
  });
});
