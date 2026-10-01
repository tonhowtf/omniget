import type { CanvasEngine } from '../engine.svelte';
import type { FrameInfo, GlPass } from './gl-layer';
import { QuadBatch, uploadTexture } from './quads';

type Source = ImageBitmap | HTMLCanvasElement | OffscreenCanvas | HTMLImageElement;

export interface Snapshot {
  source: Source;
  version: number;
  key: string;
  w: number;
  h: number;
}

interface Tex {
  tex: WebGLTexture;
  version: number;
  bytes: number;
  used: number;
}

const snapshots = new Map<string, Snapshot>();
const listeners = new Set<() => void>();

function notify(): void {
  for (const l of listeners) l();
}

function onSnapshotsChange(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

function sourceSize(s: Source): { w: number; h: number } {
  if (typeof HTMLImageElement !== 'undefined' && s instanceof HTMLImageElement) return { w: s.naturalWidth, h: s.naturalHeight };
  return { w: s.width, h: s.height };
}

export function setSnapshot(id: string, source: Source, version: number, opts: { key?: string; w?: number; h?: number } = {}): void {
  const size = sourceSize(source);
  snapshots.set(id, { source, version, key: opts.key ?? id, w: opts.w || size.w, h: opts.h || size.h });
  notify();
}

export function clearSnapshot(id: string): void {
  if (snapshots.delete(id)) notify();
}

export function getSnapshot(id: string): Snapshot | undefined {
  return snapshots.get(id);
}

export function hasSnapshot(id: string): boolean {
  return snapshots.has(id);
}

export function containRect(x: number, y: number, w: number, h: number, sw: number, sh: number): { x: number; y: number; w: number; h: number } {
  if (!(sw > 0 && sh > 0 && w > 0 && h > 0)) return { x, y, w, h };
  const k = Math.min(w / sw, h / sh);
  const cw = sw * k;
  const ch = sh * k;
  return { x: x + (w - cw) / 2, y: y + (h - ch) / 2, w: cw, h: ch };
}

export function createSnapshotPass(engine: CanvasEngine, budgetBytes: () => number): GlPass {
  const batch = new QuadBatch();
  const textures = new Map<string, Tex>();
  let gl: WebGL2RenderingContext | null = null;
  const off = onSnapshotsChange(() => engine.invalidate());

  const drop = (g: WebGL2RenderingContext, key: string) => {
    const t = textures.get(key);
    if (t && !g.isContextLost()) g.deleteTexture(t.tex);
    textures.delete(key);
  };

  return {
    id: 'canvas.snapshots',
    order: 40,
    draw(g: WebGL2RenderingContext, frame: FrameInfo) {
      if (gl !== g) {
        gl = g;
        textures.clear();
      }
      if (!snapshots.size && !textures.size) return;
      const cam = frame.camera;
      const z = cam.zoom;
      const now = frame.time;
      const inUse = new Set<string>();
      for (const v of frame.visibleNodes) {
        const snap = snapshots.get(v.id);
        if (!snap) continue;
        inUse.add(snap.key);
        let t = textures.get(snap.key);
        if (!t || t.version !== snap.version || !g.isTexture(t.tex)) {
          const tex = uploadTexture(g, snap.source, t && g.isTexture(t.tex) ? t.tex : null, false, true);
          if (!tex) continue;
          const { w, h } = sourceSize(snap.source);
          t = { tex, version: snap.version, bytes: Math.round(w * h * 4 * (4 / 3)), used: now };
          textures.set(snap.key, t);
        }
        t.used = now;
        if (v.lod === 'near') continue;
        const r = containRect((v.x - cam.x) * z, (v.y - cam.y) * z, v.w * z, v.h * z, snap.w, snap.h);
        batch.begin();
        batch.quad({ page: 0, sx: r.x, sy: r.y, w: r.w, h: r.h, u0: 0, v0: 0, u1: 1, v1: 1 });
        batch.flush(g, t.tex, frame.viewport.w, frame.viewport.h);
      }
      const live = new Set<string>();
      for (const s of snapshots.values()) live.add(s.key);
      for (const key of Array.from(textures.keys())) if (!live.has(key)) drop(g, key);
      let total = 0;
      for (const t of textures.values()) total += t.bytes;
      const budget = budgetBytes();
      if (total > budget) {
        const old = Array.from(textures.entries())
          .filter(([k]) => !inUse.has(k))
          .sort((a, b) => a[1].used - b[1].used);
        for (const [k, t] of old) {
          if (total <= budget) break;
          total -= t.bytes;
          drop(g, k);
        }
      }
    },
    dispose() {
      off();
      batch.dispose();
      if (gl && !gl.isContextLost()) for (const t of textures.values()) gl.deleteTexture(t.tex);
      textures.clear();
      gl = null;
    },
  };
}
