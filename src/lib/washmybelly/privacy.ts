import type { Risk } from "./api";

/// Privacy settings tab: the catalog compiled from privacy.sexy
/// (`wmb_privacy_catalog`) and the pure logic the screen needs.

export type PrivacyState = "applied" | "not_applied" | "unknown";
export type Recommend = "standard" | "strict" | null;
export type Preset = "standard" | "strict";

export type PrivacyItem = {
  id: string;
  category: string;
  name: string;
  docs?: string | null;
  recommend: Recommend;
  risk: Risk;
  state: PrivacyState;
  reason?: string | null;
  admin: boolean;
  restart: boolean;
  code_only?: boolean;
  code?: string | null;
};

export type PrivacyCatalog = {
  items: PrivacyItem[];
  plan_id?: string | null;
  source_url: string;
  license: string;
};

export type CategoryGroup = {
  category: string;
  items: PrivacyItem[];
  applied: number;
  /// Items that can be read and are not applied yet.
  pending: number;
};

/// An item the person can tick: its state is known, it is not applied yet,
/// and OmniGet can run it (not "code only").
export function isActionable(it: PrivacyItem): boolean {
  return it.state === "not_applied" && !it.code_only;
}

/// Items that lower security (`extreme`) never show until the person asks
/// for them, and never join a preset.
export function isExtreme(it: PrivacyItem): boolean {
  return it.risk === "extreme";
}

export function inPreset(it: PrivacyItem, preset: Preset): boolean {
  if (isExtreme(it)) return false;
  return preset === "standard" ? it.recommend === "standard" : it.recommend === "standard" || it.recommend === "strict";
}

/// Ids a preset button applies: recommended, actionable, never extreme.
export function presetIds(items: readonly PrivacyItem[], preset: Preset): string[] {
  return items.filter((it) => inPreset(it, preset) && isActionable(it)).map((it) => it.id);
}

export type Filter = {
  recommendedOnly?: boolean;
  showExtreme?: boolean;
  query?: string;
};

export function filterItems(items: readonly PrivacyItem[], f: Filter): PrivacyItem[] {
  const q = (f.query ?? "").trim().toLowerCase();
  return items.filter((it) => {
    if (isExtreme(it) && !f.showExtreme) return false;
    if (f.recommendedOnly && !it.recommend) return false;
    if (q && !it.name.toLowerCase().includes(q) && !it.category.toLowerCase().includes(q)) return false;
    return true;
  });
}

/// Groups by category, keeping the catalog's order of first appearance.
export function groupByCategory(items: readonly PrivacyItem[]): CategoryGroup[] {
  const map = new Map<string, CategoryGroup>();
  for (const it of items) {
    let g = map.get(it.category);
    if (!g) {
      g = { category: it.category, items: [], applied: 0, pending: 0 };
      map.set(it.category, g);
    }
    g.items.push(it);
    if (it.state === "applied") g.applied++;
    if (isActionable(it)) g.pending++;
  }
  return [...map.values()];
}

/// The last part of a "A / B / C" category path, for a compact heading.
export function categoryTitle(category: string): string {
  const parts = category.split(/\s*[/>]\s*/).filter(Boolean);
  return parts[parts.length - 1] ?? category;
}

export function categoryParent(category: string): string | null {
  const parts = category.split(/\s*[/>]\s*/).filter(Boolean);
  return parts.length > 1 ? parts.slice(0, -1).join(" / ") : null;
}

/// Keeps only ids that are still tickable after a reload.
export function pruneSelection(selected: ReadonlySet<string>, items: readonly PrivacyItem[]): Set<string> {
  const ok = new Set(items.filter(isActionable).map((i) => i.id));
  return new Set([...selected].filter((id) => ok.has(id)));
}

/// Extreme items are left out of "pending": they are never a to-do.
export function counts(items: readonly PrivacyItem[]): { applied: number; pending: number; unknown: number; recommendedPending: number } {
  let applied = 0;
  let pending = 0;
  let unknown = 0;
  let recommendedPending = 0;
  for (const it of items) {
    if (it.state === "applied") applied++;
    else if (it.state === "unknown") unknown++;
    if (isActionable(it) && !isExtreme(it)) {
      pending++;
      if (inPreset(it, "strict")) recommendedPending++;
    }
  }
  return { applied, pending, unknown, recommendedPending };
}

/// i18n key of an "unknown" reason the backend sends (`needs_admin`,
/// `not_readable`...), or null to show the raw text.
export function reasonKey(reason: string | null | undefined): string | null {
  if (!reason) return null;
  return /^[a-z_]+$/.test(reason) ? `washmybelly.privacy.reason.${reason}` : null;
}
