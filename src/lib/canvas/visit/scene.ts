// Pure helpers for the visitor's read-only view of a shared canvas: the
// scene arrives as JSON from the host (`crate::canvas::visit::build_scene`)
// and is drawn as cards and cords in an SVG.

import type { Scene, SceneCord, SceneNode } from './api';

export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** Bounds of every node, padded; a small default box for an empty floor. */
export function sceneBounds(scene: Scene, pad = 80): Box {
  if (!scene.nodes.length) return { x: -400, y: -300, w: 800, h: 600 };
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  for (const n of scene.nodes) {
    x0 = Math.min(x0, n.x);
    y0 = Math.min(y0, n.y);
    x1 = Math.max(x1, n.x + n.w);
    y1 = Math.max(y1, n.y + n.h);
  }
  return { x: x0 - pad, y: y0 - pad, w: x1 - x0 + pad * 2, h: y1 - y0 + pad * 2 };
}

function center(n: SceneNode): [number, number] {
  return [n.x + n.w / 2, n.y + n.h / 2];
}

/** Where the segment from the centre of `a` towards `b` leaves `a`'s box. */
function edgePoint(a: SceneNode, towards: [number, number]): [number, number] {
  const [cx, cy] = center(a);
  const dx = towards[0] - cx;
  const dy = towards[1] - cy;
  if (dx === 0 && dy === 0) return [cx, cy];
  const sx = dx !== 0 ? a.w / 2 / Math.abs(dx) : Infinity;
  const sy = dy !== 0 ? a.h / 2 / Math.abs(dy) : Infinity;
  const s = Math.min(sx, sy);
  return [cx + dx * s, cy + dy * s];
}

export interface CordPath {
  id: string;
  kind: string;
  label?: string | null;
  d: string;
  mid: [number, number];
}

/** A gently curved path per cord whose two ends are on the floor. */
export function cordPaths(scene: Scene): CordPath[] {
  const byId = new Map(scene.nodes.map((n) => [n.id, n]));
  const out: CordPath[] = [];
  for (const c of scene.cords as SceneCord[]) {
    const a = byId.get(c.from);
    const b = byId.get(c.to);
    if (!a || !b) continue;
    const p0 = edgePoint(a, center(b));
    const p1 = edgePoint(b, center(a));
    const mx = (p0[0] + p1[0]) / 2;
    const my = (p0[1] + p1[1]) / 2;
    const sag = Math.min(80, Math.hypot(p1[0] - p0[0], p1[1] - p0[1]) * 0.12);
    const cy = my + sag;
    out.push({
      id: c.id,
      kind: c.kind,
      label: c.label,
      d: `M${p0[0].toFixed(1)},${p0[1].toFixed(1)} Q${mx.toFixed(1)},${cy.toFixed(1)} ${p1[0].toFixed(1)},${p1[1].toFixed(1)}`,
      mid: [mx, (my + cy) / 2],
    });
  }
  return out;
}

/** Last `n` lines of a shared screen, for a card. */
export function screenTail(text: string | undefined, n: number): string {
  if (!text) return '';
  const lines = text.split('\n');
  return lines.slice(Math.max(0, lines.length - n)).join('\n');
}

/** A loose check of a scene coming from an event. */
export function isScene(v: unknown): v is Scene {
  const s = v as Scene | null;
  return !!s && s.house === 'canvas-v1' && Array.isArray(s.nodes) && Array.isArray(s.cords);
}
