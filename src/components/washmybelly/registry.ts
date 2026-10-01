import type { Os } from "$lib/platform";

/// What the person can expect from a module. `beta` and `experimental` get a
/// discreet badge; a module of another system never renders at all.
type ModuleState = "stable" | "beta" | "experimental";

type Requirement =
  | "full_disk_access"
  | "package_manager"
  | "not_flatpak"
  | "gamemode"
  | "battery";

type ModuleGroup = "see" | "clean" | "protect" | "perform" | "maintain";

type WmbModule = {
  id: string;
  group: ModuleGroup;
  labelKey: string;
  state: ModuleState;
  /// Empty: every system.
  platforms: Os[];
  needs?: Requirement[];
  /// Tile colours (Apple system palette) and Phosphor Fill glyph.
  glyph: string;
  from: string;
  to: string;
};

/// Screen order. Mirrors `MODULES` in `core/washmybelly/caps.rs`; the
/// backend's answer wins when both exist.
const WMB_MODULES: readonly WmbModule[] = [
  { id: "overview", group: "see", labelKey: "washmybelly.modules.overview", state: "stable", platforms: [], glyph: "gauge", from: "#5AA9FF", to: "#1E6FE8" },
  { id: "disk", group: "see", labelKey: "washmybelly.modules.disk", state: "stable", platforms: [], glyph: "hard-drives", from: "#A3A3A8", to: "#6F6F75" },
  { id: "clean", group: "clean", labelKey: "washmybelly.modules.clean", state: "stable", platforms: [], glyph: "broom", from: "#FF9BC2", to: "#E0527F" },
  { id: "uninstall", group: "clean", labelKey: "washmybelly.modules.uninstall", state: "beta", platforms: [], needs: ["not_flatpak"], glyph: "trash", from: "#FF7A6B", to: "#E0352B" },
  { id: "startup", group: "clean", labelKey: "washmybelly.modules.startup", state: "stable", platforms: [], glyph: "rocket-launch", from: "#FFB340", to: "#F28500" },
  { id: "privacy", group: "protect", labelKey: "washmybelly.modules.privacy", state: "beta", platforms: [], glyph: "eye-slash", from: "#67D27E", to: "#2F9E52" },
  { id: "tweaks", group: "protect", labelKey: "washmybelly.modules.tweaks", state: "beta", platforms: ["windows"], glyph: "sliders-horizontal", from: "#5AC8FA", to: "#0A84FF" },
  { id: "security", group: "protect", labelKey: "washmybelly.modules.security", state: "beta", platforms: ["macos"], glyph: "shield-check", from: "#34C759", to: "#248A3D" },
  { id: "power", group: "perform", labelKey: "washmybelly.modules.power", state: "beta", platforms: [], glyph: "battery-charging", from: "#30D158", to: "#1F9E40" },
  { id: "games", group: "perform", labelKey: "washmybelly.modules.games", state: "beta", platforms: [], glyph: "game-controller", from: "#C77DFF", to: "#7B3FE4" },
  { id: "perf", group: "perform", labelKey: "washmybelly.modules.perf", state: "experimental", platforms: [], glyph: "chart-line", from: "#BF5AF2", to: "#8944AB" },
  { id: "packages", group: "maintain", labelKey: "washmybelly.modules.packages", state: "beta", platforms: [], needs: ["package_manager"], glyph: "package", from: "#AC8E68", to: "#7D6446" },
  { id: "maintenance", group: "maintain", labelKey: "washmybelly.modules.maintenance", state: "beta", platforms: [], glyph: "wrench", from: "#8E8E93", to: "#5C5C60" },
  { id: "receipts", group: "maintain", labelKey: "washmybelly.modules.receipts", state: "stable", platforms: [], glyph: "receipt", from: "#64D2FF", to: "#0A84FF" },
];

export const GROUP_ORDER: readonly ModuleGroup[] = ["see", "clean", "protect", "perform", "maintain"];

/// The subset of `wmb_capabilities` the screen needs.
export type Caps = {
  os: Os;
  os_version?: string;
  arch?: string;
  is_admin?: boolean;
  has_battery?: boolean;
  modules?: { id: string; available: boolean; reason_key?: string | null; state?: ModuleState }[];
  linux?: { flatpak_sandbox?: boolean; package_managers?: string[]; gamemode?: boolean; distro_name?: string } | null;
  windows?: { edition?: string; build?: number; winget?: boolean; performance_log_users?: boolean } | null;
  macos?: { apple_silicon?: boolean; full_disk_access?: boolean; homebrew?: boolean; xcode?: boolean } | null;
  source_url?: string;
  privacy_source_url?: string;
  tools?: string[];
};

export type Availability =
  | { show: false }
  | { show: true; available: true }
  | { show: true; available: false; reasonKey: string };

function requirementMissing(req: Requirement, caps: Caps): string | null {
  switch (req) {
    case "full_disk_access":
      return caps.macos && !caps.macos.full_disk_access ? "washmybelly.reason.full_disk_access" : null;
    case "not_flatpak":
      return caps.linux?.flatpak_sandbox ? "washmybelly.reason.flatpak_sandbox" : null;
    case "package_manager":
      return caps.os === "linux" && (caps.linux?.package_managers?.length ?? 0) === 0 ? "washmybelly.reason.no_package_manager" : null;
    case "gamemode":
      return caps.os === "linux" && !caps.linux?.gamemode ? "washmybelly.reason.missing_tool" : null;
    case "battery":
      return caps.has_battery === false ? "washmybelly.reason.missing_tool" : null;
  }
}

/// A module of another system is not shown at all. One of this system that
/// misses a requirement is shown with the reason. The backend's own module
/// list, when present, has the last word.
export function availability(mod: WmbModule, caps: Caps): Availability {
  if (mod.platforms.length > 0 && !mod.platforms.includes(caps.os)) return { show: false };
  const backend = caps.modules?.find((m) => m.id === mod.id);
  if (caps.modules && !backend) return { show: false };
  if (backend && !backend.available) {
    return { show: true, available: false, reasonKey: backend.reason_key ?? "washmybelly.unavailable_platform" };
  }
  for (const req of mod.needs ?? []) {
    const reason = requirementMissing(req, caps);
    if (reason) return { show: true, available: false, reasonKey: reason };
  }
  return { show: true, available: true };
}

export function visibleModules(caps: Caps): { mod: WmbModule; availability: Availability }[] {
  return WMB_MODULES.map((mod) => ({ mod, availability: availability(mod, caps) })).filter((m) => m.availability.show);
}

export function moduleById(id: string): WmbModule | undefined {
  return WMB_MODULES.find((m) => m.id === id);
}

export function needsBadge(mod: WmbModule): boolean {
  return mod.state === "beta" || mod.state === "experimental";
}
