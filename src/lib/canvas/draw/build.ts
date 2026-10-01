// Drawing → coloured triangles in world units, the input of the mesh pass
// (and of the SVG export later). Freehand strokes go through the stroke
// generator; shapes get a fill (tint / solid / hachure / cross-hatch) and a
// clean, dashed, dotted or sketchy outline; arrows get their head. Rotation is
// applied last, around the centre of the box. Text (the `text` tool and the
// text inside shapes) is drawn by the DOM layer, not here.

import type { Drawing } from '../model/types';
import { parseColor, withAlpha, type Rgba } from '../render/color';
import { crossHatchLines, hachureLines } from './hachure';
import { arrowHead, dashPolyline, shapeMarks, shapePolygon } from './shapes';
import { roughPolygon, roughPolyline } from './sketch';
import { freehandTriangles, polylineTriangles } from './stroke';
import { fillTriangles } from './triangulate';

interface TriRun {
  /** Flat x,y per vertex (triangle list), world units. */
  tris: number[];
  color: Rgba;
}

/** Tools drawn as a freehand stroke through their points. */
const FREEHAND_TOOLS = new Set(['pen', 'brush', 'smartDraw', 'smart', 'highlighter']);

/** Ink used by drawings saved without a colour (set from the theme by the mesh pass). */
let defaultInk: Rgba = [0.55, 0.57, 0.6, 1];

export function setDefaultInk(c: Rgba): void {
  defaultInk = c;
}

function segmentsToTris(segs: number[], width: number): number[] {
  const out: number[] = [];
  for (let i = 0; i + 3 < segs.length; i += 4) {
    out.push(...polylineTriangles([segs[i], segs[i + 1], segs[i + 2], segs[i + 3]], width, false, 0));
  }
  return out;
}

/** Solid, dashed or dotted stroke of a polyline. */
function strokeTris(pts: number[], width: number, closed: boolean, pattern: Drawing['strokePattern'], caps: number): number[] {
  if (pattern === 'dashed' || pattern === 'dotted') {
    const dash = pattern === 'dashed' ? Math.max(6, width * 4) : Math.max(0.5, width * 0.2);
    const gap = pattern === 'dashed' ? Math.max(5, width * 3) : Math.max(3, width * 2);
    const out: number[] = [];
    for (const piece of dashPolyline(pts, dash, gap, closed)) out.push(...polylineTriangles(piece, width, false, pattern === 'dotted' ? 6 : 2));
    return out;
  }
  return polylineTriangles(pts, width, closed, caps);
}

/** Quadratic curve through a control point pushed `offset` off the chord's middle. */
function curvePoints(x0: number, y0: number, x1: number, y1: number, offset: number): number[] {
  const mx = (x0 + x1) / 2;
  const my = (y0 + y1) / 2;
  const len = Math.hypot(x1 - x0, y1 - y0) || 1;
  const cx = mx + (-(y1 - y0) / len) * offset;
  const cy = my + ((x1 - x0) / len) * offset;
  const out: number[] = [];
  const n = 24;
  for (let i = 0; i <= n; i++) {
    const t = i / n;
    const a = (1 - t) * (1 - t);
    const b = 2 * (1 - t) * t;
    const c = t * t;
    out.push(a * x0 + b * cx + c * x1, a * y0 + b * cy + c * y1);
  }
  return out;
}

function rotateRuns(runs: TriRun[], d: Drawing): TriRun[] {
  const deg = d.rotation ?? 0;
  if (!deg || !Number.isFinite(deg)) return runs;
  const a = (deg * Math.PI) / 180;
  const cos = Math.cos(a);
  const sin = Math.sin(a);
  const cx = d.x + d.w / 2;
  const cy = d.y + d.h / 2;
  for (const run of runs) {
    const t = run.tris;
    for (let i = 0; i + 1 < t.length; i += 2) {
      const x = t[i] - cx;
      const y = t[i + 1] - cy;
      t[i] = cx + x * cos - y * sin;
      t[i + 1] = cy + x * sin + y * cos;
    }
  }
  return runs;
}

export function buildDrawing(d: Drawing): TriRun[] {
  return rotateRuns(buildUnrotated(d), d);
}

function buildUnrotated(d: Drawing): TriRun[] {
  const base = d.color ? parseColor(d.color, defaultInk) : defaultInk;
  const color = withAlpha(base, d.opacity ?? 1);
  const width = Math.max(0.5, d.strokeWidth ?? 2);
  const runs: TriRun[] = [];

  if (FREEHAND_TOOLS.has(d.kind)) {
    if (!d.points || d.points.length < 2) return runs;
    const abs = d.points.map((v, i) => v + (i % 2 === 0 ? d.x : d.y));
    let tris: number[];
    if (d.kind === 'highlighter') tris = polylineTriangles(abs, width, false, 4);
    else if (d.strokePattern === 'dashed' || d.strokePattern === 'dotted') tris = strokeTris(abs, width, false, d.strokePattern, 4);
    else
      tris = freehandTriangles(abs, {
        size: d.kind === 'brush' ? width * 4 : width * 2,
        thinning: d.kind === 'brush' ? 0.7 : d.pressures ? 0.6 : 0.5,
        streamline: 0.35,
        taperStart: width * 3,
        taperEnd: width * 5,
        pressures: d.pressures,
      });
    runs.push({ tris, color: d.kind === 'highlighter' && d.opacity === undefined ? withAlpha(base, 0.4) : color });
    return runs;
  }

  if (d.kind === 'arrow' || d.kind === 'line') {
    if (!d.points || d.points.length < 4) return runs;
    const x0 = d.points[0] + d.x;
    const y0 = d.points[1] + d.y;
    const x1 = d.points[d.points.length - 2] + d.x;
    const y1 = d.points[d.points.length - 1] + d.y;
    const curved = d.controlPointOffset ? curvePoints(x0, y0, x1, y1, d.controlPointOffset) : null;
    const line = curved ?? [x0, y0, x1, y1];
    // The head follows the last segment of the (possibly curved) line.
    const hx = line[line.length - 4];
    const hy = line[line.length - 3];
    const head = d.kind === 'arrow' ? arrowHead(hx, hy, x1, y1, Math.max(10, width * 4)) : null;
    const tris: number[] = [];
    if (d.sketch) {
      for (const p of roughPolyline(line, d.seed ?? 1)) tris.push(...polylineTriangles(p, width, false, 2));
      if (head) for (const p of roughPolyline(head, (d.seed ?? 1) + 7, 0.6)) tris.push(...polylineTriangles(p, width, false, 2));
    } else {
      tris.push(...strokeTris(line, width, false, d.strokePattern, 4));
      if (head) tris.push(...polylineTriangles(head, width, false, 4));
    }
    runs.push({ tris, color });
    return runs;
  }

  const box = { x: d.x, y: d.y, w: d.w, h: d.h };
  const poly = shapePolygon(d.kind, box, d.cornerRadius);
  if (!poly) return runs;
  const fill = withAlpha(d.fill ? parseColor(d.fill, base) : base, d.opacity ?? 1);
  const gap = Math.max(6, width * 3.5);
  switch (d.fillStyle) {
    case 'solid':
      runs.push({ tris: fillTriangles(poly), color: withAlpha(fill, 0.35) });
      break;
    case 'tint':
      runs.push({ tris: fillTriangles(poly), color: withAlpha(fill, 0.18) });
      break;
    case 'solidOpaque':
      runs.push({ tris: fillTriangles(poly), color: fill });
      break;
    case 'hachure':
      runs.push({ tris: segmentsToTris(hachureLines(poly, gap), Math.max(1, width * 0.6)), color: fill });
      break;
    case 'cross':
      runs.push({ tris: segmentsToTris(crossHatchLines(poly, gap), Math.max(1, width * 0.6)), color: fill });
      break;
  }
  const outline: number[] = [];
  const marks = shapeMarks(d.kind, box);
  if (d.sketch) {
    for (const p of roughPolygon(poly, d.seed ?? 1)) outline.push(...polylineTriangles(p, width, false, 2));
    for (const m of marks) for (const p of roughPolyline(m, (d.seed ?? 1) + 3)) outline.push(...polylineTriangles(p, width, false, 2));
  } else {
    outline.push(...strokeTris(poly, width, true, d.strokePattern, 3));
    for (const m of marks) outline.push(...polylineTriangles(m, width, false, 3));
  }
  runs.push({ tris: outline, color });
  return runs;
}
