// The lite city's pure half (estudo 99, arquivo 09 §5.5): the sprite atlases
// the Rust side bakes from the kit (`city3d_kit_atlas`), merged into one; the
// 3D city's instances and terrain turned into 2D sprites and chunk tiles; and
// the view rotation. No GL, no DOM, no three.js.
//
// Units: the 3D city is in millimetres (x east, y up, z south); the 2D world
// is in tiles (1 tile = 2 m) with elevation in `TILE_Z` pixels. The baked
// sprites use the true orthographic projection of the 2:1 diamond, where one
// metre of height is `Z_PX_PER_M` pixels, so `z = metres * Z_PX_PER_M / TILE_Z`.

import { PALETTE_SLOTS, REGION_TILES, TILE_MM, type Blueprint, type Instance, type Lot, type Palette, type TerrainMesh } from '$lib/city3d/types';
import { DEFAULT_PALETTE } from '../three3d/palette-data';
import { CHUNK_TILES, TILE_Z, type ChunkTile, type SpriteInst } from '../types';

/** Screen pixels of one metre of height (cos 30° of 16√2 px/m). */
export const Z_PX_PER_M = 19.595917942265423;
export const Z_PER_M = Z_PX_PER_M / TILE_Z;
export const TILE_M = TILE_MM / 1000;
export const REGION_M = REGION_TILES * TILE_M;
/** Chunk textures hold this much elevation above the datum (half the texture). */
export const CHUNK_Z_MAX = CHUNK_TILES - 1;

/** One slot layer of a module under one rotation: `[frame id, slot index]`. */
export type LayerRef = [string, number];

/** The `city3d` block of a baked atlas manifest. */
export interface LiteManifestBlock {
  style: string;
  kit_hash?: string;
  bake?: string;
  tile: string;
  person: LayerRef[];
  /** key → 4 rotations → layers. */
  modules: Record<string, LayerRef[][]>;
  aliases: Record<string, string>;
}

/** What `city3d_kit_atlas` returns. */
export interface KitAtlasPayload {
  style: string;
  manifest: Record<string, unknown> & { pages: string[]; frames: Record<string, Record<string, unknown>>; city3d: LiteManifestBlock };
  /** PNG pages, base64, in `manifest.pages` order. */
  pages: string[];
}

/** Several style atlases as one (the 2D renderer holds a single atlas). */
export interface MergedAtlas {
  /** Atlas JSON for `Renderer.loadAtlas` (format of `atlas.ts`). */
  json: { version: 1; pages: string[]; frames: Record<string, Record<string, unknown>> };
  /** Base64 PNG of each page, same order as `json.pages`. */
  pages: string[];
  modules: Map<string, LayerRef[][]>;
  aliases: Map<string, string>;
  tile: string;
  person: LayerRef[];
  styles: string[];
}

export function mergeAtlases(list: readonly KitAtlasPayload[]): MergedAtlas {
  const out: MergedAtlas = {
    json: { version: 1, pages: [], frames: {} },
    pages: [],
    modules: new Map(),
    aliases: new Map(),
    tile: 'lite/tile',
    person: [],
    styles: [],
  };
  for (const a of list) {
    const block = a.manifest.city3d;
    // Pages whose frames all exist already (the shared `any` keys) still load:
    // a page mixes both, and skipping frames is enough to keep ids unique.
    const offset = out.json.pages.length;
    a.manifest.pages.forEach((p, i) => {
      out.json.pages.push(`${a.style}/${p}`);
      out.pages.push(a.pages[i] ?? '');
    });
    for (const [id, f] of Object.entries(a.manifest.frames)) {
      if (out.json.frames[id]) continue;
      out.json.frames[id] = { ...f, page: Number(f.page) + offset };
    }
    for (const [k, rots] of Object.entries(block.modules ?? {})) if (!out.modules.has(k)) out.modules.set(k, rots);
    for (const [k, v] of Object.entries(block.aliases ?? {})) if (!out.aliases.has(k)) out.aliases.set(k, v);
    if (block.tile) out.tile = block.tile;
    if (out.person.length === 0 && block.person) out.person = block.person;
    out.styles.push(a.style);
  }
  return out;
}

/**
 * Where each page goes when pages are stacked into taller ones (up to
 * `maxSide` pixels): sprites of two styles interleave after the depth sort,
 * and every page switch is a draw call, so fewer pages is fewer calls.
 */
export function stackPlan(
  sizes: ReadonlyArray<{ w: number; h: number }>,
  maxSide: number,
): { place: Array<{ page: number; y: number }>; pages: Array<{ w: number; h: number }> } {
  const place: Array<{ page: number; y: number }> = [];
  const pages: Array<{ w: number; h: number }> = [];
  for (const s of sizes) {
    const cur = pages[pages.length - 1];
    if (cur && cur.h + s.h <= maxSide && s.w <= maxSide) {
      place.push({ page: pages.length - 1, y: cur.h });
      cur.h += s.h;
      cur.w = Math.max(cur.w, s.w);
    } else {
      place.push({ page: pages.length, y: 0 });
      pages.push({ w: s.w, h: s.h });
    }
  }
  return { place, pages };
}

/** The merged atlas JSON with its frames moved to the stacked pages. */
export function restackJson(json: MergedAtlas['json'], plan: ReturnType<typeof stackPlan>): MergedAtlas['json'] {
  const frames: MergedAtlas['json']['frames'] = {};
  for (const [id, f] of Object.entries(json.frames)) {
    const at = plan.place[Number(f.page)];
    frames[id] = at ? { ...f, page: at.page, y: Number(f.y) + at.y } : f;
  }
  return { version: 1, pages: plan.pages.map((_, i) => `stack-${i}.png`), frames };
}

/** Layers of a key under a rotation; `#lod1` keys and aliases resolve to the baked module. */
export function layersFor(atlas: MergedAtlas, key: string, rot: number): LayerRef[] | null {
  let k = key.endsWith('#lod1') ? key.slice(0, -5) : key;
  let rots = atlas.modules.get(k);
  if (!rots) {
    const served = atlas.aliases.get(k);
    if (served) {
      k = served;
      rots = atlas.modules.get(k);
    }
  }
  if (!rots) return null;
  return rots[rot & 3] ?? null;
}

/** Styles a list of instances needs an atlas for (the shared `any` rides along). */
export function stylesOf(instances: readonly Instance[], fallback: string): string[] {
  const s = new Set<string>();
  for (const i of instances) {
    const st = i.key.split('/')[2];
    if (st && st !== 'any') s.add(st);
  }
  if (s.size === 0 && fallback && fallback !== 'default' && fallback !== 'bench') s.add(fallback);
  if (s.size === 0) s.add('vila-colonial');
  return [...s].sort();
}

// ── view rotation ───────────────────────────────────────────────────────────

/**
 * World metres (x east, z south) to display tiles under `c` quarter turns of
 * the view. Turning the view by `c` is turning every module by `c`, the same
 * `(x, z) → (−z, x)` step as `CanonMesh::placed` and the 3D instance matrix.
 */
export function toDisplay(xm: number, zm: number, c: number): { x: number; y: number } {
  let x = xm;
  let z = zm;
  for (let i = 0; i < (c & 3); i++) {
    const t = x;
    x = -z;
    z = t;
  }
  return { x: x / TILE_M, y: z / TILE_M };
}

/** Display tiles back to world metres. */
export function fromDisplay(x: number, y: number, c: number): { xm: number; zm: number } {
  let a = x * TILE_M;
  let b = y * TILE_M;
  for (let i = 0; i < (c & 3); i++) {
    const t = a;
    a = b;
    b = -t;
  }
  return { xm: a, zm: b };
}

// ── palettes ──────────────────────────────────────────────────────────────

function hexToInt(hex: string | undefined): number | null {
  if (!hex) return null;
  const h = hex.trim().replace(/^#/, '');
  if (!/^[0-9a-fA-F]{6}$/.test(h)) return null;
  return parseInt(h, 16);
}

/** Palette key + slot → 0xRRGGBB, with the built-in palette as the floor. */
export class LitePalettes {
  private readonly map = new Map<string, Palette>();
  private readonly cache = new Map<string, number>();
  private fallback = 'default';

  constructor(palettes: readonly Palette[] = [], defaultKey?: string) {
    this.map.set('default', DEFAULT_PALETTE);
    for (const p of palettes) if (p?.key) this.map.set(p.key, p);
    if (defaultKey && this.map.has(defaultKey)) this.fallback = defaultKey;
  }

  color(palette: string | null | undefined, slot: number): number {
    const id = `${palette ?? ''}|${slot}`;
    const hit = this.cache.get(id);
    if (hit !== undefined) return hit;
    const name = PALETTE_SLOTS[slot] ?? 'ground';
    const c =
      hexToInt(this.map.get(palette ?? '')?.slots[name]) ??
      hexToInt(this.map.get(this.fallback)?.slots[name]) ??
      hexToInt(DEFAULT_PALETTE.slots[name]) ??
      0xcccccc;
    this.cache.set(id, c);
    return c;
  }

  ground(slot: number): number {
    return this.color(this.fallback, slot);
  }
}

export function shadeTint(tint: number, k: number): number {
  const r = Math.min(255, Math.round(((tint >> 16) & 0xff) * k));
  const g = Math.min(255, Math.round(((tint >> 8) & 0xff) * k));
  const b = Math.min(255, Math.round((tint & 0xff) * k));
  return (r << 16) | (g << 8) | b;
}

// ── instances → sprites ─────────────────────────────────────────────────────

/** Draw order among modules at the same spot (the sort is stable). */
const LAYER_ORDER: Record<string, number> = {
  terreo: 0, parede: 1, torre: 2, junta: 3, porta: 4, bay: 5, telhado: 6, enfeite: 7, obra: 8, prop: 9, arvore: 10,
};

export const DETAIL_LAYERS = new Set(['enfeite', 'prop']);

export interface LiteSprite extends SpriteInst {
  /** Index into the region's instance list; -1 for people. */
  inst: number;
  /** 0 = always, 1 = detail (hidden when zoomed out), 2 = facade overlay. */
  lod: 0 | 1 | 2;
}

export interface RegionSprites {
  sprites: LiteSprite[];
  /** Instances whose key has no frames (neither baked nor aliased). */
  missing: number;
}

/**
 * Sprites of a region's instances: one per slot layer, at the dual cell
 * centre in display tiles, elevation above `datumM`, tinted by the palette.
 */
export function regionSprites(
  region: [number, number],
  instances: readonly Instance[],
  atlas: MergedAtlas,
  palettes: LitePalettes,
  rotation: number,
  datumM: number,
): RegionSprites {
  const order = instances.map((_, i) => i);
  const layerOf = (i: number) => LAYER_ORDER[instances[i].key.split('/')[0]] ?? 5;
  order.sort((a, b) => layerOf(a) - layerOf(b) || a - b);
  const ox = region[0] * REGION_M;
  const oz = region[1] * REGION_M;
  const sprites: LiteSprite[] = [];
  let missing = 0;
  for (const i of order) {
    const inst = instances[i];
    const rot = ((inst.rot & 3) + rotation) & 3;
    const layers = layersFor(atlas, inst.key, rot);
    if (!layers) {
      missing++;
      continue;
    }
    const layer = inst.key.split('/')[0];
    const lod: LiteSprite['lod'] = DETAIL_LAYERS.has(layer) ? 1 : layer === 'bay' ? 2 : 0;
    const p = toDisplay(ox + inst.x_mm / 1000, oz + inst.z_mm / 1000, rotation);
    const z = (inst.y_mm / 1000 - datumM) * Z_PER_M;
    for (const [frame, slot] of layers) {
      sprites.push({ frame, x: p.x, y: p.y, z, tint: palettes.color(inst.palette, slot), inst: i, lod });
    }
  }
  return { sprites, missing };
}

// ── terrain → tiles ───────────────────────────────────────────────────────

export interface TerrainTiles {
  /** Palette slot of each tile, row-major 64×64 (y * 64 + x). */
  slot: Uint8Array;
  /** Mean height of each tile, metres. */
  height: Float32Array;
  /** Brightness from the slope, 0.6..1. */
  shade: Float32Array;
  minM: number;
  maxM: number;
}

const LIGHT = (() => {
  const l = [0.3, 0.85, 0.6];
  const n = Math.hypot(l[0], l[1], l[2]);
  return [l[0] / n, l[1] / n, l[2] / n];
})();

/**
 * Per-tile slot (the largest triangle wins), mean height and slope shade from
 * the region's terrain mesh (vertex positions in mm, one slot per vertex).
 */
export function terrainTiles(t: TerrainMesh): TerrainTiles {
  const n = REGION_TILES * REGION_TILES;
  const slot = new Uint8Array(n).fill(7);
  const best = new Float32Array(n);
  const sum = new Float64Array(n);
  const cnt = new Uint16Array(n);
  const shade = new Float32Array(n).fill(1);
  let minM = Infinity;
  let maxM = -Infinity;
  const P = t.positions;
  for (let k = 0; k + 2 < t.indices.length; k += 3) {
    const a = t.indices[k] * 3;
    const b = t.indices[k + 1] * 3;
    const c = t.indices[k + 2] * 3;
    const cx = (P[a] + P[b] + P[c]) / 3;
    const cz = (P[a + 2] + P[b + 2] + P[c + 2]) / 3;
    const tx = Math.floor(cx / TILE_MM);
    const tz = Math.floor(cz / TILE_MM);
    if (tx < 0 || tz < 0 || tx >= REGION_TILES || tz >= REGION_TILES) continue;
    const i = tz * REGION_TILES + tx;
    const ux = P[b] - P[a], uy = P[b + 1] - P[a + 1], uz = P[b + 2] - P[a + 2];
    const vx = P[c] - P[a], vy = P[c + 1] - P[a + 1], vz = P[c + 2] - P[a + 2];
    let nx = uy * vz - uz * vy;
    let ny = uz * vx - ux * vz;
    let nz = ux * vy - uy * vx;
    const len = Math.hypot(nx, ny, nz) || 1;
    const area = len / 2;
    nx /= len;
    ny /= len;
    nz /= len;
    if (ny < 0) {
      nx = -nx;
      ny = -ny;
      nz = -nz;
    }
    for (const v of [a, b, c]) {
      const h = P[v + 1] / 1000;
      sum[i] += h;
      cnt[i]++;
      if (h < minM) minM = h;
      if (h > maxM) maxM = h;
    }
    if (area > best[i]) {
      best[i] = area;
      slot[i] = t.slots[t.indices[k]] ?? 7;
      const lam = Math.max(0, nx * LIGHT[0] + ny * LIGHT[1] + nz * LIGHT[2]);
      shade[i] = Math.max(0.6, Math.min(1, (0.55 + 0.45 * lam) / (0.55 + 0.45 * LIGHT[1])));
    }
  }
  const height = new Float32Array(n);
  for (let i = 0; i < n; i++) height[i] = cnt[i] ? sum[i] / cnt[i] : Number.isFinite(minM) ? minM : 0;
  if (!Number.isFinite(minM)) {
    minM = 0;
    maxM = 0;
  }
  return { slot, height, shade, minM, maxM };
}

/** Walkable ground for the lite people: streets, sidewalks, plazas. */
export function walkableSlot(slot: number): boolean {
  const name = PALETTE_SLOTS[slot];
  return name === 'street' || name === 'plaza';
}

/**
 * Chunk tiles of one region under the view rotation, grouped by display
 * chunk id (`cx,cy`). A rotated tile keeps its unit square; its north corner
 * on screen is the square's minimum corner.
 */
export function regionChunkTiles(
  region: [number, number],
  tiles: TerrainTiles,
  palettes: LitePalettes,
  tileFrame: string,
  rotation: number,
  datumM: number,
  into: Map<string, ChunkTile[]>,
): void {
  const ox = region[0] * REGION_TILES;
  const oy = region[1] * REGION_TILES;
  for (let ty = 0; ty < REGION_TILES; ty++) {
    for (let tx = 0; tx < REGION_TILES; tx++) {
      const i = ty * REGION_TILES + tx;
      // Centre of the tile, turned, then back to its minimum corner.
      const c = toDisplay((ox + tx + 0.5) * TILE_M, (oy + ty + 0.5) * TILE_M, rotation);
      const dx = Math.floor(c.x);
      const dy = Math.floor(c.y);
      const cx = Math.floor(dx / CHUNK_TILES);
      const cy = Math.floor(dy / CHUNK_TILES);
      const id = `${cx},${cy}`;
      let list = into.get(id);
      if (!list) {
        list = [];
        into.set(id, list);
      }
      const z = Math.max(0, Math.min(CHUNK_Z_MAX, (tiles.height[i] - datumM) * Z_PER_M));
      list.push({
        frame: tileFrame,
        lx: dx - cx * CHUNK_TILES,
        ly: dy - cy * CHUNK_TILES,
        z,
        tint: shadeTint(palettes.ground(tiles.slot[i]), tiles.shade[i]),
      });
    }
  }
}

// ── selection helpers (the same answers as the 3D picking) ──────────────────

export function lotAt(bp: Blueprint, [x, y]: [number, number]): Lot | null {
  for (const lot of bp.lots ?? []) {
    const [lx, ly, w, h] = lot.rect;
    if (x >= lx && x < lx + w && y >= ly && y < ly + h) return lot;
  }
  return null;
}

/** Region tile of an instance (the dual corner's south-east tile, clamped). */
export function instanceTile(inst: Instance): [number, number] {
  const clamp = (v: number) => Math.max(0, Math.min(REGION_TILES - 1, v));
  return [clamp(Math.floor(inst.x_mm / TILE_MM)), clamp(Math.floor(inst.z_mm / TILE_MM))];
}
