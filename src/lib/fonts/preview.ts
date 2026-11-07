import { cssString } from "./scale";
import type { CatalogFont, InstalledFamily } from "./types";

export const CDN_BASE = "https://cdn.jsdelivr.net/fontsource/fonts";

export const MAX_COMPARE = 3;

export const UNICODE_RANGES: Record<string, string> = {
  latin: "U+0000-00FF,U+0131,U+0152-0153,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+0304,U+0308,U+0329,U+2000-206F,U+20AC,U+2122,U+2191,U+2193,U+2212,U+2215,U+FEFF,U+FFFD",
  "latin-ext": "U+0100-02BA,U+02BD-02C5,U+02C7-02CC,U+02CE-02D7,U+02DD-02FF,U+0304,U+0308,U+0329,U+1D00-1DBF,U+1E00-1E9F,U+1EF2-1EFF,U+2020,U+20A0-20AB,U+20AD-20C0,U+2113,U+2C60-2C7F,U+A720-A7FF",
  cyrillic: "U+0301,U+0400-045F,U+0490-0491,U+04B0-04B1,U+2116",
  "cyrillic-ext": "U+0460-052F,U+1C80-1C8A,U+20B4,U+2DE0-2DFF,U+A640-A69F,U+FE2E-FE2F",
  greek: "U+0370-0377,U+037A-037F,U+0384-038A,U+038C,U+038E-03A1,U+03A3-03FF",
  vietnamese: "U+0102-0103,U+0110-0111,U+0128-0129,U+0168-0169,U+01A0-01A1,U+01AF-01B0,U+0300-0301,U+0303-0304,U+0308-0309,U+0323,U+0329,U+1EA0-1EF9,U+20AB",
};

const LANG_SUBSETS: Record<string, string[]> = {
  pt: ["latin-ext"],
  es: ["latin-ext"],
  fr: ["latin-ext"],
  it: ["latin-ext"],
  ru: ["cyrillic", "cyrillic-ext"],
  el: ["greek"],
  ja: ["japanese"],
  zh: ["chinese-simplified"],
  "zh-TW": ["chinese-traditional"],
  fa: ["arabic"],
  lo: ["lao"],
};

export function cdnUrl(id: string, subset: string, weight: number, style: string, ext: "woff2" | "ttf" = "woff2"): string {
  return `${CDN_BASE}/${id}@latest/${subset}-${weight}-${style}.${ext}`;
}

export function variableUrl(id: string, subset: string, style: string, axis = "wght"): string {
  return `${CDN_BASE}/${id}:vf@latest/${subset}-${axis}-${style}.woff2`;
}

export function aliasFor(id: string): string {
  return `OmniGet Preview ${id}`;
}

export function previewSubsets(font: Pick<CatalogFont, "subsets" | "default_subset">, lang: string): string[] {
  const wanted = [font.default_subset, ...(LANG_SUBSETS[lang] ?? LANG_SUBSETS[lang.split("-")[0]] ?? [])];
  const out: string[] = [];
  for (const s of wanted) if (font.subsets.includes(s) && !out.includes(s)) out.push(s);
  if (out.length === 0 && font.subsets.length) out.push(font.subsets[0]);
  return out;
}

export function nearestWeight(weights: number[], wanted: number): number {
  if (weights.length === 0) return 400;
  return weights.reduce((best, w) => {
    const d = Math.abs(w - wanted);
    const bd = Math.abs(best - wanted);
    return d < bd || (d === bd && w > best) ? w : best;
  }, weights[0]);
}

export function weightRange(weights: number[]): [number, number] {
  if (weights.length === 0) return [400, 400];
  return [Math.min(...weights), Math.max(...weights)];
}

export function fallbackFor(category: string): string {
  if (category === "monospace") return "ui-monospace, monospace";
  if (category === "serif") return "ui-serif, Georgia, serif";
  if (category === "handwriting") return "cursive";
  return "system-ui, sans-serif";
}

export type PreviewFont = {
  key: string;
  source: "catalog" | "installed";
  id: string | null;
  family: string;
  cssFamily: string;
  category: string;
  weights: number[];
  italic: boolean;
  variable: boolean;
  wghtRange: [number, number] | null;
  subsets: string[];
  defaultSubset: string;
  license: string | null;
};

export function fromCatalog(font: CatalogFont): PreviewFont {
  return {
    key: `c:${font.id}`,
    source: "catalog",
    id: font.id,
    family: font.family,
    cssFamily: aliasFor(font.id),
    category: font.category,
    weights: font.weights.length ? [...font.weights].sort((a, b) => a - b) : [400],
    italic: font.styles.includes("italic"),
    variable: font.variable,
    wghtRange: font.variable ? weightRange(font.weights) : null,
    subsets: font.subsets,
    defaultSubset: font.default_subset,
    license: font.license,
  };
}

export function fromInstalled(family: InstalledFamily, category: string): PreviewFont {
  const wght = family.axes.find((a) => a.tag === "wght");
  return {
    key: `i:${family.family}`,
    source: "installed",
    id: null,
    family: family.family,
    cssFamily: family.family,
    category: category || (family.is_monospace ? "monospace" : ""),
    weights: family.weights.length ? family.weights : [400],
    italic: family.has_italic,
    variable: family.is_variable,
    wghtRange: wght ? [wght.min, wght.max] : null,
    subsets: [],
    defaultSubset: "",
    license: null,
  };
}

export function fontStack(font: PreviewFont | null): string {
  if (!font) return "var(--font-body)";
  return `${cssString(font.cssFamily)}, ${fallbackFor(font.category)}`;
}

export function snapWeight(font: PreviewFont, wanted: number): number {
  if (font.wghtRange) return Math.min(font.wghtRange[1], Math.max(font.wghtRange[0], Math.round(wanted)));
  return nearestWeight(font.weights, wanted);
}

export function toggleCompare(list: string[], key: string, max = MAX_COMPARE): string[] {
  if (list.includes(key)) return list.filter((k) => k !== key);
  if (list.length >= max) return list;
  return [...list, key];
}
