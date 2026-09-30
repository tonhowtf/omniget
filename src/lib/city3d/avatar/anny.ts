// The baked Anny body (static/city3d/anny, see src-tauri/omniget-avatar/tools/
// bake_anny.py): loads the binaries and builds a body for a phenotype.
//
// x = mean + basis^T (table . c), where c are the exact Anny phenotype blend
// coefficients (port of Anny._get_phenotype_blendshape_coefficients) and
// x = [V*3 vertex coords, 52*3 joint heads], VRM T-pose, meters, Y up, +Z front.

export type LodTag = '1229' | '369';

interface Section { offset: number; length: number; dtype: string; shape: number[] }
interface LodManifest {
  file: string; bytes: number; sha256: string; vertices: number; triangles: number; k: number;
  sections: Record<string, Section>;
  expressions?: string[]; face_targets?: string[];
}
export interface AnnyManifest {
  schema: string;
  bones: { name: string; parent: number }[];
  parts: string[];
  phenotype: {
    labels: string[];
    variations: [string, string][];
    anchors: Record<string, number[]>;
    fixed: { cupsize: number; firmness: number; race: number };
  };
  lods: Record<LodTag, LodManifest>;
}

export interface SparseSet {
  names: string[];
  offset: Uint32Array;
  count: Uint32Array;
  index: Uint16Array;
  delta: Float32Array;
}

export interface AnnyLod {
  tag: LodTag; V: number; k: number; D: number;
  indices: Uint16Array;
  joints: Uint8Array;
  weights: Float32Array;
  part: Uint8Array;
  along: Float32Array;
  mean: Float32Array;
  basis: Float32Array;
  table: Float32Array;
  mask: Uint8Array;
  S: number;
  expr?: SparseSet;
  local?: SparseSet;
}

export interface AnnyBase {
  manifest: AnnyManifest;
  bones: { name: string; parent: number }[];
  lods: Record<LodTag, AnnyLod>;
}

export const BONE_COUNT = 52;

export function halfToFloat(h: number): number {
  const s = h & 0x8000 ? -1 : 1;
  const e = (h >> 10) & 0x1f;
  const f = h & 0x3ff;
  if (e === 0) return s * f * 2 ** -24;
  if (e === 31) return f ? NaN : s * Infinity;
  return s * (1 + f / 1024) * 2 ** (e - 15);
}

function view(buf: ArrayBuffer, s: Section): ArrayLike<number> {
  switch (s.dtype) {
    case 'uint8': return new Uint8Array(buf, s.offset, s.length);
    case 'uint16': return new Uint16Array(buf, s.offset, s.length / 2);
    case 'uint32': return new Uint32Array(buf, s.offset, s.length / 4);
    case 'float32': return new Float32Array(buf, s.offset, s.length / 4);
    case 'float16': {
      const raw = new Uint16Array(buf, s.offset, s.length / 2);
      const out = new Float32Array(raw.length);
      for (let i = 0; i < raw.length; i++) out[i] = halfToFloat(raw[i]);
      return out;
    }
    default: throw new Error(`anny: unknown dtype ${s.dtype}`);
  }
}

function sparse(buf: ArrayBuffer, m: LodManifest, name: string, names: string[]): SparseSet | undefined {
  const t = m.sections[`${name}_table`];
  if (!t) return undefined;
  const table = view(buf, t) as Uint32Array;
  const n = table.length / 2;
  const offset = new Uint32Array(n);
  const count = new Uint32Array(n);
  for (let i = 0; i < n; i++) { offset[i] = table[i * 2]; count[i] = table[i * 2 + 1]; }
  return {
    names, offset, count,
    index: view(buf, m.sections[`${name}_index`]) as Uint16Array,
    delta: view(buf, m.sections[`${name}_delta`]) as Float32Array,
  };
}

export function parseAnny(manifest: AnnyManifest, bins: Record<string, ArrayBuffer>): AnnyBase {
  const lods = {} as Record<LodTag, AnnyLod>;
  for (const tag of Object.keys(manifest.lods) as LodTag[]) {
    const m = manifest.lods[tag];
    const buf = bins[m.file];
    if (!buf) throw new Error(`anny: missing ${m.file}`);
    const s = m.sections;
    const w8 = view(buf, s.weights) as Uint8Array;
    const weights = new Float32Array(w8.length);
    for (let i = 0; i < w8.length; i += 4) {
      const sum = w8[i] + w8[i + 1] + w8[i + 2] + w8[i + 3] || 1;
      for (let k = 0; k < 4; k++) weights[i + k] = w8[i + k] / sum;
    }
    const along8 = view(buf, s.along) as Uint8Array;
    const faceNames = m.face_targets ?? [];
    lods[tag] = {
      tag, V: m.vertices, k: m.k, D: (m.vertices + BONE_COUNT) * 3,
      indices: view(buf, s.indices) as Uint16Array,
      joints: view(buf, s.joints) as Uint8Array,
      weights,
      part: view(buf, s.part) as Uint8Array,
      along: Float32Array.from(along8, (v) => v / 255),
      mean: view(buf, s.mean) as Float32Array,
      basis: view(buf, s.basis) as Float32Array,
      table: view(buf, s.pheno_table) as Float32Array,
      mask: view(buf, s.pheno_mask) as Uint8Array,
      S: s.pheno_table.shape[1],
      expr: sparse(buf, m, 'expr', m.expressions ?? []),
      local: sparse(buf, m, 'local', faceNames.flatMap((n) => [`${n}+`, `${n}-`])),
    };
  }
  return { manifest, bones: manifest.bones, lods };
}

let cached: Promise<AnnyBase> | null = null;

/** Loads (once) the baked body from `baseUrl` (default `/city3d/anny/`). */
export function loadAnny(baseUrl = '/city3d/anny/'): Promise<AnnyBase> {
  if (cached) return cached;
  cached = (async () => {
    const manifest = (await (await fetch(baseUrl + 'manifest.json')).json()) as AnnyManifest;
    const bins: Record<string, ArrayBuffer> = {};
    await Promise.all(
      Object.values(manifest.lods).map(async (l) => {
        const r = await fetch(baseUrl + l.file);
        if (!r.ok) throw new Error(`anny: ${l.file} ${r.status}`);
        bins[l.file] = await r.arrayBuffer();
      }),
    );
    return parseAnny(manifest, bins);
  })();
  cached.catch(() => (cached = null));
  return cached;
}

/** Port of the Anny phenotype mixer: 6 phenotypes (0..1) to the blend
 * coefficients of its phenotype shapes (cupsize/firmness 0.5, race 1/3). */
export function phenotypeCoefficients(base: AnnyBase, lod: AnnyLod, ph: number[]): Float64Array {
  const p = base.manifest.phenotype;
  const vals: Record<string, number> = { cupsize: p.fixed.cupsize, firmness: p.fixed.firmness };
  p.labels.forEach((l, i) => (vals[l] = ph[i] ?? 0.5));
  const phens: number[] = [];
  let feature = '';
  let group: string[] = [];
  const flush = () => {
    if (!feature) return;
    if (feature === 'race') {
      for (let i = 0; i < group.length; i++) phens.push(p.fixed.race);
    } else {
      const a = p.anchors[feature];
      const v = Math.min(Math.max(vals[feature], a[0]), a[a.length - 1]);
      let i = 0;
      while (i < a.length && a[i] < v) i++; // searchsorted, side="left"
      i = Math.min(Math.max(i, 1), a.length - 1);
      const t = (v - a[i - 1]) / (a[i] - a[i - 1]);
      for (let j = 0; j < a.length; j++) phens.push(j === i - 1 ? 1 - t : j === i ? t : 0);
    }
  };
  for (const [f, k] of p.variations) {
    if (f !== feature) { flush(); feature = f; group = []; }
    group.push(k);
  }
  flush();
  const P = phens.length;
  const c = new Float64Array(lod.S);
  for (let s = 0; s < lod.S; s++) {
    let w = 1;
    for (let j = 0; j < P; j++) if (lod.mask[s * P + j]) w *= phens[j];
    c[s] = w;
  }
  return c;
}

export interface BodyShape {
  lod: LodTag;
  /** V*3 vertex positions, feet on y = 0. */
  positions: Float32Array;
  /** 52*3 joint heads, same frame. */
  joints: Float32Array;
  height: number;
}

/** Body for phenotypes (0..1) and up to four local face changes
 * ([target index, value -1..1], index 255 = none). */
export function buildBody(
  base: AnnyBase,
  tag: LodTag,
  ph: number[],
  face: [number, number][] = [],
): BodyShape {
  const lod = base.lods[tag];
  const c = phenotypeCoefficients(base, lod, ph);
  const z = new Float64Array(lod.k);
  for (let i = 0; i < lod.k; i++) {
    let s = 0;
    const row = i * lod.S;
    for (let j = 0; j < lod.S; j++) s += lod.table[row + j] * c[j];
    z[i] = s;
  }
  const x = new Float32Array(lod.D);
  x.set(lod.mean);
  for (let i = 0; i < lod.k; i++) {
    const zi = z[i];
    const row = i * lod.D;
    for (let d = 0; d < lod.D; d++) x[d] += zi * lod.basis[row + d];
  }
  if (lod.local) {
    for (const [t, v] of face) {
      if (t === 255 || !v) continue;
      addSparse(x, lod.local, t * 2 + (v >= 0 ? 0 : 1), Math.abs(v));
    }
  }
  const V = lod.V;
  let minY = Infinity;
  let maxY = -Infinity;
  for (let i = 0; i < V; i++) {
    const y = x[i * 3 + 1];
    if (y < minY) minY = y;
    if (y > maxY) maxY = y;
  }
  for (let i = 1; i < lod.D; i += 3) x[i] -= minY;
  return {
    lod: tag,
    positions: x.subarray(0, V * 3),
    joints: x.subarray(V * 3),
    height: maxY - minY,
  };
}

export function addSparse(target: Float32Array, set: SparseSet, entry: number, weight: number) {
  const off = set.offset[entry];
  const n = set.count[entry];
  for (let i = 0; i < n; i++) {
    const v = set.index[off + i] * 3;
    const d = (off + i) * 3;
    target[v] += set.delta[d] * weight;
    target[v + 1] += set.delta[d + 1] * weight;
    target[v + 2] += set.delta[d + 2] * weight;
  }
}

/** Dense V*3 delta of one sparse entry (for morph targets). */
export function denseDelta(set: SparseSet, entry: number, V: number): Float32Array {
  const out = new Float32Array(V * 3);
  addSparse(out, set, entry, 1);
  return out;
}
