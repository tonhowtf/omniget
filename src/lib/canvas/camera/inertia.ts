// Own inertia for drag-to-pan (02-motor-do-canvas §3.2): velocity from the last
// 80 ms of samples, decay v·e^(−t/325 ms), stop below 10 px/s. Never added to
// wheel/trackpad scrolling, which already carries the system momentum.

const VELOCITY_WINDOW_MS = 80;
const DECAY_MS = 325;
const STOP_SPEED = 10;

export class VelocityTracker {
  private samples: Array<{ x: number; y: number; t: number }> = [];

  reset(): void {
    this.samples = [];
  }

  push(x: number, y: number, t: number): void {
    this.samples.push({ x, y, t });
    while (this.samples.length > 2 && t - this.samples[0].t > VELOCITY_WINDOW_MS) this.samples.shift();
  }

  /** Velocity in px/s over the window; zero when the pointer rested. */
  velocity(now: number): { vx: number; vy: number } {
    const s = this.samples;
    if (s.length < 2) return { vx: 0, vy: 0 };
    const last = s[s.length - 1];
    if (now - last.t > VELOCITY_WINDOW_MS) return { vx: 0, vy: 0 };
    const first = s[0];
    const dt = (last.t - first.t) / 1000;
    if (dt <= 0) return { vx: 0, vy: 0 };
    return { vx: (last.x - first.x) / dt, vy: (last.y - first.y) / dt };
  }
}

export class Inertia {
  private vx = 0;
  private vy = 0;
  private active = false;

  start(vx: number, vy: number): boolean {
    this.vx = vx;
    this.vy = vy;
    this.active = Math.hypot(vx, vy) >= STOP_SPEED * 4;
    return this.active;
  }

  stop(): void {
    this.active = false;
    this.vx = 0;
    this.vy = 0;
  }

  get running(): boolean {
    return this.active;
  }

  /** Advances by dt ms; returns the screen-pixel delta to pan by. */
  step(dtMs: number): { dx: number; dy: number } {
    if (!this.active) return { dx: 0, dy: 0 };
    const dt = Math.min(64, Math.max(0, dtMs));
    const decay = Math.exp(-dt / DECAY_MS);
    const dx = (this.vx * dt) / 1000;
    const dy = (this.vy * dt) / 1000;
    this.vx *= decay;
    this.vy *= decay;
    if (Math.hypot(this.vx, this.vy) < STOP_SPEED) this.active = false;
    return { dx, dy };
  }
}

/** Critically damped spring step toward `target` (rubber-band settle). */
export function springStep(
  value: number,
  velocity: number,
  target: number,
  dtMs: number,
  omega = 18,
): { value: number; velocity: number } {
  const dt = Math.min(0.064, dtMs / 1000);
  const x = value - target;
  const exp = Math.exp(-omega * dt);
  const nextX = (x + (velocity + omega * x) * dt) * exp;
  const nextV = (velocity - omega * (velocity + omega * x) * dt) * exp;
  return { value: target + nextX, velocity: nextV };
}
