// VRM 1.0 exporter: a GLTFExporter plugin that writes VRMC_vrm (humanoid,
// expressions, lookAt, firstPerson, meta) and VRMC_materials_mtoon over the
// character built in the 'export' tier (T-pose, one material per color, the 52
// ARKit morphs). The meta license is computed from the pieces: the most
// restrictive value of each field wins (CharacterStudio's rule).
import * as THREE from 'three';
import { aimTPose } from './tpose';
import { GLTFExporter } from 'three/examples/jsm/exporters/GLTFExporter.js';
import type { AnnyBase } from './anny';
import { buildCharacter, type CharacterRig } from './assemble';
import { VRM_PRESETS } from './expressions';
import type { Genome } from './genome';
import { jointOf, boneIndex, VRM_BONES } from './rig';

export type AvatarPermission = 'onlyAuthor' | 'onlySeparatelyLicensedPerson' | 'everyone';
export type CommercialUsage = 'personalNonProfit' | 'personalProfit' | 'corporation';
export type Modification = 'prohibited' | 'allowModification' | 'allowModificationRedistribution';

export interface VrmPermissions {
  avatarPermission: AvatarPermission;
  commercialUsage: CommercialUsage;
  creditNotation: 'required' | 'unnecessary';
  allowRedistribution: boolean;
  modification: Modification;
  allowExcessivelyViolentUsage: boolean;
  allowExcessivelySexualUsage: boolean;
  allowPoliticalOrReligiousUsage: boolean;
  allowAntisocialOrHateUsage: boolean;
}

export interface VrmMetaInput extends Partial<VrmPermissions> {
  name: string;
  authors: string[];
  version?: string;
  copyrightInformation?: string;
  contactInformation?: string;
}

const ORDER = {
  avatarPermission: ['onlyAuthor', 'onlySeparatelyLicensedPerson', 'everyone'],
  commercialUsage: ['personalNonProfit', 'personalProfit', 'corporation'],
  creditNotation: ['required', 'unnecessary'],
  modification: ['prohibited', 'allowModification', 'allowModificationRedistribution'],
} as const;

const OPEN: VrmPermissions = {
  avatarPermission: 'everyone', commercialUsage: 'corporation', creditNotation: 'unnecessary',
  allowRedistribution: true, modification: 'allowModificationRedistribution',
  allowExcessivelyViolentUsage: false, allowExcessivelySexualUsage: false,
  allowPoliticalOrReligiousUsage: false, allowAntisocialOrHateUsage: false,
};

const RESTRICTED: VrmPermissions = {
  avatarPermission: 'onlyAuthor', commercialUsage: 'personalNonProfit', creditNotation: 'required',
  allowRedistribution: false, modification: 'prohibited',
  allowExcessivelyViolentUsage: false, allowExcessivelySexualUsage: false,
  allowPoliticalOrReligiousUsage: false, allowAntisocialOrHateUsage: false,
};

/** What a piece license allows, in VRM meta terms. Unknown = restricted. */
export function licensePermissions(license: string): VrmPermissions {
  const l = license.toUpperCase();
  if (l.startsWith('CC0') || l === 'PUBLIC-DOMAIN') return OPEN;
  if (l.startsWith('CC-BY-NC')) return { ...OPEN, commercialUsage: 'personalNonProfit', creditNotation: 'required', modification: l.includes('ND') ? 'prohibited' : 'allowModificationRedistribution' };
  if (l.startsWith('CC-BY-ND')) return { ...OPEN, creditNotation: 'required', modification: 'prohibited' };
  if (l.startsWith('CC-BY') || l.startsWith('APACHE') || l.startsWith('MIT') || l.startsWith('GPL')) return { ...OPEN, creditNotation: 'required' };
  return RESTRICTED;
}

/** The most restrictive value of every field wins. */
export function mergePermissions(list: VrmPermissions[]): VrmPermissions {
  const out = { ...OPEN };
  for (const p of list) {
    for (const k of Object.keys(ORDER) as (keyof typeof ORDER)[]) {
      const order = ORDER[k] as readonly string[];
      if (order.indexOf(p[k]) < order.indexOf(out[k])) (out as Record<string, unknown>)[k] = p[k];
    }
    out.allowRedistribution &&= p.allowRedistribution;
    out.allowExcessivelyViolentUsage &&= p.allowExcessivelyViolentUsage;
    out.allowExcessivelySexualUsage &&= p.allowExcessivelySexualUsage;
    out.allowPoliticalOrReligiousUsage &&= p.allowPoliticalOrReligiousUsage;
    out.allowAntisocialOrHateUsage &&= p.allowAntisocialOrHateUsage;
  }
  return out;
}

function mtoon(color: THREE.Color) {
  const c = [color.r, color.g, color.b];
  return {
    specVersion: '1.0',
    transparentWithZWrite: false,
    renderQueueOffsetNumber: 0,
    shadeColorFactor: c.map((v) => v * 0.62),
    shadingShiftFactor: -0.05,
    shadingToonyFactor: 0.92,
    giEqualizationFactor: 0.9,
    matcapFactor: [0, 0, 0],
    parametricRimColorFactor: [0, 0, 0],
    rimLightingMixFactor: 1,
    outlineWidthMode: 'worldCoordinates',
    outlineWidthFactor: 0.0025,
    outlineColorFactor: [0.16, 0.11, 0.09],
    outlineLightingMixFactor: 1,
  };
}

/** GLTFExporter plugin writing the VRM 1.0 extensions for `rig`. */
class VrmPlugin {
  name = 'VRMC_vrm';
  constructor(private writer: any, private rig: CharacterRig, private meta: Record<string, unknown>) {} // eslint-disable-line @typescript-eslint/no-explicit-any

  async writeMaterialAsync(material: THREE.Material, def: Record<string, any>) { // eslint-disable-line @typescript-eslint/no-explicit-any
    const m = material as THREE.MeshStandardMaterial;
    if (!m.color) return;
    def.extensions = def.extensions ?? {};
    def.extensions.VRMC_materials_mtoon = mtoon(m.color);
    this.writer.extensionsUsed.VRMC_materials_mtoon = true;
  }

  afterParse() {
    const w = this.writer;
    const json = w.json;
    const rig = this.rig;
    const node = (o: THREE.Object3D) => w.nodeMap.get(o) as number;
    const humanBones: Record<string, { node: number }> = {};
    for (const name of VRM_BONES) {
      const b = rig.byName.get(name);
      if (b) humanBones[name] = { node: node(b) };
    }
    const bodyNode = node(rig.body);
    const dict = rig.body.morphTargetDictionary ?? {};
    const bind = (weights: Record<string, number>) =>
      Object.entries(weights)
        .filter(([k]) => dict[k] !== undefined)
        .map(([k, v]) => ({ node: bodyNode, index: dict[k], weight: v }));
    const binds = (weights: Record<string, number>) => {
      const list = bind(weights);
      return list.length ? { morphTargetBinds: list } : {};
    };
    const preset: Record<string, unknown> = {};
    for (const [name, weights] of Object.entries(VRM_PRESETS)) {
      preset[name] = {
        ...binds(weights), isBinary: false,
        overrideBlink: ['happy', 'angry', 'sad', 'relaxed', 'surprised'].includes(name) ? 'blend' : 'none',
        overrideLookAt: 'none', overrideMouth: name === 'surprised' ? 'blend' : 'none',
      };
    }
    // the schema asks for minItems 1, so an expression with no binds omits the key
    preset.neutral = { isBinary: false, overrideBlink: 'none', overrideLookAt: 'none', overrideMouth: 'none' };
    const custom: Record<string, unknown> = {};
    for (const name of rig.expressions) {
      custom[name] = { ...binds({ [name]: 1 }), isBinary: false, overrideBlink: 'none', overrideLookAt: 'none', overrideMouth: 'none' };
    }
    const head = jointOf(rig.shape, boneIndex('head'));
    const le = jointOf(rig.shape, boneIndex('leftEye'));
    const re = jointOf(rig.shape, boneIndex('rightEye'));
    const eyes = le.add(re).multiplyScalar(0.5).sub(head);
    json.extensions = json.extensions ?? {};
    json.extensions.VRMC_vrm = {
      specVersion: '1.0',
      meta: this.meta,
      humanoid: { humanBones },
      firstPerson: { meshAnnotations: [{ node: bodyNode, type: 'auto' }, ...(rig.outfit ? [{ node: node(rig.outfit), type: 'auto' }] : [])] },
      lookAt: {
        type: 'bone',
        offsetFromHeadBone: [eyes.x, eyes.y, eyes.z],
        rangeMapHorizontalInner: { inputMaxValue: 90, outputScale: 10 },
        rangeMapHorizontalOuter: { inputMaxValue: 90, outputScale: 12 },
        rangeMapVerticalDown: { inputMaxValue: 90, outputScale: 10 },
        rangeMapVerticalUp: { inputMaxValue: 90, outputScale: 10 },
      },
      expressions: { preset, custom },
    };
    w.extensionsUsed.VRMC_vrm = true;
  }
}

/**
 * The Anny rest keeps the forearm bent ~42 degrees forward, a slight wrist
 * and finger curl and soft knees. VRM 1.0 asks for a T-pose (arms straight
 * along +-X, legs straight, fingers straight), so the export poses those
 * chains, bakes the skinning (positions, normals and morph deltas) into the
 * geometry and makes the posed skeleton the new rest with identity rotations.
 * Thumbs keep their rest angle.
 */
function straightenToTPose(rig: CharacterRig) {
  aimTPose(rig);
  bakeTPose(rig);
}

function bakeTPose(rig: CharacterRig) {
  rig.skeleton.update();

  // bake linear blend skinning: p' = bindInv * sum(w * boneMatrix) * bind * p
  for (const mesh of [rig.body, rig.outfit]) {
    if (!mesh) continue;
    const g = mesh.geometry;
    const pos = g.getAttribute('position');
    const nrm = g.getAttribute('normal');
    const idx = g.getAttribute('skinIndex');
    const wt = g.getAttribute('skinWeight');
    const morphs = g.morphAttributes.position ?? [];
    const outPos = new Float32Array(pos.count * 3);
    const outNrm = nrm ? new Float32Array(nrm.count * 3) : null;
    const outMorph = morphs.map((m) => new Float32Array(m.count * 3));
    const boneMats = rig.skeleton.bones.map((bone, k) =>
      new THREE.Matrix4().multiplyMatrices(mesh.bindMatrixInverse, new THREE.Matrix4().multiplyMatrices(bone.matrixWorld, rig.skeleton.boneInverses[k])).multiply(mesh.bindMatrix));
    const m = new THREE.Matrix4();
    const lin = new THREE.Matrix3();
    const v = new THREE.Vector3();
    for (let i = 0; i < pos.count; i++) {
      const e = m.elements.fill(0);
      for (let c = 0; c < 4; c++) {
        const w = wt.getComponent(i, c);
        if (!w) continue;
        const be = boneMats[idx.getComponent(i, c)].elements;
        for (let k = 0; k < 16; k++) e[k] += w * be[k];
      }
      v.fromBufferAttribute(pos, i).applyMatrix4(m).toArray(outPos, i * 3);
      lin.setFromMatrix4(m);
      if (nrm && outNrm) v.fromBufferAttribute(nrm, i).applyMatrix3(lin).normalize().toArray(outNrm, i * 3);
      morphs.forEach((a, t) => v.fromBufferAttribute(a, i).applyMatrix3(lin).toArray(outMorph[t], i * 3));
    }
    // fresh attributes: the morph targets are shared with other rigs of the same LOD
    g.setAttribute('position', new THREE.BufferAttribute(outPos, 3));
    if (outNrm) g.setAttribute('normal', new THREE.BufferAttribute(outNrm, 3));
    if (morphs.length) {
      g.morphAttributes.position = morphs.map((a, t) => {
        const n = new THREE.Float32BufferAttribute(outMorph[t], 3);
        n.name = a.name;
        return n;
      });
    }
  }

  // the posed skeleton becomes the rest: identity rotations, joints where they are now
  const world = rig.bones.map((bone) => bone.getWorldPosition(new THREE.Vector3()));
  const rootInv = rig.root.matrixWorld.clone().invert();
  rig.bones.forEach((bone, k) => {
    const p = world[k].clone().applyMatrix4(rootInv);
    const parent = bone.parent as THREE.Bone;
    const pk = rig.bones.indexOf(parent);
    if (pk >= 0) p.sub(world[pk].clone().applyMatrix4(rootInv));
    bone.position.copy(p);
    bone.quaternion.identity();
  });
  rig.root.updateMatrixWorld(true);
  rig.skeleton.calculateInverses();
  for (const mesh of [rig.body, rig.outfit]) mesh?.bind(rig.skeleton, mesh.matrixWorld);
}

/**
 * Cleans what the Khronos and VRM validators flag: nodes kept consistent for TRS
 * instead of `matrix`, skinned meshes moved to the scene root (the root has an
 * identity transform, so nothing moves), and joint indices zeroed where their
 * weight is zero (ACCESSOR_JOINTS_USED_ZERO_WEIGHT).
 */
function prepareForGltf(scene: THREE.Scene, root: THREE.Object3D) {
  const skinned: THREE.SkinnedMesh[] = [];
  root.traverse((o) => {
    o.matrixAutoUpdate = true;
    o.matrix.decompose(o.position, o.quaternion, o.scale);
    if ((o as THREE.SkinnedMesh).isSkinnedMesh) skinned.push(o as THREE.SkinnedMesh);
  });
  for (const m of skinned) {
    scene.attach(m);
    const idx = m.geometry.getAttribute('skinIndex');
    const wt = m.geometry.getAttribute('skinWeight');
    if (!idx || !wt) continue;
    for (let i = 0; i < idx.count; i++) {
      for (let c = 0; c < idx.itemSize; c++) {
        if (wt.getComponent(i, c) === 0 && idx.getComponent(i, c) !== 0) idx.setComponent(i, c, 0);
      }
    }
    idx.needsUpdate = true;
  }
  scene.updateMatrixWorld(true);
}

export interface ExportResult {
  bytes: ArrayBuffer;
  meta: Record<string, unknown>;
  permissions: VrmPermissions;
}

/** Builds the avatar in T-pose and writes a `.vrm` (GLB). */
export async function exportVrm(base: AnnyBase, genome: Genome | string, input: VrmMetaInput): Promise<ExportResult> {
  const rig = buildCharacter(base, genome, 'export');
  try {
    const perms = mergePermissions([
      ...rig.licenses.map((l) => licensePermissions(l.license)),
      // the author's own choices count as one more piece
      { ...OPEN, ...Object.fromEntries(Object.entries(input).filter(([k]) => k in OPEN)) } as VrmPermissions,
    ]);
    const meta = {
      name: input.name || 'OmniGet avatar',
      version: input.version ?? '1',
      authors: input.authors.length ? input.authors : ['OmniGet user'],
      copyrightInformation: input.copyrightInformation ?? '',
      contactInformation: input.contactInformation ?? '',
      licenseUrl: 'https://vrm.dev/licenses/1.0/',
      thirdPartyLicenses: rig.licenses.map((l) => `${l.piece}: ${l.license}`).join('\n') +
        '\nBody: Anny by NAVER (Apache-2.0 code) with MakeHuman/MPFB2 and Face Units data (CC0-1.0).',
      ...perms,
    };
    const scene = new THREE.Scene();
    rig.root.name = 'Root';
    scene.add(rig.root);
    rig.root.position.set(0, 0, 0);
    rig.root.rotation.set(0, 0, 0);
    for (const b of rig.bones) b.quaternion.identity();
    rig.bones[0].position.copy(rig.hipsRest);
    rig.root.updateMatrixWorld(true);
    straightenToTPose(rig);
    prepareForGltf(scene, rig.root);
    const exporter = new GLTFExporter();
    exporter.register((writer) => new VrmPlugin(writer, rig, meta) as never);
    const bytes = (await exporter.parseAsync(scene, { binary: true, onlyVisible: false, trs: true })) as ArrayBuffer;
    return { bytes, meta, permissions: perms };
  } finally {
    rig.dispose();
  }
}
