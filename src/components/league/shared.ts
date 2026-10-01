export const CDRAGON = "https://raw.communitydragon.org/latest/plugins/rcp-be-lol-game-data/global/default/v1";
const RAW_CDRAGON = "https://raw.communitydragon.org/latest/plugins/rcp-be-lol-game-data/global/default";

export function assetUrl(path: string | null | undefined): string {
  if (!path) return "";
  const clean = path.replace(/^\/lol-game-data\/assets/i, "").toLowerCase();
  return `${RAW_CDRAGON}${clean}`;
}

export function formatGameTime(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${String(s).padStart(2, "0")}`;
}

export const TAG_KEYS: Record<string, string> = {
  hot_streak: "league.tag_hot_streak",
  cold_streak: "league.tag_cold_streak",
  high_winrate: "league.tag_high_winrate",
  low_winrate: "league.tag_low_winrate",
  one_trick: "league.tag_one_trick",
  high_kda: "league.tag_high_kda",
  low_kda: "league.tag_low_kda",
  flash_swap: "league.tag_flash_swap",
  high_damage_efficiency: "league.tag_high_damage_efficiency",
  low_damage_efficiency: "league.tag_low_damage_efficiency",
};

export const ROLES = ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY"] as const;
export type Role = (typeof ROLES)[number];

export const GOAL_FIELDS = [
  { key: "csPerMin", labelKey: "league.goal_cs", step: 0.1 },
  { key: "goldPerMin", labelKey: "league.goal_gold", step: 10 },
  { key: "kda", labelKey: "league.goal_kda", step: 0.1 },
  { key: "visionPerMin", labelKey: "league.goal_vision", step: 0.1 },
] as const;
export type GoalKey = (typeof GOAL_FIELDS)[number]["key"];

export type Champion = { id: number; name: string; alias: string };
export type RankedEntry = {
  tier?: string;
  division?: string;
  leaguePoints?: number;
  wins?: number;
  losses?: number;
};
export type ScoutPlayer = {
  puuid: string;
  gameName: string;
  tagLine: string;
  championId: number;
  cellId: number | null;
  isAlly: boolean;
  summonerLevel?: number;
};
export type LobbyQueue = { id: number; name: string; shortName: string; gameMode: string };
