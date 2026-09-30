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
