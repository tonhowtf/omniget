import type { CanvasEngine } from '../engine.svelte';
import { isTerminalKind } from '../model/types';
import { parseColor, withAlpha, type Rgba } from './color';
import type { FrameInfo, GlPass } from './gl-layer';
import { LineBatch } from './lines';
import { SdfBatch } from './sdf-shapes';

const KIND_TINT: Record<string, string> = {
  note: '#e9c46a',
  text: '#9aa5b1',
  file: '#8ab4f8',
  filetree: '#81c995',
  link: '#78c2d6',
  portal: '#7cc4b8',
  image: '#c58af9',
  stack: '#e9c46a',
};

export function createPlatesPass(engine: CanvasEngine, colors: () => { surface: Rgba; ink: Rgba; accent: Rgba }): GlPass {
  const sdf = new SdfBatch();
  return {
    id: 'canvas.plates',
    order: 10,
    draw(gl: WebGL2RenderingContext, frame: FrameInfo) {
      const cam = frame.camera;
      const z = cam.zoom;
      const { surface, ink } = colors();
      sdf.begin();
      for (const g of engine.store.groups) {
        const b = engine.groupBounds(g.id);
        if (!b) continue;
        const col = parseColor(g.color ?? '#7c8cff');
        sdf.rect((b.x - cam.x) * z, (b.y - cam.y) * z, b.w * z, b.h * z, {
          radius: 14 * Math.min(1, z * 1.5),
          fill: withAlpha(col, 0.07),
          strokeWidth: 1.5,
          stroke: withAlpha(col, 0.55),
          dash: 6,
        });
      }
      for (const v of frame.visibleNodes) {
        if (v.lod !== 'far' || isTerminalKind(v.kind)) continue;
        const tint = parseColor(KIND_TINT[v.kind] ?? '#9aa5b1');
        sdf.rect((v.x - cam.x) * z, (v.y - cam.y) * z, v.w * z, v.h * z, {
          radius: 4,
          fill: [
            surface[0] * 0.6 + tint[0] * 0.4,
            surface[1] * 0.6 + tint[1] * 0.4,
            surface[2] * 0.6 + tint[2] * 0.4,
            0.95,
          ],
          strokeWidth: 1,
          stroke: withAlpha(ink, 0.25),
        });
      }
      for (const v of frame.visibleNodes) {
        if (v.lod !== 'mid' || isTerminalKind(v.kind)) continue;
        sdf.rect((v.x - cam.x) * z + 2, (v.y - cam.y) * z + 4, v.w * z, v.h * z, {
          radius: 10,
          fill: [0, 0, 0, 0.18],
        });
      }
      sdf.flush(gl, frame.viewport.w, frame.viewport.h);
    },
    dispose() {
      sdf.dispose();
    },
  };
}

export function createOverlayPass(engine: CanvasEngine, colors: () => { accent: Rgba; ink: Rgba }): GlPass {
  const sdf = new SdfBatch();
  const lines = new LineBatch();
  return {
    id: 'canvas.overlay',
    order: 70,
    draw(gl: WebGL2RenderingContext, frame: FrameInfo) {
      const cam = frame.camera;
      const z = cam.zoom;
      const { accent, ink } = colors();
      const d = engine.draft;
      sdf.begin();
      lines.begin();
      if (d.marquee) {
        const m = d.marquee;
        sdf.rect((m.x - cam.x) * z, (m.y - cam.y) * z, m.w * z, m.h * z, {
          fill: withAlpha(accent, 0.08),
          strokeWidth: 1,
          stroke: withAlpha(accent, 0.9),
        });
      }
      if (d.create) {
        const m = d.create;
        sdf.rect((m.x - cam.x) * z, (m.y - cam.y) * z, m.w * z, m.h * z, {
          radius: 10,
          fill: withAlpha(accent, 0.06),
          strokeWidth: 1.5,
          stroke: accent,
          dash: 5,
        });
      }
      if (d.eraser) {
        sdf.circle((d.eraser.x - cam.x) * z, (d.eraser.y - cam.y) * z, d.eraser.r * z, {
          strokeWidth: 1.5,
          stroke: withAlpha(ink, 0.6),
        });
      }
      if (d.guides) {
        const guideColor: Rgba = [0.95, 0.35, 0.55, 0.9];
        for (const gd of d.guides) {
          if (gd.axis === 'x') {
            const x = (gd.at - cam.x) * z;
            lines.segment(x, (gd.from - cam.y) * z, x, (gd.to - cam.y) * z, 1, guideColor);
          } else {
            const y = (gd.at - cam.y) * z;
            lines.segment((gd.from - cam.x) * z, y, (gd.to - cam.x) * z, y, 1, guideColor);
          }
        }
      }
      sdf.flush(gl, frame.viewport.w, frame.viewport.h);
      lines.flush(gl, frame.viewport.w, frame.viewport.h);
    },
    dispose() {
      sdf.dispose();
      lines.dispose();
    },
  };
}
