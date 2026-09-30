// Tier-0 fallback (no WebGL2, or the context would not come back): the same
// picture through Canvas 2D, drawn from the same frame info. Passes of other
// territories are GL-only and are skipped here.

import type { CanvasEngine } from '../engine.svelte';
import { buildDrawing } from '../draw/build';
import { cordPath } from '../cords/route';
import { isTerminalKind, type Drawing } from '../model/types';
import { agentStates, stateCss } from '../state/agents.svelte';
import { gridLevel } from './grid';
import { rgbaCss, type Rgba } from './color';
import type { FrameInfo } from './gl-layer';
import { kindColor } from '../cords/traffic';

export function createCanvas2dDrawer(
  engine: CanvasEngine,
  canvas: HTMLCanvasElement,
  colors: () => { grid: Rgba; cord: Rgba; accent: Rgba; ink: Rgba },
): ((frame: FrameInfo) => void) | null {
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;
  const cache = new Map<string, { ref: Drawing; path: Path2D; color: string }[]>();

  return (frame: FrameInfo) => {
    const { dpr, w, h } = frame.viewport;
    const cam = frame.camera;
    const z = cam.zoom;
    const c = colors();
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    // Grid.
    const { spacing } = gridLevel(z);
    const major = spacing * 5;
    ctx.fillStyle = rgbaCss(c.grid);
    const x0 = Math.floor(cam.x / spacing) * spacing;
    const y0 = Math.floor(cam.y / spacing) * spacing;
    for (let gx = x0; gx < cam.x + w / z; gx += spacing) {
      for (let gy = y0; gy < cam.y + h / z; gy += spacing) {
        const big = Math.abs(gx % major) < 1e-6 && Math.abs(gy % major) < 1e-6;
        const r = big ? 1.4 : 0.9;
        ctx.fillRect((gx - cam.x) * z - r / 2, (gy - cam.y) * z - r / 2, r, r);
      }
    }

    ctx.save();
    ctx.setTransform(dpr * z, 0, 0, dpr * z, -cam.x * z * dpr, -cam.y * z * dpr);
    // Groups.
    for (const g of engine.store.groups) {
      const b = engine.groupBounds(g.id);
      if (!b) continue;
      ctx.strokeStyle = g.color ?? '#7c8cff';
      ctx.setLineDash([6 / z, 6 / z]);
      ctx.lineWidth = 1.5 / z;
      ctx.strokeRect(b.x, b.y, b.w, b.h);
      ctx.setLineDash([]);
    }
    // Drawings.
    for (const d of engine.store.drawings) {
      let runs = cache.get(d.id);
      if (!runs || runs[0]?.ref !== d) {
        runs = buildDrawing(d).map((run) => {
          const p = new Path2D();
          for (let i = 0; i + 5 < run.tris.length; i += 6) {
            p.moveTo(run.tris[i], run.tris[i + 1]);
            p.lineTo(run.tris[i + 2], run.tris[i + 3]);
            p.lineTo(run.tris[i + 4], run.tris[i + 5]);
            p.closePath();
          }
          return { ref: d, path: p, color: rgbaCss(run.color) };
        });
        cache.set(d.id, runs);
      }
      for (const r of runs) {
        ctx.fillStyle = r.color;
        ctx.fill(r.path);
      }
    }
    // Cords.
    ctx.lineWidth = 2 / z;
    for (const cn of engine.store.connections) {
      const a = engine.store.node(cn.from);
      const b = engine.store.node(cn.to);
      if (!a || !b) continue;
      const p = cordPath(a, b);
      ctx.strokeStyle = engine.selection.has(cn.id) ? rgbaCss(c.accent) : rgbaCss([c.cord[0], c.cord[1], c.cord[2], 0.4]);
      ctx.beginPath();
      ctx.moveTo(p.p0.x, p.p0.y);
      ctx.bezierCurveTo(p.c0.x, p.c0.y, p.c1.x, p.c1.y, p.p1.x, p.p1.y);
      ctx.stroke();
    }
    ctx.restore();

    // Far plates and names.
    ctx.font = '12px system-ui, sans-serif';
    ctx.textAlign = 'center';
    for (const v of frame.visibleNodes) {
      if (v.lod !== 'far') continue;
      const sx = (v.x - cam.x) * z;
      const sy = (v.y - cam.y) * z;
      const node = engine.store.node(v.id);
      ctx.fillStyle = isTerminalKind(v.kind) ? 'rgba(40,40,48,0.9)' : 'rgba(120,120,130,0.6)';
      ctx.fillRect(sx, sy, v.w * z, v.h * z);
      if (node && isTerminalKind(v.kind)) {
        ctx.fillStyle = stateCss(agentStates.stateOf(node));
        ctx.beginPath();
        ctx.arc(sx + (v.w * z) / 2, sy + (v.h * z) / 2, Math.max(4, (v.w * z) / 8), 0, Math.PI * 2);
        ctx.fill();
      }
      if (v.title) {
        ctx.fillStyle = rgbaCss(c.ink);
        ctx.fillText(v.title, sx + (v.w * z) / 2, sy + v.h * z + 14);
      }
    }
    // Envelopes.
    for (const env of engine.traffic.frame(frame.time)) {
      const sx = (env.x - cam.x) * z;
      const sy = (env.y - cam.y) * z;
      ctx.fillStyle = `#${(env.ring > 0 ? 0xf4d35e : env.color || kindColor('message')).toString(16).padStart(6, '0')}`;
      ctx.globalAlpha = env.alpha;
      ctx.fillRect(sx - 7, sy - 5, 14, 10);
      ctx.globalAlpha = 1;
    }
  };
}
