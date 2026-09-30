// Skeleton, vertex regions and head measurements of a built Anny body.
import * as THREE from 'three';
import type { AnnyBase, AnnyLod, BodyShape } from './anny';

export const VRM_BONES = [
  'hips', 'spine', 'chest', 'upperChest', 'neck', 'head', 'leftEye', 'rightEye',
  'leftUpperLeg', 'leftLowerLeg', 'leftFoot', 'leftShoulder', 'leftUpperArm', 'leftLowerArm', 'leftHand',
  'leftThumbMetacarpal', 'leftThumbProximal', 'leftThumbDistal',
  'leftIndexProximal', 'leftIndexIntermediate', 'leftIndexDistal',
  'leftMiddleProximal', 'leftMiddleIntermediate', 'leftMiddleDistal',
  'leftRingProximal', 'leftRingIntermediate', 'leftRingDistal',
  'leftLittleProximal', 'leftLittleIntermediate', 'leftLittleDistal',
  'rightUpperLeg', 'rightLowerLeg', 'rightFoot', 'rightShoulder', 'rightUpperArm', 'rightLowerArm', 'rightHand',
  'rightThumbMetacarpal', 'rightThumbProximal', 'rightThumbDistal',
  'rightIndexProximal', 'rightIndexIntermediate', 'rightIndexDistal',
  'rightMiddleProximal', 'rightMiddleIntermediate', 'rightMiddleDistal',
  'rightRingProximal', 'rightRingIntermediate', 'rightRingDistal',
  'rightLittleProximal', 'rightLittleIntermediate', 'rightLittleDistal',
] as const;
export type VrmBone = (typeof VRM_BONES)[number];

// Vertex regions (16), shared by the clothing masks of every tier.
export const R = {
  head: 0, scalp: 1, eye: 2, neck: 3, torso: 4, hipsHigh: 5, hipsLow: 6,
  upperArmHigh: 7, upperArmLow: 8, lowerArm: 9, hand: 10,
  upperLegHigh: 11, upperLegLow: 12, lowerLegHigh: 13, lowerLegLow: 14, foot: 15,
} as const;

/** Catalog cover names to region bits. */
export const COVER_BITS: Record<string, number[]> = {
  torso: [R.torso], neckLow: [R.neck], upperArmHigh: [R.upperArmHigh],
  upperArm: [R.upperArmHigh, R.upperArmLow], lowerArm: [R.lowerArm], hand: [R.hand],
  hipsHigh: [R.hipsHigh], hips: [R.hipsHigh, R.hipsLow],
  upperLegHigh: [R.upperLegHigh], upperLeg: [R.upperLegHigh, R.upperLegLow],
  lowerLegLow: [R.lowerLegLow], lowerLeg: [R.lowerLegHigh, R.lowerLegLow],
  foot: [R.foot], footSole: [R.foot],
};

export function coverMask(cover: string[]): number {
  let m = 0;
  for (const c of cover) for (const b of COVER_BITS[c] ?? []) m |= 1 << b;
  return m;
}

export interface HeadInfo {
  top: number; chin: number; eyeY: number; eyeX: number; eyeZ: number;
  centerZ: number; front: number; back: number; halfWidth: number;
  /** y of the neck joint */
  neckY: number;
}

export function jointOf(body: BodyShape, i: number): THREE.Vector3 {
  return new THREE.Vector3(body.joints[i * 3], body.joints[i * 3 + 1], body.joints[i * 3 + 2]);
}

export function boneIndex(name: VrmBone): number {
  return VRM_BONES.indexOf(name);
}

export function measureHead(lod: AnnyLod, body: BodyShape): HeadInfo {
  const p = body.positions;
  let top = -Infinity, chin = Infinity, front = -Infinity, back = Infinity, hw = 0;
  for (let i = 0; i < lod.V; i++) {
    if (lod.part[i] !== 0 && lod.part[i] !== 10) continue;
    const x = p[i * 3], y = p[i * 3 + 1], z = p[i * 3 + 2];
    top = Math.max(top, y); chin = Math.min(chin, y);
    front = Math.max(front, z); back = Math.min(back, z);
    hw = Math.max(hw, Math.abs(x));
  }
  const le = jointOf(body, boneIndex('leftEye'));
  const head = jointOf(body, boneIndex('head'));
  const neck = jointOf(body, boneIndex('neck'));
  return {
    top, chin, front, back, halfWidth: hw,
    eyeY: le.y, eyeX: Math.abs(le.x), eyeZ: le.z,
    centerZ: (head.z + (front + back) / 2) / 2,
    neckY: neck.y,
  };
}

/** Default hairline test (used by the region map and the cap builder). */
export function isScalp(h: HeadInfo, x: number, y: number, z: number, cut = 0, fringe = 0): boolean {
  const r = Math.max(1e-3, (h.front - h.back) / 2);
  const f = Math.max(-1, Math.min(1, (z - h.centerZ) / r));
  const yFront = h.eyeY + 0.5 * (h.top - h.eyeY) - fringe;
  const ySide = h.eyeY + 0.35 * (h.top - h.eyeY) + cut * 0.3;
  const yBack = h.eyeY - 0.035 + cut;
  const thr = f > 0.25 ? ySide + (yFront - ySide) * ((f - 0.25) / 0.75) : f < 0 ? ySide + (yBack - ySide) * -f : ySide;
  void x;
  return y > thr;
}

/** Region (0..15) of every vertex. */
export function vertexRegions(base: AnnyBase, lod: AnnyLod, body: BodyShape, head: HeadInfo): Uint8Array {
  const out = new Uint8Array(lod.V);
  const hipsY = body.joints[boneIndex('hips') * 3 + 1];
  const p = body.positions;
  void base;
  for (let i = 0; i < lod.V; i++) {
    const a = lod.along[i];
    const x = p[i * 3], y = p[i * 3 + 1], z = p[i * 3 + 2];
    let r: number;
    switch (lod.part[i]) {
      case 0: r = isScalp(head, x, y, z) ? R.scalp : R.head; break;
      case 1: r = R.neck; break;
      case 2: r = R.torso; break;
      case 3: r = y > hipsY ? R.hipsHigh : R.hipsLow; break;
      case 4: r = a < 0.5 ? R.upperArmHigh : R.upperArmLow; break;
      case 5: r = R.lowerArm; break;
      case 6: r = R.hand; break;
      case 7: r = a < 0.45 ? R.upperLegHigh : R.upperLegLow; break;
      case 8: r = a < 0.6 ? R.lowerLegHigh : R.lowerLegLow; break;
      case 9: r = R.foot; break;
      case 10: r = R.eye; break;
      default: r = R.torso;
    }
    out[i] = r;
  }
  return out;
}

export function vertexNormals(positions: Float32Array, indices: ArrayLike<number>, V: number): Float32Array {
  const n = new Float32Array(V * 3);
  for (let f = 0; f < indices.length; f += 3) {
    const a = indices[f] * 3, b = indices[f + 1] * 3, c = indices[f + 2] * 3;
    const e1x = positions[b] - positions[a], e1y = positions[b + 1] - positions[a + 1], e1z = positions[b + 2] - positions[a + 2];
    const e2x = positions[c] - positions[a], e2y = positions[c + 1] - positions[a + 1], e2z = positions[c + 2] - positions[a + 2];
    const nx = e1y * e2z - e1z * e2y, ny = e1z * e2x - e1x * e2z, nz = e1x * e2y - e1y * e2x;
    for (const v of [a, b, c]) { n[v] += nx; n[v + 1] += ny; n[v + 2] += nz; }
  }
  for (let i = 0; i < V * 3; i += 3) {
    const l = Math.hypot(n[i], n[i + 1], n[i + 2]) || 1;
    n[i] /= l; n[i + 1] /= l; n[i + 2] /= l;
  }
  return n;
}

/** Bones in T-pose (identity rest rotations, VRM convention). */
export function buildBones(base: AnnyBase, body: BodyShape): THREE.Bone[] {
  const bones = base.bones.map((b) => {
    const bone = new THREE.Bone();
    bone.name = b.name;
    return bone;
  });
  base.bones.forEach((b, i) => {
    const h = jointOf(body, i);
    if (b.parent >= 0) {
      h.sub(jointOf(body, b.parent));
      bones[b.parent].add(bones[i]);
    }
    bones[i].position.copy(h);
  });
  return bones;
}
