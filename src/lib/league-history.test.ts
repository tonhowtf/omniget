import { describe, it, expect } from "vitest";
import {
  filterByQueue,
  isRemake,
  queuesInGames,
  summarise,
  type HistoryGame,
} from "./league-history";

function game(
  queueId: number,
  win: boolean,
  kda: [number, number, number] = [5, 2, 5],
  gameDuration = 1800,
): HistoryGame {
  return {
    queueId,
    gameDuration,
    participants: [{ stats: { kills: kda[0], deaths: kda[1], assists: kda[2], win } }],
  };
}

describe("queuesInGames", () => {
  it("lists each queue once, most played first", () => {
    const games = [game(420, true), game(450, false), game(420, false), game(440, true)];
    expect(queuesInGames(games)).toEqual([420, 440, 450]);
  });

  it("ignores games without a usable queue id", () => {
    expect(queuesInGames([{ queueId: 0 }, {}, { queueId: -1 }])).toEqual([]);
    expect(queuesInGames([])).toEqual([]);
  });
});

describe("filterByQueue", () => {
  it("returns everything when no queue is selected", () => {
    const games = [game(420, true), game(450, false)];
    expect(filterByQueue(games, null)).toHaveLength(2);
  });

  it("keeps only the selected queue", () => {
    const games = [game(420, true), game(450, false), game(420, false)];
    expect(filterByQueue(games, 420)).toHaveLength(2);
    expect(filterByQueue(games, 900)).toHaveLength(0);
  });
});

describe("summarise", () => {
  it("counts wins, losses and the aggregate KDA", () => {
    const summary = summarise([
      game(420, true, [10, 2, 4]),
      game(420, false, [2, 6, 2]),
      game(420, true, [6, 2, 6]),
    ]);
    expect(summary).toMatchObject({ total: 3, counted: 3, wins: 2, losses: 1, winrate: 67 });
    // (18 kills + 12 assists) / 10 deaths
    expect(summary.kda).toBeCloseTo(3, 5);
  });

  it("leaves remakes out of the aggregates but still counts them", () => {
    const summary = summarise([game(420, true), game(420, false, [0, 0, 0], 120)]);
    expect(summary).toMatchObject({ total: 2, counted: 1, wins: 1, losses: 0, remakes: 1 });
    expect(isRemake(game(420, true, [0, 0, 0], 120))).toBe(true);
    expect(isRemake(game(420, true))).toBe(false);
  });

  it("reports nothing instead of dividing by zero on an empty set", () => {
    expect(summarise([])).toMatchObject({ total: 0, counted: 0, winrate: null, kda: null });
  });

  it("falls back to the raw sum when the player never died", () => {
    const summary = summarise([game(420, true, [7, 0, 3])]);
    expect(summary.kda).toBe(10);
  });

  it("survives games with no participant data", () => {
    const summary = summarise([{ queueId: 420, gameDuration: 1800 }]);
    expect(summary).toMatchObject({ total: 1, counted: 0, winrate: null });
  });
});
