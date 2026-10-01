type MatchTeam = {
  teamId?: number;
  towerKills?: number;
  inhibitorKills?: number;
  baronKills?: number;
  dragonKills?: number;
  riftHeraldKills?: number;
  bans?: { championId?: number; pickTurn?: number }[];
};

type TeamObjectives = {
  teamId: number;
  towers: number;
  inhibitors: number;
  barons: number;
  dragons: number;
  heralds: number;
};

export function teamObjectives(team: MatchTeam | undefined): TeamObjectives | null {
  if (!team || typeof team.teamId !== "number") return null;
  return {
    teamId: team.teamId,
    towers: team.towerKills ?? 0,
    inhibitors: team.inhibitorKills ?? 0,
    barons: team.baronKills ?? 0,
    dragons: team.dragonKills ?? 0,
    heralds: team.riftHeraldKills ?? 0,
  };
}

export function teamBans(team: MatchTeam | undefined): number[] {
  return (team?.bans ?? [])
    .slice()
    .sort((a, b) => (a.pickTurn ?? 0) - (b.pickTurn ?? 0))
    .map((ban) => ban.championId ?? -1)
    .filter((id) => id > 0);
}

export function findTeam(teams: readonly MatchTeam[] | undefined, teamId: number): MatchTeam | undefined {
  return (teams ?? []).find((team) => team.teamId === teamId);
}
