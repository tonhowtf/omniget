// A synthetic region with the same shape the mesher outputs: 4×4-tile
// buildings on a street grid, a column of dual-grid modules per corner
// (terreo, parede, telhado), trees, benches and fences, flat-ish terrain.
// Deterministic for a given (region, target). Used by the bench (2k and 10k
// instances) and the calibration scene until real dumps are available.

import {
  PALETTE_SLOTS,
  REGION_TILES,
  TILE_MM,
  LEVEL_MM,
  type Blueprint,
  type Instance,
  type Palette,
  type Surface,
} from '$lib/city3d/types';
import type { RegionMeshData } from './region';

export const SYNTH_STYLE = 'bench';

export const SYNTH_PALETTES: Palette[] = [
  { key: 'a', slots: { wall: '#f1e4cc', roof: '#b4533c', trim: '#fbf6ee', glass: '#83b8cf', stone: '#a7a097', door: '#5a3a2b' } },
  { key: 'b', slots: { wall: '#d9e6ee', roof: '#4f6f8f', trim: '#f4f4f4', glass: '#8fc1d6', stone: '#9c9a94', door: '#2f3e4f' } },
  { key: 'c', slots: { wall: '#f3d9c4', roof: '#7d8a4a', trim: '#fff4e0', glass: '#7fb2c9', stone: '#b0a38e', door: '#6b3b2a' } },
];

function hash(n: number): number {
  let h = Math.imul(n ^ 0x9e3779b9, 0x85ebca6b);
  h ^= h >>> 13;
  h = Math.imul(h, 0xc2b2ae35);
  h ^= h >>> 16;
  return h >>> 0;
}

function isStreet(x: number, y: number): boolean {
  const mx = x % 16;
  const my = y % 16;
  return mx === 7 || mx === 8 || my === 7 || my === 8;
}

export function synthRegion(region: [number, number], target: number): { mesh: RegionMeshData; blueprint: Blueprint } {
  const instances: Instance[] = [];
  const push = (key: string, cx: number, cy: number, k: number, rot: number, palette: string, owner: string | null, yExtra = 0) =>
    instances.push({
      key,
      x_mm: cx * TILE_MM,
      y_mm: k * LEVEL_MM + yExtra,
      z_mm: cy * TILE_MM,
      rot,
      palette,
      owner,
    });

  // Building sites: 4×4 footprints inside each 16×16 block, off the streets.
  const sites: Array<[number, number]> = [];
  for (let by = 0; by < REGION_TILES; by += 16) {
    for (let bx = 0; bx < REGION_TILES; bx += 16) {
      for (const [ox, oy] of [
        [1, 1],
        [10, 1],
        [1, 10],
        [10, 10],
        [1, 5],
        [10, 5],
      ]) {
        const x = bx + ox;
        const y = by + oy;
        if (x + 4 < REGION_TILES && y + 4 < REGION_TILES) sites.push([x, y]);
      }
    }
  }
  // Per building ~ 25 corners × (floors + 1): pick floors to hit the target.
  const perFloor = 25;
  const treesAndProps = Math.round(target * 0.08);
  const floors = Math.max(1, Math.round((target - treesAndProps) / (sites.length * perFloor) - 1));

  let b = 0;
  for (const [bx, by] of sites) {
    if (instances.length >= target - treesAndProps) break;
    const id = `b${region[0]}_${region[1]}_${b}`;
    const palette = SYNTH_PALETTES[hash(b + region[0] * 97 + region[1] * 31) % 3].key;
    const f = Math.max(1, floors + (hash(b * 7) % 3) - 1);
    b++;
    for (let cy = by; cy <= by + 4; cy++) {
      for (let cx = bx; cx <= bx + 4; cx++) {
        const ex = cx === bx ? -1 : cx === bx + 4 ? 1 : 0;
        const ey = cy === by ? -1 : cy === by + 4 ? 1 : 0;
        let kase = 'cheio';
        let rot = 0;
        if (ex && ey) {
          kase = 'convexo';
          rot = ex < 0 && ey < 0 ? 0 : ex > 0 && ey < 0 ? 1 : ex > 0 && ey > 0 ? 2 : 3;
        } else if (ex || ey) {
          kase = 'parede';
          rot = ey < 0 ? 0 : ex > 0 ? 1 : ey > 0 ? 2 : 3;
        }
        const v = hash(cx * 131 + cy * 17 + b) % 2;
        const door = ey < 0 && !ex && cx === bx + 2;
        push(door ? `porta/parede/${SYNTH_STYLE}/0` : `terreo/${kase}/${SYNTH_STYLE}/${v}`, cx, cy, 0, rot, palette, id);
        for (let k = 1; k < f; k++) push(`parede/${kase}/${SYNTH_STYLE}/${v}`, cx, cy, k, rot, palette, id);
        push(`telhado/${kase}/${SYNTH_STYLE}/0`, cx, cy, f, rot, palette, id);
      }
    }
  }

  const surface: Surface[] = new Array(REGION_TILES * REGION_TILES);
  for (let y = 0; y < REGION_TILES; y++) for (let x = 0; x < REGION_TILES; x++) surface[y * REGION_TILES + x] = isStreet(x, y) ? 'street' : 'grass';

  // Trees and benches along the streets; fences between.
  const props: Blueprint['props'] = [];
  let p = 0;
  for (let i = 0; instances.length < target && i < REGION_TILES * REGION_TILES * 4; i++) {
    const h = hash(i + 7919);
    const x = h % REGION_TILES;
    const y = (h >>> 8) % REGION_TILES;
    if (isStreet(x, y)) continue;
    const near = isStreet(x + 1, y) || isStreet(x, y + 1);
    const kind = p % 5;
    if (kind === 0 && near) {
      push(`prop/banco/any/0`, x + 1, y + 1, 0, (h >>> 20) & 3, 'default', `prop${p}`);
      props.push({ kind: 'banco', preset: 'banco', cell: [x, y], seed: h, anchors: [{ action: 'sit', cell: [x, y], level: 0, yaw: 0, capacity: 2 }] });
    } else if (kind === 1 || kind === 2) {
      push(`arvore/oak-estilizada/any/${h % 3}`, x + 1, y + 1, 0, 0, 'default', `tree${p}`);
    } else {
      push(`enfeite/cerca/${SYNTH_STYLE}/0`, x + 1, y + 1, 0, (h >>> 20) & 3, 'default', null);
    }
    p++;
  }

  // Terrain: 65×65 corners, a gentle roll, streets and grass by vertex.
  const positions: number[] = [];
  const slots: number[] = [];
  const heights: number[] = [];
  const street = PALETTE_SLOTS.indexOf('street');
  const grass = PALETTE_SLOTS.indexOf('grass');
  for (let y = 0; y <= REGION_TILES; y++) {
    for (let x = 0; x <= REGION_TILES; x++) {
      const hmm = 0;
      heights.push(hmm);
      positions.push(x * TILE_MM, hmm, y * TILE_MM);
      const s = isStreet(Math.min(63, x), Math.min(63, y)) || isStreet(Math.max(0, x - 1), Math.max(0, y - 1));
      slots.push(s ? street : grass);
    }
  }
  const indices: number[] = [];
  for (let y = 0; y < REGION_TILES; y++) {
    for (let x = 0; x < REGION_TILES; x++) {
      const a = y * 65 + x;
      const b2 = a + 1;
      const c = a + 65;
      const d = c + 1;
      indices.push(a, c, b2, b2, c, d);
    }
  }

  const mesh: RegionMeshData = {
    instances: { region, blueprint_hash: 'synthetic', instances, hash: `synthetic:${target}` },
    terrain: { region, positions, slots, indices },
  };
  const blueprint: Blueprint = {
    schema: 'synthetic',
    city_id: 'bench',
    region,
    generator_version: 'synthetic',
    program_hash: '',
    surface,
    corner_height_mm: heights,
    streets: [],
    lots: [],
    buildings: [],
    props,
    places: [],
    hash: 'synthetic',
  };
  return { mesh, blueprint };
}
