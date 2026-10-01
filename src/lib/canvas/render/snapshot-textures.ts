import type { CanvasEngine } from '../engine.svelte';
import type { FrameInfo, GlPass } from './gl-layer';
import { QuadBatch, uploadTexture } from './quads';

type Source = ImageBitmap | HTMLCanvasElement | OffscreenCanvas | HTMLImageElement;

interface Entry {
  source: Source;
  version: number;
}

const snapshots = new Map<string, Entry>();
const listeners = new Set<() => void>();

function onSnapshotsChange(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

export function createSnapshotPass(engine: CanvasEngine): GlPass {
  const batch = new QuadBatch();
  const textures = new Map<string, { tex: WebGLTexture; version: number }>();
  let gl: WebGL2RenderingContext | null = null;
  const off = onSnapshotsChange(() => engine.invalidate());

  return {
    id: 'canvas.snapshots',
    order: 40,
    draw(g: WebGL2RenderingContext, frame: FrameInfo) {
      if (gl !== g) {
        gl = g;
        textures.clear();
      }
      if (!snapshots.size) return;
      const cam = frame.camera;
      const z = cam.zoom;
      for (const v of frame.visibleNodes) {
        if (v.lod === 'near' || v.lod === 'far') continue;
        const snap = snapshots.get(v.id);
        if (!snap) continue;
        let t = textures.get(v.id);
        if (!t || t.version !== snap.version) {
          const tex = uploadTexture(g, snap.source, t?.tex ?? null);
          if (!tex) continue;
          t = { tex, version: snap.version };
          textures.set(v.id, t);
        }
        batch.begin();
        batch.quad({ page: 0, sx: (v.x - cam.x) * z, sy: (v.y - cam.y) * z, w: v.w * z, h: v.h * z, u0: 0, v0: 0, u1: 1, v1: 1 });
        batch.flush(g, t.tex, frame.viewport.w, frame.viewport.h);
      }
      for (const id of Array.from(textures.keys())) {
        if (!snapshots.has(id)) {
          if (!g.isContextLost()) g.deleteTexture(textures.get(id)!.tex);
          textures.delete(id);
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
