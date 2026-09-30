// Sprite animation clock. Sprites advance on their own 8-12 fps clock; only the
// position is interpolated per frame (decision 5 of the orchestration §4.4).

export const MIN_FPS = 8;
export const MAX_FPS = 12;

export function clampFps(fps: number): number {
  return Math.min(MAX_FPS, Math.max(MIN_FPS, Math.round(fps)));
}

/** Frame index of a clip at `elapsedMs`. Non-looping clips hold the last frame. */
export function frameIndex(elapsedMs: number, fps: number, count: number, loop = true): number {
  if (count <= 0) return 0;
  if (!Number.isFinite(elapsedMs) || elapsedMs <= 0) return 0;
  const step = Math.floor((elapsedMs * fps) / 1000);
  if (loop) return ((step % count) + count) % count;
  return Math.min(step, count - 1);
}

/** True when the clip has played through at least once. */
export function clipFinished(elapsedMs: number, fps: number, count: number): boolean {
  return Math.floor((elapsedMs * fps) / 1000) >= count;
}

/**
 * Shared clock for every animated sprite: one accumulator, advanced by dt, so a
 * frame never calls performance.now() per sprite.
 */
export class AnimClock {
  private ms = 0;
  advance(dtMs: number): void {
    if (Number.isFinite(dtMs) && dtMs > 0) this.ms += dtMs;
  }
  get elapsed(): number {
    return this.ms;
  }
  /** Index for a clip whose own start time is `startedAtMs` of this clock. */
  index(startedAtMs: number, fps: number, count: number, loop = true): number {
    return frameIndex(this.ms - startedAtMs, fps, count, loop);
  }
  reset(): void {
    this.ms = 0;
  }
}

/** Linear interpolation of a simulation position for the current frame. */
export function lerp(a: number, b: number, t: number): number {
  const c = t < 0 ? 0 : t > 1 ? 1 : t;
  return a + (b - a) * c;
}
