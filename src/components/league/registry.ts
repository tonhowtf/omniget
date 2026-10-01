type FeatureState = "stable" | "beta" | "experimental" | "unavailable";

export type Platform = "windows" | "macos" | "linux";

type Feature = {
  id: string;
  labelKey: string;
  state: FeatureState;
  platforms: Platform[];
  flag?: string;
  requiresClient?: boolean;
  requiresGame?: boolean;
};

const LEAGUE_FEATURES: readonly Feature[] = [
  { id: "scouting", labelKey: "league.scout_title", state: "stable", platforms: ["windows", "macos"], requiresClient: true },
  { id: "analysis", labelKey: "league.win_title", state: "stable", platforms: ["windows", "macos"], requiresClient: true },
  { id: "build-reference", labelKey: "league.meta_reference", state: "beta", platforms: ["windows", "macos", "linux"] },
  { id: "tier-list", labelKey: "league.tiers_title", state: "stable", platforms: ["windows", "macos", "linux"] },
  { id: "live-metrics", labelKey: "league.gold_title", state: "stable", platforms: ["windows", "macos"], requiresGame: true },
  { id: "objectives", labelKey: "league.objectives_title", state: "beta", platforms: ["windows", "macos"], requiresGame: true },
  { id: "profile-tools", labelKey: "league.profile_tools", state: "beta", platforms: ["windows", "macos"], requiresClient: true },
  { id: "restart-ux", labelKey: "league.restart_ux", state: "stable", platforms: ["windows", "macos"], requiresClient: true },
  { id: "auto-accept", labelKey: "league.auto_accept", state: "stable", platforms: ["windows", "macos"], flag: "auto_accept", requiresClient: true },
  { id: "auto-pick", labelKey: "league.auto_pick", state: "stable", platforms: ["windows", "macos"], flag: "auto_pick", requiresClient: true },
  { id: "auto-ban", labelKey: "league.auto_ban", state: "stable", platforms: ["windows", "macos"], flag: "auto_ban", requiresClient: true },
  { id: "auto-swaps", labelKey: "league.auto_swaps", state: "experimental", platforms: ["windows", "macos"], flag: "auto_accept_swaps", requiresClient: true },
  { id: "auto-requeue", labelKey: "league.auto_requeue", state: "experimental", platforms: ["windows", "macos"], flag: "auto_requeue", requiresClient: true },
  { id: "profile-editor", labelKey: "league.profile_title", state: "beta", platforms: ["windows", "macos"], requiresClient: true },
  { id: "skin-roulette", labelKey: "league.skin_roulette", state: "beta", platforms: ["windows", "macos"], flag: "skin_roulette", requiresClient: true },
  { id: "champion-raffle", labelKey: "league.raffle_title", state: "stable", platforms: ["windows", "macos"], requiresClient: true },
  { id: "pick-random", labelKey: "league.pick_random", state: "experimental", platforms: ["windows", "macos"], flag: "pick_random", requiresClient: true },
  { id: "sgp-history", labelKey: "league.sgp_enabled", state: "experimental", platforms: ["windows", "macos"], flag: "sgp_enabled", requiresClient: true },
  { id: "coach", labelKey: "league.coach_title", state: "beta", platforms: ["windows", "macos"], requiresClient: true },
];

export type Availability =
  | { available: true }
  | { available: false; reasonKey: string };

export type Context = {
  platform: Platform;
  clientConnected: boolean;
  inGame: boolean;
};

export function availability(feature: Feature, context: Context): Availability {
  if (!feature.platforms.includes(context.platform)) {
    return { available: false, reasonKey: "league.unavailable_platform" };
  }
  if (feature.requiresClient && !context.clientConnected) {
    return { available: false, reasonKey: "league.unavailable_client" };
  }
  if (feature.requiresGame && !context.inGame) {
    return { available: false, reasonKey: "league.unavailable_game" };
  }
  return { available: true };
}

export function featureById(id: string): Feature | undefined {
  return LEAGUE_FEATURES.find((feature) => feature.id === id);
}

export function needsBadge(feature: Feature): boolean {
  return feature.state === "beta" || feature.state === "experimental";
}
