export type ScoutedPlayer = { puuid?: string; isAlly?: boolean; gameName?: string };
export type PremadeGroup = { label?: string; puuids?: (string | null)[] };
export type ScoutReport = { privateProfile?: boolean } | null | undefined;

export type MarkedPlayers = { ally: string[]; enemy: string[] };

/// Players the user already wrote a note about. An empty or whitespace-only note
/// is not a mark: it is a note the user started and abandoned.
export function markedPlayers(
  players: readonly ScoutedPlayer[],
  notes: Record<string, string>,
): MarkedPlayers {
  const marked: MarkedPlayers = { ally: [], enemy: [] };
  for (const player of players) {
    const puuid = player.puuid;
    if (!puuid) continue;
    if ((notes[puuid] ?? "").trim() === "") continue;
    const label = player.gameName?.trim() || puuid.slice(0, 8);
    (player.isAlly ? marked.ally : marked.enemy).push(label);
  }
  return marked;
}

function sideOf(players: readonly ScoutedPlayer[], puuids: readonly string[]): boolean | null {
  const sides = new Set(
    puuids
      .map((puuid) => players.find((p) => p.puuid === puuid)?.isAlly)
      .filter((side): side is boolean => typeof side === "boolean"),
  );
  return sides.size === 1 ? [...sides][0] : null;
}

/// A five-player group whose profiles are all hidden is the shape of a squad
/// queueing together to farm wins. It is reported as an observation, never as an
/// accusation: both halves have to hold, and neither is proof on its own.
export function winrateSquad(
  players: readonly ScoutedPlayer[],
  premades: readonly PremadeGroup[],
  reports: Record<string, ScoutReport>,
): "ally" | "enemy" | null {
  for (const group of premades) {
    const puuids = (group.puuids ?? []).filter((p): p is string => typeof p === "string" && p !== "");
    if (puuids.length < 5) continue;
    const side = sideOf(players, puuids);
    if (side === null) continue;
    const allHidden = puuids.every((puuid) => reports[puuid]?.privateProfile === true);
    if (allHidden) return side ? "ally" : "enemy";
  }
  return null;
}
