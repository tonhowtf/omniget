import type { Rect } from '../model/types';

export type { Rect };

export interface Pt {
  x: number;
  y: number;
}

export function rect(x: number, y: number, w: number, h: number): Rect {
  return { x, y, w, h };
}

export function rectFromPoints(a: Pt, b: Pt): Rect {
  const x = Math.min(a.x, b.x);
  const y = Math.min(a.y, b.y);
  return { x, y, w: Math.abs(a.x - b.x), h: Math.abs(a.y - b.y) };
}

export function intersects(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

export function containsRect(outer: Rect, inner: Rect): boolean {
  return (
    inner.x >= outer.x &&
    inner.y >= outer.y &&
    inner.x + inner.w <= outer.x + outer.w &&
    inner.y + inner.h <= outer.y + outer.h
  );
}

export function containsPoint(r: Rect, x: number, y: number): boolean {
  return x >= r.x && y >= r.y && x <= r.x + r.w && y <= r.y + r.h;
}

export function expand(r: Rect, by: number): Rect {
  return { x: r.x - by, y: r.y - by, w: r.w + by * 2, h: r.h + by * 2 };
}

export function union(rects: Iterable<Rect>): Rect | null {
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  for (const r of rects) {
    if (r.x < x0) x0 = r.x;
    if (r.y < y0) y0 = r.y;
    if (r.x + r.w > x1) x1 = r.x + r.w;
    if (r.y + r.h > y1) y1 = r.y + r.h;
  }
  if (!Number.isFinite(x0)) return null;
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

export function center(r: Rect): Pt {
  return { x: r.x + r.w / 2, y: r.y + r.h / 2 };
}

export function dist(a: Pt, b: Pt): number {
  return Math.hypot(a.x - b.x, a.y - b.y);
}

function distToSegmentSq(px: number, py: number, ax: number, ay: number, bx: number, by: number): number {
  const dx = bx - ax;
  const dy = by - ay;
  const len = dx * dx + dy * dy;
  let t = len > 0 ? ((px - ax) * dx + (py - ay) * dy) / len : 0;
  t = t < 0 ? 0 : t > 1 ? 1 : t;
  const cx = ax + t * dx - px;
  const cy = ay + t * dy - py;
  return cx * cx + cy * cy;
}

export function distToPolyline(px: number, py: number, pts: ArrayLike<number>, ox = 0, oy = 0): number {
  if (pts.length < 2) return Infinity;
  if (pts.length < 4) return Math.hypot(px - (pts[0] + ox), py - (pts[1] + oy));
  let best = Infinity;
  for (let i = 0; i + 3 < pts.length; i += 2) {
    const d = distToSegmentSq(px, py, pts[i] + ox, pts[i + 1] + oy, pts[i + 2] + ox, pts[i + 3] + oy);
    if (d < best) best = d;
  }
  return Math.sqrt(best);
}

export function pointInPolygon(px: number, py: number, poly: ArrayLike<number>): boolean {
  let inside = false;
  const n = poly.length / 2;
  for (let i = 0, j = n - 1; i < n; j = i++) {
    const xi = poly[i * 2];
    const yi = poly[i * 2 + 1];
    const xj = poly[j * 2];
    const yj = poly[j * 2 + 1];
    if (yi > py !== yj > py && px < ((xj - xi) * (py - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

export interface Cubic {
  p0: Pt;
  c0: Pt;
  c1: Pt;
  p1: Pt;
}

function cubicAt(c: Cubic, t: number): Pt {
  const u = 1 - t;
  const a = u * u * u;
  const b = 3 * u * u * t;
  const d = 3 * u * t * t;
  const e = t * t * t;
  return {
    x: a * c.p0.x + b * c.c0.x + d * c.c1.x + e * c.p1.x,
    y: a * c.p0.y + b * c.c0.y + d * c.c1.y + e * c.p1.y,
  };
}

export function sampleCubic(c: Cubic, n = 32): number[] {
  const out: number[] = [];
  for (let i = 0; i <= n; i++) {
    const p = cubicAt(c, i / n);
    out.push(p.x, p.y);
  }
  return out;
}

export function arcLengths(pts: ArrayLike<number>): number[] {
  const out = [0];
  let acc = 0;
  for (let i = 2; i + 1 < pts.length; i += 2) {
    acc += Math.hypot(pts[i] - pts[i - 2], pts[i + 1] - pts[i - 1]);
    out.push(acc);
  }
  return out;
}

export function pointAtLength(pts: ArrayLike<number>, lens: number[], f: number): Pt & { angle: number } {
  const total = lens[lens.length - 1] ?? 0;
  const n = lens.length;
  if (n < 2 || total <= 0) return { x: pts[0] ?? 0, y: pts[1] ?? 0, angle: 0 };
  const target = Math.max(0, Math.min(1, f)) * total;
  let lo = 0;
  let hi = n - 1;
  while (lo < hi - 1) {
    const mid = (lo + hi) >> 1;
    if (lens[mid] < target) lo = mid;
    else hi = mid;
  }
  const seg = lens[hi] - lens[lo] || 1;
  const t = (target - lens[lo]) / seg;
  const x0 = pts[lo * 2];
  const y0 = pts[lo * 2 + 1];
  const x1 = pts[hi * 2];
  const y1 = pts[hi * 2 + 1];
  return { x: x0 + (x1 - x0) * t, y: y0 + (y1 - y0) * t, angle: Math.atan2(y1 - y0, x1 - x0) };
}

export function boundsOf(pts: ArrayLike<number>, ox = 0, oy = 0): Rect {
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  for (let i = 0; i + 1 < pts.length; i += 2) {
    const x = pts[i] + ox;
    const y = pts[i + 1] + oy;
    if (x < x0) x0 = x;
    if (y < y0) y0 = y;
    if (x > x1) x1 = x;
    if (y > y1) y1 = y;
  }
  if (!Number.isFinite(x0)) return { x: ox, y: oy, w: 0, h: 0 };
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

export function simplify(pts: number[], tolerance: number): number[] {
  const n = pts.length / 2;
  if (n <= 2) return pts.slice();
  const keep = new Uint8Array(n);
  keep[0] = 1;
  keep[n - 1] = 1;
  const tolSq = tolerance * tolerance;
  const stack: Array<[number, number]> = [[0, n - 1]];
  while (stack.length) {
    const [a, b] = stack.pop() as [number, number];
    let best = -1;
    let bestD = tolSq;
    for (let i = a + 1; i < b; i++) {
      const d = distToSegmentSq(pts[i * 2], pts[i * 2 + 1], pts[a * 2], pts[a * 2 + 1], pts[b * 2], pts[b * 2 + 1]);
      if (d > bestD) {
        bestD = d;
        best = i;
      }
    }
    if (best >= 0) {
      keep[best] = 1;
      stack.push([a, best], [best, b]);
    }
  }
  const out: number[] = [];
  for (let i = 0; i < n; i++) if (keep[i]) out.push(pts[i * 2], pts[i * 2 + 1]);
  return out;
}
