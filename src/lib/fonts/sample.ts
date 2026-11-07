export const PRESETS = ["pangram", "paragraph", "ui", "code"] as const;
export type PresetId = (typeof PRESETS)[number];

export type Theme = "dark" | "light";

export type Sample = {
  preset: PresetId;
  text: string;
  size: number;
  weight: number;
  italic: boolean;
  lineHeight: number;
  tracking: number;
  theme: Theme;
};

export const CODE_SAMPLE = `const scale = (base, ratio, step) =>
  base * ratio ** step; // 0O 1lI {}[] => != ===`;

export const GLYPHS = "ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz 0123456789 &@?!%";

export const LIMITS = {
  size: { min: 8, max: 160, step: 1 },
  weight: { min: 100, max: 900, step: 1 },
  lineHeight: { min: 0.8, max: 2.4, step: 0.05 },
  tracking: { min: -0.1, max: 0.3, step: 0.005 },
};

export function defaultSample(): Sample {
  return { preset: "pangram", text: "", size: 48, weight: 400, italic: false, lineHeight: 1.2, tracking: 0, theme: "dark" };
}

export function presetDefaults(preset: PresetId): Pick<Sample, "size" | "lineHeight"> {
  if (preset === "paragraph") return { size: 17, lineHeight: 1.6 };
  if (preset === "ui") return { size: 15, lineHeight: 1.45 };
  if (preset === "code") return { size: 15, lineHeight: 1.6 };
  return { size: 48, lineHeight: 1.2 };
}

export function clampTo(value: number, limit: { min: number; max: number }): number {
  if (!Number.isFinite(value)) return limit.min;
  return Math.min(limit.max, Math.max(limit.min, value));
}

export function trackingLabel(em: number): string {
  const r = Math.round(em * 1000) / 1000;
  return `${r > 0 ? "+" : ""}${r}em`;
}
