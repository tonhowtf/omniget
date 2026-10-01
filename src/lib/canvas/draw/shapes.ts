// Path generators for the drawing shapes (02-motor-do-canvas §6.2). Each returns
// a closed polygon as a flat [x,y,...] list in world units.

import type { Rect } from '../model/types';

export function rectPolygon(r: Rect, radius = 0, segs = 4): number[] {
  const rad = Math.max(0, Math.min(radius, r.w / 2, r.h / 2));
  if (rad <= 0.5) return [r.x, r.y, r.x + r.w, r.y, r.x + r.w, r.y + r.h, r.x, r.y + r.h];
  const out: number[] = [];
  const corners: Array<[number, number, number]> = [
    [r.x + r.w - rad, r.y + rad, -Math.PI / 2],
    [r.x + r.w - rad, r.y + r.h - rad, 0],
    [r.x + rad, r.y + r.h - rad, Math.PI / 2],
    [r.x + rad, r.y + rad, Math.PI],
  ];
  for (const [cx, cy, a0] of corners) {
    for (let k = 0; k <= segs; k++) {
      const a = a0 + (Math.PI / 2) * (k / segs);
      out.push(cx + Math.cos(a) * rad, cy + Math.sin(a) * rad);
    }
  }
  return out;
}

export function ellipsePolygon(r: Rect, segs?: number): number[] {
  const cx = r.x + r.w / 2;
  const cy = r.y + r.h / 2;
  const rx = r.w / 2;
  const ry = r.h / 2;
  const n = segs ?? Math.max(24, Math.min(96, Math.round((rx + ry) / 4)));
  const out: number[] = [];
  for (let i = 0; i < n; i++) {
    const a = (i / n) * Math.PI * 2;
    out.push(cx + Math.cos(a) * rx, cy + Math.sin(a) * ry);
  }
  return out;
}

function diamondPolygon(r: Rect): number[] {
  const cx = r.x + r.w / 2;
  const cy = r.y + r.h / 2;
  return [cx, r.y, r.x + r.w, cy, cx, r.y + r.h, r.x, cy];
}

export function trianglePolygon(r: Rect): number[] {
  return [r.x + r.w / 2, r.y, r.x + r.w, r.y + r.h, r.x, r.y + r.h];
}

function hexagonPolygon(r: Rect): number[] {
  const q = r.w / 4;
  const cy = r.y + r.h / 2;
  return [r.x + q, r.y, r.x + r.w - q, r.y, r.x + r.w, cy, r.x + r.w - q, r.y + r.h, r.x + q, r.y + r.h, r.x, cy];
}

function starPolygon(r: Rect, points = 5, inner = 0.45): number[] {
  const cx = r.x + r.w / 2;
  const cy = r.y + r.h / 2;
  const out: number[] = [];
  for (let i = 0; i < points * 2; i++) {
    const a = -Math.PI / 2 + (i * Math.PI) / points;
    const k = i % 2 === 0 ? 1 : inner;
    out.push(cx + Math.cos(a) * (r.w / 2) * k, cy + Math.sin(a) * (r.h / 2) * k);
  }
  return out;
}

function parallelogramPolygon(r: Rect, slant = 0.25): number[] {
  const k = r.w * slant;
  return [r.x + k, r.y, r.x + r.w, r.y, r.x + r.w - k, r.y + r.h, r.x, r.y + r.h];
}

/** Heart from the classic parametric curve, fitted to the box. */
function heartPolygon(r: Rect, segs = 64): number[] {
  const raw: number[] = [];
  let minX = Infinity;
  let maxX = -Infinity;
  let minY = Infinity;
  let maxY = -Infinity;
  for (let i = 0; i < segs; i++) {
    const t = (i / segs) * Math.PI * 2;
    const x = 16 * Math.sin(t) ** 3;
    const y = -(13 * Math.cos(t) - 5 * Math.cos(2 * t) - 2 * Math.cos(3 * t) - Math.cos(4 * t));
    raw.push(x, y);
    minX = Math.min(minX, x);
    maxX = Math.max(maxX, x);
    minY = Math.min(minY, y);
    maxY = Math.max(maxY, y);
  }
  const out: number[] = [];
  for (let i = 0; i < raw.length; i += 2) {
    out.push(r.x + ((raw[i] - minX) / (maxX - minX)) * r.w, r.y + ((raw[i + 1] - minY) / (maxY - minY)) * r.h);
  }
  return out;
}

/** Cloud: bumps (circle arcs) around an ellipse, as one outline. */
function cloudPolygon(r: Rect, bumps = 9): number[] {
  const cx = r.x + r.w / 2;
  const cy = r.y + r.h / 2;
  const rx = r.w / 2;
  const ry = r.h / 2;
  const out: number[] = [];
  const per = 10;
  for (let b = 0; b < bumps; b++) {
    for (let k = 0; k < per; k++) {
      const a = ((b + k / per) / bumps) * Math.PI * 2;
      // Each bump bulges out by up to 18% of the radius.
      const bulge = 0.82 + 0.18 * Math.sin((k / per) * Math.PI);
      out.push(cx + Math.cos(a) * rx * bulge, cy + Math.sin(a) * ry * bulge);
    }
  }
  return out;
}

/** Block arrow pointing `dir`, shaft 50% of the cross size, head 40% of the length. */
function blockArrowPolygon(r: Rect, dir: 'left' | 'right' | 'up' | 'down'): number[] {
  // Built pointing right in a unit box, then mapped.
  const unit = [0, 0.25, 0.6, 0.25, 0.6, 0, 1, 0.5, 0.6, 1, 0.6, 0.75, 0, 0.75];
  const out: number[] = [];
  for (let i = 0; i < unit.length; i += 2) {
    let u = unit[i];
    let v = unit[i + 1];
    if (dir === 'left') u = 1 - u;
    else if (dir === 'down') [u, v] = [v, u];
    else if (dir === 'up') [u, v] = [v, 1 - u];
    out.push(r.x + u * r.w, r.y + v * r.h);
  }
  return out;
}

/** Stadium (capsule) — Maestri's oval. */
function ovalPolygon(r: Rect): number[] {
  return rectPolygon(r, Math.min(r.w, r.h) / 2, 12);
}

/**
 * Polygon of a shape kind, or null for kinds that are not closed shapes.
 * Accepts the Maestri wire names and the front's short ones (`rect`).
 */
export function shapePolygon(kind: string, r: Rect, cornerRadius?: number): number[] | null {
  switch (kind) {
    case 'rect':
    case 'rectangle':
    case 'xBox':
    case 'checkBox':
      return rectPolygon(r, cornerRadius ?? (kind === 'rect' || kind === 'rectangle' ? 8 : 4));
    case 'ellipse':
      return ellipsePolygon(r);
    case 'oval':
      return cornerRadius !== undefined ? rectPolygon(r, cornerRadius, 12) : ovalPolygon(r);
    case 'diamond':
      return diamondPolygon(r);
    case 'triangle':
      return trianglePolygon(r);
    case 'hexagon':
      return hexagonPolygon(r);
    case 'star':
      return starPolygon(r);
    case 'parallelogram':
      return parallelogramPolygon(r);
    case 'heart':
      return heartPolygon(r);
    case 'cloud':
      return cloudPolygon(r);
    case 'blockArrowLeft':
      return blockArrowPolygon(r, 'left');
    case 'blockArrowRight':
      return blockArrowPolygon(r, 'right');
    case 'blockArrowUp':
      return blockArrowPolygon(r, 'up');
    case 'blockArrowDown':
      return blockArrowPolygon(r, 'down');
    default:
      return null;
  }
}

/** Open polylines drawn inside a shape (the X of xBox, the tick of checkBox). */
export function shapeMarks(kind: string, r: Rect): number[][] {
  const p = Math.min(r.w, r.h) * 0.2;
  if (kind === 'xBox') {
    return [
      [r.x + p, r.y + p, r.x + r.w - p, r.y + r.h - p],
      [r.x + r.w - p, r.y + p, r.x + p, r.y + r.h - p],
    ];
  }
  if (kind === 'checkBox') {
    return [[r.x + r.w * 0.22, r.y + r.h * 0.52, r.x + r.w * 0.42, r.y + r.h * 0.74, r.x + r.w * 0.8, r.y + r.h * 0.28]];
  }
  return [];
}

/** Two short barbs of an arrow head at (x1, y1) coming from (x0, y0). */
export function arrowHead(x0: number, y0: number, x1: number, y1: number, size: number): number[] {
  const a = Math.atan2(y1 - y0, x1 - x0);
  const spread = Math.PI / 7;
  return [
    x1 - Math.cos(a - spread) * size,
    y1 - Math.sin(a - spread) * size,
    x1,
    y1,
    x1 - Math.cos(a + spread) * size,
    y1 - Math.sin(a + spread) * size,
  ];
}

/**
 * Splits a polyline into dash pieces (`dash` on, `gap` off, in world units).
 * `closed` walks back to the first point.
 */
export function dashPolyline(pts: ArrayLike<number>, dash: number, gap: number, closed = false): number[][] {
  const n = Math.floor(pts.length / 2);
  const out: number[][] = [];
  if (n < 2) return out;
  const seg = closed ? n : n - 1;
  let on = true;
  let left = dash;
  let cur: number[] = [pts[0], pts[1]];
  for (let i = 0; i < seg; i++) {
    let x0 = pts[i * 2];
    let y0 = pts[i * 2 + 1];
    const j = (i + 1) % n;
    const x1 = pts[j * 2];
    const y1 = pts[j * 2 + 1];
    let len = Math.hypot(x1 - x0, y1 - y0);
    while (len > 1e-6) {
      const step = Math.min(left, len);
      const t = step / len;
      const nx = x0 + (x1 - x0) * t;
      const ny = y0 + (y1 - y0) * t;
      if (on) cur.push(nx, ny);
      x0 = nx;
      y0 = ny;
      len -= step;
      left -= step;
      if (left <= 1e-6) {
        if (on && cur.length >= 4) out.push(cur);
        on = !on;
        left = on ? dash : gap;
        cur = on ? [x0, y0] : [];
      }
    }
  }
  if (on && cur.length >= 4) out.push(cur);
  return out;
}
