// Geometric hit testing (02-motor-do-canvas §4.2): top-most node by z through
// the spatial index, then cords (distance to the sampled bezier), then
// drawings (polygon or polyline distance). Tolerance comes in screen pixels
// and is converted to world units. Group frames never block a click.

import type { CanvasStore } from '../model/store.svelte';
import type { Drawing } from '../model/types';
import { shapePolygon } from '../draw/shapes';
import { cordPath, sampleCord, portPoint, SIDES, type Side } from '../cords/route';
import { containsPoint, distToPolyline, pointInPolygon } from './geometry';

export type Hit =
  | { type: 'node'; id: string; zone: 'header' | 'body' | 'resize'; handle?: ResizeHandle }
  | { type: 'port'; id: string; side: Side }
  | { type: 'connection'; id: string }
  | { type: 'drawing'; id: string }
  | { type: 'none' };

export type ResizeHandle = 'n' | 's' | 'e' | 'w' | 'ne' | 'nw' | 'se' | 'sw';

/** Header strip height in screen pixels (the Maestri keeps it grabbable at any zoom). */
const HEADER_PX = 28;
/** Terminal chrome height in world units (term/metrics TERM_HEADER_H). */
const TERMINAL_HEADER = 30;

/** Height of the grab strip of a node, in world units, at this zoom. */
export function headerWorld(kind: string, h: number, zoom: number): number {
  if (kind === 'terminal' || kind === 'agent') return Math.min(h, TERMINAL_HEADER);
  if (kind === 'text') return 0;
  return Math.min(h, Math.min(56, HEADER_PX / zoom));
}
const TOLERANCE_PX = 6;
const PORT_PX = 9;

const LINEAR_DRAWINGS = new Set(['pen', 'brush', 'smartDraw', 'smart', 'highlighter', 'arrow', 'line']);

export function drawingHit(d: Drawing, x: number, y: number, tol: number): boolean {
  const lw = (d.strokeWidth ?? 2) / 2 + tol;
  if (d.rotation) {
    // Test in the drawing's own frame: rotate the point back around the centre.
    const a = (-d.rotation * Math.PI) / 180;
    const cx = d.x + d.w / 2;
    const cy = d.y + d.h / 2;
    const dx = x - cx;
    const dy = y - cy;
    x = cx + dx * Math.cos(a) - dy * Math.sin(a);
    y = cy + dx * Math.sin(a) + dy * Math.cos(a);
  }
  if (LINEAR_DRAWINGS.has(d.kind) && d.points) {
    return distToPolyline(x, y, d.points, d.x, d.y) <= lw;
  }
  const r = { x: d.x, y: d.y, w: d.w, h: d.h };
  if (d.kind === 'text') return containsPoint(r, x, y);
  const poly = shapePolygon(d.kind, r, d.cornerRadius);
  if (!poly) return containsPoint(r, x, y);
  const filled = d.fillStyle && d.fillStyle !== 'none';
  if (filled && pointInPolygon(x, y, poly)) return true;
  return distToPolyline(x, y, [...poly, poly[0], poly[1]]) <= lw;
}

interface HitOptions {
  zoom: number;
  /** Nodes whose ports are showing (selected/hovered); ports hit first. */
  portsFor?: Iterable<string>;
  /** Nodes whose resize handles are showing. */
  resizable?: Iterable<string>;
  skipDrawings?: boolean;
}

export function hitTest(store: CanvasStore, x: number, y: number, opts: HitOptions): Hit {
  const tol = TOLERANCE_PX / opts.zoom;

  if (opts.portsFor) {
    const pr = PORT_PX / opts.zoom;
    for (const id of opts.portsFor) {
      const n = store.node(id);
      if (!n) continue;
      for (const side of SIDES) {
        const p = portPoint(n, side);
        if (Math.hypot(p.x - x, p.y - y) <= pr) return { type: 'port', id, side };
      }
    }
  }

  if (opts.resizable) {
    const hr = 7 / opts.zoom;
    for (const id of opts.resizable) {
      const n = store.node(id);
      if (!n || n.locked) continue;
      const handle = resizeHandleAt(n, x, y, hr);
      if (handle) return { type: 'node', id, zone: 'resize', handle };
    }
  }

  const ids = store.nodeIndex.queryPoint(x, y, 0);
  if (ids.length) {
    let best: { id: string; z: number } | null = null;
    for (const id of ids) {
      const n = store.node(id);
      if (!n || n.stackId || !containsPoint(n, x, y)) continue;
      if (!best || n.z > best.z) best = { id, z: n.z };
    }
    if (best) {
      const n = store.node(best.id)!;
      return { type: 'node', id: best.id, zone: y - n.y <= headerWorld(n.kind, n.h, opts.zoom) ? 'header' : 'body' };
    }
  }

  for (const c of store.connections) {
    const a = store.node(c.from);
    const b = store.node(c.to);
    if (!a || !b) continue;
    const s = sampleCord(cordPath(a, b), 24);
    if (distToPolyline(x, y, s.pts) <= tol) return { type: 'connection', id: c.id };
  }

  if (!opts.skipDrawings) {
    const dids = store.drawingIndex.queryPoint(x, y, tol);
    let best: { id: string; z: number } | null = null;
    for (const id of dids) {
      const d = store.drawing(id);
      if (!d || !drawingHit(d, x, y, tol)) continue;
      const z = d.z ?? 0;
      if (!best || z > best.z) best = { id, z };
    }
    if (best) return { type: 'drawing', id: best.id };
  }
  return { type: 'none' };
}

function resizeHandleAt(
  r: { x: number; y: number; w: number; h: number },
  x: number,
  y: number,
  tol: number,
): ResizeHandle | null {
  const nearL = Math.abs(x - r.x) <= tol;
  const nearR = Math.abs(x - (r.x + r.w)) <= tol;
  const nearT = Math.abs(y - r.y) <= tol;
  const nearB = Math.abs(y - (r.y + r.h)) <= tol;
  const inX = x >= r.x - tol && x <= r.x + r.w + tol;
  const inY = y >= r.y - tol && y <= r.y + r.h + tol;
  if (nearT && nearL) return 'nw';
  if (nearT && nearR) return 'ne';
  if (nearB && nearL) return 'sw';
  if (nearB && nearR) return 'se';
  if (nearL && inY) return 'w';
  if (nearR && inY) return 'e';
  if (nearT && inX) return 'n';
  if (nearB && inX) return 's';
  return null;
}
