interface StrokeOptions {
  size: number;
  thinning?: number;
  streamline?: number;
  taperStart?: number;
  taperEnd?: number;
  pressures?: ArrayLike<number>;
}

const RATE_OF_PRESSURE_CHANGE = 0.275;
const CAP_SEGMENTS = 8;

interface Sample {
  x: number;
  y: number;
  r: number;
}

function strokeSamples(pts: ArrayLike<number>, opts: StrokeOptions): Sample[] {
  const n = Math.floor(pts.length / 2);
  if (n === 0) return [];
  const size = Math.max(0.5, opts.size);
  const thinning = opts.thinning ?? 0.5;
  const t = 1 - (opts.streamline ?? 0.4) * 0.85;
  const out: Sample[] = [];
  let px = pts[0];
  let py = pts[1];
  let pressure = opts.pressures ? (opts.pressures[0] ?? 0.5) : 0.25;
  let running = 0;
  const lengths: number[] = [];
  for (let i = 0; i < n; i++) {
    const x = i === 0 ? pts[0] : px + (pts[i * 2] - px) * t;
    const y = i === 0 ? pts[1] : py + (pts[i * 2 + 1] - py) * t;
    const d = i === 0 ? 0 : Math.hypot(x - px, y - py);
    if (i > 0 && d < 0.01 && i < n - 1) continue;
    running += d;
    if (opts.pressures) {
      pressure = opts.pressures[i] ?? pressure;
    } else {
      const sp = Math.min(1, d / size);
      const rp = Math.min(1, 1 - sp);
      pressure = Math.min(1, pressure + (rp - pressure) * (sp * RATE_OF_PRESSURE_CHANGE));
    }
    const r = size * Math.max(0.05, 0.5 - thinning * (0.5 - pressure));
    out.push({ x, y, r });
    lengths.push(running);
    px = x;
    py = y;
  }
  const total = running;
  const ts = opts.taperStart ?? 0;
  const te = opts.taperEnd ?? 0;
  if (ts > 0 || te > 0) {
    for (let i = 0; i < out.length; i++) {
      let k = 1;
      if (ts > 0) k = Math.min(k, lengths[i] / ts);
      if (te > 0) k = Math.min(k, (total - lengths[i]) / te);
      out[i].r *= Math.max(0.08, Math.min(1, k));
    }
  }
  return out;
}

function rails(s: Sample[]): { left: number[]; right: number[] } {
  const left: number[] = [];
  const right: number[] = [];
  for (let i = 0; i < s.length; i++) {
    const a = s[Math.max(0, i - 1)];
    const b = s[Math.min(s.length - 1, i + 1)];
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    const len = Math.hypot(dx, dy) || 1;
    dx /= len;
    dy /= len;
    const nx = -dy * s[i].r;
    const ny = dx * s[i].r;
    left.push(s[i].x + nx, s[i].y + ny);
    right.push(s[i].x - nx, s[i].y - ny);
  }
  return { left, right };
}

function pushFan(out: number[], cx: number, cy: number, r: number, a0: number, a1: number, segs: number): void {
  for (let k = 0; k < segs; k++) {
    const t0 = a0 + ((a1 - a0) * k) / segs;
    const t1 = a0 + ((a1 - a0) * (k + 1)) / segs;
    out.push(cx, cy, cx + Math.cos(t0) * r, cy + Math.sin(t0) * r, cx + Math.cos(t1) * r, cy + Math.sin(t1) * r);
  }
}

export function freehandTriangles(pts: ArrayLike<number>, opts: StrokeOptions): number[] {
  const s = strokeSamples(pts, opts);
  const out: number[] = [];
  if (s.length === 0) return out;
  if (s.length === 1) {
    pushFan(out, s[0].x, s[0].y, s[0].r, 0, Math.PI * 2, CAP_SEGMENTS * 2);
    return out;
  }
  const { left, right } = rails(s);
  for (let i = 0; i + 1 < s.length; i++) {
    const l0x = left[i * 2];
    const l0y = left[i * 2 + 1];
    const l1x = left[i * 2 + 2];
    const l1y = left[i * 2 + 3];
    const r0x = right[i * 2];
    const r0y = right[i * 2 + 1];
    const r1x = right[i * 2 + 2];
    const r1y = right[i * 2 + 3];
    out.push(l0x, l0y, r0x, r0y, l1x, l1y, l1x, l1y, r0x, r0y, r1x, r1y);
  }
  const a = s[0];
  const b = s[1];
  const startAng = Math.atan2(a.y - b.y, a.x - b.x);
  pushFan(out, a.x, a.y, a.r, startAng - Math.PI / 2, startAng + Math.PI / 2, CAP_SEGMENTS);
  const y = s[s.length - 1];
  const z = s[s.length - 2];
  const endAng = Math.atan2(y.y - z.y, y.x - z.x);
  pushFan(out, y.x, y.y, y.r, endAng - Math.PI / 2, endAng + Math.PI / 2, CAP_SEGMENTS);
  return out;
}

export function polylineTriangles(pts: ArrayLike<number>, width: number, closed = false, capSegs = 4): number[] {
  const out: number[] = [];
  const n = Math.floor(pts.length / 2);
  if (n < 2) return out;
  const r = width / 2;
  const count = closed ? n : n - 1;
  for (let i = 0; i < count; i++) {
    const j = (i + 1) % n;
    const ax = pts[i * 2];
    const ay = pts[i * 2 + 1];
    const bx = pts[j * 2];
    const by = pts[j * 2 + 1];
    const dx = bx - ax;
    const dy = by - ay;
    const len = Math.hypot(dx, dy);
    if (len < 1e-6) continue;
    const nx = (-dy / len) * r;
    const ny = (dx / len) * r;
    out.push(ax + nx, ay + ny, ax - nx, ay - ny, bx + nx, by + ny, bx + nx, by + ny, ax - nx, ay - ny, bx - nx, by - ny);
  }
  if (capSegs > 0 && r > 0.2) {
    for (let i = 0; i < n; i++) {
      if (!closed && i > 0 && i < n - 1 && r < 1) continue;
      pushFan(out, pts[i * 2], pts[i * 2 + 1], r, 0, Math.PI * 2, capSegs * 2);
    }
  }
  return out;
}
