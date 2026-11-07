import type { SelectItem } from "./ui";

export type UpdateItem = {
  id: string;
  name: string;
  manager: string;
  current?: string | null;
  available?: string | null;
  size?: number | null;
  admin?: boolean;
};

export type ManagerError = { manager: string; error: string };

export type UpdatesList = { items: UpdateItem[]; plan_id: string | null; managers?: string[]; errors?: ManagerError[] };

export function unreadableManagers(list: UpdatesList | null): ManagerError[] {
  return list?.errors ?? [];
}

type UpdateRow = SelectItem & { update: UpdateItem };

const MANAGER_ORDER = ["winget", "homebrew", "brew", "apt", "dnf", "pacman", "zypper", "flatpak", "snap"];

function managerRank(m: string): number {
  const i = MANAGER_ORDER.indexOf(m);
  return i < 0 ? MANAGER_ORDER.length : i;
}

function versionLine(u: Pick<UpdateItem, "current" | "available">): string {
  const cur = (u.current ?? "").trim();
  const next = (u.available ?? "").trim();
  if (cur && next) return `${cur} → ${next}`;
  return next || cur;
}

export function updateRows(items: readonly UpdateItem[]): UpdateRow[] {
  return [...items]
    .sort(
      (a, b) =>
        managerRank(a.manager) - managerRank(b.manager) ||
        a.manager.localeCompare(b.manager) ||
        a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
    )
    .map((u) => ({
      id: u.id,
      label: u.name,
      hint: versionLine(u),
      bytes: u.size ?? undefined,
      group: u.manager,
      update: u,
    }));
}

export function defaultUpdates(items: readonly UpdateItem[]): Set<string> {
  return new Set(items.map((u) => u.id));
}

export function managers(items: readonly UpdateItem[]): { manager: string; count: number }[] {
  const m = new Map<string, number>();
  for (const u of items) m.set(u.manager, (m.get(u.manager) ?? 0) + 1);
  return [...m.entries()]
    .map(([manager, count]) => ({ manager, count }))
    .sort((a, b) => managerRank(a.manager) - managerRank(b.manager) || a.manager.localeCompare(b.manager));
}

export function updatesSummary(items: readonly UpdateItem[], selected: ReadonlySet<string>): { count: number; bytes: number; admin: number } {
  let count = 0;
  let bytes = 0;
  let admin = 0;
  for (const u of items) {
    if (!selected.has(u.id)) continue;
    count++;
    bytes += u.size ?? 0;
    if (u.admin) admin++;
  }
  return { count, bytes, admin };
}

export function managerLabel(m: string): string {
  switch (m) {
    case "brew":
    case "homebrew":
      return "Homebrew";
    case "flatpak":
      return "Flatpak";
    case "snap":
      return "Snap";
    default:
      return m;
  }
}
