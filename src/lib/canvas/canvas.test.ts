import { describe, expect, it } from 'vitest';
import { triangulate } from './draw/triangulate';
import { applyOps, emptyDoc, squashOps } from './model/ops';
import { History } from './model/history';
import { GridIndex } from './spatial/grid-index';
import { drawingFromWire, drawingToWire, opToWire } from './model/wire';
import { nodeLod } from './lod';
import { neighborInDirection, resolveKey } from './interaction/keyboard';
import { flyTo } from './camera/flyto';
import { recognize } from './draw/recognize';
import type { CanvasNode, Drawing, Op } from './model/types';
import type { CanvasStore } from './model/store.svelte';

function polyArea(p: number[]): number {
  let a = 0;
  for (let i = 0, n = p.length / 2, j = n - 1; i < n; j = i++) a += p[j * 2] * p[i * 2 + 1] - p[i * 2] * p[j * 2 + 1];
  return Math.abs(a / 2);
}

function trisArea(poly: number[], idx: number[]): number {
  let a = 0;
  for (let k = 0; k < idx.length; k += 3) {
    const [i, j, l] = [idx[k], idx[k + 1], idx[k + 2]];
    a += Math.abs(
      (poly[j * 2] - poly[i * 2]) * (poly[l * 2 + 1] - poly[i * 2 + 1]) -
        (poly[l * 2] - poly[i * 2]) * (poly[j * 2 + 1] - poly[i * 2 + 1]),
    ) / 2;
  }
  return a;
}

const node = (id: string, x: number, y: number, extra: Partial<CanvasNode> = {}): CanvasNode => ({
  id,
  kind: 'note',
  x,
  y,
  w: 100,
  h: 80,
  z: 0,
  data: {},
  ...extra,
});

describe('triangulate', () => {
  it('covers convex and concave polygons in either winding', () => {
    const square = [0, 0, 10, 0, 10, 10, 0, 10];
    const l = [0, 0, 20, 0, 20, 10, 10, 10, 10, 20, 0, 20];
    for (const p of [square, l, [...l].reverse().flatMap((_, i, a) => (i % 2 === 0 ? [a[i + 1], a[i]] : []))]) {
      const idx = triangulate(p);
      expect(idx.length / 3).toBe(p.length / 2 - 2);
      expect(trisArea(p, idx)).toBeCloseTo(polyArea(p), 6);
    }
  });
});

describe('ops and history', () => {
  it('applies, inverts and squashes', () => {
    const doc = emptyDoc();
    const inv = applyOps(doc, [{ op: 'add_node', node: node('a', 0, 0) }]);
    expect(doc.nodes.has('a')).toBe(true);
    const inv2 = applyOps(doc, [{ op: 'update_node', id: 'a', patch: { x: 40, locked: true } }]);
    expect(doc.nodes.get('a')!.x).toBe(40);
    applyOps(doc, inv2);
    expect(doc.nodes.get('a')!.x).toBe(0);
    expect(doc.nodes.get('a')!.locked).toBeUndefined();
    applyOps(doc, inv);
    expect(doc.nodes.has('a')).toBe(false);
    const sq = squashOps([
      { op: 'update_node', id: 'a', patch: { x: 1 } },
      { op: 'update_node', id: 'a', patch: { y: 2 } },
    ]);
    expect(sq).toEqual([{ op: 'update_node', id: 'a', patch: { x: 1, y: 2 } }]);
  });

  it('undoes only the user and skips records an agent touched later', () => {
    const h = new History();
    const fwd: Op[] = [{ op: 'update_node', id: 'a', patch: { x: 10 } }];
    const inv: Op[] = [{ op: 'update_node', id: 'a', patch: { x: 0 } }];
    h.record('user', fwd, inv);
    h.record('agent:t1', [{ op: 'update_node', id: 'a', patch: { x: 50 } }], []);
    const step = h.undo()!;
    expect(step.ops).toEqual([]);
    expect(step.skipped).toEqual(['n:a']);
    h.record('user', [{ op: 'update_node', id: 'b', patch: { x: 1 } }], [{ op: 'update_node', id: 'b', patch: { x: 0 } }]);
    expect(h.undo()!.ops).toEqual([{ op: 'update_node', id: 'b', patch: { x: 0 } }]);
    expect(h.redo()!.ops).toEqual([{ op: 'update_node', id: 'b', patch: { x: 1 } }]);
  });

  it('squashes a drag mark into one entry', () => {
    const h = new History();
    h.beginMark();
    h.record('user', [{ op: 'update_node', id: 'a', patch: { x: 1 } }], [{ op: 'update_node', id: 'a', patch: { x: 0 } }]);
    h.record('user', [{ op: 'update_node', id: 'a', patch: { x: 2 } }], [{ op: 'update_node', id: 'a', patch: { x: 1 } }]);
    h.endMark();
    expect(h.sizes.undo).toBe(1);
    expect(h.undo()!.ops[h.sizes.redo ? 1 : 0]).toEqual({ op: 'update_node', id: 'a', patch: { x: 0 } });
  });
});

describe('grid index', () => {
  it('finds by rect and point across cells, and moves', () => {
    const g = new GridIndex(100);
    g.insert('a', { x: 90, y: 90, w: 30, h: 30 });
    g.insert('b', { x: 500, y: 500, w: 10, h: 10 });
    expect(g.queryPoint(110, 110)).toEqual(['a']);
    expect(g.queryRect({ x: 0, y: 0, w: 1000, h: 1000 }).sort()).toEqual(['a', 'b']);
    g.update('a', { x: 700, y: 700, w: 10, h: 10 });
    expect(g.queryPoint(110, 110)).toEqual([]);
    expect(g.queryPoint(705, 705)).toEqual(['a']);
  });
});

describe('wire drawings', () => {
  it('round-trips a pen stroke and a sketchy shape', () => {
    const pen: Drawing = { id: 'p', kind: 'pen', x: 5, y: 6, w: 10, h: 4, z: 2, points: [0, 0, 10, 4], color: '#fff', strokeWidth: 3 };
    const back = drawingFromWire(drawingToWire(pen));
    expect(back).toMatchObject({ kind: 'pen', x: 5, y: 6, w: 10, h: 4, points: [0, 0, 10, 4], strokeWidth: 3, z: 2 });
    const rect: Drawing = { id: 'r', kind: 'rect', x: 0, y: 0, w: 50, h: 20, color: '#f00', strokeWidth: 2, sketch: true, fillStyle: 'hachure' };
    const w = drawingToWire(rect);
    expect(w.strokePattern).toBe('sketch');
    expect(drawingFromWire(w)).toMatchObject({ kind: 'rect', w: 50, h: 20, sketch: true, fillStyle: 'hachure' });
  });

  it('sends removals as null and drawing updates as full wire fields', () => {
    const store = { drawing: () => ({ id: 'r', kind: 'rect', x: 1, y: 2, w: 3, h: 4, color: '#000', strokeWidth: 1 }) } as unknown as CanvasStore;
    expect(opToWire({ op: 'update_node', id: 'a', patch: { locked: undefined } }, store)).toEqual({
      op: 'update_node',
      id: 'a',
      patch: { locked: null },
    });
    const u = opToWire({ op: 'update_drawing', id: 'r', patch: { x: 1 } }, store) as { patch: Record<string, unknown> };
    expect(u.patch.origin).toEqual({ x: 1, y: 2 });
    expect(u.patch.groupId).toBeNull();
  });
});

describe('lod hysteresis', () => {
  it('enters near above 7.5 px and leaves below 6.5 px for terminals', () => {
    const t = { kind: 'terminal', w: 600 };
    expect(nodeLod(t, 0.6)).toBe('near');
    expect(nodeLod(t, 0.55, 'near')).toBe('near');
    expect(nodeLod(t, 0.55, 'mid')).toBe('mid');
    expect(nodeLod(t, 0.2)).toBe('far');
    expect(nodeLod(t, 0.26, 'far')).toBe('far');
    expect(nodeLod(t, 0.3, 'far')).toBe('mid');
  });
});

describe('keyboard', () => {
  const k = (key: string, mod: Partial<{ metaKey: boolean; shiftKey: boolean }> = {}) => ({
    key,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
    shiftKey: false,
    ...mod,
  });
  it('uses single-key tools only without a selection', () => {
    expect(resolveKey(k('t'), { hasSelection: false, mac: true })).toEqual({ cmd: 'create', kind: 'terminal' });
    expect(resolveKey(k('h'), { hasSelection: true, mac: true })).toEqual({ cmd: 'focus-dir', dir: 'left' });
    expect(resolveKey(k('%'), { hasSelection: true, mac: true })).toEqual({ cmd: 'split', dir: 'right' });
    expect(resolveKey(k('a', { metaKey: true, shiftKey: true }), { hasSelection: false, mac: true })).toEqual({ cmd: 'next-attention' });
  });
  it('picks the spatial neighbour', () => {
    const from = { x: 0, y: 0, w: 100, h: 100 };
    const c = [
      { id: 'right', x: 300, y: 0, w: 100, h: 100 },
      { id: 'diag', x: 150, y: 400, w: 100, h: 100 },
      { id: 'below', x: 0, y: 200, w: 100, h: 100 },
    ];
    expect(neighborInDirection(from, c, 'right')).toBe('right');
    expect(neighborInDirection(from, c, 'down')).toBe('below');
    expect(neighborInDirection(from, c, 'left')).toBeNull();
  });
});

describe('camera flight', () => {
  it('starts and ends at the requested cameras', () => {
    const a = { x: 0, y: 0, zoom: 1 };
    const b = { x: 5000, y: 2000, zoom: 0.5 };
    const f = flyTo(a, b, 1000, 800);
    const end = f.at(1);
    expect(end).toEqual(b);
    const mid = f.at(0.5);
    expect(mid.zoom).toBeLessThan(0.5); // zooms out on the way
    expect(f.durationMs).toBeGreaterThanOrEqual(250);
    expect(f.durationMs).toBeLessThanOrEqual(700);
  });
});

describe('smart draw', () => {
  it('recognises a straight stroke as a line and a closed loop as a shape', () => {
    const line: number[] = [];
    for (let i = 0; i <= 20; i++) line.push(i * 10, i * 5 + (i % 2) * 0.5);
    expect(recognize(line)?.kind).toBe('line');
    const loop: number[] = [];
    for (let i = 0; i <= 40; i++) {
      const a = (i / 40) * Math.PI * 2;
      loop.push(100 + Math.cos(a) * 80, 100 + Math.sin(a) * 50);
    }
    expect(recognize(loop)?.kind).toBe('ellipse');
  });
});
