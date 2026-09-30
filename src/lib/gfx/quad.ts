// Interleaved quad writer shared by every sprite-style pass (world batcher,
// calibration scene, canvas sprite pass). Pure: typed arrays only.

/** x, y, u, v, r, g, b, a */
export const FLOATS_PER_VERTEX = 8;
export const VERTS_PER_QUAD = 4;
export const FLOATS_PER_QUAD = FLOATS_PER_VERTEX * VERTS_PER_QUAD;
export const INDICES_PER_QUAD = 6;

export interface Quad {
  page: number;
  /** Top-left in device pixels. */
  sx: number;
  sy: number;
  w: number;
  h: number;
  u0: number;
  v0: number;
  u1: number;
  v1: number;
  flip?: boolean;
  tint?: number;
  alpha?: number;
}

export function unpackTint(tint: number): [number, number, number] {
  return [((tint >> 16) & 0xff) / 255, ((tint >> 8) & 0xff) / 255, (tint & 0xff) / 255];
}

/** Writes one quad (4 vertices) at `floatOffset`; returns the next float offset. */
export function writeQuad(out: Float32Array, floatOffset: number, q: Quad): number {
  const [r, g, b] = unpackTint(q.tint ?? 0xffffff);
  const a = q.alpha ?? 1;
  // Mirroring swaps the U coordinates; the art is never duplicated.
  const ul = q.flip ? q.u1 : q.u0;
  const ur = q.flip ? q.u0 : q.u1;
  const x0 = q.sx;
  const y0 = q.sy;
  const x1 = q.sx + q.w;
  const y1 = q.sy + q.h;
  let o = floatOffset;
  const put = (x: number, y: number, u: number, v: number) => {
    out[o] = x;
    out[o + 1] = y;
    out[o + 2] = u;
    out[o + 3] = v;
    out[o + 4] = r;
    out[o + 5] = g;
    out[o + 6] = b;
    out[o + 7] = a;
    o += FLOATS_PER_VERTEX;
  };
  put(x0, y0, ul, q.v0);
  put(x1, y0, ur, q.v0);
  put(x1, y1, ur, q.v1);
  put(x0, y1, ul, q.v1);
  return o;
}

/** Shared static index buffer: 0,1,2, 0,2,3 per quad. */
export function buildIndices(quads: number): Uint16Array {
  const idx = new Uint16Array(quads * INDICES_PER_QUAD);
  for (let i = 0; i < quads; i++) {
    const v = i * VERTS_PER_QUAD;
    const o = i * INDICES_PER_QUAD;
    idx[o] = v;
    idx[o + 1] = v + 1;
    idx[o + 2] = v + 2;
    idx[o + 3] = v;
    idx[o + 4] = v + 2;
    idx[o + 5] = v + 3;
  }
  return idx;
}
