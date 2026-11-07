import { num } from "./format";

export type Point = { t: number; x: number };

function perpendicular(p: Point, a: Point, b: Point): number {
  const dt = b.t - a.t;
  if (dt === 0) return Math.abs(p.x - a.x);
  const f = (p.t - a.t) / dt;
  return Math.abs(p.x - (a.x + (b.x - a.x) * f));
}

export function simplify(points: Point[], tolerance: number): Point[] {
  if (points.length <= 2) return points.slice();
  const keep = new Uint8Array(points.length);
  keep[0] = 1;
  keep[points.length - 1] = 1;
  const stack: [number, number][] = [[0, points.length - 1]];
  while (stack.length) {
    const [start, end] = stack.pop()!;
    let worst = -1;
    let index = -1;
    for (let i = start + 1; i < end; i++) {
      const d = perpendicular(points[i], points[start], points[end]);
      if (d > worst) {
        worst = d;
        index = i;
      }
    }
    if (index !== -1 && worst > tolerance) {
      keep[index] = 1;
      stack.push([start, index], [index, end]);
    }
  }
  return points.filter((_, i) => keep[i] === 1);
}

export function normalize(points: Point[]): Point[] {
  if (points.length === 0) return [{ t: 0, x: 0 }, { t: 1, x: 1 }];
  const t0 = points[0].t;
  const span = points[points.length - 1].t - t0 || 1;
  const out = points.map((p) => ({ t: (p.t - t0) / span, x: p.x }));
  out[0] = { t: 0, x: 0 };
  out[out.length - 1] = { t: 1, x: 1 };
  return out;
}

export function sampleFn(fn: (t: number) => number, count = 200): Point[] {
  const n = Math.max(2, Math.round(count));
  return Array.from({ length: n + 1 }, (_, i) => {
    const t = i / n;
    return { t, x: fn(t) };
  });
}

export function linearStops(points: Point[], tolerance = 0.002): Point[] {
  return simplify(normalize(points), tolerance);
}

export function toLinear(points: Point[], tolerance = 0.002): string {
  const stops = linearStops(points, tolerance);
  const parts = stops.map((p, i) => {
    const value = num(p.x, 4);
    if (i === 0 || i === stops.length - 1) return value;
    return `${value} ${num(p.t * 100, 2)}%`;
  });
  return `linear(${parts.join(", ")})`;
}

export function linearEase(stops: Point[]): (t: number) => number {
  return (t: number) => {
    if (t <= 0) return stops[0].x;
    if (t >= 1) return stops[stops.length - 1].x;
    for (let i = 1; i < stops.length; i++) {
      const b = stops[i];
      if (t <= b.t) {
        const a = stops[i - 1];
        const f = b.t === a.t ? 1 : (t - a.t) / (b.t - a.t);
        return a.x + (b.x - a.x) * f;
      }
    }
    return stops[stops.length - 1].x;
  };
}
