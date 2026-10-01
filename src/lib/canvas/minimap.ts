import type { CanvasEngine } from './engine.svelte';
import { drawingBounds } from './model/store.svelte';
import { isTerminalKind, type Rect } from './model/types';
import { union } from './spatial/geometry';
import { agentStates, stateCss } from './state/agents.svelte';

export const MINIMAP_W = 180;
export const MINIMAP_H = 120;

export interface MinimapTransform {
  world: Rect;
  scale: number;
  ox: number;
  oy: number;
}

function minimapTransform(engine: CanvasEngine, w = MINIMAP_W, h = MINIMAP_H): MinimapTransform {
  const view = engine.camera.viewRect();
  const content = engine.contentBounds();
  const u = union(content ? [content, view] : [view])!;
  const pad = Math.max(u.w, u.h) * 0.08;
  const world = { x: u.x - pad, y: u.y - pad, w: u.w + pad * 2, h: u.h + pad * 2 };
  const scale = Math.min(w / world.w, h / world.h);
  const ox = (w - world.w * scale) / 2;
  const oy = (h - world.h * scale) / 2;
  return { world, scale, ox, oy };
}

export function minimapToWorld(t: MinimapTransform, mx: number, my: number): { x: number; y: number } {
  return { x: t.world.x + (mx - t.ox) / t.scale, y: t.world.y + (my - t.oy) / t.scale };
}

const KIND_FILL: Record<string, string> = {
  note: '#e9c46a',
  text: '#9aa5b1',
  portal: '#7cc4b8',
  filetree: '#81c995',
  file: '#8ab4f8',
  image: '#c58af9',
};

export function drawMinimap(
  ctx: CanvasRenderingContext2D,
  engine: CanvasEngine,
  dpr: number,
  theme: { bg: string; view: string; drawing: string },
  w = MINIMAP_W,
  h = MINIMAP_H,
): MinimapTransform {
  const t = minimapTransform(engine, w, h);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  ctx.fillStyle = theme.bg;
  ctx.fillRect(0, 0, w, h);
  const map = (r: Rect) => ({
    x: t.ox + (r.x - t.world.x) * t.scale,
    y: t.oy + (r.y - t.world.y) * t.scale,
    w: Math.max(1.5, r.w * t.scale),
    h: Math.max(1.5, r.h * t.scale),
  });
  ctx.fillStyle = theme.drawing;
  for (const d of engine.store.drawings) {
    const r = map(drawingBounds(d));
    ctx.fillRect(r.x, r.y, r.w, r.h);
  }
  for (const n of engine.store.nodes) {
    if (n.stackId) continue;
    const r = map(n);
    if (isTerminalKind(n.kind)) {
      const state = agentStates.stateOf(n);
      ctx.fillStyle = state === 'blocked' ? '#e5484d' : stateCss(state);
    } else {
      ctx.fillStyle = KIND_FILL[n.kind] ?? '#8a8a8a';
    }
    ctx.globalAlpha = engine.selection.has(n.id) ? 1 : 0.75;
    ctx.fillRect(r.x, r.y, r.w, r.h);
  }
  ctx.globalAlpha = 1;
  const v = map(engine.camera.viewRect());
  ctx.strokeStyle = theme.view;
  ctx.lineWidth = 1.5;
  ctx.strokeRect(v.x + 0.5, v.y + 0.5, v.w - 1, v.h - 1);
  return t;
}
