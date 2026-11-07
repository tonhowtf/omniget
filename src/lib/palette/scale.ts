import { clamp, fitToGamut, normalizeHue, type Gamut, type Oklch } from "./color";

export const STEPS = [50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950] as const;
export type StepName = (typeof STEPS)[number];

export const L_TOP = 0.97;
export const L_BOTTOM = 0.26;

export type ScaleOptions = {
  hueShift?: number;
  gamut?: Gamut;
  top?: number;
  bottom?: number;
};

export type ScaleStep = { step: StepName; color: Oklch; isBase: boolean };

export function evenLightness(top = L_TOP, bottom = L_BOTTOM): number[] {
  const n = STEPS.length - 1;
  return STEPS.map((_, i) => top - ((top - bottom) * i) / n);
}

export function baseIndex(l: number, top = L_TOP, bottom = L_BOTTOM): number {
  const even = evenLightness(top, bottom);
  let best = 0;
  even.forEach((v, i) => {
    if (Math.abs(v - l) < Math.abs(even[best] - l)) best = i;
  });
  return best;
}

export function generateScale(base: Oklch, options: ScaleOptions = {}): ScaleStep[] {
  const hueShift = options.hueShift ?? 0;
  const gamut = options.gamut ?? "srgb";
  const baseL = clamp(base.l, 0.02, 0.995);
  const top = Math.max(options.top ?? L_TOP, baseL);
  const bottom = Math.min(options.bottom ?? L_BOTTOM, baseL);
  const k = baseIndex(baseL, top, bottom);
  const last = STEPS.length - 1;
  return STEPS.map((step, i) => {
    if (i === k) return { step, color: fitToGamut({ ...base, l: baseL, alpha: 1 }, gamut), isBase: true };
    let l: number;
    let t: number;
    let chromaFactor: number;
    if (i < k) {
      t = (k - i) / k;
      l = baseL + (top - baseL) * t;
      chromaFactor = 1 - 0.88 * Math.pow(t, 1.5);
    } else {
      t = (i - k) / (last - k);
      l = baseL - (baseL - bottom) * t;
      chromaFactor = 1 - 0.45 * Math.pow(t, 1.8);
    }
    const h = normalizeHue(base.h + (hueShift * (i - k)) / last);
    const color = fitToGamut({ l, c: base.c * chromaFactor, h, alpha: 1 }, gamut);
    return { step, color, isBase: false };
  });
}

export type HarmonyKind = "complementary" | "analogous" | "triadic";

export const HARMONY_KINDS: HarmonyKind[] = ["complementary", "analogous", "triadic"];

export function harmonyHues(kind: HarmonyKind, hue: number): number[] {
  switch (kind) {
    case "complementary":
      return [normalizeHue(hue + 180)];
    case "analogous":
      return [normalizeHue(hue - 30), normalizeHue(hue + 30)];
    case "triadic":
      return [normalizeHue(hue + 120), normalizeHue(hue + 240)];
  }
}

export function harmony(kind: HarmonyKind, base: Oklch, gamut: Gamut = "srgb"): Oklch[] {
  return harmonyHues(kind, base.h).map((h) => fitToGamut({ l: base.l, c: base.c, h, alpha: 1 }, gamut));
}

export function neutralFrom(base: Oklch): Oklch {
  return { l: base.l, c: Math.min(base.c, 0.018), h: base.h, alpha: 1 };
}
