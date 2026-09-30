// Runtime watchdog: if p95 of the frame time stays above 1.5x the tier budget for
// 3 consecutive seconds, drop one tier (§1.3). Never climbs back inside a session.

import { frameBudgetMs, lowerTier } from './tier';
import type { Tier } from './types';

export const WINDOW_MS = 3000;
export const OVER_BUDGET_FACTOR = 1.5;
export const TIER_CHANGED_EVENT = 'world:tier-changed';

export interface TierChangedDetail {
  from: Tier;
  to: Tier;
  p95Ms: number;
  budgetMs: number;
  reason: 'watchdog';
}

/** p95 of a sample array. Pure; does not mutate the input. */
export function p95(samples: readonly number[]): number {
  if (samples.length === 0) return 0;
  const sorted = Array.from(samples).sort((a, b) => a - b);
  const idx = Math.min(sorted.length - 1, Math.ceil(sorted.length * 0.95) - 1);
  return sorted[Math.max(0, idx)];
}

export class Watchdog {
  private samples: number[] = [];
  private stamps: number[] = [];
  private overSince: number | null = null;
  tier: Tier;

  constructor(tier: Tier) {
    this.tier = tier;
  }

  /** Feeds one frame. Returns the new tier when it dropped, else null. */
  push(frameMs: number, nowMs: number): Tier | null {
    this.samples.push(frameMs);
    this.stamps.push(nowMs);
    while (this.stamps.length > 0 && nowMs - this.stamps[0] > WINDOW_MS) {
      this.stamps.shift();
      this.samples.shift();
    }
    // Need a full window before judging.
    if (this.stamps.length < 2 || nowMs - this.stamps[0] < WINDOW_MS) return null;
    const budget = frameBudgetMs(this.tier) * OVER_BUDGET_FACTOR;
    const current = p95(this.samples);
    if (current <= budget) {
      this.overSince = null;
      return null;
    }
    if (this.overSince === null) {
      this.overSince = nowMs;
      return null;
    }
    if (nowMs - this.overSince < WINDOW_MS) return null;
    const next = lowerTier(this.tier);
    this.overSince = null;
    this.samples = [];
    this.stamps = [];
    if (next === this.tier) return null;
    this.tier = next;
    return next;
  }

  /** Called when the user or the calibration sets the tier explicitly. */
  setTier(tier: Tier): void {
    this.tier = tier;
    this.samples = [];
    this.stamps = [];
    this.overSince = null;
  }

  get lastP95(): number {
    return p95(this.samples);
  }
}

export function emitTierChanged(detail: TierChangedDetail): void {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new CustomEvent(TIER_CHANGED_EVENT, { detail }));
}
