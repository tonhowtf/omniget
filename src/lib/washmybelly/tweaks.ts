/// Windows tweaks from `wmb_tweaks_list` and the pure logic of the screen:
/// sections by category, exclusive groups as radios, what can be picked,
/// and which plan items a selection becomes.

import type { Risk } from "./api";

export type TweakState = "applied" | "not_applied" | "partial" | "unknown";

export type TweakItem = {
  id: string;
  item_id: string;
  default_item_id?: string | null;
  label: string;
  label_key?: string | null;
  description: string;
  category: string;
  group?: string | null;
  risk: Risk;
  admin: boolean;
  default_admin?: boolean;
  needs_restart: boolean;
  warning?: string | null;
  source: string;
  license: string;
  state: TweakState | string;
  available: boolean;
  reason?: string | null;
  has_default: boolean;
};

export type TweakGroup = {
  id: string;
  label: string;
  description: string;
  members: string[];
  default?: string | null;
  active?: string | null;
};

export type TweakList = {
  build: number;
  edition: string;
  categories: string[];
  groups: TweakGroup[];
  items: TweakItem[];
  plan_id: string | null;
  restore_point?: boolean;
};

export type Row = { kind: "item"; item: TweakItem } | { kind: "group"; group: TweakGroup; items: TweakItem[] };

/// The order categories are shown in (the list only sends the ones present).
export const CATEGORY_ORDER = [
  "performance",
  "gaming",
  "taskbar_start",
  "appearance",
  "explorer",
  "bloat",
  "ai",
  "updates",
  "network",
  "system",
  "features",
  "hardening",
];

export function orderedCategories(list: TweakList | null): string[] {
  if (!list) return [];
  return [...list.categories].sort((a, b) => rank(a) - rank(b));
}

function rank(c: string): number {
  const i = CATEGORY_ORDER.indexOf(c);
  return i < 0 ? CATEGORY_ORDER.length : i;
}

function matches(it: TweakItem, q: string): boolean {
  if (!q) return true;
  const s = q.toLowerCase();
  return it.label.toLowerCase().includes(s) || it.description.toLowerCase().includes(s);
}

/// Rows of one category (or every category with `null`): plain items, and
/// each exclusive group once, where its first member would be.
export function rowsFor(list: TweakList | null, category: string | null, query = ""): Row[] {
  if (!list) return [];
  const q = query.trim();
  const byGroup = new Map(list.groups.map((g) => [g.id, g]));
  const rows: Row[] = [];
  const seen = new Set<string>();
  const items = list.items.filter((i) => category === null || i.category === category);
  for (const it of items) {
    if (it.group && byGroup.has(it.group)) {
      if (seen.has(it.group)) continue;
      const g = byGroup.get(it.group)!;
      const members = g.members.map((m) => items.find((x) => x.id === m)).filter((x): x is TweakItem => !!x);
      if (q && !members.some((m) => matches(m, q)) && !g.label.toLowerCase().includes(q.toLowerCase())) continue;
      seen.add(it.group);
      rows.push({ kind: "group", group: g, items: members });
      continue;
    }
    if (matches(it, q)) rows.push({ kind: "item", item: it });
  }
  return rows;
}

/// Can be picked: offered on this machine and not already in place.
export function canPick(it: TweakItem): boolean {
  return it.available && it.state !== "applied";
}

/// Pick or unpick one tweak. Picking a group member drops the other
/// members: only one of them can be in place.
export function togglePick(list: TweakList, selected: Set<string>, it: TweakItem): Set<string> {
  const next = new Set(selected);
  if (next.has(it.id)) {
    next.delete(it.id);
    return next;
  }
  if (!canPick(it)) return next;
  if (it.group) {
    const g = list.groups.find((x) => x.id === it.group);
    for (const m of g?.members ?? []) next.delete(m);
  }
  next.add(it.id);
  return next;
}

/// Plan item ids for `wmb_plan` (kind `tweaks`), picked items only.
export function pickedItemIds(list: TweakList | null, selected: Set<string>): string[] {
  if (!list) return [];
  return list.items.filter((i) => selected.has(i.id) && canPick(i)).map((i) => i.item_id);
}

/// Picked items that need their own warning first (memory integrity).
export function extremePicks(list: TweakList | null, selected: Set<string>): TweakItem[] {
  if (!list) return [];
  return list.items.filter((i) => selected.has(i.id) && i.risk === "extreme");
}

export function pickedNeedRestart(list: TweakList | null, selected: Set<string>): boolean {
  return !!list?.items.some((i) => selected.has(i.id) && i.needs_restart);
}

/// Drop picks that are gone or no longer pickable after a reload.
export function prunePicks(list: TweakList, selected: Set<string>): Set<string> {
  const ok = new Set(list.items.filter(canPick).map((i) => i.id));
  return new Set([...selected].filter((id) => ok.has(id)));
}

export function appliedCount(list: TweakList | null, category: string | null = null): number {
  if (!list) return 0;
  return list.items.filter((i) => (category === null || i.category === category) && i.state === "applied").length;
}

/// "Back to the Windows default" is offered once the tweak is (partly) in place.
export function canRevert(it: TweakItem): boolean {
  return it.has_default && !!it.default_item_id && (it.state === "applied" || it.state === "partial") && it.reason !== "unreadable";
}

const REASONS = ["edition", "no_dedicated_gpu", "virtual_machine", "old_driver", "unknown", "needs_helper_update", "unreadable"];

export function reasonKey(reason: string | null | undefined): string | null {
  if (!reason) return null;
  return `washmybelly.tweaks.reason.${REASONS.includes(reason) ? reason : "unknown"}`;
}

export function stateKey(state: string): string {
  return `washmybelly.tweaks.state.${["applied", "not_applied", "partial", "unknown"].includes(state) ? state : "unknown"}`;
}

const SOURCES: Record<string, string> = {
  win11debloat: "Win11Debloat",
  winutil: "winutil",
  sophia: "Sophia Script",
  hardentools: "hardentools",
  omniget: "OmniGet",
};

export function sourceName(source: string): string {
  return SOURCES[source] ?? source;
}

/// Drop the pick of one exclusive group (back to what Windows has now).
export function clearGroup(list: TweakList, selected: Set<string>, groupId: string): Set<string> {
  const g = list.groups.find((x) => x.id === groupId);
  const members = new Set(g?.members ?? []);
  return new Set([...selected].filter((id) => !members.has(id)));
}
