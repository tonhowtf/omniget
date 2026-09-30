// Own minimal rough.js (MIT idea, 02-motor-do-canvas §6.3): double outline with
// seeded jitter and a slight bow on straight lines. The seed lives in the
// drawing, so a shape never "boils" between frames or sessions.

/** Mulberry32: tiny seeded PRNG, returns 0..1. */
export function prng(seed: number): () => number {
  let s = seed >>> 0 || 1;
  return () => {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export function randomSeed(): number {
  return Math.floor(Math.random() * 2 ** 31);
}

/** One wobbly line from a to b as a polyline (flat). */
export function roughLine(
  x0: number,
  y0: number,
  x1: number,
  y1: number,
  rnd: () => number,
  roughness = 1,
): number[] {
  const len = Math.hypot(x1 - x0, y1 - y0);
  const jit = Math.min(len * 0.04, 3) * roughness;
  const j = () => (rnd() - 0.5) * 2 * jit;
  // Bow: the midpoint drifts sideways a little, like a hand-drawn stroke.
  const nx = -(y1 - y0) / (len || 1);
  const ny = (x1 - x0) / (len || 1);
  const bow = (rnd() - 0.5) * Math.min(len * 0.03, 4) * roughness;
  const steps = Math.max(2, Math.min(12, Math.round(len / 24)));
  const sx = x0 + j();
  const sy = y0 + j();
  const ex = x1 + j();
  const ey = y1 + j();
  const out: number[] = [];
  for (let i = 0; i <= steps; i++) {
    const t = i / steps;
    const b = Math.sin(t * Math.PI) * bow;
    out.push(sx + (ex - sx) * t + nx * b, sy + (ey - sy) * t + ny * b);
  }
  return out;
}

/**
 * Sketchy outline of a closed polygon: every edge drawn twice with different
 * jitter. Returns a list of polylines.
 */
export function roughPolygon(poly: ArrayLike<number>, seed: number, roughness = 1, passes = 2): number[][] {
  const rnd = prng(seed);
  const n = Math.floor(poly.length / 2);
  const out: number[][] = [];
  // Smooth shapes (many points, e.g. ellipses) are drawn as whole jittered loops.
  if (n > 12) {
    for (let p = 0; p < passes; p++) {
      const loop: number[] = [];
      const phase = rnd() * Math.PI * 2;
      const amp = 1.6 * roughness;
      const overshoot = Math.floor(n * (0.05 + rnd() * 0.08));
      for (let i = 0; i <= n + overshoot; i++) {
        const k = i % n;
        const w = Math.sin(phase + i * 0.35) * amp + (rnd() - 0.5) * amp;
        const px = poly[k * 2];
        const py = poly[k * 2 + 1];
        const qx = poly[((k + 1) % n) * 2];
        const qy = poly[((k + 1) % n) * 2 + 1];
        const len = Math.hypot(qx - px, qy - py) || 1;
        loop.push(px + (-(qy - py) / len) * w, py + ((qx - px) / len) * w);
      }
      out.push(loop);
    }
    return out;
  }
  for (let p = 0; p < passes; p++) {
    for (let i = 0; i < n; i++) {
      const j = (i + 1) % n;
      out.push(roughLine(poly[i * 2], poly[i * 2 + 1], poly[j * 2], poly[j * 2 + 1], rnd, roughness * (p === 0 ? 1 : 1.4)));
    }
  }
  return out;
}

/** Sketchy open polyline (arrows, lines): each segment wobbled, drawn twice. */
export function roughPolyline(pts: ArrayLike<number>, seed: number, roughness = 1): number[][] {
  const rnd = prng(seed);
  const out: number[][] = [];
  const n = Math.floor(pts.length / 2);
  for (let p = 0; p < 2; p++) {
    for (let i = 0; i + 1 < n; i++) {
      out.push(roughLine(pts[i * 2], pts[i * 2 + 1], pts[i * 2 + 2], pts[i * 2 + 3], rnd, roughness));
    }
  }
  return out;
}
