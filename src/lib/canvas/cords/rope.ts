// Optional "rope" cord style (Maestri): a Verlet chain of 12 segments with light
// gravity, simulated only while an end moved, stopped once it settles.

import type { Pt } from '../spatial/geometry';

const ROPE_SEGMENTS = 12;
const GRAVITY = 900; // world units / s²
const ITERATIONS = 8;
const SETTLE_EPS = 0.05;

export class Rope {
  private px: Float64Array;
  private py: Float64Array;
  private ox: Float64Array;
  private oy: Float64Array;
  private segLen = 0;
  settled = false;

  constructor(a: Pt, b: Pt, readonly slack = 1.12) {
    const n = ROPE_SEGMENTS + 1;
    this.px = new Float64Array(n);
    this.py = new Float64Array(n);
    this.ox = new Float64Array(n);
    this.oy = new Float64Array(n);
    for (let i = 0; i < n; i++) {
      const t = i / ROPE_SEGMENTS;
      this.px[i] = this.ox[i] = a.x + (b.x - a.x) * t;
      this.py[i] = this.oy[i] = a.y + (b.y - a.y) * t;
    }
    this.setEnds(a, b);
  }

  /** Moves the pinned ends; wakes the simulation when they changed. */
  setEnds(a: Pt, b: Pt): void {
    const n = ROPE_SEGMENTS;
    if (this.px[0] !== a.x || this.py[0] !== a.y || this.px[n] !== b.x || this.py[n] !== b.y) {
      this.settled = false;
    }
    this.px[0] = this.ox[0] = a.x;
    this.py[0] = this.oy[0] = a.y;
    this.px[n] = this.ox[n] = b.x;
    this.py[n] = this.oy[n] = b.y;
    this.segLen = (Math.hypot(b.x - a.x, b.y - a.y) * this.slack) / ROPE_SEGMENTS;
  }

  /** One simulation step of dt ms. Returns true while still moving. */
  step(dtMs: number): boolean {
    if (this.settled) return false;
    const dt = Math.min(0.033, dtMs / 1000);
    const n = ROPE_SEGMENTS;
    let motion = 0;
    for (let i = 1; i < n; i++) {
      const vx = (this.px[i] - this.ox[i]) * 0.98;
      const vy = (this.py[i] - this.oy[i]) * 0.98;
      this.ox[i] = this.px[i];
      this.oy[i] = this.py[i];
      this.px[i] += vx;
      this.py[i] += vy + GRAVITY * dt * dt;
      motion += Math.abs(vx) + Math.abs(vy);
    }
    for (let k = 0; k < ITERATIONS; k++) {
      for (let i = 0; i < n; i++) {
        const dx = this.px[i + 1] - this.px[i];
        const dy = this.py[i + 1] - this.py[i];
        const d = Math.hypot(dx, dy) || 1e-6;
        const diff = (d - this.segLen) / d;
        const pinA = i === 0;
        const pinB = i + 1 === n;
        if (pinA && pinB) continue;
        const wa = pinA ? 0 : pinB ? 1 : 0.5;
        const wb = pinB ? 0 : pinA ? 1 : 0.5;
        this.px[i] += dx * diff * wa;
        this.py[i] += dy * diff * wa;
        this.px[i + 1] -= dx * diff * wb;
        this.py[i + 1] -= dy * diff * wb;
      }
    }
    if (motion / n < SETTLE_EPS) this.settled = true;
    return !this.settled;
  }

  /** Flat polyline of the rope. */
  points(): number[] {
    const out: number[] = [];
    for (let i = 0; i <= ROPE_SEGMENTS; i++) out.push(this.px[i], this.py[i]);
    return out;
  }
}
