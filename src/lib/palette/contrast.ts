import { clamp, fitToGamut, srgbToLinear, toRgb255, type Oklch } from "./color";

export type WcagLevel = "aaa" | "aa" | "aa_large" | "fail";

export type ContrastReport = {
  ratio: number;
  apca: number;
  normalAA: boolean;
  normalAAA: boolean;
  largeAA: boolean;
  largeAAA: boolean;
  level: WcagLevel;
};

function blendOver(fg: Oklch, bg: Oklch): [number, number, number] {
  const top = toRgb255({ ...fg, alpha: 1 });
  const bottom = toRgb255({ ...bg, alpha: 1 });
  const a = clamp(fg.alpha ?? 1, 0, 1);
  return [0, 1, 2].map((i) => top[i] * a + bottom[i] * (1 - a)) as [number, number, number];
}

export function relativeLuminance(rgb: [number, number, number]): number {
  const [r, g, b] = rgb.map((v) => srgbToLinear(v / 255));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function wcagRatioRgb(a: [number, number, number], b: [number, number, number]): number {
  const la = relativeLuminance(a);
  const lb = relativeLuminance(b);
  const hi = Math.max(la, lb);
  const lo = Math.min(la, lb);
  return (hi + 0.05) / (lo + 0.05);
}

export function wcagRatio(text: Oklch, background: Oklch): number {
  return wcagRatioRgb(blendOver(text, background), toRgb255({ ...background, alpha: 1 }));
}

function apcaY([r, g, b]: [number, number, number]): number {
  const f = (v: number) => Math.pow(v / 255, 2.4);
  return 0.2126729 * f(r) + 0.7151522 * f(g) + 0.072175 * f(b);
}

export function apcaContrastRgb(text: [number, number, number], background: [number, number, number]): number {
  const blkThrs = 0.022;
  const blkClmp = 1.414;
  const clampBlack = (y: number) => (y > blkThrs ? y : y + Math.pow(blkThrs - y, blkClmp));
  const txtY = clampBlack(apcaY(text));
  const bgY = clampBlack(apcaY(background));
  if (Math.abs(bgY - txtY) < 0.0005) return 0;
  let out: number;
  if (bgY > txtY) {
    const sapc = (Math.pow(bgY, 0.56) - Math.pow(txtY, 0.57)) * 1.14;
    out = sapc < 0.1 ? 0 : sapc - 0.027;
  } else {
    const sapc = (Math.pow(bgY, 0.65) - Math.pow(txtY, 0.62)) * 1.14;
    out = sapc > -0.1 ? 0 : sapc + 0.027;
  }
  return out * 100;
}

export function apcaContrast(text: Oklch, background: Oklch): number {
  return apcaContrastRgb(blendOver(text, background), toRgb255({ ...background, alpha: 1 }));
}

export function levelOf(ratio: number): WcagLevel {
  if (ratio >= 7) return "aaa";
  if (ratio >= 4.5) return "aa";
  if (ratio >= 3) return "aa_large";
  return "fail";
}

const ROUND = (ratio: number) => Math.floor(ratio * 100) / 100;

export function ratioText(ratio: number, digits = 2): string {
  const f = 10 ** digits;
  return (Math.floor(ratio * f + 1e-9) / f).toFixed(digits);
}

export function contrastReport(text: Oklch, background: Oklch): ContrastReport {
  const ratio = ROUND(wcagRatio(text, background));
  return {
    ratio,
    apca: apcaContrast(text, background),
    normalAA: ratio >= 4.5,
    normalAAA: ratio >= 7,
    largeAA: ratio >= 3,
    largeAAA: ratio >= 4.5,
    level: levelOf(ratio),
  };
}

export type FixTarget = "text" | "background";

export type FixSuggestion = { color: Oklch; ratio: number; deltaL: number } | null;

export function suggestFix(text: Oklch, background: Oklch, target: FixTarget = "text", min = 4.5): FixSuggestion {
  const moving = target === "text" ? text : background;
  const test = (l: number) => {
    const candidate = fitToGamut({ ...moving, l }, "srgb");
    const ratio = target === "text" ? wcagRatio(candidate, background) : wcagRatio(text, candidate);
    return { candidate, ratio: ROUND(ratio) };
  };
  if (test(moving.l).ratio >= min) return { color: moving, ratio: test(moving.l).ratio, deltaL: 0 };
  let best: FixSuggestion = null;
  for (const dir of [1, -1]) {
    for (let step = 1; step <= 1000; step++) {
      const l = moving.l + (dir * step) / 1000;
      if (l < 0 || l > 1) break;
      const { candidate, ratio } = test(l);
      if (ratio >= min) {
        const deltaL = Math.abs(l - moving.l);
        if (!best || deltaL < best.deltaL) best = { color: { ...candidate, alpha: moving.alpha }, ratio, deltaL };
        break;
      }
    }
  }
  return best;
}
