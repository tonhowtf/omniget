// Everything the route has to load before it can draw: the two published
// atlases, a generated page for the HUD bars, and the house map.
//
// The renderer holds one atlas at a time, so the Omni sheet and the casa-v1
// tile sheet are merged here into a single atlas object with the page indices
// rewritten. Frame ids are already namespaced by the atlas tools ('omni/...',
// 'casa-v1/...'), so nothing collides.
//
// The pure half (frame lookup, chunk building) has no DOM in it and is what
// the tests exercise; the loading half touches `fetch` and `createImageBitmap`
// and runs only in the route.

import { resolveDir, type AtlasData } from '$lib/world/render/atlas';
import { frameIndex } from '$lib/world/render/anim';
import { chunkId } from '$lib/world/render/camera';
import { CHUNK_TILES, type ChunkId, type ChunkTile } from '$lib/world/render/types';

export const OMNI_ATLAS_URL = '/world/omni/atlas.json';
export const CASA_ATLAS_URL = '/world/tiles/casa-v1/atlas.json';
/** Visual theme only: the same tile keys and collision metadata as casa-v1. */
export const CRAFT_ATLAS_URL = '/world/tiles/craft-v1/atlas.json';
/** The workshop's town art (ground, houses, props, crops), for the city. */
export const VALE_ATLAS_URL = '/world/tiles/vale-v1/atlas.json';
export const HOUSE_MAP_URL = '/world/house-v1.json';

export const ERR_ASSETS = {
  MAP_MISSING: 'ERR_WORLD_MAP_MISSING',
  MAP_INVALID: 'ERR_WORLD_MAP_INVALID',
  ATLAS_MISSING: 'ERR_WORLD_ATLAS_MISSING',
} as const;

/** Animation ids 0..7 of the crate, in the atlas' spelling. */
export const ANIM_CLIPS = ['idle', 'walk', 'sit', 'sleep', 'work', 'talk', 'wave', 'yawn'] as const;
/** Direction ids 0..7 of the crate: S, SW, W, NW, N, NE, E, SE. */
export const DIR_NAMES = ['S', 'SW', 'W', 'NW', 'N', 'NE', 'E', 'SE'] as const;
/** The sheet that exists; the crate's yawn has no art of its own yet. */
const CLIP_FALLBACK: Record<string, string> = { yawn: 'idle' };

/** The empty index of every map layer. */
export const EMPTY_TILE = 255;
/** Screen pixels of height for one z unit, mirrored from the renderer. */
const TILE_Z = 32;

export interface TileDef {
  key: string;
  height: number;
  occludes: boolean;
  footprint: [number, number];
  walkable: boolean;
}

export interface ChunkDef {
  cx: number;
  cy: number;
  floor: number[];
  wall: number[];
  object: number[];
  height: number[];
  /** Per-cell floor colour, 0xRRGGBB (sRGB), multiplied over the floor sprite; 0 or absent = white. */
  tint?: number[];
}

export interface SlotDef {
  id: string;
  kind: 'floor' | 'wall' | 'table';
  tile: [number, number];
  accepts: string[];
  default?: string | null;
  dir?: number;
}

export interface MapDef {
  version: number;
  id: string;
  atlas: string;
  chunk_tiles: number;
  palette: TileDef[];
  chunks: ChunkDef[];
  slots: SlotDef[];
  objects: { id: number; kind: string; tile: [number, number]; dir: number; slot?: string | null }[];
  rooms: { id: string; rect: [number, number, number, number] }[];
  markers: { name: string; tile: [number, number] }[];
}

export interface WorldAtlas {
  json: unknown;
  pages: ImageBitmap[];
}

interface RawAtlas {
  version: number;
  pages: string[];
  page_sizes?: [number, number][];
  frames: Record<string, { page: number; [k: string]: unknown }>;
  anims?: Record<string, unknown>;
  tiles?: Record<string, unknown>;
}

/**
 * Merges several published atlases into one. Page indices of every atlas but
 * the first are shifted by the pages already taken.
 */
export function mergeAtlases(parts: RawAtlas[]): { json: RawAtlas; pageUrls: string[][] } {
  const out: RawAtlas = { version: 1, pages: [], frames: {}, anims: {}, tiles: {} };
  const pageUrls: string[][] = [];
  for (const part of parts) {
    const offset = out.pages.length;
    out.pages.push(...part.pages);
    pageUrls.push(part.pages);
    for (const [id, frame] of Object.entries(part.frames)) {
      out.frames[id] = { ...frame, page: frame.page + offset };
    }
    Object.assign(out.anims as object, part.anims ?? {});
    Object.assign(out.tiles as object, part.tiles ?? {});
  }
  return { json: out, pageUrls };
}

/**
 * The HUD page: an energy bar track and nine fills, drawn at load time rather
 * than shipped as art, because it is nine rectangles and a border.
 */
export function buildHudAtlasPart(): { part: RawAtlas; draw: (ctx: CanvasRenderingContext2D) => void } {
  const W = 26;
  const H = 4;
  const frames: RawAtlas['frames'] = {
    // Pivot: horizontally centred, bottom on the anchor point.
    'ui/bar/bg': { page: 0, x: 0, y: 0, w: W, h: H, pivot: [W / 2, H], z_base: 0 },
  };
  for (let i = 0; i <= 8; i++) {
    const w = Math.max(1, i * 3);
    frames[`ui/bar/${i}`] = {
      page: 0,
      x: 0,
      y: 6 + i * 4,
      w,
      h: 2,
      // One pixel in from the track's left edge, so every fill starts flush.
      pivot: [W / 2 - 1, 3],
      z_base: 0,
    };
  }
  const draw = (ctx: CanvasRenderingContext2D) => {
    ctx.clearRect(0, 0, 64, 64);
    ctx.fillStyle = '#11131a';
    ctx.fillRect(0, 0, W, H);
    ctx.clearRect(1, 1, W - 2, H - 2);
    ctx.fillStyle = '#ffffff';
    for (let i = 0; i <= 8; i++) {
      ctx.fillRect(0, 6 + i * 4, Math.max(1, i * 3), 2);
    }
  };
  return { part: { version: 1, pages: ['ui-0.png'], frames, anims: {}, tiles: {} }, draw };
}

/** Frame and mirror flag for an agent, from its animation id and direction. */
export function agentSprite(
  atlas: AtlasData,
  anim: number,
  dir: number,
  elapsedMs: number,
): { frame: string; flip: boolean } {
  const clipName = ANIM_CLIPS[anim] ?? 'idle';
  const dirName = DIR_NAMES[dir] ?? 'S';
  const tryClip = (name: string) => {
    const id = `omni/${name}`;
    if (!atlas.anims.has(id)) return null;
    try {
      return resolveDir(atlas, id, dirName as never);
    } catch {
      return null;
    }
  };
  const resolved = tryClip(clipName) ?? tryClip(CLIP_FALLBACK[clipName] ?? 'idle') ?? tryClip('idle');
  if (!resolved) return { frame: `omni/idle/S/0`, flip: false };
  const idx = frameIndex(elapsedMs, resolved.fps, resolved.frames.length, resolved.loop);
  return { frame: resolved.frames[idx], flip: resolved.flip };
}

/** Atlas frame of a placed object, by the palette key the snapshot carries. */
export function objectFrame(atlas: AtlasData, kind: string): string | null {
  return atlas.tiles.get(kind)?.frame ?? null;
}

/** Elevation, in tile units, of a map height given in atlas pixels. */
export function zFromHeight(px: number): number {
  return px / TILE_Z;
}

/**
 * The baked contents of every chunk of the map: floor first, then the wall
 * layer at its own elevation, then the static object layer. Dynamic objects
 * are not here — they arrive in the snapshot and are drawn as sprites, because
 * the player moves them between slots.
 */
export function buildChunkTiles(map: MapDef, atlas: AtlasData): Map<ChunkId, ChunkTile[]> {
  const out = new Map<ChunkId, ChunkTile[]>();
  const frameOf = (idx: number): string | null => {
    const def = map.palette[idx];
    if (!def) return null;
    return atlas.tiles.get(def.key)?.frame ?? null;
  };
  for (const chunk of map.chunks) {
    const tiles: ChunkTile[] = [];
    for (const layer of ['floor', 'wall', 'object'] as const) {
      const cells = chunk[layer];
      if (!Array.isArray(cells)) continue;
      for (let i = 0; i < CHUNK_TILES * CHUNK_TILES; i++) {
        const idx = cells[i];
        if (idx === undefined || idx === EMPTY_TILE) continue;
        const frame = frameOf(idx);
        if (!frame) continue;
        const tint = layer === 'floor' ? chunk.tint?.[i] : undefined;
        tiles.push({
          frame,
          lx: i % CHUNK_TILES,
          ly: Math.floor(i / CHUNK_TILES),
          z: layer === 'floor' ? 0 : zFromHeight(map.palette[idx]?.height ?? 0),
          tint: tint && tint !== 0xffffff ? tint : undefined,
        });
      }
    }
    out.set(chunkId(chunk.cx, chunk.cy), tiles);
  }
  return out;
}

/** The tile a named marker sits on, for the opening camera position. */
export function marker(map: MapDef, name: string): { x: number; y: number } | null {
  const m = map.markers?.find((it) => it.name === name);
  return m ? { x: m.tile[0], y: m.tile[1] } : null;
}

/** The slot a tile belongs to, for the "click a slot to place an object" menu. */
export function slotAt(map: MapDef, x: number, y: number): SlotDef | null {
  return map.slots?.find((s) => s.tile[0] === x && s.tile[1] === y) ?? null;
}

async function json<T>(url: string, code: string): Promise<T> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${code}: ${url} (${res.status})`);
  return (await res.json()) as T;
}

/** Loads both published atlases plus the generated HUD page. */
export async function loadWorldAtlas(tileAtlasUrl = CRAFT_ATLAS_URL, extraTileAtlasUrls: string[] = []): Promise<WorldAtlas> {
  const urls = [tileAtlasUrl, ...extraTileAtlasUrls];
  const [omni, ...tileAtlases] = await Promise.all([
    json<RawAtlas>(OMNI_ATLAS_URL, ERR_ASSETS.ATLAS_MISSING),
    ...urls.map((u) => json<RawAtlas>(u, ERR_ASSETS.ATLAS_MISSING)),
  ]);
  const hud = buildHudAtlasPart();
  const merged = mergeAtlases([omni, ...tileAtlases, hud.part]);
  const pages = await Promise.all([
    ...omni.pages.map((p) => bitmap(`/world/omni/${p}`)),
    ...tileAtlases.flatMap((atlas, i) => atlas.pages.map((p) => bitmap(`${urls[i].slice(0, urls[i].lastIndexOf('/') + 1)}${p}`))),
    hudBitmap(hud.draw),
  ]);
  return { json: merged.json, pages };
}

async function bitmap(url: string): Promise<ImageBitmap> {
  const res = await fetch(url);
  if (!res.ok) throw new Error(`${ERR_ASSETS.ATLAS_MISSING}: ${url} (${res.status})`);
  return await createImageBitmap(await res.blob());
}

async function hudBitmap(draw: (ctx: CanvasRenderingContext2D) => void): Promise<ImageBitmap> {
  const canvas =
    typeof OffscreenCanvas !== 'undefined'
      ? new OffscreenCanvas(64, 64)
      : Object.assign(document.createElement('canvas'), { width: 64, height: 64 });
  const ctx = (canvas as HTMLCanvasElement).getContext('2d') as CanvasRenderingContext2D | null;
  if (!ctx) throw new Error(ERR_ASSETS.ATLAS_MISSING);
  draw(ctx);
  return await createImageBitmap(canvas as unknown as CanvasImageSource);
}

/** Loads the house. Its absence is a named error, not a blank screen. */
export async function loadHouseMap(mapUrl = HOUSE_MAP_URL): Promise<MapDef> {
  const map = await json<MapDef>(mapUrl, ERR_ASSETS.MAP_MISSING);
  if (map?.version !== 1 || !Array.isArray(map.chunks) || map.chunks.length === 0) {
    throw new Error(`${ERR_ASSETS.MAP_INVALID}: ${mapUrl}`);
  }
  if (map.chunk_tiles !== CHUNK_TILES) {
    throw new Error(`${ERR_ASSETS.MAP_INVALID}: chunk_tiles ${map.chunk_tiles}`);
  }
  return map;
}
