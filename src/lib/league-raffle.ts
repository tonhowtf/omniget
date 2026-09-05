/**
 * Pure helpers behind the lane and champion raffles, kept out of the
 * components so the odds can be tested without a client.
 */

export const LANES = ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY"] as const;
export type Lane = (typeof LANES)[number];

/** Champion classes as the client tags them in `roles`. */
export const CHAMPION_CLASSES = ["fighter", "mage", "assassin", "marksman", "support", "tank"] as const;
export type ChampionClass = (typeof CHAMPION_CLASSES)[number];

/**
 * Draws a lane, never the one just drawn when there is a choice, so hitting
 * the button twice feels like a new draw instead of a stuck one.
 */
export function drawLane(previous: Lane | null, random: () => number = Math.random): Lane {
  const pool = previous ? LANES.filter((l) => l !== previous) : [...LANES];
  const index = Math.min(pool.length - 1, Math.max(0, Math.floor(random() * pool.length)));
  return pool[index];
}

/** The second preference that pairs with a drawn lane: another draw or fill. */
export function secondaryLane(first: Lane, random: () => number = Math.random): Lane | "FILL" {
  // Fill half the time keeps queues short; the other half is a real second lane.
  if (random() < 0.5) return "FILL";
  return drawLane(first, random);
}
