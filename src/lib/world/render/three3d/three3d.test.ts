import { describe, expect, it } from 'vitest';
import { Matrix4, Vector3 } from 'three';
import { fallbackKit, fallbackParts, parseKey, type Kit } from './kit';
import { createMaterials, buildBrushNoise } from './materials';
import { PaletteBook } from './palette';
import { applyCellLod, buildRegion, instanceMatrix, REGION_M } from './region';
import { parseStatus } from './source';
import { SYNTH_PALETTES, synthRegion } from './synth';
import { TIER3D_BUDGETS } from '../tier';

describe('synthetic region', () => {
  it('hits the requested instance counts deterministically', () => {
    for (const n of [2000, 10000]) {
      const a = synthRegion([0, 0], n).mesh.instances.instances;
      const b = synthRegion([0, 0], n).mesh.instances.instances;
      expect(a.length).toBe(n);
      expect(JSON.stringify(a)).toBe(JSON.stringify(b));
    }
  });
});

describe('kit fallback', () => {
  it('gives every layer some geometry and nothing for vazio', () => {
    for (const key of ['terreo/cheio/x/0', 'parede/convexo/x/1', 'telhado/parede/x/0', 'porta/parede/x/0', 'arvore/oak/any/2', 'prop/banco/any/0', 'enfeite/cerca/x/0', 'obra/andaime/x/0', 'torre/cheio/x/0']) {
      expect(fallbackParts(key).length, key).toBeGreaterThan(0);
    }
    expect(fallbackParts('parede/vazio/x/0')).toHaveLength(0);
    expect(parseKey('a/b/c/d')).toEqual({ layer: 'a', kase: 'b', style: 'c', variant: 'd' });
  });
});

describe('region build', () => {
  it('groups instances by (key, slot) and keeps picks per instance', () => {
    const { mesh } = synthRegion([1, 2], 2000);
    const built = buildRegion(mesh, fallbackKit('bench'), new PaletteBook(SYNTH_PALETTES), createMaterials(), { outline: true, shadows: true });
    expect(built.instanceCount).toBe(2000);
    expect(built.group.position.x).toBe(REGION_M);
    expect(built.group.position.z).toBe(2 * REGION_M);
    const meshes = [...built.structure, ...built.detail.map((d) => d.mesh)];
    const keys = new Set(meshes.map((m) => m.name.split('|')[0]));
    expect(keys.size).toBeGreaterThan(8);
    for (const m of meshes) expect((m.userData.picks as unknown[]).length).toBe(m.count);
    expect(built.outlines.length).toBeGreaterThan(0);
    expect(built.terrain).not.toBeNull();
    built.dispose();
  });

  it('swaps cells to the far variants past the LOD distance', () => {
    const { mesh } = synthRegion([0, 0], 2000);
    const base = fallbackKit('bench');
    // Every non-tree key gets a one-part far variant; trees keep none.
    const far = new Map<string, ReturnType<Kit['parts']>>();
    const kit: Kit = {
      ...base,
      farParts(key) {
        if (parseKey(key).layer === 'arvore') return null;
        let p = far.get(key);
        if (!p) {
          p = [base.parts(key)[0]];
          far.set(key, p);
        }
        return p;
      },
    };
    const built = buildRegion(mesh, kit, new PaletteBook(SYNTH_PALETTES), createMaterials(), { outline: true, shadows: true });
    expect(built.cells.length).toBeGreaterThan(4);
    const cell = built.cells.find((c) => !c.detail && c.near.length > 0 && c.far.length > 0)!;
    expect(cell).toBeTruthy();
    const b = { lodDistance: 60, detailDistance: 40, outline: true };
    const at = (d: number) => cell.center.clone().add(new Vector3(0, d, 0));
    applyCellLod(built, at(10), b);
    expect(cell.near.every((m) => m.visible)).toBe(true);
    expect(cell.far.every((m) => !m.visible)).toBe(true);
    expect(cell.outlines.some((o) => o.visible)).toBe(true);
    applyCellLod(built, at(200), b);
    expect(cell.near.every((m) => !m.visible)).toBe(true);
    expect(cell.far.every((m) => m.visible)).toBe(true);
    expect(cell.both.every((m) => m.visible)).toBe(true);
    expect(cell.outlines.every((o) => !o.visible)).toBe(true);
    // Detail cells disappear past the detail distance.
    for (const c of built.cells.filter((c) => c.detail)) {
      if (c.center.distanceTo(at(200)) > b.detailDistance) expect([...c.near, ...c.far, ...c.both].every((m) => !m.visible)).toBe(true);
    }
    built.dispose();
  });

  it('turns a quarter clockwise seen from above', () => {
    const m = instanceMatrix({ key: 'k', x_mm: 2000, y_mm: 0, z_mm: 0, rot: 1, palette: 'a', owner: null }, new Matrix4());
    // North (−z) rotated clockwise from above points east (+x).
    const v = new Vector3(0, 0, -1).applyMatrix4(m).sub(new Vector3(2, 0, 0));
    expect(v.x).toBeCloseTo(1);
    expect(v.z).toBeCloseTo(0);
  });
});

describe('status and tiers', () => {
  it('reads palettes, style and regions out of a charter', () => {
    const info = parseStatus({ mode: 'local', city: { charter: { style: 'vila', palette: 'p', palettes: [{ key: 'p', slots: {} }], districts: [{ regions: [[0, 0], [1, 0]] }] } } });
    expect(info.exists).toBe(true);
    expect(info.style).toBe('vila');
    expect(info.regions).toEqual([[0, 0], [1, 0]]);
    expect(parseStatus({ mode: 'local', city: null }).exists).toBe(false);
  });

  it('never turns a 3D effect on as the tier goes down', () => {
    for (let t = 1; t <= 3; t++) {
      const lo = TIER3D_BUDGETS[(t - 1) as 0 | 1 | 2];
      const hi = TIER3D_BUDGETS[t as 1 | 2 | 3];
      expect(lo.drawDistance).toBeLessThanOrEqual(hi.drawDistance);
      expect(lo.shadowMap).toBeLessThanOrEqual(hi.shadowMap);
      expect(lo.instances).toBeLessThanOrEqual(hi.instances);
      expect(lo.fullCharacters).toBeLessThanOrEqual(hi.fullCharacters);
      expect(Number(lo.outline)).toBeLessThanOrEqual(Number(hi.outline));
    }
  });

  it('builds the brush noise in range and deterministically', () => {
    const a = buildBrushNoise(32);
    expect(a.length).toBe(1024);
    expect(buildBrushNoise(32)).toEqual(a);
  });
});
