import { GRID, type Rect } from '../model/types';

export function snap(v: number, grid = GRID): number {
  return Math.round(v / grid) * grid;
}

export interface Guide {
  axis: 'x' | 'y';
  at: number;
  from: number;
  to: number;
}

interface SnapResult {
  dx: number;
  dy: number;
  guides: Guide[];
}

export function alignGuides(moving: Rect, others: Rect[], tol: number): SnapResult {
  const mx = [moving.x, moving.x + moving.w / 2, moving.x + moving.w];
  const my = [moving.y, moving.y + moving.h / 2, moving.y + moving.h];
  let bestX: { d: number; at: number; o: Rect } | null = null;
  let bestY: { d: number; at: number; o: Rect } | null = null;
  for (const o of others) {
    const ox = [o.x, o.x + o.w / 2, o.x + o.w];
    const oy = [o.y, o.y + o.h / 2, o.y + o.h];
    for (const a of mx)
      for (const b of ox) {
        const d = b - a;
        if (Math.abs(d) <= tol && (!bestX || Math.abs(d) < Math.abs(bestX.d))) bestX = { d, at: b, o };
      }
    for (const a of my)
      for (const b of oy) {
        const d = b - a;
        if (Math.abs(d) <= tol && (!bestY || Math.abs(d) < Math.abs(bestY.d))) bestY = { d, at: b, o };
      }
  }
  const guides: Guide[] = [];
  if (bestX) {
    const o = bestX.o;
    guides.push({
      axis: 'x',
      at: bestX.at,
      from: Math.min(o.y, moving.y) - 16,
      to: Math.max(o.y + o.h, moving.y + moving.h) + 16,
    });
  }
  if (bestY) {
    const o = bestY.o;
    guides.push({
      axis: 'y',
      at: bestY.at,
      from: Math.min(o.x, moving.x) - 16,
      to: Math.max(o.x + o.w, moving.x + moving.w) + 16,
    });
  }
  return { dx: bestX?.d ?? 0, dy: bestY?.d ?? 0, guides };
}

export function magnetSnap(
  moving: Rect,
  others: Rect[],
  tol: number,
): { dx: number; dy: number; snappedX: boolean; snappedY: boolean } {
  let bx: number | null = null;
  let by: number | null = null;
  const overlapsY = (o: Rect) => moving.y < o.y + o.h + tol && o.y < moving.y + moving.h + tol;
  const overlapsX = (o: Rect) => moving.x < o.x + o.w + tol && o.x < moving.x + moving.w + tol;
  for (const o of others) {
    if (overlapsY(o)) {
      for (const d of [o.x + o.w - moving.x, o.x - (moving.x + moving.w)]) {
        if (Math.abs(d) <= tol && (bx === null || Math.abs(d) < Math.abs(bx))) bx = d;
      }
    }
    if (overlapsX(o)) {
      for (const d of [o.y + o.h - moving.y, o.y - (moving.y + moving.h)]) {
        if (Math.abs(d) <= tol && (by === null || Math.abs(d) < Math.abs(by))) by = d;
      }
    }
  }
  return { dx: bx ?? 0, dy: by ?? 0, snappedX: bx !== null, snappedY: by !== null };
}

export type AlignMode = 'left' | 'hcenter' | 'right' | 'top' | 'vcenter' | 'bottom' | 'hdistribute' | 'vdistribute';

export function arrange(items: Array<Rect & { id: string }>, mode: AlignMode): Map<string, { x: number; y: number }> {
  const out = new Map<string, { x: number; y: number }>();
  if (items.length < 2) return out;
  const minX = Math.min(...items.map((r) => r.x));
  const maxX = Math.max(...items.map((r) => r.x + r.w));
  const minY = Math.min(...items.map((r) => r.y));
  const maxY = Math.max(...items.map((r) => r.y + r.h));
  const cx = (minX + maxX) / 2;
  const cy = (minY + maxY) / 2;
  switch (mode) {
    case 'left':
      for (const r of items) out.set(r.id, { x: minX, y: r.y });
      break;
    case 'right':
      for (const r of items) out.set(r.id, { x: maxX - r.w, y: r.y });
      break;
    case 'hcenter':
      for (const r of items) out.set(r.id, { x: snap(cx - r.w / 2), y: r.y });
      break;
    case 'top':
      for (const r of items) out.set(r.id, { x: r.x, y: minY });
      break;
    case 'bottom':
      for (const r of items) out.set(r.id, { x: r.x, y: maxY - r.h });
      break;
    case 'vcenter':
      for (const r of items) out.set(r.id, { x: r.x, y: snap(cy - r.h / 2) });
      break;
    case 'hdistribute': {
      const sorted = items.slice().sort((a, b) => a.x - b.x);
      const total = sorted.reduce((s, r) => s + r.w, 0);
      const gap = (maxX - minX - total) / (sorted.length - 1);
      let x = minX;
      for (const r of sorted) {
        out.set(r.id, { x: snap(x), y: r.y });
        x += r.w + gap;
      }
      break;
    }
    case 'vdistribute': {
      const sorted = items.slice().sort((a, b) => a.y - b.y);
      const total = sorted.reduce((s, r) => s + r.h, 0);
      const gap = (maxY - minY - total) / (sorted.length - 1);
      let y = minY;
      for (const r of sorted) {
        out.set(r.id, { x: r.x, y: snap(y) });
        y += r.h + gap;
      }
      break;
    }
  }
  return out;
}

export function nearestFreeSpot(x: number, y: number, w: number, h: number, taken: Rect[], gap = GRID): { x: number; y: number } {
  const free = (px: number, py: number) =>
    !taken.some(
      (r) => px < r.x + r.w + gap && r.x < px + w + gap && py < r.y + r.h + gap && r.y < py + h + gap,
    );
  const sx = snap(x);
  const sy = snap(y);
  if (free(sx, sy)) return { x: sx, y: sy };
  const step = GRID * 4;
  for (let ring = 1; ring < 60; ring++) {
    for (let i = -ring; i <= ring; i++) {
      const cands = [
        [sx + i * step, sy - ring * step],
        [sx + i * step, sy + ring * step],
        [sx - ring * step, sy + i * step],
        [sx + ring * step, sy + i * step],
      ];
      for (const [px, py] of cands) if (free(px, py)) return { x: px, y: py };
    }
  }
  return { x: sx, y: sy };
}
