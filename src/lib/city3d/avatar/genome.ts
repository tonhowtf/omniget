// Genome v1, 32 bytes, mirror of `omniget-avatar/src/genome.rs`. The hex
// string (64 chars) is what travels and what the commands return.
import { catalog } from './catalog';

export const GENOME_LEN = 32;
export const NO_FACE = 255;

export interface Genome {
  version: number; body: number;
  /** gender, age, muscle, weight, height, proportions; 0..255 */
  phenotypes: number[];
  /** four (target index, value -127..127); index 255 = none */
  face: [number, number][];
  skin: number; skinVar: number; hair: number; hairColor: number;
  eyes: number; brows: number; beard: number;
  top: number; bottom: number; shoes: number; accessory: number;
  colors: [number, number, number];
  gait: number; idle: number; mood: number; blink: number; seed: number;
}

export function decodeGenome(input: string | Uint8Array): Genome {
  const b = typeof input === 'string' ? hexToBytes(input) : input;
  if (b.length !== GENOME_LEN) throw new Error(`genome must be ${GENOME_LEN} bytes`);
  if (b[0] >> 4 !== 1) throw new Error(`unsupported genome version ${b[0] >> 4}`);
  const i8 = (v: number) => (v > 127 ? v - 256 : v);
  return {
    version: b[0] >> 4, body: b[0] & 15,
    phenotypes: Array.from(b.slice(1, 7)),
    face: [0, 1, 2, 3].map((i) => [b[7 + i * 2], i8(b[8 + i * 2])] as [number, number]),
    skin: b[15], skinVar: b[16], hair: b[17], hairColor: b[18], eyes: b[19],
    brows: b[20] >> 4, beard: b[20] & 15,
    top: b[21], bottom: b[22], shoes: b[23], accessory: b[24],
    colors: [b[25], b[26], b[27]],
    gait: b[28] >> 4, idle: b[28] & 15, mood: b[29] >> 4, blink: b[29] & 15,
    seed: b[30] | (b[31] << 8),
  };
}

export function encodeGenome(g: Genome): string {
  const b = new Uint8Array(GENOME_LEN);
  b[0] = ((g.version & 15) << 4) | (g.body & 15);
  g.phenotypes.forEach((v, i) => (b[1 + i] = clampByte(v)));
  g.face.forEach(([t, v], i) => {
    b[7 + i * 2] = t & 255;
    b[8 + i * 2] = Math.max(-127, Math.min(127, Math.round(v))) & 255;
  });
  b[15] = g.skin; b[16] = g.skinVar; b[17] = g.hair; b[18] = g.hairColor; b[19] = g.eyes;
  b[20] = ((g.brows & 15) << 4) | (g.beard & 15);
  b[21] = g.top; b[22] = g.bottom; b[23] = g.shoes; b[24] = g.accessory;
  b[25] = g.colors[0]; b[26] = g.colors[1]; b[27] = g.colors[2];
  b[28] = ((g.gait & 15) << 4) | (g.idle & 15);
  b[29] = ((g.mood & 15) << 4) | (g.blink & 15);
  b[30] = g.seed & 255; b[31] = (g.seed >> 8) & 255;
  return bytesToHex(b);
}

/** Readable problems, same rules as the Rust `Genome::validate`. */
export function validateGenome(g: Genome): string[] {
  const c = catalog;
  const e: string[] = [];
  const range = (what: string, v: number, n: number) => {
    if (v < 0 || v >= n) e.push(`${what} ${v} outside 0..${n}`);
  };
  range('skin', g.skin, c.skin_tones.length);
  range('hair style', g.hair, c.hair_styles.length);
  range('hair color', g.hairColor, c.hair_colors.length);
  range('eye color', g.eyes, c.eye_colors.length);
  range('brows', g.brows, c.brows.length);
  range('beard', g.beard, c.beards.length);
  range('top', g.top, c.tops.length);
  range('bottom', g.bottom, c.bottoms.length);
  range('shoes', g.shoes, c.shoes.length);
  range('accessory', g.accessory, c.accessories.length);
  g.colors.forEach((v) => range('cloth color', v, c.cloth_colors.length));
  g.face.forEach(([t]) => t !== NO_FACE && range('face target', t, c.face_targets.length));
  range('gait', g.gait, c.gaits.length);
  range('idle', g.idle, c.idles.length);
  range('mood', g.mood, c.moods.length);
  if (!e.length) {
    const full = !!c.tops[g.top].full_body;
    if (g.top === 0) e.push('a top is required');
    if (full && g.bottom !== 0) e.push('full-body top: bottom must be none');
    if (!full && g.bottom === 0) e.push('a bottom is required');
  }
  return e;
}

/** Applies the piece restrictions (full-body tops drop the bottom). */
export function fixGenome(g: Genome): Genome {
  const out = { ...g, colors: [...g.colors] as [number, number, number], face: g.face.map((f) => [...f] as [number, number]) };
  if (out.top === 0) out.top = Math.max(1, catalog.tops.findIndex((t) => t.id === 't-shirt'));
  const full = !!catalog.tops[out.top]?.full_body;
  if (full) out.bottom = 0;
  else if (out.bottom === 0) out.bottom = Math.max(1, catalog.bottoms.findIndex((t) => t.id === 'pants'));
  return out;
}

/** Phenotype i as 0..1. */
export const pheno = (g: Genome, i: number) => g.phenotypes[i] / 255;

/** Deterministic hash of a genome string for caches and seeds. */
export function genomeKey(hex: string): number {
  let h = 2166136261;
  for (let i = 0; i < hex.length; i++) h = Math.imul(h ^ hex.charCodeAt(i), 16777619);
  return h >>> 0;
}

export function hexToBytes(s: string): Uint8Array {
  const t = s.trim();
  if (!/^[0-9a-fA-F]*$/.test(t) || t.length % 2) throw new Error('invalid hex');
  const out = new Uint8Array(t.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(t.slice(i * 2, i * 2 + 2), 16);
  return out;
}

export function bytesToHex(b: Uint8Array): string {
  return Array.from(b, (x) => x.toString(16).padStart(2, '0')).join('');
}

function clampByte(v: number) {
  return Math.max(0, Math.min(255, Math.round(v)));
}

/** A small seeded PRNG (mulberry32) for the client-side micro variation. */
export function mulberry32(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
