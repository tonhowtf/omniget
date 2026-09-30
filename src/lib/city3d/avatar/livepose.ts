// Live pose (webcam or remote player), estudo 13 §4.2. `LivePose` is the
// decoded form that `applyPose(id, pose)` takes; `encodePoseFrame` /
// `decodePoseFrame` are the 64-80 byte wire format (smallest-three
// quaternions with 10 bits per component).
import * as THREE from 'three';
import type { VrmBone } from './rig';

export interface LivePose {
  /** local rotations on the normalized VRM rig (identity = T-pose), x y z w */
  bones?: Partial<Record<VrmBone, [number, number, number, number]>>;
  /** hips offset from rest, meters */
  hips?: [number, number, number];
  /** VRM presets (aa ih ou ee oh blink blinkLeft blinkRight happy angry sad
   * relaxed surprised) or ARKit blendshape names, 0..1 */
  expressions?: Record<string, number>;
  /** gaze yaw and pitch, radians */
  look?: [number, number];
  /** finger curls 0..1: left thumb..little, right thumb..little */
  fingers?: number[];
  /** sender timestamp, ms */
  t?: number;
}

/** The 11 bones of the stream, in wire order. */
export const STREAM_BONES: VrmBone[] = [
  'head', 'neck', 'chest', 'spine', 'hips',
  'leftUpperArm', 'leftLowerArm', 'leftHand', 'rightUpperArm', 'rightLowerArm', 'rightHand',
];
/** The 12 expression weights of the stream, in wire order. */
export const STREAM_EXPRESSIONS = ['aa', 'ih', 'ou', 'ee', 'oh', 'blinkLeft', 'blinkRight', 'happy', 'angry', 'sad', 'relaxed', 'surprised'];

const F_BONES = 1, F_HIPS = 2, F_FACE = 4, F_FINGERS = 8;

function packQuat(q: [number, number, number, number]): number {
  const a = q.map(Math.abs);
  let big = 0;
  for (let i = 1; i < 4; i++) if (a[i] > a[big]) big = i;
  const s = q[big] < 0 ? -1 : 1;
  let bits = big;
  let shift = 2;
  for (let i = 0; i < 4; i++) {
    if (i === big) continue;
    const v = (q[i] * s) / Math.SQRT2 + 0.5; // [-1/sqrt2, 1/sqrt2] -> [0, 1]
    bits |= Math.max(0, Math.min(1023, Math.round(v * 1023))) << shift;
    shift += 10;
  }
  return bits >>> 0;
}

function unpackQuat(bits: number): [number, number, number, number] {
  const big = bits & 3;
  const out = [0, 0, 0, 0] as [number, number, number, number];
  let shift = 2;
  let sum = 0;
  for (let i = 0; i < 4; i++) {
    if (i === big) continue;
    const v = (((bits >>> shift) & 1023) / 1023 - 0.5) * 2 * Math.SQRT1_2;
    out[i] = v;
    sum += v * v;
    shift += 10;
  }
  out[big] = Math.sqrt(Math.max(0, 1 - sum));
  return out;
}

export function encodePoseFrame(actor: number, frame: number, pose: LivePose): Uint8Array {
  let flags = 0;
  if (pose.bones) flags |= F_BONES;
  if (pose.hips) flags |= F_HIPS;
  if (pose.expressions || pose.look) flags |= F_FACE;
  if (pose.fingers) flags |= F_FINGERS;
  const size = 6 + (flags & F_BONES ? 44 : 0) + (flags & F_HIPS ? 6 : 0) + (flags & F_FACE ? 14 : 0) + (flags & F_FINGERS ? 10 : 0);
  const buf = new DataView(new ArrayBuffer(size));
  buf.setUint16(0, actor, true);
  buf.setUint16(2, frame & 0xffff, true);
  buf.setUint16(4, flags, true);
  let o = 6;
  if (flags & F_BONES) {
    for (const b of STREAM_BONES) {
      buf.setUint32(o, packQuat(pose.bones?.[b] ?? [0, 0, 0, 1]), true);
      o += 4;
    }
  }
  if (flags & F_HIPS) {
    for (let k = 0; k < 3; k++) { buf.setInt16(o, Math.round((pose.hips![k] ?? 0) * 1000), true); o += 2; }
  }
  if (flags & F_FACE) {
    for (const e of STREAM_EXPRESSIONS) buf.setUint8(o++, Math.round(Math.max(0, Math.min(1, pose.expressions?.[e] ?? 0)) * 255));
    for (let k = 0; k < 2; k++) buf.setInt8(o++, Math.round(Math.max(-1, Math.min(1, (pose.look?.[k] ?? 0) / Math.PI)) * 127));
  }
  if (flags & F_FINGERS) {
    for (let k = 0; k < 10; k++) buf.setUint8(o++, Math.round(Math.max(0, Math.min(1, pose.fingers?.[k] ?? 0)) * 255));
  }
  return new Uint8Array(buf.buffer);
}

export function decodePoseFrame(bytes: Uint8Array): { actor: number; frame: number; pose: LivePose } {
  const buf = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const actor = buf.getUint16(0, true);
  const frame = buf.getUint16(2, true);
  const flags = buf.getUint16(4, true);
  const pose: LivePose = {};
  let o = 6;
  if (flags & F_BONES) {
    pose.bones = {};
    for (const b of STREAM_BONES) { pose.bones[b] = unpackQuat(buf.getUint32(o, true)); o += 4; }
  }
  if (flags & F_HIPS) {
    pose.hips = [0, 0, 0];
    for (let k = 0; k < 3; k++) { pose.hips[k] = buf.getInt16(o, true) / 1000; o += 2; }
  }
  if (flags & F_FACE) {
    pose.expressions = {};
    for (const e of STREAM_EXPRESSIONS) pose.expressions[e] = buf.getUint8(o++) / 255;
    pose.look = [(buf.getInt8(o++) / 127) * Math.PI, (buf.getInt8(o++) / 127) * Math.PI];
  }
  if (flags & F_FINGERS) {
    pose.fingers = [];
    for (let k = 0; k < 10; k++) pose.fingers.push(buf.getUint8(o++) / 255);
  }
  return { actor, frame, pose };
}

/** Finger curls to bone rotations (same axes as posecode `curl`). */
export function fingerRotations(curls: number[]): [VrmBone, THREE.Quaternion][] {
  const out: [VrmBone, THREE.Quaternion][] = [];
  const names = ['Thumb', 'Index', 'Middle', 'Ring', 'Little'];
  ['left', 'right'].forEach((side, s) => {
    const L = side === 'left' ? 1 : -1;
    names.forEach((F, f) => {
      const c = (curls[s * 5 + f] ?? 0) * (F === 'Thumb' ? 60 : 90) * THREE.MathUtils.DEG2RAD;
      const segs = F === 'Thumb' ? ['Metacarpal', 'Proximal', 'Distal'] : ['Proximal', 'Intermediate', 'Distal'];
      for (const seg of segs) {
        const q = F === 'Thumb'
          ? new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 1, 0), -L * c * 0.6)
          : new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 0, 1), -L * c);
        out.push([`${side}${F}${seg}` as VrmBone, q]);
      }
    });
  });
  return out;
}
