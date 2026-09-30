// smartDraw (Maestri): on release, a freehand stroke becomes a line,
// rectangle, ellipse or triangle when the fit error is small enough.

import type { Rect } from '../model/types';
import { boundsOf, distToPolyline, simplify } from '../spatial/geometry';
import { ellipsePolygon, rectPolygon, trianglePolygon } from './shapes';

export type Recognized =
  | { kind: 'line'; x0: number; y0: number; x1: number; y1: number }
  | { kind: 'rect' | 'ellipse' | 'triangle'; rect: Rect };

/** Mean distance of the stroke points to a polygon outline, relative to its size. */
function fitError(pts: number[], outline: number[], size: number): number {
  const closed = [...outline, outline[0], outline[1]];
  let sum = 0;
  const n = pts.length / 2;
  for (let i = 0; i < n; i++) sum += distToPolyline(pts[i * 2], pts[i * 2 + 1], closed);
  return sum / n / Math.max(1, size);
}

/** `pts` flat, absolute. Returns null when nothing fits well. */
export function recognize(pts: number[]): Recognized | null {
  const n = pts.length / 2;
  if (n < 3) return null;
  const b = boundsOf(pts);
  const size = Math.max(b.w, b.h);
  if (size < 12) return null;
  const x0 = pts[0];
  const y0 = pts[1];
  const x1 = pts[pts.length - 2];
  const y1 = pts[pts.length - 1];
  const gap = Math.hypot(x1 - x0, y1 - y0);

  // Open stroke: a straight line when every point is near the chord.
  if (gap > size * 0.35) {
    const err = fitError(pts, [x0, y0, x1, y1], size) ;
    if (err < 0.035) return { kind: 'line', x0, y0, x1, y1 };
    return null;
  }

  const candidates: Array<{ kind: 'rect' | 'ellipse' | 'triangle'; err: number }> = [
    { kind: 'rect', err: fitError(pts, rectPolygon(b), size) },
    { kind: 'ellipse', err: fitError(pts, ellipsePolygon(b, 48), size) },
    { kind: 'triangle', err: fitError(pts, trianglePolygon(b), size) },
  ];
  // Corners decide between rect and ellipse when both fit similarly.
  const corners = simplify(pts, size * 0.08).length / 2 - 1;
  for (const c of candidates) {
    if (c.kind === 'rect' && corners >= 4 && corners <= 6) c.err *= 0.8;
    if (c.kind === 'ellipse' && corners > 6) c.err *= 0.8;
    if (c.kind === 'triangle' && corners === 3) c.err *= 0.7;
  }
  candidates.sort((a, b2) => a.err - b2.err);
  const best = candidates[0];
  if (best.err > 0.08) return null;
  return { kind: best.kind, rect: b };
}
