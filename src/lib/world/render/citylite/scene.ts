// The lite city: the 3D city's regions drawn by the 2D renderer (gl2, then
// gl1, then Canvas 2D) with the kit baked into sprites. It answers the same
// questions as `three3d/city.ts` (selection for the inspector, stats for the
// HUD, sleep/wake) so `CityView.svelte` can swap one for the other.
//
// Terrain goes into the renderer's baked chunks (one quad per chunk per
// frame); modules and people are sprites sorted by x + y + z. The view turns
// in quarter turns (Q/E, right click): a turn swaps every sprite for its
// baked rotation and re-bakes the chunks.

import type { Blueprint, Instance } from '$lib/city3d/types';
import { REGION_TILES } from '$lib/city3d/types';
import type { CitySelection, CityStats } from '../three3d/city';
import type { RegionMeshData } from '../three3d/region';
import { parseStatus, type CityInfo } from '../three3d/status';
import { sortByDepth, cullSprites } from '../batcher';
import { parseAtlas, type AtlasData } from '../atlas';
import { clampZoom, makeCamera, screenToWorld, visibleChunks, worldToScreen } from '../camera';
import { createRenderer, probeBackends } from '../index';
import { CANVAS_REPLACED_EVENT, type BackendName, type Camera, type ChunkTile, type FrameStats, type Renderer, type Tier } from '../types';
import {
  DETAIL_LAYERS,
  REGION_M,
  TILE_M,
  Z_PER_M,
  fromDisplay,
  instanceTile,
  lotAt,
  mergeAtlases,
  regionChunkTiles,
  regionSprites,
  restackJson,
  stackPlan,
  stylesOf,
  terrainTiles,
  toDisplay,
  walkableSlot,
  LitePalettes,
  type KitAtlasPayload,
  type LiteSprite,
  type MergedAtlas,
  type TerrainTiles,
} from './model';

/** Where the lite city's data comes from (the 3D source minus the kit). */
export interface LiteSource {
  info(): Promise<CityInfo | null>;
  mesh(region: [number, number]): Promise<RegionMeshData>;
  blueprint(region: [number, number]): Promise<Blueprint | null>;
  atlas(style: string): Promise<KitAtlasPayload>;
  onChanged(cb: (region: [number, number], revision: string) => void): Promise<() => void>;
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const core = await import('@tauri-apps/api/core');
  return (await core.invoke(cmd, args)) as T;
}

export function tauriLiteSource(): LiteSource {
  return {
    async info() {
      try {
        return parseStatus(await invoke('city3d_status'));
      } catch {
        return null;
      }
    },
    mesh: (region) => invoke<RegionMeshData>('city3d_region_mesh', { region }),
    async blueprint(region) {
      try {
        const r = await invoke<{ blueprint?: Blueprint | null }>('city3d_region', { region });
        return r?.blueprint ?? null;
      } catch {
        return null;
      }
    },
    atlas: (style) => invoke<KitAtlasPayload>('city3d_kit_atlas', { style }),
    async onChanged(cb) {
      try {
        const { listen } = await import('@tauri-apps/api/event');
        return await listen<{ region: [number, number]; revision: string }>('city3d:changed', (e) => {
          const r = e.payload?.region;
          if (Array.isArray(r)) cb([Number(r[0]), Number(r[1])], String(e.payload.revision ?? ''));
        });
      } catch {
        return () => {};
      }
    },
  };
}

export interface LiteCitySceneOpts {
  canvas: HTMLCanvasElement;
  source?: LiteSource;
  /** Forces a 2D backend; default gl2 → gl1 → canvas2d by what exists. */
  backend?: BackendName;
  /** People walking the streets. */
  people?: number;
  onSelect?: (sel: CitySelection | null) => void;
  onStats?: (s: CityStats) => void;
  onCanvasReplaced?: (c: HTMLCanvasElement) => void;
  onError?: (code: string) => void;
}

export interface LiteFrameReport extends FrameStats {
  /** Wall time of `frame()` plus a 1 px readback when GL (forces the GPU). */
  wallMs: number;
}

export interface LiteCityScene {
  readonly kind: 'lite';
  readonly info: CityInfo | null;
  readonly backend: BackendName;
  readonly ready: Promise<void>;
  readonly camera: Camera;
  readonly rotation: number;
  /** Lite mode has no rig: live poses are a 3D feature. */
  applyPose(pose: unknown): boolean;
  setCameraMode(m: unknown): void;
  setTier(t: Tier): void;
  lookAtTile(region: [number, number], tile: [number, number]): void;
  /** Camera in display tiles and zoom. */
  setView(x: number, y: number, zoom: number): void;
  turn(dir: 1 | -1): void;
  reloadRegion(region: [number, number]): Promise<void>;
  /** Picks at client coordinates, like a click (also emits `onSelect`). */
  pickAt(clientX: number, clientY: number): CitySelection | null;
  /** Draws one frame now (bench), synchronised with a 1 px readback. */
  drawNow(): LiteFrameReport;
  /** People move this much simulated time (bench). */
  step(dtMs: number): void;
  resize(w: number, h: number, dpr: number): void;
  sleep(): void;
  wake(): Promise<void>;
  destroy(): void;
}

interface LoadedRegion {
  region: [number, number];
  instances: Instance[];
  tiles: TerrainTiles;
  blueprint: Blueprint | null;
  sprites: LiteSprite[];
}

interface Walker {
  x: number;
  y: number;
  tx: number;
  ty: number;
  body: number;
  head: number;
}

const FPS = 30;
const DETAIL_ZOOM = 0.6;
const OVERLAY_ZOOM = 0.4;
const SKIN = [0xe8c4a0, 0xc99a74, 0x9c6b48, 0x6e4a32, 0xf1d3b8];
const CLOTH = [0x3c5da0, 0xb9654c, 0x436d59, 0xd9a441, 0x7a4f8c, 0x2f2f33, 0xe4d6b6];

function mulberry(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function seedOf(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}

async function pageBitmap(b64: string): Promise<ImageBitmap> {
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  const blob = new Blob([bytes], { type: 'image/png' });
  return await createImageBitmap(blob, { premultiplyAlpha: 'premultiply' });
}

/**
 * Stacks the style pages into as few textures as fit `STACK_SIDE`, rewriting
 * `merged.json` in place. Null when there is no canvas to draw with.
 */
async function stackBitmaps(merged: MergedAtlas, bitmaps: ImageBitmap[]): Promise<ImageBitmap[] | null> {
  if (typeof document === 'undefined') return null;
  const plan = stackPlan(bitmaps.map((b) => ({ w: b.width, h: b.height })), STACK_SIDE);
  if (plan.pages.length >= bitmaps.length) return null;
  const out: ImageBitmap[] = [];
  for (let p = 0; p < plan.pages.length; p++) {
    const c = document.createElement('canvas');
    c.width = plan.pages[p].w;
    c.height = plan.pages[p].h;
    const ctx = c.getContext('2d');
    if (!ctx) return null;
    bitmaps.forEach((b, i) => {
      if (plan.place[i].page === p) ctx.drawImage(b, 0, plan.place[i].y);
    });
    out.push(await createImageBitmap(c, { premultiplyAlpha: 'premultiply' }));
  }
  merged.json = restackJson(merged.json, plan);
  return out;
}

/** Tallest stacked page: 4096 is within every WebGL implementation we target. */
const STACK_SIDE = 4096;

const regionKey = (r: [number, number]) => `${r[0]},${r[1]}`;

export async function createLiteCityScene(opts: LiteCitySceneOpts): Promise<LiteCityScene> {
  const source = opts.source ?? tauriLiteSource();
  let canvas = opts.canvas;
  const renderer: Renderer = createRenderer();
  const has = probeBackends();
  const backend: BackendName = opts.backend ?? (has.gl2 ? 'gl2' : has.gl1 ? 'gl1' : 'canvas2d');
  // Tier 1 of the 2D table: 512² chunk textures (tier 0 would halve them and
  // blur the ground), DPR 1. The frame loop caps itself at FPS.
  const caps = await renderer.init(canvas, { tier: 1, backend, dpr: 1 });
  const onReplaced = (e: Event) => {
    const next = (e as CustomEvent<HTMLCanvasElement>).detail;
    if (!next) return;
    detachInput(canvas);
    canvas = next;
    attachInput(canvas);
    opts.onCanvasReplaced?.(canvas);
  };
  canvas.addEventListener(CANVAS_REPLACED_EVENT, onReplaced);

  const info = await source.info();
  const palettes = new LitePalettes(info?.palettes ?? [], info?.defaultPalette);
  const cam: Camera = makeCamera(canvas.width || 1, canvas.height || 1, 0, 0, 1);
  let rotation = 0;
  let datumM: number | null = null;
  let atlas: MergedAtlas | null = null;
  let atlasData: AtlasData | null = null;
  const atlasCache = new Map<string, Promise<KitAtlasPayload>>();
  let atlasStyles: string[] = [];
  let pageBitmaps: ImageBitmap[] = [];
  let alphaPages: Array<{ w: number; h: number; a: Uint8Array } | null> | null = null;
  const regions = new Map<string, LoadedRegion>();
  let staticSprites: LiteSprite[] = [];
  let drawn: LiteSprite[] = [];
  let spritesDirty = true;
  let chunkIds: string[] = [];
  let destroyed = false;
  let sleeping = false;
  let dirty = true;
  let missingKeys = 0;

  // ── atlas ──────────────────────────────────────────────────────────────
  async function ensureAtlas(styles: string[]): Promise<boolean> {
    const want = [...new Set([...atlasStyles, ...styles])].sort();
    if (atlas && want.length === atlasStyles.length) return false;
    const list: KitAtlasPayload[] = [];
    for (const s of want) {
      let p = atlasCache.get(s);
      if (!p) {
        p = source.atlas(s);
        atlasCache.set(s, p);
      }
      try {
        list.push(await p);
      } catch (e) {
        atlasCache.delete(s);
        opts.onError?.(e instanceof Error ? e.message : String(e));
      }
    }
    if (list.length === 0) throw new Error('ERR_CITY3D_ATLAS: no atlas');
    const merged = mergeAtlases(list);
    let bitmaps = await Promise.all(merged.pages.map(pageBitmap));
    if (bitmaps.length > 1) {
      const stacked = await stackBitmaps(merged, bitmaps);
      if (stacked) {
        for (const b of bitmaps) b.close?.();
        bitmaps = stacked;
      }
    }
    if (destroyed) return false;
    for (const b of pageBitmaps) b.close?.();
    pageBitmaps = bitmaps;
    alphaPages = null;
    renderer.loadAtlas(merged.json, bitmaps);
    atlas = merged;
    atlasData = parseAtlas(merged.json);
    atlasStyles = want;
    return true;
  }

  // ── regions ────────────────────────────────────────────────────────────
  function rebuildRegionSprites(r: LoadedRegion): void {
    if (!atlas || datumM === null) return;
    const out = regionSprites(r.region, r.instances, atlas, palettes, rotation, datumM);
    r.sprites = out.sprites;
    missingKeys = out.missing;
  }

  function rebuildAll(): void {
    staticSprites = [];
    for (const r of regions.values()) {
      rebuildRegionSprites(r);
      for (const s of r.sprites) staticSprites.push(s);
    }
    rebakeChunks();
    spritesDirty = true;
    dirty = true;
  }

  function rebakeChunks(): void {
    if (!atlas || datumM === null) return;
    const byChunk = new Map<string, ChunkTile[]>();
    for (const r of regions.values()) regionChunkTiles(r.region, r.tiles, palettes, atlas.tile, rotation, datumM, byChunk);
    // Chunks that no longer hold anything are baked empty (the store has no delete).
    for (const id of chunkIds) if (!byChunk.has(id)) renderer.bakeChunk(id, []);
    for (const [id, tiles] of byChunk) renderer.bakeChunk(id, tiles);
    chunkIds = [...byChunk.keys()];
  }

  async function loadRegion(r: [number, number]): Promise<void> {
    const [mesh, blueprint] = await Promise.all([source.mesh(r), source.blueprint(r)]);
    if (destroyed) return;
    const tiles = terrainTiles(mesh.terrain);
    // One datum for the whole view: the first region's lowest ground, so the
    // baked chunks (which cannot go below it) keep their full height range.
    if (datumM === null) datumM = tiles.minM;
    const fresh = await ensureAtlas(stylesOf(mesh.instances.instances, info?.style ?? 'vila-colonial'));
    regions.set(regionKey(r), { region: r, instances: mesh.instances.instances, tiles, blueprint, sprites: [] });
    if (fresh) rebuildAll();
    else {
      const loaded = regions.get(regionKey(r))!;
      rebuildRegionSprites(loaded);
      staticSprites = [];
      for (const x of regions.values()) for (const s of x.sprites) staticSprites.push(s);
      rebakeChunks();
      spritesDirty = true;
      dirty = true;
    }
    seedWalkers();
    kick();
  }

  // ── people ─────────────────────────────────────────────────────────────
  const walkers: Walker[] = [];
  const peopleWanted = opts.people ?? 12;
  const rand = mulberry(seedOf(info?.seed ?? 'city'));

  function walkable(wx: number, wz: number): boolean {
    const r: [number, number] = [Math.floor(wx / REGION_TILES), Math.floor(wz / REGION_TILES)];
    const reg = regions.get(regionKey(r));
    if (!reg) return false;
    const tx = wx - r[0] * REGION_TILES;
    const tz = wz - r[1] * REGION_TILES;
    return walkableSlot(reg.tiles.slot[tz * REGION_TILES + tx]);
  }

  function groundM(wx: number, wz: number): number {
    const tx = Math.floor(wx);
    const tz = Math.floor(wz);
    const r: [number, number] = [Math.floor(tx / REGION_TILES), Math.floor(tz / REGION_TILES)];
    const reg = regions.get(regionKey(r));
    if (!reg) return datumM ?? 0;
    return reg.tiles.height[(tz - r[1] * REGION_TILES) * REGION_TILES + (tx - r[0] * REGION_TILES)];
  }

  function seedWalkers(): void {
    if (walkers.length >= peopleWanted) return;
    const cells: Array<[number, number]> = [];
    for (const reg of regions.values()) {
      for (let i = 0; i < reg.tiles.slot.length; i++) {
        if (walkableSlot(reg.tiles.slot[i])) {
          cells.push([reg.region[0] * REGION_TILES + (i % REGION_TILES), reg.region[1] * REGION_TILES + Math.floor(i / REGION_TILES)]);
        }
      }
    }
    if (cells.length === 0) return;
    while (walkers.length < peopleWanted) {
      const [x, y] = cells[Math.floor(rand() * cells.length)];
      walkers.push({
        x: x + 0.5,
        y: y + 0.5,
        tx: x + 0.5,
        ty: y + 0.5,
        body: CLOTH[Math.floor(rand() * CLOTH.length)],
        head: SKIN[Math.floor(rand() * SKIN.length)],
      });
    }
  }

  /** World tiles per second of a walker (1.3 m/s). */
  const SPEED = 1.3 / TILE_M;
  function stepPeople(dt: number): boolean {
    let moved = false;
    for (const w of walkers) {
      const dx = w.tx - w.x;
      const dy = w.ty - w.y;
      const d = Math.hypot(dx, dy);
      if (d < 1e-3) {
        const dirs: Array<[number, number]> = [[1, 0], [-1, 0], [0, 1], [0, -1]];
        const opts2 = dirs.filter(([ax, ay]) => walkable(Math.floor(w.x) + ax, Math.floor(w.y) + ay));
        if (opts2.length === 0) continue;
        const [ax, ay] = opts2[Math.floor(rand() * opts2.length)];
        w.tx = Math.floor(w.x) + ax + 0.5;
        w.ty = Math.floor(w.y) + ay + 0.5;
        continue;
      }
      const s = Math.min(d, SPEED * dt);
      w.x += (dx / d) * s;
      w.y += (dy / d) * s;
      moved = true;
    }
    return moved;
  }

  function peopleSprites(): LiteSprite[] {
    if (!atlas || datumM === null || atlas.person.length === 0) return [];
    const out: LiteSprite[] = [];
    for (const w of walkers) {
      const p = toDisplay(w.x * TILE_M, w.y * TILE_M, rotation);
      const z = (groundM(w.x, w.y) - datumM) * Z_PER_M;
      for (const [frame, slot] of atlas.person) {
        out.push({ frame, x: p.x, y: p.y, z, tint: slot === 15 ? w.body : w.head, inst: -1, lod: 0 });
      }
    }
    return out;
  }

  // ── frame ──────────────────────────────────────────────────────────────
  let lastZoomBand = -1;
  function visibleStatic(): LiteSprite[] {
    const band = cam.zoom < OVERLAY_ZOOM ? 2 : cam.zoom < DETAIL_ZOOM ? 1 : 0;
    if (!spritesDirty && band === lastZoomBand) return drawn;
    lastZoomBand = band;
    spritesDirty = false;
    drawn = band === 0 ? staticSprites : staticSprites.filter((s) => s.lod === 0 || (band === 1 && s.lod === 2));
    return drawn;
  }

  let frameSprites: LiteSprite[] = [];
  let lastStats: FrameStats = { cpuMs: 0, drawCalls: 0, sprites: 0, chunksBaked: 0 };
  function render(): FrameStats {
    const base = visibleStatic();
    const people = peopleSprites();
    frameSprites = people.length ? base.concat(people) : base;
    lastStats = renderer.frame({ camera: cam, chunksVisible: visibleChunks(cam), sprites: frameSprites, texts: [] }, 1000 / FPS);
    // Chunks still dirty get baked two per frame: keep drawing until done.
    if (lastStats.chunksBaked > 0) dirty = true;
    return lastStats;
  }

  let raf = 0;
  let last = 0;
  let frames = 0;
  let fpsStart = 0;
  let fps = 0;
  let lastStatsAt = 0;
  function kick(): void {
    dirty = true;
    if (destroyed || sleeping || raf) return;
    raf = requestAnimationFrame(tick);
  }
  function tick(now: number): void {
    raf = 0;
    if (destroyed || sleeping) return;
    if (last !== 0 && now - last < 1000 / FPS - 1.5) {
      raf = requestAnimationFrame(tick);
      return;
    }
    const dt = last === 0 ? 1 / FPS : Math.min(0.25, (now - last) / 1000);
    last = now;
    if (stepPeople(dt)) dirty = true;
    if (keys.size) panByKeys(dt);
    if (dirty) {
      dirty = false;
      render();
      frames++;
    }
    if (fpsStart === 0) fpsStart = now;
    else if (now - fpsStart >= 500) {
      fps = (frames * 1000) / (now - fpsStart);
      frames = 0;
      fpsStart = now;
    }
    if (now - lastStatsAt > 500) {
      lastStatsAt = now;
      opts.onStats?.(snapshot());
    }
    raf = requestAnimationFrame(tick);
  }

  function snapshot(): CityStats {
    let instances = 0;
    for (const r of regions.values()) instances += r.instances.length;
    return {
      fps,
      tier: 0,
      cpuMs: lastStats.cpuMs,
      drawCalls: lastStats.drawCalls,
      triangles: lastStats.sprites * 2,
      instances: lastStats.sprites,
      instancesLoaded: instances,
      regions: regions.size,
      people: walkers.length,
      rigged: false,
      kitReal: missingKeys === 0,
      memoryBytes: renderer.memory().bytes,
      compileMs: 0,
      gpu: `lite ${caps.backend}`,
    };
  }

  // ── input ──────────────────────────────────────────────────────────────
  const dpr = () => (canvas.width || 1) / Math.max(1, canvas.getBoundingClientRect().width || canvas.width || 1);
  let drag: { x: number; y: number; cx: number; cy: number; moved: number; button: number } | null = null;
  const keys = new Set<string>();

  const onDown = (e: PointerEvent) => {
    drag = { x: e.clientX, y: e.clientY, cx: cam.x, cy: cam.y, moved: 0, button: e.button };
    try {
      canvas.setPointerCapture(e.pointerId);
    } catch {
      // Synthetic events have no capture; harmless.
    }
  };
  const onMove = (e: PointerEvent) => {
    if (!drag || drag.button !== 0) return;
    const k = dpr();
    const dx = (e.clientX - drag.x) * k;
    const dy = (e.clientY - drag.y) * k;
    drag.moved = Math.max(drag.moved, Math.hypot(dx, dy));
    // Screen delta → world delta on the ground plane.
    const a = dx / (32 * cam.zoom);
    const b = dy / (16 * cam.zoom);
    cam.x = drag.cx - (a + b) / 2;
    cam.y = drag.cy - (b - a) / 2;
    kick();
  };
  const onUp = (e: PointerEvent) => {
    const d = drag;
    drag = null;
    if (!d) return;
    const moved = Math.max(d.moved, Math.hypot(e.clientX - d.x, e.clientY - d.y));
    if (moved > 5) return;
    if (d.button === 2) {
      scene.turn(e.shiftKey ? -1 : 1);
      return;
    }
    if (d.button === 0) opts.onSelect?.(scene.pickAt(e.clientX, e.clientY));
  };
  const onWheel = (e: WheelEvent) => {
    e.preventDefault();
    const rect = canvas.getBoundingClientRect();
    const k = dpr();
    const sx = (e.clientX - rect.left) * k;
    const sy = (e.clientY - rect.top) * k;
    const before = screenToWorld(cam, sx, sy);
    cam.zoom = clampZoom(cam.zoom * Math.exp(-e.deltaY * 0.0015));
    const after = screenToWorld(cam, sx, sy);
    cam.x += before.x - after.x;
    cam.y += before.y - after.y;
    kick();
  };
  const onKeyDown = (e: KeyboardEvent) => {
    const k = e.key.toLowerCase();
    if (k === 'q') scene.turn(-1);
    else if (k === 'e') scene.turn(1);
    else if (['w', 'a', 's', 'd', 'arrowup', 'arrowdown', 'arrowleft', 'arrowright'].includes(k)) {
      keys.add(k);
      kick();
    }
  };
  const onKeyUp = (e: KeyboardEvent) => keys.delete(e.key.toLowerCase());
  const onContext = (e: Event) => e.preventDefault();
  function panByKeys(dt: number): void {
    const step = (12 / cam.zoom) * dt;
    let sx = 0;
    let sy = 0;
    if (keys.has('a') || keys.has('arrowleft')) sx -= 1;
    if (keys.has('d') || keys.has('arrowright')) sx += 1;
    if (keys.has('w') || keys.has('arrowup')) sy -= 1;
    if (keys.has('s') || keys.has('arrowdown')) sy += 1;
    // Screen right = (+x, −y), screen down = (+x, +y).
    cam.x += (sx + sy) * step * 0.7;
    cam.y += (sy - sx) * step * 0.7;
    dirty = true;
  }
  function attachInput(c: HTMLCanvasElement): void {
    c.addEventListener('pointerdown', onDown);
    c.addEventListener('pointermove', onMove);
    c.addEventListener('pointerup', onUp);
    c.addEventListener('wheel', onWheel, { passive: false });
    c.addEventListener('keydown', onKeyDown);
    c.addEventListener('keyup', onKeyUp);
    c.addEventListener('contextmenu', onContext);
  }
  function detachInput(c: HTMLCanvasElement): void {
    c.removeEventListener('pointerdown', onDown);
    c.removeEventListener('pointermove', onMove);
    c.removeEventListener('pointerup', onUp);
    c.removeEventListener('wheel', onWheel);
    c.removeEventListener('keydown', onKeyDown);
    c.removeEventListener('keyup', onKeyUp);
    c.removeEventListener('contextmenu', onContext);
  }
  attachInput(canvas);

  // ── picking ────────────────────────────────────────────────────────────
  function alphaMaps(): Array<{ w: number; h: number; a: Uint8Array } | null> {
    if (alphaPages) return alphaPages;
    alphaPages = pageBitmaps.map((bmp) => {
      try {
        const c = document.createElement('canvas');
        c.width = bmp.width;
        c.height = bmp.height;
        const ctx = c.getContext('2d', { willReadFrequently: true });
        if (!ctx) return null;
        ctx.drawImage(bmp, 0, 0);
        const px = ctx.getImageData(0, 0, bmp.width, bmp.height).data;
        const a = new Uint8Array(bmp.width * bmp.height);
        for (let i = 0; i < a.length; i++) a[i] = px[i * 4 + 3];
        return { w: bmp.width, h: bmp.height, a };
      } catch {
        return null;
      }
    });
    return alphaPages;
  }

  function spriteHit(s: LiteSprite, sx: number, sy: number): boolean {
    const f = atlasData?.frames.get(s.frame);
    if (!f) return false;
    const p = worldToScreen(cam, s.x, s.y, s.z);
    const u = (sx - (p.sx - f.pivotX * cam.zoom)) / cam.zoom;
    const v = (sy - (p.sy - f.pivotY * cam.zoom)) / cam.zoom;
    if (u < 0 || v < 0 || u >= f.w || v >= f.h) return false;
    const page = alphaMaps()[f.page];
    if (!page) return true; // no pixels to read: the rectangle decides
    const ax = f.x + Math.floor(u);
    const ay = f.y + Math.floor(v);
    return page.a[ay * page.w + ax] > 40;
  }

  function regionOfSprite(s: LiteSprite): LoadedRegion | null {
    for (const r of regions.values()) if (r.sprites.includes(s)) return r;
    return null;
  }

  function pick(clientX: number, clientY: number): CitySelection | null {
    if (!atlasData) return null;
    const rect = canvas.getBoundingClientRect();
    const k = dpr();
    const sx = (clientX - rect.left) * k;
    const sy = (clientY - rect.top) * k;
    const shown = cullSprites(visibleStatic(), atlasData, cam) as LiteSprite[];
    const order = sortByDepth(shown, atlasData);
    for (let i = order.length - 1; i >= 0; i--) {
      const s = shown[order[i]];
      if (s.inst < 0 || !spriteHit(s, sx, sy)) continue;
      const reg = regionOfSprite(s);
      if (!reg) continue;
      const inst = reg.instances[s.inst];
      const tile = instanceTile(inst);
      const bp = reg.blueprint;
      const layer = inst.key.split('/')[0];
      const building = inst.owner ? (bp?.buildings.find((b) => b.id === inst.owner) ?? null) : null;
      const lot = building ? (bp?.lots.find((l) => l.id === building.lot) ?? null) : bp ? lotAt(bp, tile) : null;
      const kind: CitySelection['kind'] = building
        ? 'building'
        : DETAIL_LAYERS.has(layer) || layer === 'arvore'
          ? 'prop'
          : inst.owner
            ? 'building'
            : 'module';
      return { kind, region: reg.region, id: inst.owner, owner: building?.owner_ref ?? lot?.owner_ref ?? null, key: inst.key, tile, building, lot };
    }
    // Ground: the plane at the tile's height, refined twice.
    let w = screenToWorld(cam, sx, sy);
    for (let it = 0; it < 2; it++) {
      const m = fromDisplay(w.x, w.y, rotation);
      const h = (groundM(m.xm / TILE_M, m.zm / TILE_M) - (datumM ?? 0)) * Z_PER_M;
      w = screenToWorld(cam, sx, sy + h * 32 * cam.zoom);
    }
    const m = fromDisplay(w.x, w.y, rotation);
    const region: [number, number] = [Math.floor(m.xm / REGION_M), Math.floor(m.zm / REGION_M)];
    const reg = regions.get(regionKey(region));
    if (!reg) return null;
    const tile: [number, number] = [
      Math.max(0, Math.min(REGION_TILES - 1, Math.floor((m.xm - region[0] * REGION_M) / TILE_M))),
      Math.max(0, Math.min(REGION_TILES - 1, Math.floor((m.zm - region[1] * REGION_M) / TILE_M))),
    ];
    const lot = reg.blueprint ? lotAt(reg.blueprint, tile) : null;
    const building = lot?.building ? (reg.blueprint?.buildings.find((b) => b.id === lot.building) ?? null) : null;
    return { kind: lot ? 'lot' : 'ground', region, id: lot?.id ?? null, owner: lot?.owner_ref ?? null, key: null, tile, building, lot };
  }

  // ── camera helpers ─────────────────────────────────────────────────────
  function centreOn(xm: number, zm: number): void {
    const p = toDisplay(xm, zm, rotation);
    const h = datumM === null ? 0 : (groundM(xm / TILE_M, zm / TILE_M) - datumM) * Z_PER_M;
    // Aim at the ground's height: shift along the screen's vertical.
    cam.x = p.x - h;
    cam.y = p.y - h;
    kick();
  }

  const offChanged = await source.onChanged((r) => void scene.reloadRegion(r));

  const scene: LiteCityScene = {
    kind: 'lite',
    info,
    backend: caps.backend,
    ready: Promise.resolve(),
    get camera() {
      return cam;
    },
    get rotation() {
      return rotation;
    },
    applyPose() {
      return false;
    },
    setCameraMode() {
      // One camera in lite mode: fixed tilt, quarter turns.
    },
    setTier() {
      // The lite mode is already the floor.
    },
    lookAtTile(region, tile) {
      centreOn(region[0] * REGION_M + (tile[0] + 0.5) * TILE_M, region[1] * REGION_M + (tile[1] + 0.5) * TILE_M);
    },
    setView(x, y, zoom) {
      cam.x = x;
      cam.y = y;
      cam.zoom = clampZoom(zoom);
      kick();
    },
    turn(dir) {
      // Keep the point under the centre of the screen where it is.
      const m = fromDisplay(cam.x, cam.y, rotation);
      rotation = (rotation + (dir > 0 ? 1 : 3)) & 3;
      const p = toDisplay(m.xm, m.zm, rotation);
      cam.x = p.x;
      cam.y = p.y;
      rebuildAll();
      kick();
    },
    async reloadRegion(r) {
      try {
        await loadRegion(r);
      } catch (e) {
        opts.onError?.(e instanceof Error ? e.message : String(e));
      }
    },
    pickAt(clientX, clientY) {
      return pick(clientX, clientY);
    },
    drawNow() {
      const t0 = performance.now();
      const s = render();
      const gl = backend === 'canvas2d' ? null : (canvas.getContext(backend === 'gl2' ? 'webgl2' : 'webgl') as WebGLRenderingContext | null);
      if (gl) {
        const px = new Uint8Array(4);
        gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
      }
      return { ...s, wallMs: performance.now() - t0 };
    },
    step(dtMs) {
      stepPeople(dtMs / 1000);
    },
    resize(w, h, ratio) {
      const r = Math.min(ratio, 1);
      renderer.resize(w, h, r);
      cam.width = Math.max(1, Math.round(w * r));
      cam.height = Math.max(1, Math.round(h * r));
      kick();
    },
    sleep() {
      if (sleeping) return;
      sleeping = true;
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
      renderer.sleep();
    },
    async wake() {
      if (!sleeping) return;
      await renderer.wake();
      sleeping = false;
      rebakeChunks();
      kick();
    },
    destroy() {
      destroyed = true;
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
      offChanged();
      detachInput(canvas);
      canvas.removeEventListener(CANVAS_REPLACED_EVENT, onReplaced);
      renderer.destroy();
      for (const b of pageBitmaps) b.close?.();
      pageBitmaps = [];
      regions.clear();
    },
  };

  // ── first view ─────────────────────────────────────────────────────────
  const list: [number, number][] = info?.regions.length ? info.regions.slice(0, 12) : [[0, 0]];
  const first = list[0];
  (scene as { ready: Promise<void> }).ready = (async () => {
    await loadRegion(first);
    centreOn(first[0] * REGION_M + REGION_M / 2, first[1] * REGION_M + REGION_M / 2);
    // The rest in the background, nearest first.
    for (const r of list.slice(1)) void scene.reloadRegion(r);
    kick();
  })();

  return scene;
}
