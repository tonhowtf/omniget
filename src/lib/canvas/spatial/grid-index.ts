import type { Rect } from '../model/types';

const CELL = 512;

export class GridIndex {
  private cells = new Map<string, Set<string>>();
  private rects = new Map<string, Rect>();

  constructor(readonly cell = CELL) {}

  private keysOf(r: Rect): string[] {
    const c = this.cell;
    const x0 = Math.floor(r.x / c);
    const y0 = Math.floor(r.y / c);
    const x1 = Math.floor((r.x + Math.max(0, r.w)) / c);
    const y1 = Math.floor((r.y + Math.max(0, r.h)) / c);
    const out: string[] = [];
    if ((x1 - x0 + 1) * (y1 - y0 + 1) > 4096) return ['*'];
    for (let y = y0; y <= y1; y++) for (let x = x0; x <= x1; x++) out.push(`${x},${y}`);
    return out;
  }

  insert(id: string, r: Rect): void {
    if (this.rects.has(id)) this.remove(id);
    const copy = { x: r.x, y: r.y, w: r.w, h: r.h };
    this.rects.set(id, copy);
    for (const k of this.keysOf(copy)) {
      let set = this.cells.get(k);
      if (!set) {
        set = new Set();
        this.cells.set(k, set);
      }
      set.add(id);
    }
  }

  remove(id: string): void {
    const r = this.rects.get(id);
    if (!r) return;
    for (const k of this.keysOf(r)) {
      const set = this.cells.get(k);
      if (!set) continue;
      set.delete(id);
      if (set.size === 0) this.cells.delete(k);
    }
    this.rects.delete(id);
  }

  update(id: string, r: Rect): void {
    const old = this.rects.get(id);
    if (old && old.x === r.x && old.y === r.y && old.w === r.w && old.h === r.h) return;
    this.insert(id, r);
  }

  get(id: string): Rect | undefined {
    return this.rects.get(id);
  }

  clear(): void {
    this.cells.clear();
    this.rects.clear();
  }

  get size(): number {
    return this.rects.size;
  }

  queryRect(r: Rect): string[] {
    const seen = new Set<string>();
    const out: string[] = [];
    const visit = (set: Set<string> | undefined) => {
      if (!set) return;
      for (const id of set) {
        if (seen.has(id)) continue;
        seen.add(id);
        const b = this.rects.get(id);
        if (!b) continue;
        if (b.x <= r.x + r.w && r.x <= b.x + b.w && b.y <= r.y + r.h && r.y <= b.y + b.h) out.push(id);
      }
    };
    visit(this.cells.get('*'));
    const keys = this.keysOf(r);
    if (keys[0] === '*') {
      for (const set of this.cells.values()) visit(set);
    } else {
      for (const k of keys) visit(this.cells.get(k));
    }
    return out;
  }

  queryPoint(x: number, y: number, tolerance = 0): string[] {
    return this.queryRect({ x: x - tolerance, y: y - tolerance, w: tolerance * 2, h: tolerance * 2 });
  }
}
