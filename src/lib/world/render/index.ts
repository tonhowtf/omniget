// Public entry of the OmniGet world renderer (contract: docs/agents/f6-renderer.md §4).
// One contract, three backends: gl2 (floor), gl1 (fallback), canvas2d (tier 0).
// Nothing here runs unless the /world route imported it dynamically.

import { backendForTier, budgetForTier, frameBudgetMs } from './tier';
import { emitTierChanged, Watchdog } from './watchdog';
import {
  ERR,
  RenderError,
  type AtlasHandle,
  type BackendName,
  type Caps,
  type ChunkId,
  type ChunkTile,
  type DynamicFrame,
  type FrameStats,
  type InitOpts,
  type Renderer,
  type Scene,
  type TextHandle,
  type TextStyle,
  type Tier,
} from './types';

export { calibrate, calibrateDetailed, median } from './calibrate';
export type { CalibrationDetail } from './calibrate';
export {
  buildBenchScene,
  createSyntheticAtlas,
  benchChunkTiles,
  benchChunkIds,
  benchTextLabels,
  attachBenchTexts,
  primeBenchChunks,
} from './bench-scene';
export type { BenchSceneOpts } from './bench-scene';
export { runSelftest, reportSelftest, summariseFrames, SELFTEST_LOG_PREFIX } from './selftest';
export type { SelftestReport, SelftestOpts, SelftestSceneReport } from './selftest';
export { budgetForTier, tierFromMedian, backendForTier, frameBudgetMs, TIER_BUDGETS } from './tier';
export type { TierBudget } from './tier';
export { TIER_CHANGED_EVENT, Watchdog, p95 } from './watchdog';
export type { TierChangedDetail } from './watchdog';
export { makeCamera, worldToScreen, screenToWorld, visibleChunks, chunkId, project, clampZoom } from './camera';
export { parseAtlas, resolveDir } from './atlas';
export { AnimClock, frameIndex } from './anim';
export * from './types';

/** Probes which WebGL versions the webview can actually create. */
export function probeBackends(): { gl2: boolean; gl1: boolean } {
  if (typeof document === 'undefined') return { gl2: false, gl1: false };
  const canvas = document.createElement('canvas');
  canvas.width = 1;
  canvas.height = 1;
  let gl2 = false;
  let gl1 = false;
  try {
    const c2 = canvas.getContext('webgl2');
    gl2 = !!c2;
    (c2?.getExtension('WEBGL_lose_context') as { loseContext(): void } | null)?.loseContext();
  } catch {
    gl2 = false;
  }
  if (!gl2) {
    try {
      const c1 = canvas.getContext('webgl');
      gl1 = !!c1;
      (c1?.getExtension('WEBGL_lose_context') as { loseContext(): void } | null)?.loseContext();
    } catch {
      gl1 = false;
    }
  }
  return { gl2, gl1: gl1 || gl2 };
}

/**
 * The renderer the app talks to. The concrete backend is chosen inside `init`
 * and imported dynamically, so a tier-3 machine never downloads the Canvas 2D
 * path and a tier-0 machine never downloads more GL than it uses.
 */
export function createRenderer(): Renderer {
  let impl: Renderer | null = null;
  let chosen: BackendName = 'gl2';

  function need(): Renderer {
    if (!impl) throw new RenderError(ERR.NOT_INITIALISED, 'renderer not initialised');
    return impl;
  }

  return {
    get tier() {
      return impl ? impl.tier : 0;
    },
    setTier(tier: Tier) {
      impl?.setTier(tier);
    },
    async init(canvas: HTMLCanvasElement, opts: InitOpts): Promise<Caps> {
      const probe = opts.backend ? { gl2: true, gl1: true } : probeBackends();
      chosen = opts.backend ?? backendForTier(opts.tier, probe);
      if (chosen === 'canvas2d') {
        const { createCanvas2dRenderer } = await import('./canvas2d');
        impl = createCanvas2dRenderer();
      } else {
        const { createGlRenderer } = await import('./gl2');
        impl = createGlRenderer(chosen === 'gl1' ? 1 : 2);
      }
      try {
        return await impl.init(canvas, opts);
      } catch (e) {
        // A GL backend that cannot start is not a dead end: tier 0 always runs.
        if (chosen === 'canvas2d') throw e;
        const { createCanvas2dRenderer } = await import('./canvas2d');
        impl = createCanvas2dRenderer();
        chosen = 'canvas2d';
        return impl.init(canvas, { ...opts, tier: 0, backend: 'canvas2d' });
      }
    },
    loadAtlas(json: unknown, pages: ImageBitmap[]): AtlasHandle {
      return need().loadAtlas(json, pages);
    },
    frame(scene: Scene, dt: number): FrameStats {
      return need().frame(scene, dt);
    },
    bakeChunk(id: ChunkId, tiles: ChunkTile[]): void {
      need().bakeChunk(id, tiles);
    },
    invalidateChunk(id: ChunkId): void {
      need().invalidateChunk(id);
    },
    setDynamicPage(page: ImageBitmap | null, frames: Record<string, DynamicFrame>): void {
      need().setDynamicPage(page, frames);
    },
    memory() {
      return impl ? impl.memory() : { textures: 0, bytes: 0, dynamicPage: 0 };
    },
    text(str: string, style?: TextStyle): TextHandle {
      return need().text(str, style);
    },
    resize(w: number, h: number, dpr: number): void {
      impl?.resize(w, h, dpr);
    },
    sleep(): void {
      impl?.sleep();
    },
    async wake(): Promise<void> {
      await impl?.wake();
    },
    destroy(): void {
      impl?.destroy();
      impl = null;
    },
  };
}

/** requestAnimationFrame callbacks this module has run since load (I-8 proof). */
let rafTicks = 0;
export function getRafTicks(): number {
  return rafTicks;
}
export function resetRafTicks(): void {
  rafTicks = 0;
}

export interface LoopHandle {
  start(): void;
  stop(): void;
  readonly running: boolean;
  readonly lastStats: FrameStats;
  readonly fps: number;
}

export interface LoopOpts {
  /** Lowers the tier when the p95 frame time stays over budget (§1.3). */
  watchdog?: boolean;
  onStats?: (stats: FrameStats, fps: number) => void;
  onTierChanged?: (tier: Tier) => void;
}

/**
 * The frame loop. `stop()` cancels the rAF: with the route hidden the renderer
 * runs zero callbacks, which is what "0 rAF after sleep()" means in §1.2.
 */
export function createLoop(
  renderer: Renderer,
  getScene: (dtMs: number) => Scene,
  opts: LoopOpts = {},
): LoopHandle {
  let handle = 0;
  let running = false;
  let last = 0;
  let fps = 0;
  let frames = 0;
  let fpsWindowStart = 0;
  let lastStats: FrameStats = { cpuMs: 0, drawCalls: 0, sprites: 0, chunksBaked: 0 };
  const watchdog = new Watchdog(renderer.tier);

  const tick = (now: number) => {
    if (!running) return;
    rafTicks++;
    const dt = last === 0 ? 16.7 : now - last;
    last = now;
    const scene = getScene(dt);
    lastStats = renderer.frame(scene, dt);
    frames++;
    if (fpsWindowStart === 0) fpsWindowStart = now;
    else if (now - fpsWindowStart >= 500) {
      fps = (frames * 1000) / (now - fpsWindowStart);
      frames = 0;
      fpsWindowStart = now;
    }
    if (opts.watchdog !== false) {
      const from = watchdog.tier;
      const next = watchdog.push(dt, now);
      if (next !== null) {
        renderer.setTier(next);
        emitTierChanged({
          from,
          to: next,
          p95Ms: watchdog.lastP95,
          budgetMs: frameBudgetMs(from),
          reason: 'watchdog',
        });
        opts.onTierChanged?.(next);
      }
    }
    opts.onStats?.(lastStats, fps);
    handle = requestAnimationFrame(tick);
  };

  return {
    start() {
      if (running) return;
      running = true;
      last = 0;
      fpsWindowStart = 0;
      watchdog.setTier(renderer.tier);
      handle = requestAnimationFrame(tick);
    },
    stop() {
      running = false;
      if (handle) cancelAnimationFrame(handle);
      handle = 0;
    },
    get running() {
      return running;
    },
    get lastStats() {
      return lastStats;
    },
    get fps() {
      return fps;
    },
  };
}

/** Sprites a tier is allowed to animate at once, for the caller to cap its scene. */
export function agentCap(tier: Tier): number {
  return budgetForTier(tier).agents;
}
