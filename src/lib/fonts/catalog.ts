import type { CatalogFont, InstalledFamily } from "./types";

export const CATEGORIES = ["sans-serif", "serif", "monospace", "display", "handwriting", "icons", "other"] as const;
export type CategoryId = (typeof CATEGORIES)[number];

export const CATEGORY_KEYS: Record<CategoryId, string> = {
  "sans-serif": "fonts.cat_sans",
  serif: "fonts.cat_serif",
  monospace: "fonts.cat_mono",
  display: "fonts.cat_display",
  handwriting: "fonts.cat_script",
  icons: "fonts.cat_icons",
  other: "fonts.cat_other",
};

export function categoryKey(category: string): string {
  return CATEGORY_KEYS[(CATEGORIES as readonly string[]).includes(category) ? (category as CategoryId) : "other"];
}

type Curated = { id: string; family: string; category: CategoryId; by: string; weights: number[]; italic: boolean; variable: boolean };

const ALL = [100, 200, 300, 400, 500, 600, 700, 800, 900];

const CURATED: Curated[] = [
  { id: "geist", family: "Geist", category: "sans-serif", by: "Vercel", weights: ALL, italic: true, variable: true },
  { id: "geist-mono", family: "Geist Mono", category: "monospace", by: "Vercel", weights: ALL, italic: true, variable: true },
  { id: "inter", family: "Inter", category: "sans-serif", by: "Rasmus Andersson", weights: ALL, italic: true, variable: true },
  { id: "jetbrains-mono", family: "JetBrains Mono", category: "monospace", by: "JetBrains", weights: [100, 200, 300, 400, 500, 600, 700, 800], italic: true, variable: true },
  { id: "ibm-plex-sans", family: "IBM Plex Sans", category: "sans-serif", by: "IBM", weights: [100, 200, 300, 400, 500, 600, 700], italic: true, variable: true },
  { id: "ibm-plex-mono", family: "IBM Plex Mono", category: "monospace", by: "IBM", weights: [100, 200, 300, 400, 500, 600, 700], italic: true, variable: false },
  { id: "source-sans-3", family: "Source Sans 3", category: "sans-serif", by: "Adobe", weights: [200, 300, 400, 500, 600, 700, 800, 900], italic: true, variable: true },
  { id: "fira-code", family: "Fira Code", category: "monospace", by: "Nikita Prokopov", weights: [300, 400, 500, 600, 700], italic: false, variable: true },
  { id: "atkinson-hyperlegible", family: "Atkinson Hyperlegible", category: "sans-serif", by: "Braille Institute", weights: [400, 700], italic: true, variable: false },
  { id: "manrope", family: "Manrope", category: "sans-serif", by: "Mikhail Sharanda", weights: [200, 300, 400, 500, 600, 700, 800], italic: false, variable: true },
  { id: "space-grotesk", family: "Space Grotesk", category: "sans-serif", by: "Florian Karsten", weights: [300, 400, 500, 600, 700], italic: false, variable: true },
  { id: "noto-sans", family: "Noto Sans", category: "sans-serif", by: "Google", weights: ALL, italic: true, variable: true },
];

export const CURATED_IDS: string[] = CURATED.map((c) => c.id);

export function curatedBy(id: string): string | null {
  return CURATED.find((c) => c.id === id)?.by ?? null;
}

export function curatedFallback(): CatalogFont[] {
  return CURATED.map((c) => ({
    id: c.id,
    family: c.family,
    category: c.category,
    subsets: ["latin", "latin-ext"],
    default_subset: "latin",
    weights: c.weights,
    styles: c.italic ? ["normal", "italic"] : ["normal"],
    variable: c.variable,
    license: "OFL-1.1",
    license_class: "open",
    source: "other",
  }));
}

export function normalize(text: string): string {
  return text
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, " ")
    .trim();
}

export type CatalogFilter = { query: string; category: string | null; includeOther: boolean };

function score(font: CatalogFont, q: string): number {
  const name = normalize(font.family);
  const curated = CURATED_IDS.indexOf(font.id);
  if (!q) return curated >= 0 ? curated : 1000;
  if (name === q) return 0;
  if (name.startsWith(q)) return 10 + (curated >= 0 ? 0 : 1);
  if (name.split(" ").some((w) => w.startsWith(q))) return 20;
  if (name.includes(q) || font.id.includes(q.replace(/ /g, "-"))) return 30;
  return -1;
}

export function filterCatalog(fonts: CatalogFont[], filter: CatalogFilter): CatalogFont[] {
  const q = normalize(filter.query);
  const ranked: { font: CatalogFont; s: number }[] = [];
  for (const font of fonts) {
    if (!filter.includeOther && font.license_class !== "open") continue;
    if (filter.category && font.category !== filter.category) continue;
    const s = score(font, q);
    if (s >= 0) ranked.push({ font, s });
  }
  ranked.sort((a, b) => a.s - b.s || a.font.family.localeCompare(b.font.family));
  return ranked.map((r) => r.font);
}

export function mergeCatalog(remote: CatalogFont[], fallback: CatalogFont[] = curatedFallback()): CatalogFont[] {
  const ids = new Set(remote.map((f) => f.id));
  return [...remote, ...fallback.filter((f) => !ids.has(f.id))];
}

export function hiddenCount(fonts: CatalogFont[], filter: CatalogFilter): number {
  if (filter.includeOther) return 0;
  return filterCatalog(fonts, { ...filter, includeOther: true }).length - filterCatalog(fonts, filter).length;
}

export function installedNames(families: InstalledFamily[]): Set<string> {
  return new Set(families.map((f) => normalize(f.family)));
}

export function isInstalled(font: CatalogFont, names: Set<string>): boolean {
  return names.has(normalize(font.family));
}

export function categoryIndex(catalog: CatalogFont[]): Map<string, string> {
  const index = new Map<string, string>();
  for (const f of catalog) index.set(normalize(f.family), f.category);
  return index;
}

export function installedCategory(family: InstalledFamily, index: Map<string, string>): string {
  const hit = index.get(normalize(family.family));
  if (hit) return hit;
  if (family.is_monospace) return "monospace";
  return "";
}

export function filterInstalled(families: InstalledFamily[], query: string, category: string | null, index: Map<string, string>): InstalledFamily[] {
  const q = normalize(query);
  return families.filter((f) => {
    if (q && !normalize(f.family).includes(q)) return false;
    if (!category) return true;
    if (category === "monospace") return f.is_monospace || installedCategory(f, index) === "monospace";
    return installedCategory(f, index) === category;
  });
}
