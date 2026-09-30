// Cord geometry (02-motor-do-canvas §5): a cubic bezier between node ports on
// the four edges; the side pair that minimises the length wins; tangents are
// normal to the exit edge with a control offset of clamp(0.5·d, 40, 240).

import type { Rect } from '../model/types';
import { arcLengths, sampleCubic, type Cubic, type Pt } from '../spatial/geometry';

export type Side = 'top' | 'right' | 'bottom' | 'left';

export const SIDES: Side[] = ['top', 'right', 'bottom', 'left'];

export function portPoint(r: Rect, side: Side): Pt {
  switch (side) {
    case 'top':
      return { x: r.x + r.w / 2, y: r.y };
    case 'right':
      return { x: r.x + r.w, y: r.y + r.h / 2 };
    case 'bottom':
      return { x: r.x + r.w / 2, y: r.y + r.h };
    case 'left':
      return { x: r.x, y: r.y + r.h / 2 };
  }
}

export function sideNormal(side: Side): Pt {
  switch (side) {
    case 'top':
      return { x: 0, y: -1 };
    case 'right':
      return { x: 1, y: 0 };
    case 'bottom':
      return { x: 0, y: 1 };
    case 'left':
      return { x: -1, y: 0 };
  }
}

export interface CordPath extends Cubic {
  fromSide: Side;
  toSide: Side;
}

function controlOffset(d: number): number {
  return Math.min(240, Math.max(40, d * 0.5));
}

/** Best bezier between two rectangles (or a rectangle and a free point). */
export function cordPath(from: Rect, to: Rect | Pt): CordPath {
  const toRect = 'w' in to ? (to as Rect) : null;
  let best: CordPath | null = null;
  let bestLen = Infinity;
  for (const fs of SIDES) {
    const p0 = portPoint(from, fs);
    const n0 = sideNormal(fs);
    const targets: Array<[Side, Pt]> = toRect
      ? SIDES.map((s) => [s, portPoint(toRect, s)] as [Side, Pt])
      : [[oppositeOf(fs), to as Pt]];
    for (const [ts, p1] of targets) {
      const dx = p1.x - p0.x;
      const dy = p1.y - p0.y;
      // Skip ports facing away from each other when a better pair exists.
      const facing = n0.x * dx + n0.y * dy;
      const n1 = sideNormal(ts);
      const facing1 = -(n1.x * dx + n1.y * dy);
      const len = Math.hypot(dx, dy) + (facing < 0 ? 400 : 0) + (toRect && facing1 < 0 ? 400 : 0);
      if (len < bestLen) {
        bestLen = len;
        const d = Math.hypot(dx, dy);
        const k = controlOffset(d);
        const c0 = { x: p0.x + n0.x * k, y: p0.y + n0.y * k };
        const c1 = toRect
          ? { x: p1.x + n1.x * k, y: p1.y + n1.y * k }
          : { x: p1.x - (dx / (d || 1)) * k * 0.3, y: p1.y - (dy / (d || 1)) * k * 0.3 };
        best = { p0, c0, c1, p1, fromSide: fs, toSide: ts };
      }
    }
  }
  return best as CordPath;
}

function oppositeOf(s: Side): Side {
  return s === 'top' ? 'bottom' : s === 'bottom' ? 'top' : s === 'left' ? 'right' : 'left';
}

export interface SampledCord {
  path: CordPath;
  /** Flat polyline, 33 points. */
  pts: number[];
  /** Arc-length table (32 segments) for uniform-speed travel. */
  lens: number[];
}

export function sampleCord(path: CordPath, n = 32): SampledCord {
  const pts = sampleCubic(path, n);
  return { path, pts, lens: arcLengths(pts) };
}

/** Reversed copy of a sampled cord (for a message going to → from). */
export function reverseCord(c: SampledCord): SampledCord {
  const pts: number[] = [];
  for (let i = c.pts.length - 2; i >= 0; i -= 2) pts.push(c.pts[i], c.pts[i + 1]);
  return { path: c.path, pts, lens: arcLengths(pts) };
}

/**
 * Circuit routing (Settings → Canvas → Cord style): axis-aligned segments
 * with at most two 90° turns between the same ports the bezier uses.
 */
export function circuitPoints(path: CordPath): number[] {
  const { p0, p1 } = path;
  const h0 = path.fromSide === 'left' || path.fromSide === 'right';
  const h1 = path.toSide === 'left' || path.toSide === 'right';
  const pts: Pt[] = [p0];
  if (h0 && h1) {
    const mx = (p0.x + p1.x) / 2;
    pts.push({ x: mx, y: p0.y }, { x: mx, y: p1.y });
  } else if (!h0 && !h1) {
    const my = (p0.y + p1.y) / 2;
    pts.push({ x: p0.x, y: my }, { x: p1.x, y: my });
  } else if (h0) {
    pts.push({ x: p1.x, y: p0.y });
  } else {
    pts.push({ x: p0.x, y: p1.y });
  }
  pts.push(p1);
  const out: number[] = [];
  for (const p of pts) out.push(p.x, p.y);
  return out;
}
