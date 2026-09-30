import { describe, expect, it } from 'vitest';
import { decideLive, inside, type Candidate, type PolicyInput } from './policy';

const container = { x: 0, y: 0, w: 1000, h: 800 };

function cand(id: string, over: Partial<Candidate> = {}): Candidate {
  return { id, rect: { x: 10, y: 10, w: 400, h: 300 }, stableMs: 1000, covered: false, priority: 1, ...over };
}

function input(over: Partial<PolicyInput> = {}): PolicyInput {
  return { cameraStill: true, zoom: 1, container, budget: 4, pageVisible: true, candidates: [cand('a')], ...over };
}

describe('portal live policy', () => {
  it('goes live only when everything holds', () => {
    expect(decideLive(input())).toEqual(new Set(['a']));
    expect(decideLive(input({ cameraStill: false })).size).toBe(0);
    expect(decideLive(input({ zoom: 0.4 })).size).toBe(0);
    expect(decideLive(input({ zoom: 2.1 })).size).toBe(0);
    expect(decideLive(input({ pageVisible: false })).size).toBe(0);
    expect(decideLive(input({ candidates: [cand('a', { covered: true })] })).size).toBe(0);
    expect(decideLive(input({ candidates: [cand('a', { stableMs: 20 })] })).size).toBe(0);
    expect(decideLive(input({ candidates: [cand('a', { rect: { x: 900, y: 10, w: 400, h: 300 } })] })).size).toBe(0);
  });

  it('keeps the budget, highest priority first', () => {
    const cs = ['a', 'b', 'c', 'd', 'e'].map((id, i) => cand(id, { priority: i }));
    expect(decideLive(input({ candidates: cs, budget: 2 }))).toEqual(new Set(['e', 'd']));
  });

  it('inside tolerates half a pixel', () => {
    expect(inside({ x: -0.4, y: 0, w: 1000.6, h: 10 }, container)).toBe(true);
    expect(inside({ x: -2, y: 0, w: 10, h: 10 }, container)).toBe(false);
  });
});
