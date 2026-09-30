import { describe, expect, it } from 'vitest';
import type { ChunkTile } from '../types';
import {
  LitePalettes,
  Z_PER_M,
  fromDisplay,
  layersFor,
  mergeAtlases,
  regionChunkTiles,
  regionSprites,
  restackJson,
  stackPlan,
  stylesOf,
  terrainTiles,
  toDisplay,
  type KitAtlasPayload,
} from './model';

function payload(style: string, keys: string[]): KitAtlasPayload {
  const frames: Record<string, Record<string, unknown>> = {};
  const modules: Record<string, [string, number][][]> = {};
  for (const k of keys) {
    modules[k] = [0, 1, 2, 3].map((r) => {
      const id = `${k}@${r}#0`;
      frames[id] = { page: 0, x: 0, y: 0, w: 4, h: 4, pivot: [2, 4], z_base: 0 };
      return [[id, 0]];
    });
  }
  frames['lite/tile'] = { page: 0, x: 0, y: 0, w: 64, h: 32, pivot: [32, 0] };
  return {
    style,
    manifest: {
      version: 1,
      pages: [`${style}-0.png`],
      frames,
      city3d: { style, tile: 'lite/tile', person: [], modules, aliases: { 'parede/cheio/vila-a/3': 'parede/cheio/vila-a/0' } },
    },
    pages: ['AAAA'],
  };
}

describe('citylite model', () => {
  it('merges style atlases with page offsets and no duplicate frames', () => {
    const m = mergeAtlases([payload('vila-a', ['parede/cheio/vila-a/0', 'arvore/oak/any/0']), payload('vila-b', ['telhado/x/vila-b/0', 'arvore/oak/any/0'])]);
    expect(m.json.pages).toHaveLength(2);
    expect(m.json.frames['telhado/x/vila-b/0@1#0'].page).toBe(1);
    expect(m.json.frames['arvore/oak/any/0@0#0'].page).toBe(0);
    expect(m.styles).toEqual(['vila-a', 'vila-b']);
  });

  it('resolves far variants and aliases to the baked module', () => {
    const m = mergeAtlases([payload('vila-a', ['parede/cheio/vila-a/0'])]);
    expect(layersFor(m, 'parede/cheio/vila-a/0#lod1', 2)?.[0][0]).toBe('parede/cheio/vila-a/0@2#0');
    expect(layersFor(m, 'parede/cheio/vila-a/3', 5)?.[0][0]).toBe('parede/cheio/vila-a/0@1#0');
    expect(layersFor(m, 'nada/x/y/0', 0)).toBeNull();
  });

  it('turns the view in quarter turns and back', () => {
    for (let c = 0; c < 4; c++) {
      const d = toDisplay(10, 4, c);
      const w = fromDisplay(d.x, d.y, c);
      expect(w.xm).toBeCloseTo(10);
      expect(w.zm).toBeCloseTo(4);
    }
    // One turn: east goes south (clockwise from above), like CanonMesh::placed.
    const d = toDisplay(2, 0, 1);
    expect(d.x).toBeCloseTo(0);
    expect(d.y).toBeCloseTo(1);
  });

  it('places a module sprite on its dual corner with the rotation added', () => {
    const m = mergeAtlases([payload('vila-a', ['parede/cheio/vila-a/0'])]);
    const inst = { key: 'parede/cheio/vila-a/0', x_mm: 4000, y_mm: 6000, z_mm: 8000, rot: 1, palette: 'p', owner: 'b1' };
    const out = regionSprites([1, 0], [inst], m, new LitePalettes([{ key: 'p', slots: { wall: '#102030' } }]), 1, 3);
    expect(out.missing).toBe(0);
    const s = out.sprites[0];
    expect(s.frame).toBe('parede/cheio/vila-a/0@2#0');
    const want = toDisplay(128 + 4, 8, 1);
    expect(s.x).toBeCloseTo(want.x);
    expect(s.y).toBeCloseTo(want.y);
    expect(s.z).toBeCloseTo(3 * Z_PER_M);
    expect(s.tint).toBe(0x102030);
    expect(stylesOf([inst], 'x')).toEqual(['vila-a']);
  });

  it('turns a terrain mesh into per-tile slots, heights and chunks', () => {
    // One flat quad over tile (1, 2) at 5 m, slot 10 (street).
    const x0 = 2000, z0 = 4000;
    const mesh = {
      region: [0, 0] as [number, number],
      positions: [x0, 5000, z0, x0 + 2000, 5000, z0, x0 + 2000, 5000, z0 + 2000, x0, 5000, z0 + 2000],
      slots: [10, 10, 10, 10],
      indices: [0, 2, 1, 0, 3, 2],
    };
    const t = terrainTiles(mesh);
    expect(t.slot[2 * 64 + 1]).toBe(10);
    expect(t.height[2 * 64 + 1]).toBeCloseTo(5);
    expect(t.minM).toBeCloseTo(5);
    expect(t.shade[2 * 64 + 1]).toBeCloseTo(1);
    for (let c = 0; c < 4; c++) {
      const chunks = new Map<string, ChunkTile[]>();
      regionChunkTiles([0, 0], t, new LitePalettes(), 'lite/tile', c, 5, chunks);
      expect(chunks.size).toBe(16);
      let n = 0;
      for (const list of chunks.values()) {
        n += list.length;
        for (const tile of list) {
          expect(tile.lx).toBeGreaterThanOrEqual(0);
          expect(tile.lx).toBeLessThan(16);
          expect(tile.ly).toBeGreaterThanOrEqual(0);
          expect(tile.ly).toBeLessThan(16);
        }
      }
      expect(n).toBe(64 * 64);
    }
  });

  it('stacks style pages into fewer textures and moves their frames', () => {
    const m = mergeAtlases([payload('vila-a', ['parede/cheio/vila-a/0']), payload('vila-b', ['telhado/x/vila-b/0'])]);
    const plan = stackPlan([{ w: 2048, h: 1088 }, { w: 2048, h: 1088 }], 4096);
    expect(plan.pages).toEqual([{ w: 2048, h: 2176 }]);
    const json = restackJson(m.json, plan);
    expect(json.pages).toHaveLength(1);
    expect(json.frames['telhado/x/vila-b/0@0#0']).toMatchObject({ page: 0, y: 1088 });
    expect(stackPlan([{ w: 2048, h: 3000 }, { w: 2048, h: 3000 }], 4096).pages).toHaveLength(2);
  });
});
