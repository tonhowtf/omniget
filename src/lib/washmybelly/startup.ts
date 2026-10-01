/// Startup items from `wmb_startup_list` and the pure logic of the screen:
/// grouping by type, the third-party filter, what a toggle can do.

type StartupKind =
  | "login_item"
  | "agent"
  | "daemon"
  | "run"
  | "run_once"
  | "startup_folder"
  | "task"
  | "service"
  | "autostart"
  | "user_service";

export type StartupItem = {
  id: string;
  name: string;
  kind: StartupKind | string;
  scope: "user" | "system" | string;
  location: string;
  command: string;
  enabled: boolean;
  running?: boolean | null;
  signer?: string | null;
  signer_verified?: boolean;
  team_id?: string | null;
  installed?: number | null;
  impact?: { ms: number; level: "low" | "medium" | "high" | string } | null;
  can_disable: boolean;
  can_enable: boolean;
  needs_admin: boolean;
  reason?: string | null;
  recommended?: boolean;
  third_party: boolean;
};

export type StartupList = {
  items: StartupItem[];
  total_bytes: number;
  plan_id: string | null;
  enabled_third_party?: number;
  include_system?: boolean;
  notes?: string[];
};

type StartupGroup = "login" | "agents" | "services" | "tasks" | "autostart";

export const GROUP_ORDER: readonly StartupGroup[] = ["login", "agents", "services", "tasks", "autostart"];

export function groupOf(kind: string): StartupGroup {
  switch (kind) {
    case "login_item":
    case "run":
    case "run_once":
    case "startup_folder":
      return "login";
    case "agent":
    case "daemon":
      return "agents";
    case "service":
    case "user_service":
      return "services";
    case "task":
      return "tasks";
    default:
      return "autostart";
  }
}

/// The toggle can move this item to the other state.
export function canToggle(it: StartupItem): boolean {
  return it.enabled ? it.can_disable : it.can_enable;
}

export function filterItems(items: StartupItem[], opts: { thirdPartyOnly: boolean; query?: string }): StartupItem[] {
  const q = (opts.query ?? "").trim().toLowerCase();
  return items.filter((it) => {
    if (opts.thirdPartyOnly && !it.third_party) return false;
    if (!q) return true;
    return [it.name, it.location, it.command, it.signer ?? ""].some((s) => s.toLowerCase().includes(q));
  });
}

export function groupItems(items: StartupItem[]): { group: StartupGroup; items: StartupItem[]; on: number }[] {
  return GROUP_ORDER.map((group) => {
    const list = items
      .filter((it) => groupOf(it.kind) === group)
      .sort((a, b) => Number(b.third_party) - Number(a.third_party) || a.name.localeCompare(b.name));
    return { group, items: list, on: list.filter((i) => i.enabled).length };
  }).filter((g) => g.items.length > 0);
}

/// Ids of the chosen items that are on and can be turned off.
export function disableIds(items: StartupItem[], selected: Set<string>): string[] {
  return items.filter((it) => selected.has(it.id) && it.enabled && it.can_disable).map((it) => it.id);
}

/// i18n key of a reason code from the backend (`washmybelly.startup.reason.*`).
export const REASONS = [
  "system_item",
  "daemon_until_restart",
  "settings_only",
  "run_once",
  "service_manual",
  "protected",
  "unsupported_name",
  "other_desktop",
  "desktop_part",
  "turned_off_here",
] as const;

export function reasonKey(reason: string | null | undefined): string | null {
  if (!reason) return null;
  return (REASONS as readonly string[]).includes(reason) ? `washmybelly.startup.reason.${reason}` : null;
}

/// Reasons that are only a note: the item can still be switched.
export function isSoftReason(reason: string | null | undefined): boolean {
  return reason === "daemon_until_restart" || reason === "service_manual" || reason === "other_desktop" || reason === "turned_off_here";
}

/// Notes from the scan worth showing (`btm_unavailable: ...` → its code).
export function noteCodes(notes: string[] | undefined): string[] {
  return [...new Set((notes ?? []).map((n) => n.split(":")[0].trim()).filter(Boolean))];
}

/// The letter tile of a row: first letter or digit of the name.
export function initial(name: string): string {
  const m = name.match(/[\p{L}\p{N}]/u);
  return m ? m[0].toUpperCase() : "?";
}

/// A stable hue for the tile, from the name.
export function hueOf(name: string): number {
  let h = 0;
  for (const c of name) h = (h * 31 + c.codePointAt(0)!) % 360;
  return h;
}
