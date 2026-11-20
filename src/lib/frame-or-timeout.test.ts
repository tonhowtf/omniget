import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { onFrameOrAfter } from "./frame-or-timeout";

describe("onFrameOrAfter", () => {
  let frames: Array<() => void>;

  beforeEach(() => {
    vi.useFakeTimers();
    frames = [];
    vi.stubGlobal("requestAnimationFrame", (cb: () => void) => frames.push(cb));
    vi.stubGlobal("cancelAnimationFrame", (id: number) => {
      frames[id - 1] = () => {};
    });
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("runs on the next frame when the window is being painted", () => {
    const run = vi.fn();
    onFrameOrAfter(run, 250);
    frames[0]();
    vi.advanceTimersByTime(1000);
    expect(run).toHaveBeenCalledTimes(1);
  });

  it("still runs when no frame ever comes, as in a hidden window", () => {
    const run = vi.fn();
    onFrameOrAfter(run, 250);
    vi.advanceTimersByTime(249);
    expect(run).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(run).toHaveBeenCalledTimes(1);
  });

  it("does not run twice when the frame comes after the timeout", () => {
    const run = vi.fn();
    onFrameOrAfter(run, 250);
    vi.advanceTimersByTime(250);
    frames[0]();
    expect(run).toHaveBeenCalledTimes(1);
  });
});
