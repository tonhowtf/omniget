export type Bezier = [number, number, number, number];

export type PresetGroup = "css" | "material" | "apple" | "expressive";

export type BezierPreset = { id: string; label: string; group: PresetGroup; value: Bezier };

export const Y_MIN = -0.75;
export const Y_MAX = 1.75;

export const PRESETS: BezierPreset[] = [
  { id: "linear", label: "linear", group: "css", value: [0, 0, 1, 1] },
  { id: "ease", label: "ease", group: "css", value: [0.25, 0.1, 0.25, 1] },
  { id: "ease-in", label: "ease-in", group: "css", value: [0.42, 0, 1, 1] },
  { id: "ease-out", label: "ease-out", group: "css", value: [0, 0, 0.58, 1] },
  { id: "ease-in-out", label: "ease-in-out", group: "css", value: [0.42, 0, 0.58, 1] },
  { id: "md-standard", label: "Material standard", group: "material", value: [0.2, 0, 0, 1] },
  { id: "md-emphasized-decel", label: "Material emphasized decelerate", group: "material", value: [0.05, 0.7, 0.1, 1] },
  { id: "md-emphasized-accel", label: "Material emphasized accelerate", group: "material", value: [0.3, 0, 0.8, 0.15] },
  { id: "md-legacy", label: "Material legacy", group: "material", value: [0.4, 0, 0.2, 1] },
  { id: "apple-fluid", label: "Apple fluid", group: "apple", value: [0.36, 0.66, 0.04, 1] },
  { id: "apple-snappy", label: "Apple snappy", group: "apple", value: [0.32, 0.72, 0, 1] },
  { id: "apple-gentle", label: "Apple gentle", group: "apple", value: [0.25, 0.46, 0.45, 0.94] },
  { id: "out-quart", label: "easeOutQuart", group: "expressive", value: [0.25, 1, 0.5, 1] },
  { id: "out-expo", label: "easeOutExpo", group: "expressive", value: [0.16, 1, 0.3, 1] },
  { id: "in-out-expo", label: "easeInOutExpo", group: "expressive", value: [0.87, 0, 0.13, 1] },
  { id: "in-out-circ", label: "easeInOutCirc", group: "expressive", value: [0.85, 0, 0.15, 1] },
  { id: "out-back", label: "easeOutBack", group: "expressive", value: [0.34, 1.56, 0.64, 1] },
  { id: "in-back", label: "easeInBack", group: "expressive", value: [0.36, 0, 0.66, -0.56] },
  { id: "in-out-back", label: "easeInOutBack", group: "expressive", value: [0.68, -0.6, 0.32, 1.6] },
];

export const DEFAULT_BEZIER: Bezier = [0.25, 0.1, 0.25, 1];

export function clampBezier(b: Bezier): Bezier {
  const cx = (v: number) => Math.min(1, Math.max(0, v));
  const cy = (v: number) => Math.min(Y_MAX, Math.max(Y_MIN, v));
  return [cx(b[0]), cy(b[1]), cx(b[2]), cy(b[3])];
}

export function moveHandle(b: Bezier, handle: 0 | 1, x: number, y: number): Bezier {
  const next = b.slice() as Bezier;
  const round = (v: number) => Math.round(v * 1000) / 1000;
  next[handle * 2] = Math.min(1, Math.max(0, round(x)));
  next[handle * 2 + 1] = Math.min(Y_MAX, Math.max(Y_MIN, round(y)));
  return next;
}

function coeffs(p1: number, p2: number): [number, number, number] {
  const c = 3 * p1;
  const b = 3 * (p2 - p1) - c;
  const a = 1 - c - b;
  return [a, b, c];
}

export function bezierEase(bezier: Bezier): (t: number) => number {
  const [x1, y1, x2, y2] = bezier;
  if (x1 === y1 && x2 === y2) return (t) => Math.min(1, Math.max(0, t));
  const [ax, bx, cx] = coeffs(x1, x2);
  const [ay, by, cy] = coeffs(y1, y2);
  const sampleX = (s: number) => ((ax * s + bx) * s + cx) * s;
  const sampleY = (s: number) => ((ay * s + by) * s + cy) * s;
  const slopeX = (s: number) => (3 * ax * s + 2 * bx) * s + cx;

  const solve = (x: number): number => {
    let s = x;
    for (let i = 0; i < 8; i++) {
      const err = sampleX(s) - x;
      if (Math.abs(err) < 1e-7) return s;
      const d = slopeX(s);
      if (Math.abs(d) < 1e-6) break;
      s -= err / d;
    }
    let lo = 0;
    let hi = 1;
    s = x;
    for (let i = 0; i < 60; i++) {
      const v = sampleX(s);
      if (Math.abs(v - x) < 1e-7) return s;
      if (v < x) lo = s;
      else hi = s;
      s = (lo + hi) / 2;
    }
    return s;
  };

  return (t: number) => {
    if (t <= 0) return 0;
    if (t >= 1) return 1;
    return sampleY(solve(t));
  };
}

export function sameBezier(a: Bezier, b: Bezier, eps = 1e-3): boolean {
  return a.every((v, i) => Math.abs(v - b[i]) < eps);
}

export function findPreset(b: Bezier): BezierPreset | undefined {
  return PRESETS.find((p) => sameBezier(p.value, b));
}

export function overshoots(b: Bezier): boolean {
  return b[1] < 0 || b[1] > 1 || b[3] < 0 || b[3] > 1;
}
