import type { FontDetails } from "./types";

const KNOWN_SUBSETS = new Set([
  "latin",
  "latin-ext",
  "cyrillic",
  "cyrillic-ext",
  "greek",
  "greek-ext",
  "vietnamese",
  "arabic",
  "hebrew",
  "devanagari",
  "thai",
  "japanese",
  "korean",
  "chinese-simplified",
  "chinese-traditional",
  "math",
  "symbols",
]);

export function hasFullFiles(details: Pick<FontDetails, "source"> | null): boolean {
  return details?.source === "google";
}

export function subsetOptions(details: Pick<FontDetails, "subsets" | "files"> | null): string[] {
  if (!details) return [];
  const withFiles = new Set(details.files.filter((f) => f.ttf).map((f) => f.subset));
  return details.subsets.filter((s) => withFiles.has(s));
}

export function pickSubset(details: Pick<FontDetails, "subsets" | "files" | "default_subset">): string {
  const options = subsetOptions(details);
  if (options.includes(details.default_subset)) return details.default_subset;
  if (options.includes("latin")) return "latin";
  return options[0] ?? details.default_subset ?? "latin";
}

export function plannedCount(details: Pick<FontDetails, "files" | "source"> | null, subset: string): number {
  if (!details) return 0;
  if (hasFullFiles(details)) return new Set(details.files.map((f) => `${f.weight}:${f.style}`)).size;
  return details.files.filter((f) => f.subset === subset && f.ttf).length;
}

export function subsetKey(subset: string): string | null {
  return KNOWN_SUBSETS.has(subset) ? `fonts.subset_${subset.replace(/-/g, "_")}` : null;
}
