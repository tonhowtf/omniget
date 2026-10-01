// L0: the single WebGL2 context of the canvas (02-motor-do-canvas §2.2).
// Everything drawn on the GPU is a pass registered here and run in `order`:
// grid (0), group frames and far plates (10), drawings (20), cords (30),
// snapshots (40), terminal bodies (45, terminal-front), far sprites (50),
// envelopes (60), overlay (70). Renders on demand only: no frame unless the
// engine asks for one.
//
// Contract for passes from other territories (briefing "Motor do canvas"):
// `draw` may be called with a new WebGL2RenderingContext after a context
// loss; a pass must compare the `gl` it gets with the one it built its
// resources on and rebuild when they differ. `dispose` runs on unregister and
// when the layer is destroyed.

import { ContextTracker } from '$lib/gfx/context';
import { getGlContext } from '$lib/gfx/gl';
import { glBudget, type GlLease } from '$lib/gfx/gl-budget';
import type { Lod } from '../lod';

export interface CameraInfo {
  /** World coordinate at the viewport's top-left corner. */
  x: number;
  y: number;
  zoom: number;
}

export interface ViewportInfo {
  /** CSS pixels. */
  w: number;
  h: number;
  dpr: number;
}

export interface NodeView {
  id: string;
  kind: string;
  x: number;
  y: number;
  w: number;
  h: number;
  z?: number;
  title?: string;
  data?: Record<string, unknown> | unknown;
  /** Semantic zoom level of the node this frame. */
  lod?: Lod;
  /** Lifted or docked node: its rect in canvas CSS px at scale 1 (world frame untouched). */
  screen?: { x: number; y: number; w: number; h: number };
}

export interface FrameInfo {
  camera: CameraInfo;
  viewport: ViewportInfo;
  visibleNodes: NodeView[];
  /** Lifted and docked nodes, drawn in screen space over everything (not in `visibleNodes`). */
  screenNodes?: NodeView[];
  /** performance.now() of the frame, ms. */
  time: number;
}

export interface GlPass {
  id: string;
  order: number;
  draw(gl: WebGL2RenderingContext, frame: FrameInfo): void;
  dispose?(): void;
}

const passes = new Map<string, GlPass>();
let passesVersion = 0;
const passListeners = new Set<() => void>();

/** Registers (or replaces) a pass; returns the unregister function. */
export function registerPass(pass: GlPass): () => void {
  const prev = passes.get(pass.id);
  if (prev && prev !== pass) prev.dispose?.();
  passes.set(pass.id, pass);
  passesVersion++;
  for (const l of passListeners) l();
  return () => {
    if (passes.get(pass.id) !== pass) return;
    passes.delete(pass.id);
    passesVersion++;
    pass.dispose?.();
    for (const l of passListeners) l();
  };
}

function registeredPasses(): GlPass[] {
  return Array.from(passes.values()).sort((a, b) => a.order - b.order);
}

/** Lets the engine redraw when a pass arrives or leaves. */
export function onPassesChange(fn: () => void): () => void {
  passListeners.add(fn);
  return () => passListeners.delete(fn);
}

/** Screen (CSS px) of a world point for a frame. */
export function worldToScreen(cam: CameraInfo, x: number, y: number): { x: number; y: number } {
  return { x: (x - cam.x) * cam.zoom, y: (y - cam.y) * cam.zoom };
}

const CONTEXT_RETRY_DELAY_MS = 1500;
const CONTEXT_MAX_RETRIES = 3;

interface GlLayerCallbacks {
  /** The context died and will not come back: switch to Canvas 2D. */
  onFallback?: () => void;
  /** Context is back: redraw everything. */
  onRestored?: () => void;
}

export class GlLayer {
  canvas: HTMLCanvasElement | null = null;
  gl: WebGL2RenderingContext | null = null;
  private tracker = new ContextTracker();
  private lease: GlLease | null = null;
  private retries = 0;
  private retryTimer: ReturnType<typeof setTimeout> | null = null;
  private cbs: GlLayerCallbacks = {};
  width = 0;
  height = 0;
  dpr = 1;

  /** Returns false when WebGL2 is unavailable (caller falls back to Canvas 2D). */
  init(canvas: HTMLCanvasElement, cbs: GlLayerCallbacks = {}): boolean {
    this.canvas = canvas;
    this.cbs = cbs;
    this.lease = glBudget.acquire({
      kind: 'canvas',
      priority: 100,
      label: 'canvas-l0',
      onEvict: () => {
        // The budget reclaimed our slot: drop the context and fall back.
        this.drop();
        this.cbs.onFallback?.();
      },
    });
    const gl = getGlContext(canvas, 2) as WebGL2RenderingContext | null;
    if (!gl) {
      this.lease?.release();
      this.lease = null;
      return false;
    }
    this.gl = gl;
    this.tracker.attach(canvas, {
      onLost: () => this.onLost(),
      onRestored: () => {
        this.retries = 0;
        this.cbs.onRestored?.();
      },
    });
    return true;
  }

  private onLost(): void {
    // Munder Difflin's glRecovery: wait 1.5 s, try again up to 3 times.
    if (this.retryTimer) return;
    this.retryTimer = setTimeout(() => {
      this.retryTimer = null;
      if (!this.gl || !this.canvas) return;
      if (!this.gl.isContextLost()) {
        this.retries = 0;
        this.cbs.onRestored?.();
        return;
      }
      this.retries++;
      if (this.retries > CONTEXT_MAX_RETRIES) {
        this.cbs.onFallback?.();
        return;
      }
      const ext = this.gl.getExtension('WEBGL_lose_context');
      try {
        ext?.restoreContext();
      } catch {
        // Nothing to do: the next retry or the fallback takes over.
      }
      this.onLost();
    }, CONTEXT_RETRY_DELAY_MS);
  }

  get live(): boolean {
    return !!this.gl && !this.gl.isContextLost();
  }

  resize(w: number, h: number, dpr: number): void {
    this.width = w;
    this.height = h;
    this.dpr = dpr;
    if (!this.canvas) return;
    const pw = Math.max(1, Math.round(w * dpr));
    const ph = Math.max(1, Math.round(h * dpr));
    if (this.canvas.width !== pw) this.canvas.width = pw;
    if (this.canvas.height !== ph) this.canvas.height = ph;
  }

  render(frame: FrameInfo): void {
    const gl = this.gl;
    if (!gl || gl.isContextLost()) return;
    this.lease?.touch();
    gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight);
    gl.clearColor(0, 0, 0, 0);
    gl.clear(gl.COLOR_BUFFER_BIT);
    gl.enable(gl.BLEND);
    // Premultiplied alpha end to end.
    gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
    for (const pass of registeredPasses()) {
      try {
        pass.draw(gl, frame);
        // Diagnostic: set globalThis.__glDebug = true to name the pass that left a GL error.
        if ((globalThis as { __glDebug?: boolean }).__glDebug) {
          const e = gl.getError();
          if (e) console.warn(`[canvas] pass ${pass.id} left GL error ${e}`);
        }
      } catch (err) {
        // One broken pass must not take the whole canvas down.
        console.warn(`[canvas] pass ${pass.id} failed`, err);
      }
    }
  }

  private drop(): void {
    this.tracker.detach();
    this.gl = null;
  }

  destroy(): void {
    if (this.retryTimer) clearTimeout(this.retryTimer);
    this.retryTimer = null;
    const gl = this.gl;
    this.tracker.loseDeliberately(gl);
    this.tracker.detach();
    this.gl = null;
    this.lease?.release();
    this.lease = null;
    this.canvas = null;
  }
}
