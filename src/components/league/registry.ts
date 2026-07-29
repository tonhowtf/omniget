/// What the user can expect from a feature. `experimental` and `beta` are marked
/// in the UI with a discreet badge; `unavailable` renders an explanation of why
/// instead of a broken control.
export type FeatureState = "stable" | "beta" | "experimental" | "unavailable";

export type Platform = "windows" | "macos" | "linux";

export type Feature = {
  id: string;
  /// i18n key for the human-readable name.
  labelKey: string;
  state: FeatureState;
  platforms: Platform[];
  /// Key under `settings.league` that switches it off, when there is one.
  flag?: string;
  /// Reason the feature cannot run, as an i18n key. Only meaningful together
  /// with the checks below.
  requiresClient?: boolean;
  requiresGame?: boolean;
};

/// Single place that records what each league surface needs to work, so a
/// disabled control can always explain itself instead of silently failing.
export const LEAGUE_FEATURES: readonly Feature[] = [
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
];

export type Availability =
  | { available: true }
  | { available: false; reasonKey: string };

export type Context = {
  platform: Platform;
  clientConnected: boolean;
  inGame: boolean;
};

/// Why a feature cannot be used right now, in the order the user can act on:
/// platform first (nothing to do about it), then the client, then the game.
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

/// Features worth marking in the UI, so a badge is never guesswork.
export function needsBadge(feature: Feature): boolean {
  return feature.state === "beta" || feature.state === "experimental";
}
