import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { agentCap, createLoop, getRafTicks, resetRafTicks } from './index';
import type { FrameStats, Renderer, Scene, Tier } from './types';
import { makeCamera } from './camera';

/** Minimal renderer double: counts frames, records sleep/wake. */
function fakeRenderer(): Renderer & { frames: number; slept: number; woke: number } {
  let tier: Tier = 3;
  const r = {
    frames: 0,
    slept: 0,
    woke: 0,
    get tier() {
      return tier;
    },
    setTier(next: Tier) {
      tier = next;
    },
    async init() {
      return {
        backend: 'gl2' as const,
        tier,
        maxTextureSize: 4096,
        renderTexture: true,
        timerQuery: false,
        dpr: 1,
      };
    },
    loadAtlas: () => ({ id: 1, frames: new Set<string>() }),
    frame(): FrameStats {
      r.frames++;
      return { cpuMs: 1, drawCalls: 2, sprites: 3, chunksBaked: 0 };
    },
    bakeChunk() {},
    invalidateChunk() {},
    setDynamicPage() {},
    memory: () => ({ textures: 0, bytes: 0, dynamicPage: 0 }),
    text: () => ({ id: 1, w: 10, h: 10 }),
    resize() {},
    sleep() {
      r.slept++;
    },
    async wake() {
      r.woke++;
    },
    destroy() {},
  };
  return r as Renderer & { frames: number; slept: number; woke: number };
}

/** Manual rAF pump: nothing runs unless the test asks for it. */
class RafPump {
  private queue = new Map<number, FrameRequestCallback>();
  private next = 1;
  now = 0;
  install(): void {
    globalThis.requestAnimationFrame = ((cb: FrameRequestCallback) => {
      const id = this.next++;
      this.queue.set(id, cb);
      return id;
    }) as typeof requestAnimationFrame;
    globalThis.cancelAnimationFrame = ((id: number) => {
      this.queue.delete(id);
    }) as typeof cancelAnimationFrame;
  }
  /** Runs every callback queued right now. Returns how many ran. */
  flush(stepMs = 16): number {
    const pending = Array.from(this.queue.entries());
    this.queue.clear();
    this.now += stepMs;
    for (const [, cb] of pending) cb(this.now);
    return pending.length;
  }
  get pending(): number {
    return this.queue.size;
  }
}

const scene = (): Scene => ({
  camera: makeCamera(800, 600),
  chunksVisible: [],
  sprites: [],
  texts: [],
});

describe('createLoop', () => {
  let pump: RafPump;
  const realRaf = globalThis.requestAnimationFrame;
  const realCancel = globalThis.cancelAnimationFrame;

  beforeEach(() => {
    pump = new RafPump();
    pump.install();
    resetRafTicks();
  });

  afterEach(() => {
    globalThis.requestAnimationFrame = realRaf;
    globalThis.cancelAnimationFrame = realCancel;
  });

  it('runs one frame per rAF and re-arms itself', () => {
    const r = fakeRenderer();
    const loop = createLoop(r, scene);
    loop.start();
    expect(pump.pending).toBe(1);
    pump.flush();
    pump.flush();
    pump.flush();
    expect(r.frames).toBe(3);
    expect(getRafTicks()).toBe(3);
    expect(loop.running).toBe(true);
    loop.stop();
  });

  it('leaves zero rAF callbacks after stop() + sleep()', () => {
    const r = fakeRenderer();
    const loop = createLoop(r, scene);
    loop.start();
    pump.flush();
    pump.flush();
    const framesBefore = r.frames;
    loop.stop();
    r.sleep();
    resetRafTicks();
    // Nothing is queued, and pumping the (empty) queue runs nothing.
    expect(pump.pending).toBe(0);
    for (let i = 0; i < 60; i++) pump.flush();
    expect(getRafTicks()).toBe(0);
    expect(r.frames).toBe(framesBefore);
    expect(r.slept).toBe(1);
    expect(loop.running).toBe(false);
  });

  it('a frame already queued when stop() lands does not render', () => {
    const r = fakeRenderer();
    const loop = createLoop(r, scene);
    loop.start();
    loop.stop();
    pump.flush();
    expect(r.frames).toBe(0);
  });

  it('start() twice does not double the rAF chain', () => {
    const r = fakeRenderer();
    const loop = createLoop(r, scene);
    loop.start();
    loop.start();
    expect(pump.pending).toBe(1);
    pump.flush();
    expect(r.frames).toBe(1);
    loop.stop();
  });

  it('drops the tier through the watchdog and tells the caller', () => {
    const r = fakeRenderer();
    const seen: number[] = [];
    const loop = createLoop(r, scene, { onTierChanged: (t) => seen.push(t) });
    loop.start();
    for (let i = 0; i < 400 && seen.length === 0; i++) pump.flush(40);
    loop.stop();
    expect(seen[0]).toBe(2);
    expect(r.tier).toBe(2);
  });
});

describe('agentCap', () => {
  it('follows the tier table', () => {
    expect(agentCap(0)).toBe(4);
    expect(agentCap(1)).toBe(8);
    expect(agentCap(2)).toBe(24);
    expect(agentCap(3)).toBe(64);
  });
});
