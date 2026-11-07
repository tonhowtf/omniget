export type RatioPreset = { value: number; key: string };

export const RATIOS: RatioPreset[] = [
  { value: 1.067, key: "minor_second" },
  { value: 1.125, key: "major_second" },
  { value: 1.2, key: "minor_third" },
  { value: 1.25, key: "major_third" },
  { value: 1.333, key: "perfect_fourth" },
  { value: 1.414, key: "augmented_fourth" },
  { value: 1.5, key: "perfect_fifth" },
  { value: 1.618, key: "golden_ratio" },
];

export type ScaleOptions = {
  base: number;
  ratio: number;
  stepsDown: number;
  stepsUp: number;
  fluid: boolean;
  maxBase: number;
  maxRatio: number;
  minViewport: number;
  maxViewport: number;
  rootSize?: number;
};

export type ScaleStep = {
  name: string;
  exponent: number;
  min: number;
  max: number;
  value: string;
  lineHeight: number;
};

export function round(value: number, places = 4): number {
  const f = 10 ** places;
  return Math.round(value * f) / f;
}

export function num(value: number, places = 4): string {
  const r = round(value, places);
  return Object.is(r, -0) ? "0" : String(r);
}

export function sizeAt(base: number, ratio: number, exponent: number): number {
  return base * ratio ** exponent;
}

export function stepName(exponent: number): string {
  if (exponent === 0) return "base";
  if (exponent === -1) return "sm";
  if (exponent === -2) return "xs";
  if (exponent < -2) return `${-exponent - 1}xs`;
  if (exponent === 1) return "lg";
  if (exponent === 2) return "xl";
  return `${exponent - 1}xl`;
}

export function rem(px: number, rootSize = 16): string {
  return `${num(px / rootSize)}rem`;
}

export function fluidClamp(minPx: number, maxPx: number, minViewport: number, maxViewport: number, rootSize = 16): string {
  if (round(minPx, 3) === round(maxPx, 3) || maxViewport <= minViewport) return rem(minPx, rootSize);
  const slope = (maxPx - minPx) / (maxViewport - minViewport);
  const intercept = minPx - slope * minViewport;
  const lo = Math.min(minPx, maxPx);
  const hi = Math.max(minPx, maxPx);
  const offset = round(intercept / rootSize);
  const middle = offset === 0 ? `${num(slope * 100)}vw` : `${num(slope * 100)}vw ${offset < 0 ? "-" : "+"} ${rem(Math.abs(intercept), rootSize)}`;
  return `clamp(${rem(lo, rootSize)}, ${middle}, ${rem(hi, rootSize)})`;
}

export function lineHeightFor(px: number): number {
  if (px <= 18) return 1.5;
  if (px <= 24) return 1.4;
  if (px <= 36) return 1.25;
  if (px <= 56) return 1.15;
  return 1.05;
}

export function buildScale(options: ScaleOptions): ScaleStep[] {
  const rootSize = options.rootSize ?? 16;
  const steps: ScaleStep[] = [];
  for (let exponent = -Math.max(0, options.stepsDown); exponent <= Math.max(0, options.stepsUp); exponent++) {
    const min = sizeAt(options.base, options.ratio, exponent);
    const max = options.fluid ? sizeAt(options.maxBase, options.maxRatio, exponent) : min;
    const value = options.fluid ? fluidClamp(min, max, options.minViewport, options.maxViewport, rootSize) : rem(min, rootSize);
    steps.push({ name: stepName(exponent), exponent, min: round(min, 2), max: round(max, 2), value, lineHeight: lineHeightFor(Math.max(min, max)) });
  }
  return steps;
}

export function cssString(value: string): string {
  return `"${value.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
}

export function fontStackValue(family: string | null, fallback: string): string {
  return family ? `${cssString(family)}, ${fallback}` : fallback;
}

export function toCssVariables(steps: ScaleStep[], family: string | null = null, fallback = "system-ui, sans-serif"): string {
  const lines = [":root {"];
  if (family) lines.push(`  --font-body: ${fontStackValue(family, fallback)};`);
  for (const step of steps) {
    lines.push(`  --text-${step.name}: ${step.value};`);
    lines.push(`  --leading-${step.name}: ${step.lineHeight};`);
  }
  lines.push("}");
  return lines.join("\n");
}

export function tailwindFontToken(category: string): "sans" | "serif" | "mono" {
  if (category === "monospace") return "mono";
  if (category === "serif") return "serif";
  return "sans";
}

const TAILWIND_FALLBACK = { sans: "ui-sans-serif, system-ui, sans-serif", serif: "ui-serif, Georgia, serif", mono: "ui-monospace, monospace" };

export function toTailwindTheme(steps: ScaleStep[], family: string | null = null, category = ""): string {
  const lines = ["@theme {"];
  const token = tailwindFontToken(category);
  if (family) lines.push(`  --font-${token}: ${fontStackValue(family, TAILWIND_FALLBACK[token])};`);
  for (const step of steps) {
    lines.push(`  --text-${step.name}: ${step.value};`);
    lines.push(`  --text-${step.name}--line-height: ${step.lineHeight};`);
  }
  lines.push("}");
  return lines.join("\n");
}
