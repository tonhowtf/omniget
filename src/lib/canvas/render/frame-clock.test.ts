import { describe, expect, it } from 'vitest';
import { FrameClock, type FrameHost } from './frame-clock';

function fakeHost() {
  let nextId = 1;
  const frames = new Map<number, (t: number) => void>();
  const timers = new Map<number, { cb: () => void; ms: number }>();
  const host: FrameHost & { isHidden: boolean } = {
    isHidden: false,
    hidden: () => host.isHidden,
    requestAnimationFrame: (cb) => {
      const id = nextId++;
      frames.set(id, cb);
      return id;
    },
    cancelAnimationFrame: (id) => void frames.delete(id),
    setTimeout: (cb, ms) => {
      const id = nextId++;
      timers.set(id, { cb, ms });
      return id;
    },
    clearTimeout: (id) => void timers.delete(id as number),
    now: () => 42,
  };
  const runFrames = () => {
    const list = [...frames.values()];
    frames.clear();
    for (const cb of list) cb(42);
  };
  const runTimers = () => {
    const list = [...timers.values()];
    timers.clear();
    for (const t of list) t.cb();
  };
  return { host, frames, timers, runFrames, runTimers };
}

describe('FrameClock', () => {
  it('draws on the next animation frame while the page is visible', () => {
    const f = fakeHost();
    let ticks = 0;
    const clock = new FrameClock(() => ticks++, f.host);
    clock.request();
    clock.request();
    expect(f.frames.size).toBe(1);
    f.runFrames();
    expect(ticks).toBe(1);
  });

  it('still draws when a change arrives while the window is hidden', () => {
    const f = fakeHost();
    let ticks = 0;
    const clock = new FrameClock(() => ticks++, f.host);
    clock.request();
    f.host.isHidden = true;
    clock.request();
    expect(f.frames.size).toBe(0);
    f.runTimers();
    expect(ticks).toBe(1);
    clock.request();
    f.runTimers();
    expect(ticks).toBe(2);
  });

  it('does not push a pending hidden frame back on every change', () => {
    const f = fakeHost();
    const clock = new FrameClock(() => {}, f.host);
    f.host.isHidden = true;
    clock.request();
    const first = [...f.timers.keys()];
    clock.request();
    expect([...f.timers.keys()]).toEqual(first);
  });

  it('a later frame waits its delay and yields to an earlier request', () => {
    const f = fakeHost();
    let ticks = 0;
    const clock = new FrameClock(() => ticks++, f.host);
    clock.later(500);
    expect([...f.timers.values()].map((t) => t.ms)).toEqual([500]);
    clock.request();
    expect(f.timers.size).toBe(0);
    f.runFrames();
    expect(ticks).toBe(1);
  });

  it('stops for good', () => {
    const f = fakeHost();
    let ticks = 0;
    const clock = new FrameClock(() => ticks++, f.host);
    clock.request();
    clock.stop();
    clock.request();
    clock.later(10);
    f.runFrames();
    f.runTimers();
    expect(ticks).toBe(0);
  });
});
