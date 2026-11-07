import {
  clamp,
  fitToGamut,
  normalizeHue,
  oklabToOklch,
  oklchToOklab,
  oklchToSrgb,
  srgbToOklch,
  toHex,
  formatOklch,
  type Oklch,
} from "./color";

export type Space = "oklch" | "oklab" | "srgb";
export type EasingKind = "linear" | "ease-in" | "ease-out" | "ease-in-out" | "custom";
export type Bezier = [number, number, number, number];
export type Stop = { color: Oklch; pos: number };
export type GradientDef = {
  stops: Stop[];
  space: Space;
  easing: EasingKind;
  bezier: Bezier;
  angle: number;
  samples: number;
};

export const SPACES: Space[] = ["oklch", "oklab", "srgb"];
export const EASINGS: EasingKind[] = ["linear", "ease-in", "ease-out", "ease-in-out", "custom"];

export const PRESETS: Record<Exclude<EasingKind, "custom">, Bezier> = {
  linear: [0, 0, 1, 1],
  "ease-in": [0.42, 0, 1, 1],
  "ease-out": [0, 0, 0.58, 1],
  "ease-in-out": [0.42, 0, 0.58, 1],
};

export function bezierOf(def: Pick<GradientDef, "easing" | "bezier">): Bezier {
  return def.easing === "custom" ? def.bezier : PRESETS[def.easing];
}

export function cubicBezier([x1, y1, x2, y2]: Bezier): (t: number) => number {
  const cx = 3 * x1;
  const bx = 3 * (x2 - x1) - cx;
  const ax = 1 - cx - bx;
  const cy = 3 * y1;
  const by = 3 * (y2 - y1) - cy;
  const ay = 1 - cy - by;
  const sampleX = (t: number) => ((ax * t + bx) * t + cx) * t;
  const sampleY = (t: number) => ((ay * t + by) * t + cy) * t;
  const slopeX = (t: number) => (3 * ax * t + 2 * bx) * t + cx;
  const solve = (x: number) => {
    let t = x;
    for (let i = 0; i < 8; i++) {
      const err = sampleX(t) - x;
      if (Math.abs(err) < 1e-7) return t;
      const d = slopeX(t);
      if (Math.abs(d) < 1e-6) break;
      t -= err / d;
    }
    let lo = 0;
    let hi = 1;
    t = x;
    for (let i = 0; i < 40; i++) {
      const v = sampleX(t);
      if (Math.abs(v - x) < 1e-7) return t;
      if (v < x) lo = t;
      else hi = t;
      t = (lo + hi) / 2;
    }
    return t;
  };
  return (x: number) => {
    if (x <= 0) return 0;
    if (x >= 1) return 1;
    return sampleY(solve(x));
  };
}

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

function lerpHue(a: number, b: number, t: number): number {
  let d = normalizeHue(b) - normalizeHue(a);
  if (d > 180) d -= 360;
  if (d < -180) d += 360;
  return normalizeHue(a + d * t);
}

export function mix(a: Oklch, b: Oklch, t: number, space: Space): Oklch {
  const alpha = lerp(a.alpha ?? 1, b.alpha ?? 1, t);
  if (space === "oklch") {
    const achroA = a.c < 1e-4;
    const achroB = b.c < 1e-4;
    const ha = achroA ? b.h : a.h;
    const hb = achroB ? a.h : b.h;
    return { l: lerp(a.l, b.l, t), c: lerp(a.c, b.c, t), h: lerpHue(ha, hb, t), alpha };
  }
  if (space === "oklab") {
    const x = oklchToOklab(a);
    const y = oklchToOklab(b);
    return { ...oklabToOklch({ l: lerp(x.l, y.l, t), a: lerp(x.a, y.a, t), b: lerp(x.b, y.b, t) }), alpha };
  }
  const x = oklchToSrgb(fitToGamut(a));
  const y = oklchToSrgb(fitToGamut(b));
  return srgbToOklch({ r: lerp(x.r, y.r, t), g: lerp(x.g, y.g, t), b: lerp(x.b, y.b, t) }, alpha);
}

export function sortedStops(stops: Stop[]): Stop[] {
  return [...stops].map((s) => ({ ...s, pos: clamp(s.pos, 0, 1) })).sort((a, b) => a.pos - b.pos);
}

export function sampleAt(def: GradientDef, x: number, space: Space = def.space): Oklch {
  const stops = sortedStops(def.stops);
  if (stops.length === 0) return { l: 0, c: 0, h: 0, alpha: 1 };
  if (x <= stops[0].pos) return stops[0].color;
  const lastStop = stops[stops.length - 1];
  if (x >= lastStop.pos) return lastStop.color;
  const ease = cubicBezier(bezierOf(def));
  for (let i = 0; i < stops.length - 1; i++) {
    const a = stops[i];
    const b = stops[i + 1];
    if (x >= a.pos && x <= b.pos) {
      const span = b.pos - a.pos;
      const local = span <= 0 ? 1 : (x - a.pos) / span;
      return mix(a.color, b.color, ease(local), space);
    }
  }
  return lastStop.color;
}

export function expandStops(def: GradientDef, space: Space = def.space): Stop[] {
  const stops = sortedStops(def.stops);
  if (stops.length < 2) return stops;
  const linearSrgb = space === "srgb" && def.easing === "linear";
  const perSegment = linearSrgb ? 1 : Math.max(2, Math.round(def.samples));
  const ease = cubicBezier(bezierOf(def));
  const out: Stop[] = [stops[0]];
  for (let i = 0; i < stops.length - 1; i++) {
    const a = stops[i];
    const b = stops[i + 1];
    for (let j = 1; j <= perSegment; j++) {
      const t = j / perSegment;
      const color = j === perSegment ? b.color : mix(a.color, b.color, ease(t), space);
      out.push({ color, pos: lerp(a.pos, b.pos, t) });
    }
  }
  return out;
}

function pct(pos: number): string {
  const v = Math.round(pos * 10000) / 100;
  return `${v}%`;
}

export function gradientCss(def: GradientDef, space: Space = def.space, format: "hex" | "oklch" = "hex"): string {
  const stops = expandStops(def, space);
  const write = (c: Oklch) => (format === "hex" ? toHex(c) : formatOklch(fitToGamut(c, "p3")));
  const body = stops.map((s) => `${write(s.color)} ${pct(s.pos)}`).join(", ");
  return `linear-gradient(${Math.round(def.angle)}deg, ${body})`;
}

export function nativeGradientCss(def: GradientDef): string {
  const stops = sortedStops(def.stops);
  const body = stops.map((s) => `${formatOklch(s.color)} ${pct(s.pos)}`).join(", ");
  const space = def.space === "srgb" ? "srgb" : def.space;
  return `linear-gradient(in ${space}${def.space === "oklch" ? " shorter hue" : ""} ${Math.round(def.angle)}deg, ${body})`;
}

