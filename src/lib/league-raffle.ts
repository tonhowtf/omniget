const LANES = ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY"] as const;
export type Lane = (typeof LANES)[number];

export const CHAMPION_CLASSES = ["fighter", "mage", "assassin", "marksman", "support", "tank"] as const;

export function drawLane(previous: Lane | null, random: () => number = Math.random): Lane {
  const pool = previous ? LANES.filter((l) => l !== previous) : [...LANES];
  const index = Math.min(pool.length - 1, Math.max(0, Math.floor(random() * pool.length)));
  return pool[index];
}

export function secondaryLane(first: Lane, random: () => number = Math.random): Lane | "FILL" {
  if (random() < 0.5) return "FILL";
  return drawLane(first, random);
}
