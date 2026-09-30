// Procedural clothing, hair and accessories over a built Anny body.
//
// There is no garment art yet, so every piece is generated here:
// - garments are shells of the body: the covered vertex regions pushed out
//   along the normal, skinned with the body weights (so they follow any body);
// - skirts and coat tails are tubes fitted to the leg cross-sections;
// - hair is a scalp shell with parametric extras (tail sheet, ponytail, bun,
//   braids, crest), brows and beards are small shapes and shells;
// - accessories are primitives rigidly skinned to one bone.
// MakeHuman MHCLO garments (CC0) will replace the shells as catalog pieces;
// the fitting math (3 body vertices + barycentric weights + offset) slots into
// `shell()` below.
import * as THREE from 'three';
import type { AnnyLod, BodyShape } from './anny';
import type { Catalog } from './catalog';
import { hexColor } from './catalog';
import type { Genome } from './genome';
import { mulberry32 } from './genome';
import { boneIndex, coverMask, isScalp, jointOf, R, type HeadInfo, type VrmBone } from './rig';

export type RGB = [number, number, number];

/** Collects skinned triangles with a color id per vertex. */
export class MeshBuilder {
  pos: number[] = [];
  color: number[] = [];
  joints: number[] = [];
  weights: number[] = [];
  index: number[] = [];
  palette: RGB[] = [];
  /** license tag per color id, for the exporter's meta */
  paletteNames: string[] = [];

  colorId(rgb: RGB, name = ''): number {
    const i = this.palette.findIndex((c, k) => c[0] === rgb[0] && c[1] === rgb[1] && c[2] === rgb[2] && this.paletteNames[k] === name);
    if (i >= 0) return i;
    this.palette.push(rgb);
    this.paletteNames.push(name);
    return this.palette.length - 1;
  }

  vertex(x: number, y: number, z: number, color: number, j: number[], w: number[]): number {
    this.pos.push(x, y, z);
    this.color.push(color);
    this.joints.push(j[0] ?? 0, j[1] ?? 0, j[2] ?? 0, j[3] ?? 0);
    this.weights.push(w[0] ?? 1, w[1] ?? 0, w[2] ?? 0, w[3] ?? 0);
    return this.pos.length / 3 - 1;
  }

  tri(a: number, b: number, c: number) {
    this.index.push(a, b, c);
  }

  /** A three geometry transformed by `m`, rigidly skinned to `bone`. */
  rigid(geo: THREE.BufferGeometry, m: THREE.Matrix4, color: number, bone: number) {
    const g = geo;
    const p = g.getAttribute('position');
    const v = new THREE.Vector3();
    const start = this.pos.length / 3;
    for (let i = 0; i < p.count; i++) {
      v.fromBufferAttribute(p, i).applyMatrix4(m);
      this.pos.push(v.x, v.y, v.z);
      this.color.push(color);
      this.joints.push(bone, 0, 0, 0);
      this.weights.push(1, 0, 0, 0);
    }
    if (g.index) {
      const ix = g.index.array;
      for (let i = 0; i < ix.length; i += 3) this.tri(start + ix[i], start + ix[i + 1], start + ix[i + 2]);
    } else {
      for (let i = 0; i < p.count; i += 3) this.tri(start + i, start + i + 1, start + i + 2);
    }
    geo.dispose();
  }

  get vertexCount() {
    return this.pos.length / 3;
  }

  /** Runtime geometry: vertex colors (linear) from the palette. */
  build(linear: (c: RGB) => RGB): THREE.BufferGeometry {
    const g = new THREE.BufferGeometry();
    g.setAttribute('position', new THREE.Float32BufferAttribute(this.pos, 3));
    const col = new Float32Array(this.color.length * 3);
    const lin = this.palette.map(linear);
    this.color.forEach((id, i) => {
      const c = lin[id];
      col[i * 3] = c[0]; col[i * 3 + 1] = c[1]; col[i * 3 + 2] = c[2];
    });
    g.setAttribute('color', new THREE.BufferAttribute(col, 3));
    g.setAttribute('skinIndex', new THREE.Uint16BufferAttribute(this.joints, 4));
    g.setAttribute('skinWeight', new THREE.Float32BufferAttribute(this.weights, 4));
    g.setIndex(this.index);
    g.computeVertexNormals();
    g.boundingSphere = new THREE.Sphere(new THREE.Vector3(0, 1, 0), 3);
    return g;
  }

  /** Export geometry: one group per face color (no vertex colors). */
  buildGrouped(): { geometry: THREE.BufferGeometry; groupColors: number[] } {
    const byColor = new Map<number, number[]>();
    for (let f = 0; f < this.index.length; f += 3) {
      const c = this.color[this.index[f]];
      let list = byColor.get(c);
      if (!list) byColor.set(c, (list = []));
      list.push(this.index[f], this.index[f + 1], this.index[f + 2]);
    }
    const g = new THREE.BufferGeometry();
    g.setAttribute('position', new THREE.Float32BufferAttribute(this.pos, 3));
    g.setAttribute('skinIndex', new THREE.Uint16BufferAttribute(this.joints, 4));
    g.setAttribute('skinWeight', new THREE.Float32BufferAttribute(this.weights, 4));
    const idx: number[] = [];
    const groupColors: number[] = [];
    for (const [c, list] of byColor) {
      g.addGroup(idx.length, list.length, groupColors.length);
      groupColors.push(c);
      for (const i of list) idx.push(i);
    }
    g.setIndex(idx);
    g.computeVertexNormals();
    return { geometry: g, groupColors };
  }
}

export interface OutfitContext {
  cat: Catalog;
  lod: AnnyLod;
  body: BodyShape;
  normals: Float32Array;
  regions: Uint8Array;
  head: HeadInfo;
  genome: Genome;
  /** when false, only colors are resolved (reduced and crowd tiers) */
  geometry: boolean;
}

export interface OutfitColors {
  skin: RGB; eye: RGB; hair: RGB; brow: RGB; top: RGB; bottom: RGB; shoes: RGB; accent: RGB;
}

export function shade(c: RGB, f: number): RGB {
  return [Math.min(1, c[0] * f), Math.min(1, c[1] * f), Math.min(1, c[2] * f)];
}

export function outfitColors(cat: Catalog, g: Genome): OutfitColors {
  const skin0 = hexColor(cat.skin_tones[g.skin]?.hex ?? 'c68642');
  const v = ((g.skinVar & 15) - 7.5) / 7.5;
  const warm = (((g.skinVar >> 4) & 15) - 7.5) / 7.5;
  const skin: RGB = [
    Math.min(1, skin0[0] * (1 + v * 0.04 + warm * 0.02)),
    Math.min(1, skin0[1] * (1 + v * 0.04)),
    Math.min(1, skin0[2] * (1 + v * 0.04 - warm * 0.03)),
  ];
  const hair = hexColor(cat.hair_colors[g.hairColor]?.hex ?? '3a2618');
  const cloth = (i: number) => hexColor(cat.cloth_colors[i] ?? '8f8a80');
  const accent = cloth(g.colors[2]);
  return {
    skin, hair,
    eye: hexColor(cat.eye_colors[g.eyes]?.hex ?? '2e1c12'),
    brow: shade(hair, 0.75),
    top: cloth(g.colors[0]),
    bottom: cloth(g.colors[1]),
    shoes: shade(accent, 0.55),
    accent,
  };
}

/** Region bits covered by the genome's garments, per slot. */
export function coverBits(cat: Catalog, g: Genome) {
  const top = cat.tops[g.top];
  const bottom = cat.bottoms[g.bottom];
  const shoes = cat.shoes[g.shoes];
  const hair = cat.hair_styles[g.hair];
  return {
    top: coverMask(top?.cover ?? []),
    bottom: coverMask(bottom?.cover ?? []),
    shoes: coverMask(shoes?.cover ?? []),
    hair: hair && hair.shape !== 'none' ? 1 << R.scalp : 0,
  };
}

const J = (n: VrmBone) => boneIndex(n);

/** Builds every procedural piece into `b`. Returns the region bits whose body
 * faces are hidden (fully under a garment). */
export function buildOutfit(ctx: OutfitContext, b: MeshBuilder, colors: OutfitColors): number {
  const { cat, genome: g } = ctx;
  const rnd = mulberry32(g.seed * 7919 + 17);
  let hidden = 0;
  const top = cat.tops[g.top];
  const bottom = cat.bottoms[g.bottom];
  const shoes = cat.shoes[g.shoes];

  const legBits = (1 << R.upperLegHigh) | (1 << R.upperLegLow) | (1 << R.lowerLegHigh) | (1 << R.lowerLegLow);

  if (shoes && shoes.cover.length) {
    const m = coverMask(shoes.cover);
    shell(ctx, b, m, shoes.offset, b.colorId(colors.shoes, 'shoes'));
    hidden |= m;
  }
  if (bottom && bottom.cover.length) {
    let m = coverMask(bottom.cover);
    if ((bottom.skirt ?? 0) > 0) m &= ~legBits;
    shell(ctx, b, m, bottom.offset, b.colorId(colors.bottom, 'bottom'));
    hidden |= m;
    if (bottom.skirt) skirtTube(ctx, b, bottom.skirt, bottom.offset + 0.006, b.colorId(colors.bottom, 'bottom'));
  }
  if (top && top.cover.length) {
    let m = coverMask(top.cover);
    if ((top.skirt ?? 0) > 0) m &= ~legBits;
    shell(ctx, b, m, top.offset, b.colorId(colors.top, 'top'));
    hidden |= m;
    if (top.skirt) skirtTube(ctx, b, top.skirt, top.offset + 0.008, b.colorId(colors.top, 'top'));
    if (top.apron) apron(ctx, b, b.colorId(shade(colors.accent, 1.1), 'top'));
  }

  hidden |= hair(ctx, b, colors, rnd);
  brows(ctx, b, colors);
  beard(ctx, b, colors);
  eyes(ctx, b, colors);
  accessory(ctx, b, colors);
  return hidden;
}

/** Shell of the regions in `mask`, pushed `offset` meters along the normal. */
export function shell(ctx: OutfitContext, b: MeshBuilder, mask: number, offset: number, color: number, jitter?: (i: number) => number) {
  const { lod, body, normals, regions } = ctx;
  const map = new Int32Array(lod.V).fill(-1);
  const inMask = (v: number) => (mask >> regions[v]) & 1;
  const ix = lod.indices;
  const p = body.positions;
  for (let f = 0; f < ix.length; f += 3) {
    const a = ix[f], c = ix[f + 1], d = ix[f + 2];
    // any covered corner takes the face: garment edges then overlap instead of
    // leaving a jagged strip of skin along the coarse LOD triangles
    if (!inMask(a) && !inMask(c) && !inMask(d)) continue;
    for (const v of [a, c, d]) {
      if (map[v] >= 0) continue;
      const o = offset + (jitter ? jitter(v) : 0);
      const j = v * 4;
      b.pos.push(p[v * 3] + normals[v * 3] * o, p[v * 3 + 1] + normals[v * 3 + 1] * o, p[v * 3 + 2] + normals[v * 3 + 2] * o);
      b.color.push(color);
      b.joints.push(lod.joints[j], lod.joints[j + 1], lod.joints[j + 2], lod.joints[j + 3]);
      b.weights.push(lod.weights[j], lod.weights[j + 1], lod.weights[j + 2], lod.weights[j + 3]);
      map[v] = b.pos.length / 3 - 1;
    }
    b.tri(map[a], map[c], map[d]);
  }
}

/** Elliptic cross-section of the given parts at height y. */
function section(ctx: OutfitContext, y: number, parts: number[], band = 0.035) {
  const { lod, body } = ctx;
  const p = body.positions;
  let minX = Infinity, maxX = -Infinity, minZ = Infinity, maxZ = -Infinity;
  for (let i = 0; i < lod.V; i++) {
    if (!parts.includes(lod.part[i])) continue;
    if (Math.abs(p[i * 3 + 1] - y) > band) continue;
    minX = Math.min(minX, p[i * 3]); maxX = Math.max(maxX, p[i * 3]);
    minZ = Math.min(minZ, p[i * 3 + 2]); maxZ = Math.max(maxZ, p[i * 3 + 2]);
  }
  if (!isFinite(minX)) return null;
  return { cx: (minX + maxX) / 2, cz: (minZ + maxZ) / 2, rx: (maxX - minX) / 2, rz: (maxZ - minZ) / 2 };
}

function skirtTube(ctx: OutfitContext, b: MeshBuilder, length: number, offset: number, color: number) {
  const { body } = ctx;
  const hips = jointOf(body, J('hips'));
  const knee = jointOf(body, J('leftLowerLeg'));
  const foot = jointOf(body, J('leftFoot'));
  const yTop = hips.y + 0.07;
  const yEnd = yTop - length * (yTop - foot.y);
  const rings = 8, seg = 20;
  type Ring = { rx: number; rz: number; cx: number; cz: number };
  let prev: Ring | null = null;
  const ringStart: number[] = [];
  for (let r = 0; r <= rings; r++) {
    const t = r / rings;
    const y = yTop + (yEnd - yTop) * t;
    const s: Ring = section(ctx, y, [2, 3, 7, 8]) ?? prev ?? { cx: 0, cz: hips.z, rx: 0.16, rz: 0.12 };
    let rx = s.rx + offset + t * 0.05 * length;
    let rz = s.rz + offset + t * 0.04 * length;
    if (prev) { rx = Math.max(rx, prev.rx); rz = Math.max(rz, prev.rz); }
    const ring: Ring = { cx: 0, cz: s.cz, rx, rz };
    prev = ring;
    ringStart.push(b.vertexCount);
    const legW = Math.min(0.55, Math.max(0, (hips.y - y) / Math.max(0.05, hips.y - knee.y)) * 0.55);
    for (let k = 0; k < seg; k++) {
      const a = (k / seg) * Math.PI * 2;
      const x = ring.cx + Math.cos(a) * rx;
      const z = ring.cz + Math.sin(a) * rz;
      const side = x >= 0 ? J('leftUpperLeg') : J('rightUpperLeg');
      const sw = legW * Math.min(1, Math.abs(x) / Math.max(0.01, rx) * 1.5);
      b.vertex(x, y, z, color, [J('hips'), side], [1 - sw, sw]);
    }
  }
  for (let r = 0; r < rings; r++) {
    for (let k = 0; k < seg; k++) {
      const a = ringStart[r] + k, a2 = ringStart[r] + ((k + 1) % seg);
      const c = ringStart[r + 1] + k, c2 = ringStart[r + 1] + ((k + 1) % seg);
      b.tri(a, a2, c);
      b.tri(a2, c2, c);
    }
  }
}

function apron(ctx: OutfitContext, b: MeshBuilder, color: number) {
  const { body } = ctx;
  const chest = jointOf(body, J('upperChest'));
  const hips = jointOf(body, J('hips'));
  const knee = jointOf(body, J('leftLowerLeg'));
  const y0 = chest.y + 0.02, y1 = hips.y - 0.55 * (hips.y - knee.y);
  const rows = 6;
  const idx: number[][] = [];
  for (let r = 0; r <= rows; r++) {
    const y = y0 + (y1 - y0) * (r / rows);
    const s = section(ctx, y, [2, 3, 7]);
    const w = (s ? s.rx : 0.15) * 0.8;
    const z = (s ? s.cz + s.rz : chest.z + 0.1) + 0.012;
    const bone = y > chest.y - 0.1 ? J('chest') : y > hips.y ? J('spine') : J('hips');
    idx.push([-1, 0, 1].map((k) => b.vertex(k * w, y, z + (k === 0 ? 0.004 : 0), color, [bone], [1])));
  }
  for (let r = 0; r < rows; r++) {
    for (let k = 0; k < 2; k++) {
      const a = idx[r][k], a2 = idx[r][k + 1], c = idx[r + 1][k], c2 = idx[r + 1][k + 1];
      b.tri(a, c, a2);
      b.tri(a2, c, c2);
    }
  }
}

function hair(ctx: OutfitContext, b: MeshBuilder, colors: OutfitColors, rnd: () => number): number {
  const { cat, genome: g, lod, body, head: h } = ctx;
  const style = cat.hair_styles[g.hair];
  if (!style || style.shape === 'none') return 0;
  const color = b.colorId(colors.hair, 'hair');
  const p = body.positions;
  const cut = style.cut ?? 0;
  const fringe = style.fringe ?? 0;
  const thick = style.thickness ?? 0.012;
  const headBone = J('head');
  // scalp mask for this cut: its own region test, on head and neck vertices
  const regions = ctx.regions.slice();
  for (let i = 0; i < lod.V; i++) {
    if (lod.part[i] !== 0 && lod.part[i] !== 1) continue;
    const x = p[i * 3], y = p[i * 3 + 1], z = p[i * 3 + 2];
    if (lod.part[i] === 1 && z > h.centerZ - 0.02) continue;
    if (isScalp(h, x, y, z, cut, fringe)) regions[i] = R.scalp;
    else if (regions[i] === R.scalp) regions[i] = R.head;
  }
  const bumps = style.bumps ?? 0;
  const jitter = bumps ? (v: number) => (Math.sin(v * 12.9898) * 43758.5453 % 1 + 1) % 1 * bumps : undefined;
  if (style.shape === 'crest') {
    const x0 = 0.022;
    for (let s = 0; s < 7; s++) {
      const t = s / 6;
      const a = -0.35 + t * 1.9;
      const cy = h.eyeY + 0.03 + Math.cos(a) * (h.top - h.eyeY);
      const cz = h.centerZ + Math.sin(a) * (h.front - h.back) * 0.5;
      const m = new THREE.Matrix4().compose(
        new THREE.Vector3(0, cy + thick * 0.4, cz),
        new THREE.Quaternion().setFromEuler(new THREE.Euler(-a + Math.PI / 2, 0, 0)),
        new THREE.Vector3(x0 * 2, thick, 0.045),
      );
      b.rigid(new THREE.BoxGeometry(1, 1, 1), m, color, headBone);
    }
    // shaved sides
    shell({ ...ctx, regions }, b, 1 << R.scalp, 0.002, b.colorId(shade(colors.hair, 0.85), 'hair'));
    return 1 << R.scalp;
  }
  shell({ ...ctx, regions }, b, 1 << R.scalp, thick, color, jitter);
  // with long cuts the hair also covers the back of the neck
  const back = new THREE.Vector3(0, 0, h.back);
  if (style.tail) hairSheet(ctx, b, color, thick, style.tail, cut);
  if (style.ponytail) {
    const len = style.ponytail;
    const m = new THREE.Matrix4().compose(
      new THREE.Vector3(0, h.eyeY + 0.02 - len * 0.45, back.z - 0.04),
      new THREE.Quaternion().setFromEuler(new THREE.Euler(-0.25, 0, 0)),
      new THREE.Vector3(1, 1, 1),
    );
    b.rigid(new THREE.CylinderGeometry(0.028, 0.012, len, 8, 3), m, color, headBone);
    const knot = new THREE.Matrix4().makeTranslation(0, h.eyeY + 0.03, back.z - 0.015);
    b.rigid(new THREE.SphereGeometry(0.03, 8, 6), knot, color, headBone);
  }
  if (style.bun) {
    const r = style.bun;
    const m = new THREE.Matrix4().makeTranslation(0, h.eyeY + 0.07, back.z - r * 0.5);
    b.rigid(new THREE.SphereGeometry(r, 10, 8), m, color, headBone);
  }
  if (style.braids) {
    for (const sx of [-1, 1]) {
      const len = style.braids;
      for (let k = 0; k < 6; k++) {
        const t = (k + 0.5) / 6;
        const m = new THREE.Matrix4().makeTranslation(sx * h.halfWidth * 0.55, h.eyeY - 0.02 - t * len, h.centerZ - 0.05 - t * 0.03);
        const bone = t < 0.3 ? headBone : J('upperChest');
        b.rigid(new THREE.SphereGeometry(0.018 - t * 0.004, 6, 5), m, color, bone);
      }
    }
  }
  void rnd;
  return 1 << R.scalp;
}

/** Curved sheet behind the head and down the back (long hair, locs). */
function hairSheet(ctx: OutfitContext, b: MeshBuilder, color: number, thick: number, tail: number, cut: number) {
  const { body, head: h } = ctx;
  const chest = jointOf(body, J('upperChest'));
  const y0 = h.eyeY + 0.04;
  const y1 = h.eyeY - 0.035 + cut - tail;
  const rows = 7, seg = 11;
  const start: number[] = [];
  for (let r = 0; r <= rows; r++) {
    const t = r / rows;
    const y = y0 + (y1 - y0) * t;
    const s = y > h.chin ? section(ctx, y, [0], 0.02) : section(ctx, y, [1, 2], 0.03);
    const rx = Math.max(h.halfWidth * 0.9, s ? s.rx * 0.9 : 0) + thick + 0.004;
    const rz = Math.max((h.front - h.back) * 0.45, s ? s.rz : 0) + thick + 0.004;
    const cz = s ? s.cz : h.centerZ;
    const headW = Math.max(0, 1 - Math.max(0, (h.chin - y) / Math.max(0.05, h.chin - chest.y)));
    start.push(b.vertexCount);
    for (let k = 0; k < seg; k++) {
      const a = Math.PI * (0.05 + 0.9 * (k / (seg - 1))) + Math.PI; // behind: angles pi..2pi (z negative)
      const x = Math.cos(a) * rx;
      const z = cz + Math.sin(a) * rz * (y > h.chin ? 1 : 1.05);
      b.vertex(x, y, z, color, [J('head'), J('upperChest')], [headW, 1 - headW]);
    }
  }
  for (let r = 0; r < rows; r++) {
    for (let k = 0; k < seg - 1; k++) {
      const a = start[r] + k, a2 = a + 1, c = start[r + 1] + k, c2 = c + 1;
      b.tri(a, c, a2);
      b.tri(a2, c, c2);
    }
  }
}

function surfaceZ(ctx: OutfitContext, x: number, y: number, r = 0.02): number {
  const { lod, body } = ctx;
  const p = body.positions;
  let best = -Infinity;
  for (let i = 0; i < lod.V; i++) {
    if (lod.part[i] !== 0 && lod.part[i] !== 10) continue;
    if (Math.hypot(p[i * 3] - x, p[i * 3 + 1] - y) > r) continue;
    best = Math.max(best, p[i * 3 + 2]);
  }
  return isFinite(best) ? best : ctx.head.front;
}

function brows(ctx: OutfitContext, b: MeshBuilder, colors: OutfitColors) {
  const { cat, genome: g, head: h } = ctx;
  const kind = cat.brows[g.brows] ?? 'none';
  if (kind === 'none') return;
  const size: Record<string, [number, number, number]> = {
    thin: [0.026, 0.0035, 0], natural: [0.028, 0.005, 0.1], thick: [0.03, 0.008, 0.08],
    arched: [0.028, 0.005, 0.28], straight: [0.03, 0.006, 0], bushy: [0.032, 0.011, 0.1], short: [0.02, 0.006, 0.05],
  };
  const [w, t, tilt] = size[kind] ?? size.natural;
  const color = b.colorId(colors.brow, 'hair');
  for (const sx of [-1, 1]) {
    const x = sx * (h.eyeX + 0.004), y = h.eyeY + 0.021;
    const z = surfaceZ(ctx, x, y, 0.018) + 0.001;
    const m = new THREE.Matrix4().compose(
      new THREE.Vector3(x, y, z),
      new THREE.Quaternion().setFromEuler(new THREE.Euler(0, sx * 0.25, -sx * tilt)),
      new THREE.Vector3(w, t, 0.006),
    );
    b.rigid(new THREE.BoxGeometry(1, 1, 1), m, color, J('head'));
  }
}

function beard(ctx: OutfitContext, b: MeshBuilder, colors: OutfitColors) {
  const { cat, genome: g, head: h, lod, body } = ctx;
  const kind = cat.beards[g.beard] ?? 'none';
  if (kind === 'none') return;
  const color = b.colorId(kind === 'stubble' ? mix(colors.skin, colors.hair, 0.45) : colors.hair, 'hair');
  const mouthY = h.eyeY - 0.055;
  const p = body.positions;
  const regions = ctx.regions.slice();
  const BEARD = 16; // temporary region id outside the 16 body regions
  for (let i = 0; i < lod.V; i++) {
    if (lod.part[i] !== 0) continue;
    const x = p[i * 3], y = p[i * 3 + 1], z = p[i * 3 + 2];
    if (z < h.centerZ - 0.005) continue;
    let on = false;
    switch (kind) {
      case 'stubble': case 'short-beard': case 'full-beard': case 'long-beard':
        on = y < mouthY + 0.02 && !(Math.abs(x) < 0.022 && Math.abs(y - mouthY) < 0.008);
        break;
      case 'goatee': on = y < mouthY - 0.004 && Math.abs(x) < 0.025; break;
      case 'chin-strap': on = y < mouthY - 0.01 && (Math.abs(x) > 0.03 || y < mouthY - 0.035); break;
    }
    if (on) regions[i] = BEARD;
  }
  const off = { stubble: 0.0015, 'short-beard': 0.005, 'full-beard': 0.01, 'long-beard': 0.012, goatee: 0.006, 'chin-strap': 0.005 }[kind] ?? 0.004;
  if (kind !== 'mustache') shell({ ...ctx, regions }, b, 1 << BEARD, off, color);
  if (kind === 'mustache' || kind === 'full-beard' || kind === 'long-beard' || kind === 'goatee') {
    const y = mouthY + 0.012;
    const z = surfaceZ(ctx, 0, y, 0.012) + 0.003;
    const m = new THREE.Matrix4().compose(new THREE.Vector3(0, y, z), new THREE.Quaternion(), new THREE.Vector3(0.045, 0.009, 0.01));
    b.rigid(new THREE.BoxGeometry(1, 1, 1), m, b.colorId(colors.hair, 'hair'), J('head'));
  }
  if (kind === 'long-beard') {
    const m = new THREE.Matrix4().compose(
      new THREE.Vector3(0, h.chin - 0.035, surfaceZ(ctx, 0, h.chin + 0.01, 0.02) - 0.01),
      new THREE.Quaternion().setFromEuler(new THREE.Euler(Math.PI + 0.35, 0, 0)),
      new THREE.Vector3(1, 1, 1),
    );
    b.rigid(new THREE.ConeGeometry(0.04, 0.09, 8, 1), m, color, J('head'));
  }
}

function mix(a: RGB, c: RGB, t: number): RGB {
  return [a[0] + (c[0] - a[0]) * t, a[1] + (c[1] - a[1]) * t, a[2] + (c[2] - a[2]) * t];
}

/** Eyes: white ball and iris disk on each eye bone, so look-at moves them. */
function eyes(ctx: OutfitContext, b: MeshBuilder, colors: OutfitColors) {
  const { body } = ctx;
  for (const name of ['leftEye', 'rightEye'] as const) {
    const e = jointOf(body, J(name));
    const z = surfaceZ(ctx, e.x, e.y, 0.012);
    const white = new THREE.Matrix4().compose(new THREE.Vector3(e.x, e.y, z - 0.004), new THREE.Quaternion(), new THREE.Vector3(1, 0.8, 0.5));
    b.rigid(new THREE.SphereGeometry(0.0105, 10, 8), white, b.colorId([0.95, 0.94, 0.92], 'eye'), J(name));
    const iris = new THREE.Matrix4().compose(new THREE.Vector3(e.x, e.y, z + 0.0012), new THREE.Quaternion(), new THREE.Vector3(1, 1, 0.35));
    b.rigid(new THREE.SphereGeometry(0.0058, 10, 8), iris, b.colorId(colors.eye, 'eye'), J(name));
    const pupil = new THREE.Matrix4().compose(new THREE.Vector3(e.x, e.y, z + 0.0026), new THREE.Quaternion(), new THREE.Vector3(1, 1, 0.3));
    b.rigid(new THREE.SphereGeometry(0.0026, 8, 6), pupil, b.colorId([0.03, 0.03, 0.03], 'eye'), J(name));
  }
}

function accessory(ctx: OutfitContext, b: MeshBuilder, colors: OutfitColors) {
  const { cat, genome: g, head: h, body } = ctx;
  const a = cat.accessories[g.accessory];
  if (!a || !a.shape) return;
  const head = J('head');
  const accent = b.colorId(colors.accent, 'accessory');
  const dark = b.colorId([0.08, 0.07, 0.07], 'accessory');
  const T = (x: number, y: number, z: number, s: [number, number, number] = [1, 1, 1], e?: THREE.Euler) =>
    new THREE.Matrix4().compose(new THREE.Vector3(x, y, z), new THREE.Quaternion().setFromEuler(e ?? new THREE.Euler()), new THREE.Vector3(...s));
  const crownY = h.eyeY + 0.55 * (h.top - h.eyeY);
  const hr = Math.max(h.halfWidth, (h.front - h.back) / 2);
  const hairT = cat.hair_styles[g.hair]?.thickness ?? 0;
  switch (a.shape) {
    case 'glasses':
    case 'sunglasses': {
      const z = surfaceZ(ctx, h.eyeX, h.eyeY, 0.02) + 0.012;
      for (const sx of [-1, 1]) {
        if (a.shape === 'glasses') {
          b.rigid(new THREE.TorusGeometry(0.014, 0.0022, 5, 14), T(sx * h.eyeX, h.eyeY, z), dark, head);
        } else {
          b.rigid(new THREE.BoxGeometry(0.032, 0.02, 0.004), T(sx * h.eyeX, h.eyeY - 0.002, z), dark, head);
        }
        b.rigid(new THREE.BoxGeometry(0.004, 0.004, (z - h.centerZ) * 0.9), T(sx * (h.halfWidth - 0.004), h.eyeY + 0.004, z - (z - h.centerZ) * 0.45), dark, head);
      }
      b.rigid(new THREE.BoxGeometry(0.014, 0.003, 0.003), T(0, h.eyeY + 0.004, z), dark, head);
      break;
    }
    case 'cap': {
      const r = hr + hairT + 0.006;
      b.rigid(new THREE.SphereGeometry(r, 14, 8, 0, Math.PI * 2, 0, Math.PI / 2), T(0, crownY - 0.01, h.centerZ, [1, 0.8, 1]), accent, head);
      b.rigid(new THREE.CylinderGeometry(0.07, 0.07, 0.006, 14, 1, false, -Math.PI / 2, Math.PI), T(0, crownY - 0.005, h.front - 0.01, [1, 1, 0.9]), accent, head);
      break;
    }
    case 'brim-hat': {
      const r = hr + hairT + 0.008;
      b.rigid(new THREE.CylinderGeometry(r * 0.9, r, 0.08, 14), T(0, crownY + 0.03, h.centerZ), b.colorId([0.85, 0.74, 0.45], 'accessory'), head);
      b.rigid(new THREE.CylinderGeometry(r + 0.07, r + 0.07, 0.006, 18), T(0, crownY - 0.005, h.centerZ), b.colorId([0.85, 0.74, 0.45], 'accessory'), head);
      b.rigid(new THREE.CylinderGeometry(r * 0.93, r * 0.98, 0.018, 14), T(0, crownY + 0.008, h.centerZ), accent, head);
      break;
    }
    case 'beanie': {
      const r = hr + hairT + 0.008;
      b.rigid(new THREE.SphereGeometry(r, 14, 8, 0, Math.PI * 2, 0, Math.PI / 2), T(0, crownY - 0.03, h.centerZ, [1, 1.05, 1]), accent, head);
      b.rigid(new THREE.CylinderGeometry(r + 0.004, r + 0.004, 0.025, 14, 1, true), T(0, crownY - 0.03, h.centerZ), b.colorId(shade(colors.accent, 0.8), 'accessory'), head);
      break;
    }
    case 'top-hat': {
      const r = hr + hairT + 0.006;
      b.rigid(new THREE.CylinderGeometry(r * 0.85, r * 0.85, 0.16, 14), T(0, crownY + 0.08, h.centerZ), dark, head);
      b.rigid(new THREE.CylinderGeometry(r + 0.04, r + 0.04, 0.006, 16), T(0, crownY, h.centerZ), dark, head);
      break;
    }
    case 'headband': {
      const r = hr + hairT + 0.004;
      b.rigid(new THREE.TorusGeometry(r, 0.007, 5, 20), T(0, crownY - 0.005, h.centerZ, [1, 1, (h.front - h.back) / 2 / r + 0.05], new THREE.Euler(Math.PI / 2 - 0.2, 0, 0)), accent, head);
      break;
    }
    case 'headscarf': {
      const r = hr + hairT + 0.01;
      b.rigid(new THREE.SphereGeometry(r, 14, 10, 0, Math.PI * 2, 0, Math.PI * 0.62), T(0, h.eyeY + 0.01, h.centerZ - 0.01, [1, 1.15, 1.02], new THREE.Euler(-0.35, 0, 0)), accent, head);
      break;
    }
    case 'scarf': {
      const neck = jointOf(body, J('neck'));
      const s = section(ctx, neck.y - 0.01, [1, 2], 0.03);
      const rx = (s?.rx ?? 0.06) + 0.02, rz = (s?.rz ?? 0.06) + 0.02;
      b.rigid(new THREE.TorusGeometry(1, 0.22, 6, 18), T(0, neck.y - 0.015, s?.cz ?? neck.z, [rx, rz, 0.1], new THREE.Euler(Math.PI / 2, 0, 0)), accent, J('upperChest'));
      b.rigid(new THREE.BoxGeometry(0.05, 0.22, 0.012), T(0.05, neck.y - 0.12, (s?.cz ?? neck.z) + rz + 0.005, [1, 1, 1], new THREE.Euler(0.1, 0, 0.1)), accent, J('upperChest'));
      break;
    }
    case 'backpack': {
      const chest = jointOf(body, J('chest'));
      const s = section(ctx, chest.y, [2], 0.04);
      const z = (s ? s.cz - s.rz : chest.z - 0.12) - 0.07;
      b.rigid(new THREE.BoxGeometry(0.26, 0.34, 0.13), T(0, chest.y + 0.02, z), accent, J('chest'));
      b.rigid(new THREE.BoxGeometry(0.2, 0.12, 0.04), T(0, chest.y - 0.08, z - 0.08), b.colorId(shade(colors.accent, 0.8), 'accessory'), J('chest'));
      break;
    }
    case 'earrings': {
      for (const sx of [-1, 1]) {
        b.rigid(new THREE.SphereGeometry(0.006, 6, 5), T(sx * (h.halfWidth + 0.002), h.eyeY - 0.045, h.centerZ - 0.005), b.colorId([0.95, 0.8, 0.35], 'accessory'), head);
      }
      break;
    }
  }
}
