// Canvas 2D backend = tier 0. drawImage per sprite, chunk baked into an
// OffscreenCanvas and blitted once. Sprites carry alpha only; a baked tile's
// tint is applied through a scratch canvas (multiply, then keep the sprite's alpha).

import { parseAtlas, type AtlasData } from './atlas';
import { sortByDepth } from './batcher';
import { chunkLayout, chunkTilePx, sortTiles, ChunkStore } from './chunk';
import { parseChunkId, worldToScreen } from './camera';
import { TextAtlas } from './text';
import { budgetForTier } from './tier';
import {
  CHUNK_TILES,
  ERR,
  RenderError,
  type AtlasHandle,
  type Caps,
  type ChunkId,
  type ChunkTile,
  type FrameStats,
  type InitOpts,
  type Renderer,
  type Scene,
  type TextHandle,
  type TextStyle,
  type Tier,
} from './types';

type AnyCanvas = HTMLCanvasElement | OffscreenCanvas;
type Ctx2D = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;

/** One reusable scratch surface for tinting a tile while baking. */
let tintScratch: AnyCanvas | null = null;
function tinted(bmp: ImageBitmap, f: { x: number; y: number; w: number; h: number }, dw: number, dh: number, tint: number): AnyCanvas | null {
  const w = Math.max(1, Math.ceil(dw));
  const h = Math.max(1, Math.ceil(dh));
  if (!tintScratch) tintScratch = offscreen(Math.max(64, w, h));
  if (tintScratch.width < w || tintScratch.height < h) {
    tintScratch.width = Math.max(tintScratch.width, w);
    tintScratch.height = Math.max(tintScratch.height, h);
  }
  const c = tintScratch.getContext('2d') as Ctx2D | null;
  if (!c) return null;
  c.globalCompositeOperation = 'source-over';
  c.clearRect(0, 0, tintScratch.width, tintScratch.height);
  c.drawImage(bmp, f.x, f.y, f.w, f.h, 0, 0, w, h);
  c.globalCompositeOperation = 'multiply';
  c.fillStyle = `#${tint.toString(16).padStart(6, '0')}`;
  c.fillRect(0, 0, w, h);
  c.globalCompositeOperation = 'destination-in';
  c.drawImage(bmp, f.x, f.y, f.w, f.h, 0, 0, w, h);
  c.globalCompositeOperation = 'source-over';
  return tintScratch;
}

function offscreen(size: number): AnyCanvas {
  if (typeof OffscreenCanvas !== 'undefined') return new OffscreenCanvas(size, size);
  if (typeof document === 'undefined') throw new RenderError(ERR.NO_CANVAS2D, 'no canvas');
  const c = document.createElement('canvas');
  c.width = size;
  c.height = size;
  return c;
}

export function createCanvas2dRenderer(): Renderer {
  let canvas: HTMLCanvasElement | null = null;
  let ctx: CanvasRenderingContext2D | null = null;
  let atlas: AtlasData | null = null;
  let bitmaps: ImageBitmap[] = [];
  let tier: Tier = 0;
  let width = 1;
  let height = 1;
  let dpr = 1;
  let caps: Caps | null = null;
  let sleeping = false;
  const chunks = new ChunkStore();
  const chunkCanvases = new Map<ChunkId, AnyCanvas>();
  const textAtlas = new TextAtlas();

  function bake(id: ChunkId): boolean {
    const entry = chunks.get(id);
    if (!entry || !atlas) return false;
    const size = budgetForTier(tier).chunkTexture;
    let target = chunkCanvases.get(id);
    if (!target) {
      target = offscreen(size);
      chunkCanvases.set(id, target);
    }
    const c2 = target.getContext('2d') as Ctx2D | null;
    if (!c2) return false;
    c2.clearRect(0, 0, size, size);
    const layout = chunkLayout(size);
    for (const t of sortTiles(entry.tiles)) {
      const f = atlas.frames.get(t.frame);
      if (!f) continue;
      const bmp = bitmaps[f.page];
      if (!bmp) continue;
      const p = chunkTilePx(layout, t.lx, t.ly, t.z ?? 0);
      const px = t.flip ? f.w - f.pivotX : f.pivotX;
      const dx = p.px - px * layout.scale;
      const dy = p.py - f.pivotY * layout.scale;
      const dw = f.w * layout.scale;
      const dh = f.h * layout.scale;
      const scratch = t.tint !== undefined && t.tint !== 0xffffff ? tinted(bmp, f, dw, dh, t.tint) : null;
      if (t.flip) {
        c2.save();
        c2.translate(dx + dw, dy);
        c2.scale(-1, 1);
        if (scratch) c2.drawImage(scratch as CanvasImageSource, 0, 0, Math.ceil(dw), Math.ceil(dh), 0, 0, dw, dh);
        else c2.drawImage(bmp, f.x, f.y, f.w, f.h, 0, 0, dw, dh);
        c2.restore();
      } else if (scratch) {
        c2.drawImage(scratch as CanvasImageSource, 0, 0, Math.ceil(dw), Math.ceil(dh), dx, dy, dw, dh);
      } else {
        c2.drawImage(bmp, f.x, f.y, f.w, f.h, dx, dy, dw, dh);
      }
    }
    chunks.markBaked(id);
    return true;
  }

  const renderer: Renderer = {
    get tier() {
      return tier;
    },
    setTier(next: Tier) {
      if (next === tier) return;
      tier = next;
      chunkCanvases.clear();
      chunks.invalidateAll();
      if (caps) caps.tier = next;
    },

    async init(target: HTMLCanvasElement, opts: InitOpts): Promise<Caps> {
      canvas = target;
      tier = opts.tier;
      dpr = Math.min(opts.dpr ?? (globalThis.devicePixelRatio || 1), budgetForTier(tier).maxDpr);
      const c = target.getContext('2d', { alpha: true });
      if (!c) throw new RenderError(ERR.NO_CANVAS2D, 'canvas 2d unavailable');
      ctx = c;
      ctx.imageSmoothingEnabled = false;
      width = target.width || 1;
      height = target.height || 1;
      sleeping = false;
      caps = {
        backend: 'canvas2d',
        tier,
        maxTextureSize: 4096,
        renderTexture: true,
        timerQuery: false,
        dpr,
      };
      return caps;
    },

    loadAtlas(json: unknown, pages: ImageBitmap[]): AtlasHandle {
      const data = parseAtlas(json);
      if (pages.length < data.pages.length) {
        throw new RenderError(ERR.ATLAS_INVALID, 'fewer bitmaps than atlas pages');
      }
      atlas = data;
      bitmaps = pages;
      chunks.invalidateAll();
      chunkCanvases.clear();
      return { id: 1, frames: new Set(data.frames.keys()) };
    },

    frame(scene: Scene, _dt: number): FrameStats {
      const t0 = performance.now();
      const stats: FrameStats = { cpuMs: 0, drawCalls: 0, sprites: 0, chunksBaked: 0 };
      if (!ctx || !atlas || sleeping) {
        stats.cpuMs = performance.now() - t0;
        return stats;
      }
      for (const id of chunks.dirtyAmong(scene.chunksVisible, 1)) {
        if (bake(id)) stats.chunksBaked++;
      }
      chunks.touch(scene.chunksVisible);
      ctx.clearRect(0, 0, width, height);

      const size = budgetForTier(tier).chunkTexture;
      const layout = chunkLayout(size);
      const side = (size / layout.scale) * scene.camera.zoom;
      const shift = (layout.originX / layout.scale) * scene.camera.zoom;
      const shiftY = (layout.originY / layout.scale) * scene.camera.zoom;
      for (const id of scene.chunksVisible) {
        const baked = chunkCanvases.get(id);
        if (!baked) continue;
        const { cx, cy } = parseChunkId(id);
        const o = worldToScreen(scene.camera, cx * CHUNK_TILES, cy * CHUNK_TILES);
        ctx.drawImage(baked as CanvasImageSource, o.sx - shift, o.sy - shiftY, side, side);
        stats.drawCalls++;
      }

      for (const i of sortByDepth(scene.sprites, atlas)) {
        const s = scene.sprites[i];
        const f = atlas.frames.get(s.frame);
        if (!f) continue;
        const bmp = bitmaps[f.page];
        if (!bmp) continue;
        const p = worldToScreen(scene.camera, s.x, s.y, s.z);
        const px = s.flip ? f.w - f.pivotX : f.pivotX;
        const dx = p.sx - px * scene.camera.zoom;
        const dy = p.sy - f.pivotY * scene.camera.zoom;
        const dw = f.w * scene.camera.zoom;
        const dh = f.h * scene.camera.zoom;
        if (dx + dw < 0 || dy + dh < 0 || dx > width || dy > height) continue;
        const alpha = s.alpha ?? 1;
        if (alpha !== 1) ctx.globalAlpha = alpha;
        if (s.flip) {
          ctx.save();
          ctx.translate(dx + dw, dy);
          ctx.scale(-1, 1);
          ctx.drawImage(bmp, f.x, f.y, f.w, f.h, 0, 0, dw, dh);
          ctx.restore();
        } else {
          ctx.drawImage(bmp, f.x, f.y, f.w, f.h, dx, dy, dw, dh);
        }
        if (alpha !== 1) ctx.globalAlpha = 1;
        stats.sprites++;
        stats.drawCalls++;
      }

      const page = textAtlas.page;
      if (page) {
        for (const ti of scene.texts) {
          const rect = textAtlas.rect(ti.handle.id);
          if (!rect) continue;
          const p = worldToScreen(scene.camera, ti.x, ti.y, ti.z);
          const alpha = ti.alpha ?? 1;
          if (alpha !== 1) ctx.globalAlpha = alpha;
          ctx.drawImage(
            page as CanvasImageSource,
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            p.sx - rect.w / 2,
            p.sy - rect.h,
            rect.w,
            rect.h,
          );
          if (alpha !== 1) ctx.globalAlpha = 1;
          stats.drawCalls++;
        }
      }

      stats.cpuMs = performance.now() - t0;
      return stats;
    },

    bakeChunk(id: ChunkId, tiles: ChunkTile[]): void {
      chunks.set(id, tiles);
    },

    invalidateChunk(id: ChunkId): void {
      chunks.invalidate(id);
    },

    text(str: string, style?: TextStyle): TextHandle {
      return textAtlas.get(str, style);
    },

    resize(w: number, h: number, nextDpr: number): void {
      dpr = Math.min(nextDpr, budgetForTier(tier).maxDpr);
      width = Math.max(1, Math.round(w * dpr));
      height = Math.max(1, Math.round(h * dpr));
      if (canvas) {
        canvas.width = width;
        canvas.height = height;
        const c = canvas.getContext('2d');
        if (c) {
          ctx = c;
          ctx.imageSmoothingEnabled = false;
        }
      }
      if (caps) caps.dpr = dpr;
    },

    sleep(): void {
      sleeping = true;
      chunkCanvases.clear();
      chunks.invalidateAll();
      textAtlas.release();
    },

    async wake(): Promise<void> {
      sleeping = false;
      chunks.invalidateAll();
    },

    destroy(): void {
      chunkCanvases.clear();
      chunks.clear();
      textAtlas.release();
      atlas = null;
      bitmaps = [];
      ctx = null;
      canvas = null;
    },
  };

  return renderer;
}

export default createCanvas2dRenderer;
