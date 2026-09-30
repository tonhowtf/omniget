// Atlas parsing and lookup. Pure: no GL, no image decoding.
// Format: static/world/atlas.schema.json (owned by f6-atlas-tools).

import { ERR, RenderError } from './types';

export interface AtlasFrame {
  page: number;
  x: number;
  y: number;
  w: number;
  h: number;
  /** Ground contact point, pixels from the frame's top-left. */
  pivotX: number;
  pivotY: number;
  /** Added to the world z when sorting. */
  zBase: number;
}

export type Dir8 = 'S' | 'SW' | 'W' | 'NW' | 'N' | 'SE' | 'E' | 'NE';

export const DIRS: Dir8[] = ['S', 'SW', 'W', 'NW', 'N', 'NE', 'E', 'SE'];

/** The three directions that exist only as a horizontal mirror of another. */
export const MIRRORED: Record<string, 'SW' | 'W' | 'NW'> = { SE: 'SW', E: 'W', NE: 'NW' };

export interface AtlasClip {
  fps: number;
  loop: boolean;
  /** Unique directions only; the mirrored ones resolve through `mirrorOf`. */
  dirs: Record<string, string[]>;
  mirrorOf: Record<string, string>;
}

export interface AtlasTile {
  frame: string;
  height: number;
  occludes: boolean;
  footprint: [number, number];
  walkable: boolean;
}

export interface AtlasData {
  version: number;
  pages: string[];
  frames: Map<string, AtlasFrame>;
  anims: Map<string, AtlasClip>;
  tiles: Map<string, AtlasTile>;
}

function bad(msg: string): never {
  throw new RenderError(ERR.ATLAS_INVALID, msg);
}

function int(v: unknown, what: string): number {
  if (typeof v !== 'number' || !Number.isFinite(v)) bad(`${what} is not a number`);
  return v as number;
}

export function parseAtlas(json: unknown): AtlasData {
  if (!json || typeof json !== 'object') bad('atlas is not an object');
  const raw = json as Record<string, unknown>;
  if (raw.version !== 1) bad(`unsupported atlas version ${String(raw.version)}`);
  const pages = raw.pages;
  if (!Array.isArray(pages) || pages.length === 0) bad('atlas has no pages');

  const frames = new Map<string, AtlasFrame>();
  const rawFrames = raw.frames;
  if (!rawFrames || typeof rawFrames !== 'object') bad('atlas has no frames');
  for (const [id, v] of Object.entries(rawFrames as Record<string, unknown>)) {
    const f = v as Record<string, unknown>;
    const pivot = f.pivot;
    if (!Array.isArray(pivot) || pivot.length !== 2) bad(`frame ${id} has no pivot`);
    const page = int(f.page, `frame ${id} page`);
    if (page < 0 || page >= pages.length) bad(`frame ${id} points at page ${page}`);
    const w = int(f.w, `frame ${id} w`);
    const h = int(f.h, `frame ${id} h`);
    if (w <= 0 || h <= 0) bad(`frame ${id} has empty rect`);
    frames.set(id, {
      page,
      x: int(f.x, `frame ${id} x`),
      y: int(f.y, `frame ${id} y`),
      w,
      h,
      pivotX: int(pivot[0], `frame ${id} pivot x`),
      pivotY: int(pivot[1], `frame ${id} pivot y`),
      zBase: typeof f.z_base === 'number' ? (f.z_base as number) : 0,
    });
  }

  const anims = new Map<string, AtlasClip>();
  const rawAnims = raw.anims;
  if (rawAnims && typeof rawAnims === 'object') {
    for (const [id, v] of Object.entries(rawAnims as Record<string, unknown>)) {
      const a = v as Record<string, unknown>;
      const fps = int(a.fps, `anim ${id} fps`);
      if (fps < 1 || fps > 30) bad(`anim ${id} fps ${fps} out of range`);
      const dirsRaw = a.dirs;
      if (!dirsRaw || typeof dirsRaw !== 'object') bad(`anim ${id} has no dirs`);
      const dirs: Record<string, string[]> = {};
      const mirrorOf: Record<string, string> = {};
      for (const [dir, dv] of Object.entries(dirsRaw as Record<string, unknown>)) {
        if (Array.isArray(dv)) {
          if (dv.length === 0) bad(`anim ${id}/${dir} is empty`);
          for (const fid of dv) {
            if (typeof fid !== 'string' || !frames.has(fid)) {
              bad(`anim ${id}/${dir} references unknown frame ${String(fid)}`);
            }
          }
          dirs[dir] = dv as string[];
        } else if (dv && typeof dv === 'object' && 'mirror_of' in (dv as object)) {
          const src = (dv as Record<string, unknown>).mirror_of;
          if (typeof src !== 'string') bad(`anim ${id}/${dir} mirror_of is not a string`);
          mirrorOf[dir] = src;
        } else {
          bad(`anim ${id}/${dir} is neither a frame list nor a mirror`);
        }
      }
      for (const [dir, src] of Object.entries(mirrorOf)) {
        if (!dirs[src]) bad(`anim ${id}/${dir} mirrors ${src}, which has no frames`);
        if (MIRRORED[dir] !== src) bad(`anim ${id}/${dir} must mirror ${MIRRORED[dir] ?? '?'}`);
      }
      anims.set(id, {
        fps,
        loop: a.loop === undefined ? true : a.loop === true,
        dirs,
        mirrorOf,
      });
    }
  }

  const tiles = new Map<string, AtlasTile>();
  const rawTiles = raw.tiles;
  if (rawTiles && typeof rawTiles === 'object') {
    for (const [id, v] of Object.entries(rawTiles as Record<string, unknown>)) {
      const t = v as Record<string, unknown>;
      if (typeof t.frame !== 'string' || !frames.has(t.frame)) {
        bad(`tile ${id} references unknown frame ${String(t.frame)}`);
      }
      const fp = Array.isArray(t.footprint) ? (t.footprint as number[]) : [1, 1];
      tiles.set(id, {
        frame: t.frame as string,
        height: int(t.height, `tile ${id} height`),
        occludes: t.occludes === true,
        footprint: [fp[0] ?? 1, fp[1] ?? 1],
        walkable: t.walkable === undefined ? true : t.walkable === true,
      });
    }
  }

  return { version: 1, pages: pages as string[], frames, anims, tiles };
}

export function getFrame(atlas: AtlasData, id: string): AtlasFrame {
  const f = atlas.frames.get(id);
  if (!f) throw new RenderError(ERR.FRAME_UNKNOWN, `unknown frame ${id}`);
  return f;
}

/** Texture coordinates of a frame inside its page. */
export function frameUV(
  f: AtlasFrame,
  pageW: number,
  pageH: number,
): { u0: number; v0: number; u1: number; v1: number } {
  return { u0: f.x / pageW, v0: f.y / pageH, u1: (f.x + f.w) / pageW, v1: (f.y + f.h) / pageH };
}

/**
 * Frame list for a direction. The three mirrored directions come back with
 * `flip: true` over the source direction — mirroring lives in the batcher.
 */
export function resolveDir(
  atlas: AtlasData,
  animId: string,
  dir: Dir8,
): { frames: string[]; flip: boolean; fps: number; loop: boolean } {
  const clip = atlas.anims.get(animId);
  if (!clip) throw new RenderError(ERR.FRAME_UNKNOWN, `unknown anim ${animId}`);
  const direct = clip.dirs[dir];
  if (direct) return { frames: direct, flip: false, fps: clip.fps, loop: clip.loop };
  const src = clip.mirrorOf[dir];
  if (!src || !clip.dirs[src]) {
    throw new RenderError(ERR.FRAME_UNKNOWN, `anim ${animId} has no direction ${dir}`);
  }
  return { frames: clip.dirs[src], flip: true, fps: clip.fps, loop: clip.loop };
}

/** Bytes of GPU memory the atlas pages take as RGBA8 (no mipmaps). */
export function atlasTextureBytes(pageSizes: Array<{ w: number; h: number }>): number {
  let total = 0;
  for (const p of pageSizes) total += p.w * p.h * 4;
  return total;
}
