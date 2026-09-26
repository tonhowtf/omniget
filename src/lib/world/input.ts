// Input for the world route: click to walk, drag to pan, wheel to zoom, keys
// to nudge the camera, hover to pick an agent.
//
// The maths is pure and lives at the top of the file (so vitest can check that
// zooming keeps the tile under the cursor put); the listener plumbing is one
// function at the bottom that returns its own cleanup. Nothing here survives
// the route: `attachInput` detaches every listener it added.

import { clampZoom, screenToWorld } from '$lib/world/render/camera';
import { CANVAS_REPLACED_EVENT, type Camera } from '$lib/world/render/types';

/** Pointer travel, in device pixels, past which a press is a drag, not a click. */
export const DRAG_SLOP = 4;
/** One wheel notch. */
export const ZOOM_STEP = 1.1;
/** How far a click may land from an agent's feet and still pick it, in tiles. */
export const PICK_RADIUS_TILES = 0.75;
/** Camera nudge per key press, in tiles. */
export const KEY_STEP = 1;

export interface Tile {
  x: number;
  y: number;
}

/** The tile under a point, floored: the same integer the simulation uses. */
export function tileAt(cam: Camera, sx: number, sy: number): Tile {
  const w = screenToWorld(cam, sx, sy);
  return { x: Math.floor(w.x), y: Math.floor(w.y) };
}

/** Drag the ground: the world point under the pointer stays under the pointer. */
export function panByPixels(cam: Camera, dxPx: number, dyPx: number): void {
  const a = screenToWorld(cam, 0, 0);
  const b = screenToWorld(cam, dxPx, dyPx);
  cam.x -= b.x - a.x;
  cam.y -= b.y - a.y;
}

/** Zoom about a point on the canvas, which is what a wheel over a tile means. */
export function zoomAt(cam: Camera, factor: number, sx: number, sy: number): void {
  const before = screenToWorld(cam, sx, sy);
  const next = clampZoom(cam.zoom * factor);
  if (next === cam.zoom) return;
  cam.zoom = next;
  const after = screenToWorld(cam, sx, sy);
  cam.x += before.x - after.x;
  cam.y += before.y - after.y;
}

export interface Pickable {
  id: number;
  /** Tile-unit position, already interpolated. */
  x: number;
  y: number;
}

/**
 * The agent under a point, or null. Ties go to the one drawn last (in front),
 * which is the one the user sees on top.
 */
export function pickAgent(
  items: readonly Pickable[],
  cam: Camera,
  sx: number,
  sy: number,
  radiusTiles = PICK_RADIUS_TILES,
): number | null {
  const w = screenToWorld(cam, sx, sy);
  const r2 = radiusTiles * radiusTiles;
  let best: number | null = null;
  let bestDepth = -Infinity;
  for (const it of items) {
    const dx = it.x - w.x;
    const dy = it.y - w.y;
    if (dx * dx + dy * dy > r2) continue;
    const depth = it.x + it.y;
    if (depth >= bestDepth) {
      bestDepth = depth;
      best = it.id;
    }
  }
  return best;
}

/** Camera nudge for a key, in tiles, or null when the key is not ours. */
export function keyPan(key: string): Tile | null {
  switch (key) {
    case 'ArrowUp':
    case 'w':
    case 'W':
      return { x: -KEY_STEP, y: -KEY_STEP };
    case 'ArrowDown':
    case 's':
    case 'S':
      return { x: KEY_STEP, y: KEY_STEP };
    case 'ArrowLeft':
    case 'a':
    case 'A':
      return { x: -KEY_STEP, y: KEY_STEP };
    case 'ArrowRight':
    case 'd':
    case 'D':
      return { x: KEY_STEP, y: -KEY_STEP };
    default:
      return null;
  }
}

export function keyZoom(key: string): number | null {
  if (key === '+' || key === '=') return ZOOM_STEP;
  if (key === '-' || key === '_') return 1 / ZOOM_STEP;
  return null;
}

export interface InputHandlers {
  /** A click that landed on no agent: walk there. */
  onTile(tile: Tile, ev: PointerEvent): void;
  /** A click that landed on an agent. */
  onAgent(id: number, ev: PointerEvent): void;
  /** Pointer moved over an agent, or off every agent (null). */
  onHover(id: number | null): void;
  /** The camera changed; the route re-renders on the next frame anyway. */
  onCamera?(): void;
  /** Escape, for closing the object picker. */
  onEscape?(): void;
  /** The pointer is over this tile (no drag in progress). */
  onHoverTile?(tile: Tile): void;
  /** First look at a key; true means it was consumed. */
  onKey?(ev: KeyboardEvent): boolean;
}

export interface InputOpts {
  camera: Camera;
  /** Agents to hit-test, rebuilt by the caller every frame. */
  pickables(): readonly Pickable[];
  /** CSS pixels to device pixels, which is the canvas' own scale. */
  dpr(): number;
}

/**
 * Wires one canvas. Returns the detach function; call it when the route goes
 * away, or the listeners outlive the page they belong to.
 */
export function attachInput(
  canvas: HTMLCanvasElement,
  opts: InputOpts,
  handlers: InputHandlers,
): () => void {
  let dragging = false;
  let moved = false;
  let lastX = 0;
  let lastY = 0;
  let pointerId = -1;

  const toCanvas = (ev: { clientX: number; clientY: number }): { sx: number; sy: number } => {
    const rect = canvas.getBoundingClientRect();
    const scale = opts.dpr();
    return { sx: (ev.clientX - rect.left) * scale, sy: (ev.clientY - rect.top) * scale };
  };

  const onPointerDown = (ev: PointerEvent) => {
    if (ev.button !== 0) return;
    dragging = true;
    moved = false;
    pointerId = ev.pointerId;
    const p = toCanvas(ev);
    lastX = p.sx;
    lastY = p.sy;
    canvas.setPointerCapture?.(ev.pointerId);
  };

  const onPointerMove = (ev: PointerEvent) => {
    const p = toCanvas(ev);
    if (dragging && ev.pointerId === pointerId) {
      const dx = p.sx - lastX;
      const dy = p.sy - lastY;
      if (!moved && Math.abs(dx) + Math.abs(dy) > DRAG_SLOP) moved = true;
      if (moved) {
        panByPixels(opts.camera, dx, dy);
        handlers.onCamera?.();
      }
      lastX = p.sx;
      lastY = p.sy;
      return;
    }
    handlers.onHover(pickAgent(opts.pickables(), opts.camera, p.sx, p.sy));
    handlers.onHoverTile?.(tileAt(opts.camera, p.sx, p.sy));
  };

  const onPointerUp = (ev: PointerEvent) => {
    if (ev.pointerId !== pointerId) return;
    canvas.releasePointerCapture?.(ev.pointerId);
    dragging = false;
    pointerId = -1;
    if (moved) return;
    const p = toCanvas(ev);
    const hit = pickAgent(opts.pickables(), opts.camera, p.sx, p.sy);
    if (hit !== null) handlers.onAgent(hit, ev);
    else handlers.onTile(tileAt(opts.camera, p.sx, p.sy), ev);
  };

  const onPointerLeave = () => {
    dragging = false;
    pointerId = -1;
    handlers.onHover(null);
  };

  const onWheel = (ev: WheelEvent) => {
    ev.preventDefault();
    const p = toCanvas(ev);
    zoomAt(opts.camera, ev.deltaY < 0 ? ZOOM_STEP : 1 / ZOOM_STEP, p.sx, p.sy);
    handlers.onCamera?.();
  };

  const onKeyDown = (ev: KeyboardEvent) => {
    if (handlers.onKey?.(ev)) {
      ev.preventDefault();
      return;
    }
    if (ev.metaKey || ev.ctrlKey || ev.altKey) return;
    if (ev.key === 'Escape') {
      handlers.onEscape?.();
      return;
    }
    const pan = keyPan(ev.key);
    if (pan) {
      opts.camera.x += pan.x;
      opts.camera.y += pan.y;
      handlers.onCamera?.();
      ev.preventDefault();
      return;
    }
    const zoom = keyZoom(ev.key);
    if (zoom) {
      zoomAt(opts.camera, zoom, opts.camera.width / 2, opts.camera.height / 2);
      handlers.onCamera?.();
      ev.preventDefault();
    }
  };

  canvas.addEventListener('pointerdown', onPointerDown);
  canvas.addEventListener('pointermove', onPointerMove);
  canvas.addEventListener('pointerup', onPointerUp);
  canvas.addEventListener('pointerleave', onPointerLeave);
  canvas.addEventListener('pointercancel', onPointerLeave);
  canvas.addEventListener('wheel', onWheel, { passive: false });
  canvas.addEventListener('keydown', onKeyDown);

  return () => {
    canvas.removeEventListener('pointerdown', onPointerDown);
    canvas.removeEventListener('pointermove', onPointerMove);
    canvas.removeEventListener('pointerup', onPointerUp);
    canvas.removeEventListener('pointerleave', onPointerLeave);
    canvas.removeEventListener('pointercancel', onPointerLeave);
    canvas.removeEventListener('wheel', onWheel);
    canvas.removeEventListener('keydown', onKeyDown);
  };
}

/**
 * Keep a binding on the canvas the renderer is actually drawing to. A GL
 * wake whose context did not come back swaps the element for a fresh one
 * (`CANVAS_REPLACED_EVENT` on the old one); the binding on the old element is
 * torn down, `onSwap` learns the new element, and the binding is made again
 * there. Returns the teardown of whatever binding is current.
 */
export function followCanvas<T extends EventTarget>(
  canvas: T,
  bind: (el: T) => () => void,
  onSwap?: (next: T) => void,
): () => void {
  let current = canvas;
  let unbind = bind(current);
  const onReplaced = (e: Event) => {
    const next = (e as CustomEvent<T>).detail;
    if (!next || next === current) return;
    unbind();
    current.removeEventListener(CANVAS_REPLACED_EVENT, onReplaced);
    current = next;
    onSwap?.(next);
    unbind = bind(next);
    next.addEventListener(CANVAS_REPLACED_EVENT, onReplaced);
  };
  current.addEventListener(CANVAS_REPLACED_EVENT, onReplaced);
  return () => {
    unbind();
    current.removeEventListener(CANVAS_REPLACED_EVENT, onReplaced);
  };
}
