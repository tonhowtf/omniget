import type { CanvasEngine } from '../engine.svelte';
import { circuitPoints, cordPath, sampleCord, portPoint } from '../cords/route';
import { canvasPrefs } from '../prefs.svelte';
import { Rope } from '../cords/rope';
import { isTerminalKind } from '../model/types';
import { intersects } from '../spatial/geometry';
import { agentStates, STATE_COLORS } from '../state/agents.svelte';
import { hexToRgba, withAlpha, type Rgba } from './color';
import type { FrameInfo, GlPass } from './gl-layer';
import { LineBatch } from './lines';
import { SdfBatch } from './sdf-shapes';

export function createCordsPass(engine: CanvasEngine, colors: () => { cord: Rgba; accent: Rgba; ink: Rgba }): GlPass {
  const lines = new LineBatch();
  const flaps = new LineBatch();
  const sdf = new SdfBatch();
  const ropes = new Map<string, Rope>();
  let lastTime = 0;

  return {
    id: 'canvas.cords',
    order: 30,
    draw(gl: WebGL2RenderingContext, frame: FrameInfo) {
      const cam = frame.camera;
      const z = cam.zoom;
      const { cord: cordColor, accent, ink } = colors();
      const view = engine.camera.viewRect(0.1);
      const active = engine.traffic.activeConnections();
      const dt = lastTime ? frame.time - lastTime : 16;
      lastTime = frame.time;
      let ropeMoving = false;
      const prefs = canvasPrefs();
      const globalStyle = prefs.rope_routing === 'circuit' ? 'circuit' : prefs.cord_style;
      const toScreen = (pts: number[]) => {
        const out = new Array<number>(pts.length);
        for (let i = 0; i < pts.length; i += 2) {
          out[i] = (pts[i] - cam.x) * z;
          out[i + 1] = (pts[i + 1] - cam.y) * z;
        }
        return out;
      };

      lines.begin();
      for (const c of engine.store.connections) {
        const a = engine.store.node(c.from);
        const b = engine.store.node(c.to);
        if (!a || !b) continue;
        const bx = Math.min(a.x, b.x);
        const by = Math.min(a.y, b.y);
        const box = { x: bx, y: by, w: Math.max(a.x + a.w, b.x + b.w) - bx, h: Math.max(a.y + a.h, b.y + b.h) - by };
        if (!intersects(box, view)) continue;
        const path = cordPath(a, b);
        let pts: number[];
        const style = (c as { style?: string }).style ?? globalStyle;
        if (style === 'circuit') {
          pts = circuitPoints(path);
        } else if (style === 'rope') {
          let rope = ropes.get(c.id);
          if (!rope) {
            rope = new Rope(path.p0, path.p1);
            ropes.set(c.id, rope);
          } else rope.setEnds(path.p0, path.p1);
          if (rope.step(dt)) ropeMoving = true;
          pts = rope.points();
        } else {
          pts = sampleCord(path, 32).pts;
        }
        const selected = engine.selection.has(c.id);
        let color = withAlpha(cordColor, 0.4);
        const remoteOf = (n: typeof a) =>
          n.kind === 'connector' ? String((n.data as Record<string, unknown> | undefined)?.remoteNodeId ?? n.id) : n.id;
        if (active.has(c.id) || engine.traffic.pairActive(remoteOf(a), remoteOf(b))) {
          const src = isTerminalKind(a.kind) ? STATE_COLORS[agentStates.stateOf(a)] : 0x6cb8e6;
          color = hexToRgba(src, 0.95);
        }
        if (selected) color = accent;
        const width = (selected ? 3 : 2) * (z < 0.3 ? 0.75 : 1);
        lines.polyline(toScreen(pts), width, color, c.kind === 'note' || c.kind === 'portal' ? 6 : 0);
        const end = path.p1;
        sdf.circle((end.x - cam.x) * z, (end.y - cam.y) * z, 3.2, { fill: color });
      }
      if (globalStyle !== 'rope') {
        for (const id of Array.from(ropes.keys())) {
          const c = engine.store.connections.find((x) => x.id === id) as { style?: string } | undefined;
          if (c?.style !== 'rope') ropes.delete(id);
        }
      }

      const draftCord = engine.draft.cord;
      if (draftCord) {
        const from = engine.store.node(draftCord.from);
        const target = draftCord.target ? engine.store.node(draftCord.target) : null;
        if (from) {
          const path = cordPath(from, target ?? { x: draftCord.x, y: draftCord.y });
          lines.polyline(toScreen(sampleCord(path, 24).pts), 2, accent, target ? 0 : 7);
          const p0 = portPoint(from, path.fromSide);
          sdf.circle((p0.x - cam.x) * z, (p0.y - cam.y) * z, 4, { fill: accent });
        }
      }

      lines.flush(gl, frame.viewport.w, frame.viewport.h);

      const envs = engine.traffic.frame(frame.time);
      flaps.begin();
      for (const env of envs) {
        const sx = (env.x - cam.x) * z;
        const sy = (env.y - cam.y) * z;
        if (env.ring > 0) {
          const r = 6 + env.ring * 14;
          sdf.circle(sx, sy, r, { strokeWidth: 2, stroke: hexToRgba(0xf4d35e, env.alpha) });
          continue;
        }
        const col = hexToRgba(env.color, 1);
        sdf.push({ cx: sx, cy: sy, hw: 7, hh: 5, radius: 1.5, rotation: env.angle, fill: col, strokeWidth: 1, stroke: withAlpha(ink, 0.8) });
        const c = Math.cos(env.angle);
        const s = Math.sin(env.angle);
        const rot = (x: number, y: number) => [sx + x * c - y * s, sy + x * s + y * c];
        const [ax, ay] = rot(-6, -4);
        const [mx, my] = rot(0, 0.5);
        const [bx2, by2] = rot(6, -4);
        flaps.polyline([ax, ay, mx, my, bx2, by2], 1, withAlpha(ink, 0.7));
      }
      sdf.flush(gl, frame.viewport.w, frame.viewport.h);
      flaps.flush(gl, frame.viewport.w, frame.viewport.h);
      if (ropeMoving) engine.invalidate();
    },
    dispose() {
      lines.dispose();
      flaps.dispose();
      sdf.dispose();
      ropes.clear();
    },
  };
}
