export type HistoryGame = {
  gameId?: number;
  queueId?: number;
  gameMode?: string;
  gameDuration?: number;
  participants?: { stats?: { kills?: number; deaths?: number; assists?: number; win?: boolean } }[];
};

export type HistorySummary = {
  total: number;
  counted: number;
  wins: number;
  losses: number;
  winrate: number | null;
  kda: number | null;
  remakes: number;
};

/// A game that ended before this mark was a remake: it carries no result worth
/// averaging, so it is listed but kept out of the aggregates.
const REMAKE_SECONDS = 180;

export function isRemake(game: HistoryGame): boolean {
  return (game.gameDuration ?? 0) < REMAKE_SECONDS;
}

export function queuesInGames(games: readonly HistoryGame[]): number[] {
  const counts = new Map<number, number>();
  for (const game of games) {
    const id = game.queueId;
    if (typeof id !== "number" || id <= 0) continue;
    counts.set(id, (counts.get(id) ?? 0) + 1);
  }
  return [...counts.entries()]
    .sort((a, b) => b[1] - a[1] || a[0] - b[0])
    .map(([id]) => id);
}

export function filterByQueue<T extends HistoryGame>(games: readonly T[], queueId: number | null): T[] {
  if (queueId === null) return [...games];
  return games.filter((game) => game.queueId === queueId);
}

export function summarise(games: readonly HistoryGame[]): HistorySummary {
  let wins = 0;
  let losses = 0;
  let kills = 0;
  let deaths = 0;
  let assists = 0;
  let remakes = 0;

  for (const game of games) {
    if (isRemake(game)) {
      remakes += 1;
      continue;
    }
    const stats = game.participants?.[0]?.stats;
    if (!stats) continue;
    if (stats.win) wins += 1;
    else losses += 1;
    kills += stats.kills ?? 0;
    deaths += stats.deaths ?? 0;
    assists += stats.assists ?? 0;
  }

  const counted = wins + losses;
  return {
    total: games.length,
    counted,
    wins,
    losses,
    winrate: counted > 0 ? Math.round((wins / counted) * 100) : null,
    // A deathless streak has no ratio to report, so it falls back to the sum.
    kda: counted > 0 ? (deaths > 0 ? (kills + assists) / deaths : kills + assists) : null,
    remakes,
  };
}
