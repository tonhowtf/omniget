export type SpringParams = { stiffness: number; damping: number; mass: number; velocity: number };

export type SpringSample = { t: number; x: number };

export type SpringResult = {
  samples: SpringSample[];
  settleMs: number;
  settled: boolean;
  min: number;
  max: number;
  dampingRatio: number;
};

export const DEFAULT_SPRING: SpringParams = { stiffness: 100, damping: 10, mass: 1, velocity: 0 };

export const SPRING_PRESETS: { id: string; params: SpringParams }[] = [
  { id: "default", params: { stiffness: 100, damping: 10, mass: 1, velocity: 0 } },
  { id: "gentle", params: { stiffness: 120, damping: 14, mass: 1, velocity: 0 } },
  { id: "wobbly", params: { stiffness: 180, damping: 12, mass: 1, velocity: 0 } },
  { id: "stiff", params: { stiffness: 210, damping: 20, mass: 1, velocity: 0 } },
  { id: "snappy", params: { stiffness: 400, damping: 30, mass: 1, velocity: 0 } },
  { id: "slow", params: { stiffness: 280, damping: 60, mass: 1, velocity: 0 } },
];

export const SETTLE_TOLERANCE = 0.001;
const MAX_SECONDS = 30;
const STEP = 1 / 1000;

export function dampingRatio(p: SpringParams): number {
  if (p.stiffness <= 0 || p.mass <= 0) return 0;
  return p.damping / (2 * Math.sqrt(p.stiffness * p.mass));
}

export function simulateSpring(p: SpringParams, sampleEveryMs = 4, tolerance = SETTLE_TOLERANCE): SpringResult {
  const k = Math.max(0, p.stiffness);
  const c = Math.max(0, p.damping);
  const m = Math.max(1e-6, p.mass);
  const accel = (x: number, v: number) => (-k * (x - 1) - c * v) / m;
  let x = 0;
  let v = p.velocity;
  let t = 0;
  let lastOutside = 0;
  let settled = false;
  let min = 0;
  let max = 0;
  const samples: SpringSample[] = [{ t: 0, x: 0 }];
  const every = Math.max(1, Math.round(sampleEveryMs));
  const totalSteps = Math.round(MAX_SECONDS / STEP);
  for (let step = 1; step <= totalSteps; step++) {
    const k1x = v;
    const k1v = accel(x, v);
    const k2x = v + (k1v * STEP) / 2;
    const k2v = accel(x + (k1x * STEP) / 2, k2x);
    const k3x = v + (k2v * STEP) / 2;
    const k3v = accel(x + (k2x * STEP) / 2, k3x);
    const k4x = v + k3v * STEP;
    const k4v = accel(x + k3x * STEP, k4x);
    x += (STEP / 6) * (k1x + 2 * k2x + 2 * k3x + k4x);
    v += (STEP / 6) * (k1v + 2 * k2v + 2 * k3v + k4v);
    t = step * STEP;
    if (x < min) min = x;
    if (x > max) max = x;
    if (Math.abs(x - 1) > tolerance) lastOutside = t;
    if (step % every === 0) samples.push({ t: t * 1000, x });
    const energy = 0.5 * k * (x - 1) ** 2 + 0.5 * m * v * v;
    const reach = k > 0 ? Math.sqrt((2 * energy) / k) : Infinity;
    if (reach <= tolerance) {
      settled = true;
      break;
    }
  }
  const settleMs = settled ? lastOutside * 1000 : MAX_SECONDS * 1000;
  const cut = Math.max(settleMs, every);
  const trimmed = samples.filter((s) => s.t <= cut);
  if (trimmed[trimmed.length - 1]?.t !== cut) trimmed.push({ t: cut, x: settled ? 1 : x });
  return { samples: trimmed, settleMs, settled, min, max, dampingRatio: dampingRatio(p) };
}

export function springAt(result: SpringResult, ms: number): number {
  const s = result.samples;
  if (ms <= 0) return s[0].x;
  const last = s[s.length - 1];
  if (ms >= last.t) return result.settled ? 1 : last.x;
  let lo = 0;
  let hi = s.length - 1;
  while (hi - lo > 1) {
    const mid = (lo + hi) >> 1;
    if (s[mid].t <= ms) lo = mid;
    else hi = mid;
  }
  const a = s[lo];
  const b = s[hi];
  const f = b.t === a.t ? 0 : (ms - a.t) / (b.t - a.t);
  return a.x + (b.x - a.x) * f;
}

export type SvelteSpring = { stiffness: number; damping: number; clamped: boolean };

export function toSvelteSpring(p: SpringParams, fps = 60): SvelteSpring {
  const m = Math.max(1e-6, p.mass);
  const rawStiffness = p.stiffness / m / (fps * fps);
  const rawDamping = p.damping / m / fps;
  const stiffness = Math.min(1, Math.max(0, rawStiffness));
  const damping = Math.min(1, Math.max(0, rawDamping));
  return { stiffness, damping, clamped: stiffness !== rawStiffness || damping !== rawDamping };
}

export type SwiftSpring = { response: number; dampingFraction: number };

export function toSwiftUI(p: SpringParams): SwiftSpring {
  const m = Math.max(1e-6, p.mass);
  const k = Math.max(1e-6, p.stiffness);
  return { response: (2 * Math.PI) / Math.sqrt(k / m), dampingFraction: p.damping / (2 * Math.sqrt(k * m)) };
}

export function fromSwiftUI(s: SwiftSpring, mass = 1): SpringParams {
  const omega = (2 * Math.PI) / Math.max(1e-6, s.response);
  const stiffness = omega * omega * mass;
  const damping = s.dampingFraction * 2 * Math.sqrt(stiffness * mass);
  return { stiffness, damping, mass, velocity: 0 };
}
