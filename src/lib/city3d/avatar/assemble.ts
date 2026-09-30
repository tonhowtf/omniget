// Genome -> skinned character on the VRM skeleton, in three tiers of detail:
// - 'full': LOD 1229 body with the 52 ARKit morphs, procedural clothing,
//   hair and accessories, outline;
// - 'reduced': LOD 1229 body only, clothing as region colors, no morphs;
// - 'export': like 'full' but with one material per color (for VRM).
import * as THREE from 'three';
import { buildBody, denseDelta, type AnnyBase, type AnnyLod, type BodyShape } from './anny';
import { catalog, type Catalog } from './catalog';
import { decodeGenome, pheno, type Genome } from './genome';
import { srgbToLinear, outlineMaterial, toonMaterial } from './materials';
import { buildOutfit, coverBits, MeshBuilder, outfitColors, type OutfitColors, type RGB } from './outfit';
import { buildBones, measureHead, R, vertexNormals, vertexRegions, type HeadInfo } from './rig';

export type Tier = 'full' | 'reduced' | 'export';

export interface CharacterRig {
  tier: Tier;
  genome: Genome;
  root: THREE.Group;
  bones: THREE.Bone[];
  byName: Map<string, THREE.Bone>;
  skeleton: THREE.Skeleton;
  body: THREE.SkinnedMesh;
  outfit: THREE.SkinnedMesh | null;
  outlines: THREE.SkinnedMesh[];
  shape: BodyShape;
  head: HeadInfo;
  hipsRest: THREE.Vector3;
  /** ARKit names in morph order ('full' and 'export' only) */
  expressions: string[];
  /** license of each piece used, for the VRM meta */
  licenses: { piece: string; license: string }[];
  /** export tier: sRGB color per material, parallel to the mesh materials */
  exportColors?: { body: RGB[]; outfit: RGB[] };
  dispose(): void;
}

const morphCache = new WeakMap<AnnyLod, THREE.Float32BufferAttribute[]>();

function expressionMorphs(lod: AnnyLod): THREE.Float32BufferAttribute[] {
  let m = morphCache.get(lod);
  if (!m && lod.expr) {
    m = lod.expr.names.map((n, i) => {
      const a = new THREE.Float32BufferAttribute(denseDelta(lod.expr!, i, lod.V), 3);
      a.name = n;
      return a;
    });
    morphCache.set(lod, m);
  }
  return m ?? [];
}

export function phenotypesOf(g: Genome): number[] {
  return [0, 1, 2, 3, 4, 5].map((i) => pheno(g, i));
}

export function faceOf(g: Genome): [number, number][] {
  return g.face.map(([t, v]) => [t, v / 127] as [number, number]);
}

/** Body region colors for tiers without garment geometry. */
export function regionColors(cat: Catalog, g: Genome, colors: OutfitColors): RGB[] {
  const bits = coverBits(cat, g);
  const out: RGB[] = [];
  for (let r = 0; r < 16; r++) {
    const bit = 1 << r;
    let c: RGB = colors.skin;
    if (r === R.eye) c = colors.eye;
    if (bits.hair & bit) c = colors.hair;
    if (bits.bottom & bit) c = colors.bottom;
    if (bits.top & bit) c = colors.top;
    if (bits.shoes & bit) c = colors.shoes;
    out.push(c);
  }
  return out;
}

export function buildCharacter(
  base: AnnyBase,
  genome: Genome | string,
  tier: Tier,
  opts: { outline?: boolean; cat?: Catalog } = {},
): CharacterRig {
  const cat = opts.cat ?? catalog;
  const g = typeof genome === 'string' ? decodeGenome(genome) : genome;
  const lod = base.lods['1229'];
  const shape = buildBody(base, '1229', phenotypesOf(g), faceOf(g));
  const normals = vertexNormals(shape.positions, lod.indices, lod.V);
  const head = measureHead(lod, shape);
  const regions = vertexRegions(base, lod, shape, head);
  const colors = outfitColors(cat, g);

  const bones = buildBones(base, shape);
  const root = new THREE.Group();
  root.name = 'resident';
  root.add(bones[0]);
  root.updateMatrixWorld(true);
  const skeleton = new THREE.Skeleton(bones);
  const byName = new Map(bones.map((b) => [b.name, b]));

  // outfit first: it decides which body faces are hidden
  let hidden = 0;
  const builder = new MeshBuilder();
  if (tier !== 'reduced') {
    hidden = buildOutfit({ cat, lod, body: shape, normals, regions, head, genome: g, geometry: true }, builder, colors);
  }

  // body colors: skin and eyes (full/export), plus clothing regions (reduced)
  const regionRGB: RGB[] =
    tier === 'reduced'
      ? regionColors(cat, g, colors)
      : Array.from({ length: 16 }, (_, r) => (r === R.eye ? colors.eye : colors.skin));
  const faces: number[] = [];
  const ix = lod.indices;
  for (let f = 0; f < ix.length; f += 3) {
    const a = ix[f], b = ix[f + 1], c = ix[f + 2];
    const h = (hidden >> regions[a]) & (hidden >> regions[b]) & (hidden >> regions[c]) & 1;
    if (!h) faces.push(a, b, c);
  }
  const geo = new THREE.BufferGeometry();
  geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(shape.positions), 3));
  geo.setAttribute('normal', new THREE.BufferAttribute(normals, 3));
  geo.setAttribute('skinIndex', new THREE.BufferAttribute(Uint16Array.from(lod.joints), 4));
  geo.setAttribute('skinWeight', new THREE.BufferAttribute(new Float32Array(lod.weights), 4));

  let bodyMat: THREE.Material | THREE.Material[];
  let exportColors: CharacterRig['exportColors'];
  if (tier === 'export') {
    // one group per color (skin, eye)
    const groups = new Map<number, number[]>();
    const palette: RGB[] = [];
    for (let f = 0; f < faces.length; f += 3) {
      const rgb = regionRGB[regions[faces[f]]];
      let id = palette.indexOf(rgb);
      if (id < 0) { palette.push(rgb); id = palette.length - 1; }
      let l = groups.get(id);
      if (!l) groups.set(id, (l = []));
      l.push(faces[f], faces[f + 1], faces[f + 2]);
    }
    const idx: number[] = [];
    const mats: THREE.Material[] = [];
    const bodyColors: RGB[] = [];
    for (const [id, l] of groups) {
      geo.addGroup(idx.length, l.length, mats.length);
      for (const v of l) idx.push(v);
      mats.push(exportMaterial(palette[id], id === palette.indexOf(colors.eye) ? 'eye' : 'skin'));
      bodyColors.push(palette[id]);
    }
    geo.setIndex(idx);
    bodyMat = mats;
    exportColors = { body: bodyColors, outfit: [] };
  } else {
    const lin = regionRGB.map(srgbToLinear);
    const col = new Float32Array(lod.V * 3);
    for (let i = 0; i < lod.V; i++) {
      const c = lin[regions[i]];
      col[i * 3] = c[0]; col[i * 3 + 1] = c[1]; col[i * 3 + 2] = c[2];
    }
    geo.setAttribute('color', new THREE.BufferAttribute(col, 3));
    geo.setIndex(new THREE.BufferAttribute(Uint16Array.from(faces), 1));
    bodyMat = toonMaterial();
  }
  const expressions: string[] = [];
  if (tier !== 'reduced' && lod.expr) {
    geo.morphAttributes.position = expressionMorphs(lod);
    geo.morphTargetsRelative = true;
    expressions.push(...lod.expr.names);
  }
  // not computeBoundingSphere(): it would walk all 52 morph targets
  geo.boundingSphere = new THREE.Sphere(new THREE.Vector3(0, shape.height / 2, 0), shape.height);
  const body = new THREE.SkinnedMesh(geo, bodyMat);
  body.name = 'Body';
  body.frustumCulled = false;
  if (expressions.length) {
    body.updateMorphTargets();
    body.morphTargetDictionary = Object.fromEntries(expressions.map((n, i) => [n, i]));
    body.morphTargetInfluences = new Array(expressions.length).fill(0);
  }
  root.add(body);
  body.bind(skeleton);

  let outfit: THREE.SkinnedMesh | null = null;
  if (builder.vertexCount) {
    if (tier === 'export') {
      const { geometry, groupColors } = builder.buildGrouped();
      const mats = groupColors.map((c) => exportMaterial(builder.palette[c], builder.paletteNames[c] || 'cloth'));
      outfit = new THREE.SkinnedMesh(geometry, mats);
      exportColors!.outfit = groupColors.map((c) => builder.palette[c]);
    } else {
      outfit = new THREE.SkinnedMesh(builder.build(srgbToLinear), toonMaterial({ doubleSide: true }));
    }
    outfit.name = 'Outfit';
    outfit.frustumCulled = false;
    root.add(outfit);
    outfit.bind(skeleton);
  }

  const outlines: THREE.SkinnedMesh[] = [];
  if (tier === 'full' && opts.outline !== false) {
    const om = outlineMaterial();
    for (const m of [body, outfit]) {
      if (!m) continue;
      const o = new THREE.SkinnedMesh(m.geometry, om);
      o.name = m.name + 'Outline';
      o.frustumCulled = false;
      root.add(o);
      o.bind(skeleton);
      outlines.push(o);
    }
  }

  const licenses = pieceLicenses(cat, g);
  return {
    tier, genome: g, root, bones, byName, skeleton, body, outfit, outlines, shape, head,
    hipsRest: bones[0].position.clone(),
    expressions, licenses, exportColors,
    dispose() {
      body.geometry.morphAttributes = {};
      body.geometry.dispose();
      outfit?.geometry.dispose();
      const mats = new Set<THREE.Material>();
      for (const m of [body, outfit, ...outlines]) {
        if (!m) continue;
        (Array.isArray(m.material) ? m.material : [m.material]).forEach((x) => mats.add(x));
      }
      mats.forEach((m) => m.dispose());
      skeleton.dispose();
      root.removeFromParent();
    },
  };
}

function exportMaterial(rgb: RGB, name: string): THREE.MeshStandardMaterial {
  const m = new THREE.MeshStandardMaterial({ color: new THREE.Color(...srgbToLinear(rgb)), roughness: 1, metalness: 0 });
  m.name = name;
  return m;
}

export function pieceLicenses(cat: Catalog, g: Genome): { piece: string; license: string }[] {
  const out = [{ piece: 'body: Anny (NAVER) with MakeHuman/MPFB2 data', license: 'CC0-1.0' }];
  const add = (slot: string, p?: { id: string; license?: string }) => {
    if (p && p.id !== 'none' && p.id !== 'barefoot') out.push({ piece: `${slot}: ${p.id} (OmniGet procedural)`, license: p.license ?? 'CC0-1.0' });
  };
  add('top', cat.tops[g.top]);
  add('bottom', cat.bottoms[g.bottom]);
  add('shoes', cat.shoes[g.shoes]);
  add('accessory', cat.accessories[g.accessory]);
  const hair = cat.hair_styles[g.hair];
  if (hair && hair.shape !== 'none') out.push({ piece: `hair: ${hair.id} (OmniGet procedural)`, license: 'CC0-1.0' });
  return out;
}
