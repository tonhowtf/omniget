import { describe, expect, it } from 'vitest';
import {
  backendForTier,
  cityBackendFromCalibration,
  CITY3D_TIER0_FRAME_MS,
  budgetForTier,
  frameBudgetMs,
  lowerTier,
  tierFromMedian,
  TIER_BUDGETS,
} from './tier';
import { p95, Watchdog } from './watchdog';
import { clampFps, frameIndex, clipFinished, AnimClock, lerp } from './anim';
import { nextState } from './context';

describe('tierFromMedian', () => {
  it('follows the thresholds of the plan §1.3', () => {
    expect(tierFromMedian(2)).toBe(3);
    expect(tierFromMedian(4)).toBe(3);
    expect(tierFromMedian(4.1)).toBe(2);
    expect(tierFromMedian(10)).toBe(2);
    expect(tierFromMedian(11)).toBe(1);
    expect(tierFromMedian(25)).toBe(1);
    expect(tierFromMedian(25.5)).toBe(0);
  });

  it('falls to tier 0 without WebGL or with a context lost during calibration', () => {
    expect(tierFromMedian(1, { webglAvailable: false })).toBe(0);
    expect(tierFromMedian(1, { contextLost: true })).toBe(0);
    expect(tierFromMedian(Number.NaN)).toBe(0);
  });
});

describe('budget table', () => {
  it('encodes the §1.2 row of every tier', () => {
    expect(budgetForTier(0)).toMatchObject({ fps: 20, agents: 4, chunkTexture: 256, bloom: false });
    expect(budgetForTier(1)).toMatchObject({ fps: 30, cpuMs: 6, agents: 8, chunkTexture: 512 });
    expect(budgetForTier(2)).toMatchObject({ agents: 24, dayNight: true, bloom: false });
    expect(budgetForTier(3)).toMatchObject({ agents: 64, bloom: true, chunkTexture: 1024 });
  });

  it('never turns an effect on as the tier goes down', () => {
    const tiers = [0, 1, 2, 3] as const;
    for (let i = 1; i < tiers.length; i++) {
      const low = TIER_BUDGETS[tiers[i - 1]];
      const high = TIER_BUDGETS[tiers[i]];
      expect(low.agents).toBeLessThanOrEqual(high.agents);
      expect(Number(low.particles)).toBeLessThanOrEqual(Number(high.particles));
      expect(Number(low.bloom)).toBeLessThanOrEqual(Number(high.bloom));
    }
  });

  it('gives the frame budget in ms', () => {
    expect(frameBudgetMs(0)).toBe(50);
    expect(frameBudgetMs(3)).toBeCloseTo(16.67, 1);
  });
});

describe('backendForTier', () => {
  it('sends tier 0 to canvas 2d and everything else to the best GL available', () => {
    expect(backendForTier(0, { gl2: true, gl1: true })).toBe('canvas2d');
    expect(backendForTier(2, { gl2: true, gl1: true })).toBe('gl2');
    expect(backendForTier(2, { gl2: false, gl1: true })).toBe('gl1');
    expect(backendForTier(2, { gl2: false, gl1: false })).toBe('canvas2d');
  });

  it('lowerTier stops at zero', () => {
    expect(lowerTier(3)).toBe(2);
    expect(lowerTier(0)).toBe(0);
  });
});

describe('p95', () => {
  it('takes the 95th percentile of the window', () => {
    const xs = Array.from({ length: 100 }, (_, i) => i + 1);
    expect(p95(xs)).toBe(95);
    expect(p95([])).toBe(0);
    expect(p95([7])).toBe(7);
  });
});

describe('Watchdog', () => {
  it('drops a tier only after two full windows over budget', () => {
    const w = new Watchdog(3);
    let now = 0;
    let dropped: number | null = null;
    for (let i = 0; i < 400 && dropped === null; i++) {
      now += 40; // 40 ms per frame = 2.4x the tier 3 budget
      dropped = w.push(40, now);
    }
    expect(dropped).toBe(2);
    expect(now).toBeGreaterThanOrEqual(6000);
  });

  it('never drops while inside the budget', () => {
    const w = new Watchdog(1);
    let now = 0;
    for (let i = 0; i < 1000; i++) {
      now += 16;
      expect(w.push(16, now)).toBeNull();
    }
    expect(w.tier).toBe(1);
  });

  it('forgets the window when the tier is set by hand', () => {
    const w = new Watchdog(2);
    w.push(100, 0);
    w.setTier(1);
    expect(w.tier).toBe(1);
    expect(w.lastP95).toBe(0);
  });
});

describe('anim', () => {
  it('clamps the sprite clock to 8-12 fps', () => {
    expect(clampFps(1)).toBe(8);
    expect(clampFps(60)).toBe(12);
    expect(clampFps(10)).toBe(10);
  });

  it('advances the frame index on its own clock, looping', () => {
    expect(frameIndex(0, 10, 4)).toBe(0);
    expect(frameIndex(99, 10, 4)).toBe(0);
    expect(frameIndex(100, 10, 4)).toBe(1);
    expect(frameIndex(500, 10, 4)).toBe(1);
    expect(frameIndex(400, 10, 4)).toBe(0);
    expect(frameIndex(1000, 10, 4)).toBe(2);
  });

  it('holds the last frame when the clip does not loop', () => {
    expect(frameIndex(10_000, 10, 4, false)).toBe(3);
    expect(clipFinished(400, 10, 4)).toBe(true);
    expect(clipFinished(200, 10, 4)).toBe(false);
  });

  it('shares one accumulator across sprites', () => {
    const c = new AnimClock();
    c.advance(250);
    c.advance(250);
    expect(c.elapsed).toBe(500);
    expect(c.index(0, 8, 4)).toBe(0);
    c.advance(-5);
    expect(c.elapsed).toBe(500);
  });

  it('interpolates positions clamped to [0,1]', () => {
    expect(lerp(0, 10, 0.5)).toBe(5);
    expect(lerp(0, 10, -1)).toBe(0);
    expect(lerp(0, 10, 9)).toBe(10);
  });
});

describe('context state machine', () => {
  it('treats sleep and loss as different states', () => {
    expect(nextState('new', 'init')).toBe('live');
    expect(nextState('live', 'sleep')).toBe('sleeping');
    expect(nextState('sleeping', 'lost')).toBe('sleeping');
    expect(nextState('live', 'lost')).toBe('lost');
    expect(nextState('sleeping', 'wake')).toBe('restoring');
    expect(nextState('restoring', 'restored')).toBe('live');
  });

  it('is a dead end once destroyed', () => {
    expect(nextState('live', 'destroy')).toBe('destroyed');
    expect(nextState('destroyed', 'init')).toBe('destroyed');
    expect(nextState('destroyed', 'wake')).toBe('destroyed');
  });
});

describe('city backend from the 3D calibration', () => {
  it('goes lite over the tier-0 budget or when 3D cannot run', () => {
    expect(cityBackendFromCalibration({ medianMs: 12, backend: 'gl2', contextLostDuring: false })).toBe('3d');
    expect(cityBackendFromCalibration({ medianMs: CITY3D_TIER0_FRAME_MS + 1, backend: 'gl2', contextLostDuring: false })).toBe('lite');
    expect(cityBackendFromCalibration({ medianMs: Infinity, backend: 'none', contextLostDuring: false })).toBe('lite');
    expect(cityBackendFromCalibration({ medianMs: 5, backend: 'gl2', contextLostDuring: true })).toBe('lite');
  });
});
