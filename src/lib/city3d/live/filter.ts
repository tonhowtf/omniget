// One Euro filter (Casiez, Roussel and Vogel, CHI 2012): low jitter when the
// signal is slow, low lag when it moves fast.

import { qAlign, qNorm, type Quat } from './math';

export interface OneEuroParams {
  /** Hz. Lower means smoother at rest. */
  minCutoff: number;
  /** Speed coefficient. Higher means less lag on fast motion. */
  beta: number;
  /** Hz for the derivative. */
  dCutoff: number;
}

export const FACE_FILTER: OneEuroParams = { minCutoff: 1.5, beta: 0.6, dCutoff: 1 };
export const BODY_FILTER: OneEuroParams = { minCutoff: 1.0, beta: 0.4, dCutoff: 1 };
export const WEIGHT_FILTER: OneEuroParams = { minCutoff: 2.5, beta: 1.5, dCutoff: 1 };

const alpha = (cutoff: number, dt: number): number => {
  const tau = 1 / (2 * Math.PI * cutoff);
  return 1 / (1 + tau / dt);
};

export class OneEuro {
  private x: number | null = null;
  private dx = 0;
  private t = 0;

  constructor(private p: OneEuroParams) {}

  reset(): void {
    this.x = null;
    this.dx = 0;
  }

  /** `tSec` is a monotonic time in seconds. */
  filter(value: number, tSec: number): number {
    if (this.x === null) {
      this.x = value;
      this.t = tSec;
      return value;
    }
    const dt = Math.max(1e-3, tSec - this.t);
    this.t = tSec;
    const dxRaw = (value - this.x) / dt;
    this.dx += alpha(this.p.dCutoff, dt) * (dxRaw - this.dx);
    const cutoff = this.p.minCutoff + this.p.beta * Math.abs(this.dx);
    this.x += alpha(cutoff, dt) * (value - this.x);
    return this.x;
  }
}

/** One Euro per component of a quaternion, kept in one hemisphere. */
export class OneEuroQuat {
  private f: OneEuro[];
  private last: Quat | null = null;

  constructor(p: OneEuroParams) {
    this.f = [new OneEuro(p), new OneEuro(p), new OneEuro(p), new OneEuro(p)];
  }

  reset(): void {
    this.f.forEach((f) => f.reset());
    this.last = null;
  }

  filter(q: Quat, tSec: number): Quat {
    const a = this.last ? qAlign(q, this.last) : q;
    const out = qNorm([
      this.f[0].filter(a[0], tSec),
      this.f[1].filter(a[1], tSec),
      this.f[2].filter(a[2], tSec),
      this.f[3].filter(a[3], tSec),
    ]);
    this.last = out;
    return out;
  }
}

/** A keyed bank of scalar filters, created on first use. */
export class OneEuroBank {
  private m = new Map<string, OneEuro>();
  constructor(private p: OneEuroParams) {}
  filter(key: string, value: number, tSec: number): number {
    let f = this.m.get(key);
    if (!f) {
      f = new OneEuro(this.p);
      this.m.set(key, f);
    }
    return f.filter(value, tSec);
  }
  reset(): void {
    this.m.clear();
  }
}

export class OneEuroQuatBank {
  private m = new Map<string, OneEuroQuat>();
  constructor(private p: OneEuroParams) {}
  filter(key: string, q: Quat, tSec: number): Quat {
    let f = this.m.get(key);
    if (!f) {
      f = new OneEuroQuat(this.p);
      this.m.set(key, f);
    }
    return f.filter(q, tSec);
  }
  reset(): void {
    this.m.clear();
  }
}
