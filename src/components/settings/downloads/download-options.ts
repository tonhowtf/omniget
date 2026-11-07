export const QUALITY_CARDS = ["best", "1080p", "720p"] as const;
export const LOWER_QUALITIES = ["480p", "360p"] as const;
export const RULE_QUALITIES = ["best", "1080p", "720p", "480p", "360p"] as const;

export type QualityCard = (typeof QUALITY_CARDS)[number];

export function qualityCard(value: string | null | undefined): QualityCard | "" {
  return (QUALITY_CARDS as readonly string[]).includes(value ?? "") ? (value as QualityCard) : "";
}

export function lowerQuality(value: string | null | undefined): string {
  return (LOWER_QUALITIES as readonly string[]).includes(value ?? "") ? (value as string) : "";
}

export function customQuality(value: string | null | undefined): string {
  const v = (value ?? "").trim();
  if (!v || qualityCard(v) || lowerQuality(v)) return "";
  return v;
}

export function captionLocaleValue(value: string | null | undefined): string {
  const v = (value ?? "").trim();
  return !v || v.toLowerCase() === "auto" ? "auto" : v;
}

export function saveMode(alwaysAsk: boolean): "folder" | "ask" {
  return alwaysAsk ? "ask" : "folder";
}

export function ruleQualityOptions(current: string | null | undefined): string[] {
  const v = (current ?? "").trim();
  const base: string[] = [...RULE_QUALITIES];
  return v && !base.includes(v) ? [...base, v] : base;
}
