import type { IconSet } from "./types";

export const RECOMMENDED = ["lucide", "ph", "tabler", "hugeicons", "heroicons", "material-symbols", "simple-icons", "iconoir", "ri"] as const;

export const STROKE_SETS = new Set(["lucide", "tabler", "iconoir", "hugeicons", "humbleicons", "majesticons", "mingcute", "basil"]);

export type LicenseKind = "open" | "attribution" | "copyleft" | "restricted" | "unknown";

const OPEN = new Set(["MIT", "ISC", "Apache-2.0", "CC0-1.0", "Unlicense", "OFL-1.1", "MPL-2.0", "BSD-2-Clause", "BSD-3-Clause", "0BSD"]);

export function licenseKind(spdx: string | null | undefined): LicenseKind {
  if (!spdx) return "unknown";
  const s = spdx.trim();
  if (OPEN.has(s)) return "open";
  if (/-NC\b|-NC-|-ND\b|-ND-/i.test(s)) return "restricted";
  if (/^CC-BY/i.test(s)) return "attribution";
  if (/^(L?GPL|AGPL)/i.test(s)) return "copyleft";
  return "unknown";
}

export function needsWarning(kind: LicenseKind): boolean {
  return kind !== "open";
}

export function licenseLabel(set: Pick<IconSet, "license" | "license_spdx"> | null | undefined): string {
  return set?.license_spdx ?? set?.license ?? "?";
}

export const CATEGORY_ORDER = [
  "interface",
  "interface_mixed",
  "interface_sized",
  "material",
  "colorful",
  "brands",
  "developer",
  "emoji",
  "flags",
  "places",
  "themed",
  "seasonal",
  "detailed",
  "apps",
  "other",
  "legacy",
] as const;

export type CategoryKey = (typeof CATEGORY_ORDER)[number];

const CATEGORY_MAP: Record<string, CategoryKey> = {
  "UI 24px": "interface",
  "UI Other / Mixed Grid": "interface_mixed",
  "UI 16px / 32px": "interface_sized",
  Material: "material",
  "UI Multicolor": "colorful",
  Logos: "brands",
  Programming: "developer",
  Emoji: "emoji",
  Flags: "flags",
  Location: "places",
  Thematic: "themed",
  "Holiday / Seasonal": "seasonal",
  "High Detail": "detailed",
  "High Detail Multicolor": "detailed",
  Apps: "apps",
  "Archive / Unmaintained": "legacy",
};

export function categoryOf(category: string | null | undefined): CategoryKey {
  return (category && CATEGORY_MAP[category]) || "other";
}

export function groupSets(sets: IconSet[], filter = ""): { key: CategoryKey; sets: IconSet[] }[] {
  const q = filter.trim().toLowerCase();
  const groups = new Map<CategoryKey, IconSet[]>();
  for (const set of sets) {
    if (q && !set.name.toLowerCase().includes(q) && !set.prefix.includes(q)) continue;
    const key = categoryOf(set.category);
    const list = groups.get(key) ?? [];
    list.push(set);
    groups.set(key, list);
  }
  return CATEGORY_ORDER.filter((k) => groups.has(k)).map((key) => ({
    key,
    sets: groups.get(key)!.sort((a, b) => b.total - a.total || a.name.localeCompare(b.name)),
  }));
}

export function orderRecommended(sets: IconSet[]): IconSet[] {
  const byPrefix = new Map(sets.map((s) => [s.prefix, s]));
  return RECOMMENDED.map((p) => byPrefix.get(p)).filter((s): s is IconSet => Boolean(s));
}

const PACKAGES: Record<string, string> = {
  lucide: "lucide-svelte",
  ph: "phosphor-svelte",
  tabler: "@tabler/icons-svelte",
  hugeicons: "@hugeicons/svelte @hugeicons/core-free-icons",
  heroicons: "heroicons",
  "material-symbols": "material-symbols",
  "simple-icons": "simple-icons",
  iconoir: "iconoir",
  ri: "remixicon",
  mdi: "@mdi/js",
  bi: "bootstrap-icons",
  "fa6-solid": "@fortawesome/free-solid-svg-icons",
  "fa6-regular": "@fortawesome/free-regular-svg-icons",
  "fa6-brands": "@fortawesome/free-brands-svg-icons",
  octicon: "@primer/octicons",
  "radix-icons": "@radix-icons/svelte",
  feather: "feather-icons",
  ion: "ionicons",
  carbon: "carbon-icons-svelte",
  fluent: "@fluentui/svg-icons",
};

export function npmPackage(prefix: string): string | null {
  return PACKAGES[prefix] ?? null;
}

export function installCommand(prefix: string): string | null {
  const pkg = npmPackage(prefix);
  return pkg ? `npm install ${pkg}` : null;
}

export function iconifyId(prefix: string, name: string): string {
  return `${prefix}:${name}`;
}

export function splitId(id: string): { prefix: string; name: string } | null {
  const i = id.indexOf(":");
  if (i <= 0 || i === id.length - 1) return null;
  return { prefix: id.slice(0, i), name: id.slice(i + 1) };
}

const GENERIC = new Set(["solid", "outline", "line", "fill", "filled", "bold", "light", "thin", "duotone", "regular", "rounded", "sharp", "outlined", "twotone", "16", "20", "24", "32"]);

export function relatedQuery(name: string): string {
  const tokens = name.split(/[-_]/).filter((t) => t && !GENERIC.has(t) && !/^\d+$/.test(t));
  const pick = tokens.find((t) => t.length >= 3) ?? tokens[0];
  return pick ?? name;
}

export function isStrokeSet(prefix: string): boolean {
  return STROKE_SETS.has(prefix);
}
