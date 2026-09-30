// Tier table and thresholds, encoded from docs/llm-world-execution-plan.md §1.2/§1.3.
// Degradation is declared here, never emergent (decision 10 of the orchestration §4.4).
// Deciding a tier from a GPU string, navigator, core count or OS is forbidden: we measure.

import type { BackendName, Tier } from './types';

export interface TierBudget {
  tier: Tier;
  /** Target frames per second. */
  fps: number;
  /** Render CPU budget per frame, ms. */
  cpuMs: number;
  /** Side of the render texture baked per chunk. */
  chunkTexture: 256 | 512 | 1024;
  /** Animated agents on screen. */
  agents: number;
  particles: boolean;
  shadows: boolean;
  dayNight: boolean;
  bloom: boolean;
  /** Resident texture budget in bytes; Infinity = unlimited. */
  textureBytes: number;
  /** 'hover' = only hovered names, 'names+1' = names plus one balloon, 'all' = everything. */
  text: 'hover' | 'names+1' | 'all';
  /** Device pixel ratio ceiling for the world canvas. */
  maxDpr: number;
}

const MB = 1024 * 1024;

export const TIER_BUDGETS: Record<Tier, TierBudget> = {
  0: {
    tier: 0,
    fps: 20,
    cpuMs: 10,
    chunkTexture: 256,
    agents: 4,
    particles: false,
    shadows: false,
    dayNight: false,
    bloom: false,
    textureBytes: 32 * MB,
    text: 'hover',
    maxDpr: 1,
  },
  1: {
    tier: 1,
    fps: 30,
    cpuMs: 6,
    chunkTexture: 512,
    agents: 8,
    particles: false,
    shadows: true,
    dayNight: false,
    bloom: false,
    textureBytes: 64 * MB,
    text: 'names+1',
    maxDpr: 1,
  },
  2: {
    tier: 2,
    fps: 60,
    cpuMs: 4,
    chunkTexture: 512,
    agents: 24,
    particles: true,
    shadows: true,
    dayNight: true,
    bloom: false,
    textureBytes: 192 * MB,
    text: 'all',
    maxDpr: 2,
  },
  3: {
    tier: 3,
    fps: 60,
    cpuMs: 4,
    chunkTexture: 1024,
    agents: 64,
    particles: true,
    shadows: true,
    dayNight: true,
    bloom: true,
    textureBytes: Infinity,
    text: 'all',
    maxDpr: 2,
  },
};

export function budgetForTier(tier: Tier): TierBudget {
  return TIER_BUDGETS[tier];
}

/** Median frame time of the calibration scene, in ms, to a tier (§1.3). */
export function tierFromMedian(
  medianMs: number,
  opts: { webglAvailable?: boolean; contextLost?: boolean } = {},
): Tier {
  if (opts.webglAvailable === false || opts.contextLost === true) return 0;
  if (!Number.isFinite(medianMs) || medianMs <= 0) return 0;
  if (medianMs <= 4) return 3;
  if (medianMs <= 10) return 2;
  if (medianMs <= 25) return 1;
  return 0;
}

/** Backend a tier runs on by default; the caller may still force one. */
export function backendForTier(tier: Tier, has: { gl2: boolean; gl1: boolean }): BackendName {
  if (tier === 0) return 'canvas2d';
  if (has.gl2) return 'gl2';
  if (has.gl1) return 'gl1';
  return 'canvas2d';
}

export function lowerTier(tier: Tier): Tier {
  return (tier > 0 ? tier - 1 : 0) as Tier;
}

/** Frame budget the watchdog compares p95 against (1.5x = downgrade, §1.3). */
export function frameBudgetMs(tier: Tier): number {
  return 1000 / TIER_BUDGETS[tier].fps;
}

/**
 * 3D budget of a tier, for the `three3d` backend of the city (estudo 99, M0).
 * Same tiers and the same measuring method as the 2D table; only the knobs
 * differ. Distances are metres (1 tile = 2 m, 1 region = 128 m).
 */
export interface Tier3DBudget {
  tier: Tier;
  /** Target frames per second (the loop skips frames above it). */
  fps: number;
  /** Camera far plane and region streaming radius, metres. */
  drawDistance: number;
  /** Beyond this, `enfeite` and `prop` chunks are hidden, metres. */
  detailDistance: number;
  /**
   * Beyond this (camera to cell centre, metres) a cell draws the kit's far
   * variants (`<key>#lod1`) and loses its outline.
   */
  lodDistance: number;
  /** Shadow map side; 0 = no shadow at all. */
  shadowMap: 0 | 1024 | 2048 | 4096;
  /** Inverted-hull outline on buildings (one extra draw per mesh). */
  outline: boolean;
  /** Device pixel ratio ceiling of the 3D canvas. */
  maxDpr: number;
  /** MSAA on the default framebuffer. */
  antialias: boolean;
  /** Instances drawn at once; regions past it are not streamed in. */
  instances: number;
  /** Rigged characters at once; the rest are drawn as capsules. */
  fullCharacters: number;
  /** People walking around (rigged or not). */
  people: number;
}

export const TIER3D_BUDGETS: Record<Tier, Tier3DBudget> = {
  0: {
    tier: 0,
    fps: 20,
    drawDistance: 110,
    detailDistance: 36,
    lodDistance: 55,
    shadowMap: 0,
    outline: false,
    maxDpr: 1,
    antialias: false,
    instances: 3_000,
    fullCharacters: 0,
    people: 4,
  },
  1: {
    tier: 1,
    fps: 30,
    drawDistance: 180,
    detailDistance: 70,
    lodDistance: 85,
    shadowMap: 1024,
    outline: false,
    maxDpr: 1,
    antialias: false,
    instances: 12_000,
    fullCharacters: 4,
    people: 12,
  },
  2: {
    tier: 2,
    fps: 60,
    drawDistance: 280,
    detailDistance: 130,
    lodDistance: 105,
    shadowMap: 2048,
    outline: true,
    maxDpr: 1.5,
    antialias: true,
    instances: 40_000,
    fullCharacters: 16,
    people: 32,
  },
  3: {
    tier: 3,
    fps: 60,
    drawDistance: 420,
    detailDistance: 220,
    lodDistance: 170,
    shadowMap: 4096,
    outline: true,
    maxDpr: 2,
    antialias: true,
    instances: 120_000,
    fullCharacters: 48,
    people: 64,
  },
};

export function budget3dForTier(tier: Tier): Tier3DBudget {
  return TIER3D_BUDGETS[tier];
}

/**
 * Frame budget of the 3D city's tier 0 (M0: ~30 fps at p95 on software GL).
 * A 3D calibration scene measured over it draws the city in lite mode: the
 * same regions with the 2D renderer and the kit baked into sprites.
 */
export const CITY3D_TIER0_FRAME_MS = 33;

export type CityBackend = 'lite' | '3d';

/** Lite mode when the 3D scene could not run or ran over the tier-0 budget. */
export function cityBackendFromCalibration(r: {
  medianMs: number;
  backend: BackendName | 'none';
  contextLostDuring: boolean;
}): CityBackend {
  if (r.backend === 'none' || r.contextLostDuring) return 'lite';
  if (!Number.isFinite(r.medianMs) || r.medianMs <= 0) return 'lite';
  return r.medianMs > CITY3D_TIER0_FRAME_MS ? 'lite' : '3d';
}
