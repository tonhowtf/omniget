// The live pose format (estudo 13 §4.2): what the webcam solver, the gesture
// player and the network all speak, and what the character system's
// `applyPose(id, pose)` receives.
//
// Bones are local rotations in the three-vrm *normalized* humanoid: rest pose
// is the T-pose, the model faces +Z, its left is +X, and every bone's rest
// rotation is identity. Expressions are 0..1 weights with VRM preset names;
// the full ARKit set rides along only for "perfect sync" avatars.
//
// Wire frame (little endian), 64 bytes for the usual upper body + face:
//   u16 actor, u16 seq, u16 mask
//   11 × u32 upper-body bones ("smallest three", 2 + 3×10 bits), if the bit is set
//   6 × u32 leg bones                         (mask bit 11)
//   3 × i16 hips offset in millimetres          (mask bit 12)
//   12 × u8 expressions + 2 × i8 gaze           (mask bit 15)
//   10 × u8 finger curls                        (mask bit 13)
//   52 × 6 bits ARKit blendshapes, 39 bytes     (mask bit 14)

import { clamp, lerp, qNorm, qSlerp, type Quat, type Vec3 } from './math';

export type { Quat, Vec3 } from './math';

/** Upper-body bones, in wire order (mask bits 0..10). */
export const UPPER_BONES = [
  'hips',
  'spine',
  'chest',
  'neck',
  'head',
  'leftUpperArm',
  'leftLowerArm',
  'leftHand',
  'rightUpperArm',
  'rightLowerArm',
  'rightHand',
] as const;

/** Leg bones, sent as one block (mask bit 11). */
export const LEG_BONES = [
  'leftUpperLeg',
  'leftLowerLeg',
  'leftFoot',
  'rightUpperLeg',
  'rightLowerLeg',
  'rightFoot',
] as const;

export const POSE_BONES = [...UPPER_BONES, ...LEG_BONES] as const;
export type PoseBone = (typeof POSE_BONES)[number];

/** VRM preset expressions carried on the wire, in wire order. */
export const EXPRESSIONS = [
  'aa',
  'ih',
  'ou',
  'ee',
  'oh',
  'blinkLeft',
  'blinkRight',
  'happy',
  'angry',
  'sad',
  'relaxed',
  'surprised',
] as const;
export type ExpressionName = (typeof EXPRESSIONS)[number];

/** Finger curls, 0 = open, 1 = fist. */
export const FINGERS = [
  'leftThumb',
  'leftIndex',
  'leftMiddle',
  'leftRing',
  'leftLittle',
  'rightThumb',
  'rightIndex',
  'rightMiddle',
  'rightRing',
  'rightLittle',
] as const;
export type FingerName = (typeof FINGERS)[number];

/** The 52 ARKit blendshapes (MediaPipe emits all but `tongueOut`). */
export const ARKIT_BLENDSHAPES = [
  'browDownLeft', 'browDownRight', 'browInnerUp', 'browOuterUpLeft', 'browOuterUpRight',
  'cheekPuff', 'cheekSquintLeft', 'cheekSquintRight',
  'eyeBlinkLeft', 'eyeBlinkRight',
  'eyeLookDownLeft', 'eyeLookDownRight', 'eyeLookInLeft', 'eyeLookInRight',
  'eyeLookOutLeft', 'eyeLookOutRight', 'eyeLookUpLeft', 'eyeLookUpRight',
  'eyeSquintLeft', 'eyeSquintRight', 'eyeWideLeft', 'eyeWideRight',
  'jawForward', 'jawLeft', 'jawOpen', 'jawRight',
  'mouthClose', 'mouthDimpleLeft', 'mouthDimpleRight', 'mouthFrownLeft', 'mouthFrownRight',
  'mouthFunnel', 'mouthLeft', 'mouthLowerDownLeft', 'mouthLowerDownRight',
  'mouthPressLeft', 'mouthPressRight', 'mouthPucker', 'mouthRight',
  'mouthRollLower', 'mouthRollUpper', 'mouthShrugLower', 'mouthShrugUpper',
  'mouthSmileLeft', 'mouthSmileRight', 'mouthStretchLeft', 'mouthStretchRight',
  'mouthUpperUpLeft', 'mouthUpperUpRight', 'noseSneerLeft', 'noseSneerRight',
  'tongueOut',
] as const;
export type ArkitName = (typeof ARKIT_BLENDSHAPES)[number];

export interface Gaze {
  /** Radians, positive = the avatar looks to its left (+X). */
  yaw: number;
  /** Radians, positive = up. */
  pitch: number;
}

export interface LivePose {
  /** Sequence number (u16 on the wire). */
  seq: number;
  /** Milliseconds on the producer's clock (or arrival time on a receiver). */
  t: number;
  /** Local rotations in the normalized VRM humanoid. Missing bone = leave it to the animation. */
  bones: Partial<Record<PoseBone, Quat>>;
  /** Hips offset from rest, metres (recorded gestures; the webcam leaves it out). */
  hips?: Vec3;
  expressions: Partial<Record<ExpressionName, number>>;
  gaze?: Gaze;
  fingers?: Partial<Record<FingerName, number>>;
  /** ARKit weights for perfect-sync avatars. */
  blendshapes?: Partial<Record<ArkitName, number>>;
  /** Column-major 4×4 head transform in camera space. Local only, never on the wire. */
  headMatrix?: number[];
}

export const emptyPose = (seq = 0, t = 0): LivePose => ({ seq, t, bones: {}, expressions: {} });

const MASK_LEGS = 1 << 11;
const MASK_HIPS = 1 << 12;
const MASK_FINGERS = 1 << 13;
const MASK_BLEND = 1 << 14;
const MASK_FACE = 1 << 15;

const GAZE_RANGE = Math.PI / 4;
const Q_RANGE = Math.SQRT1_2;

export interface EncodeOptions {
  actor: number;
  /** Send the 52 ARKit weights too (only when receivers support perfect sync). */
  perfectSync?: boolean;
}

/** Smallest-three packing: 2 bits for the dropped index, 3 × 10 bits. */
export function packQuat(q: Quat): number {
  const n = qNorm(q);
  let big = 0;
  for (let i = 1; i < 4; i++) if (Math.abs(n[i]) > Math.abs(n[big])) big = i;
  const sign = n[big] < 0 ? -1 : 1;
  let out = big;
  for (let i = 0; i < 4; i++) {
    if (i === big) continue;
    const v = clamp((n[i] * sign) / Q_RANGE, -1, 1);
    out = out * 1024 + Math.round((v * 0.5 + 0.5) * 1023);
  }
  return out >>> 0;
}

export function unpackQuat(word: number): Quat {
  const big = word >>> 30;
  const vals = [(word >>> 20) & 1023, (word >>> 10) & 1023, word & 1023].map(
    (v) => ((v / 1023) * 2 - 1) * Q_RANGE,
  );
  const q: number[] = [];
  let k = 0;
  let sum = 0;
  for (let i = 0; i < 4; i++) {
    if (i === big) {
      q.push(0);
      continue;
    }
    const v = vals[k++];
    q.push(v);
    sum += v * v;
  }
  q[big] = Math.sqrt(Math.max(0, 1 - sum));
  return qNorm(q as Quat);
}

const u8 = (v: number) => Math.round(clamp(v, 0, 1) * 255);
const i8 = (v: number) => Math.round(clamp(v, -1, 1) * 127);

/** Serializes a pose. See the header of this file for the layout. */
export function encodePose(pose: LivePose, opts: EncodeOptions): Uint8Array {
  let mask = 0;
  UPPER_BONES.forEach((b, i) => {
    if (pose.bones[b]) mask |= 1 << i;
  });
  const hasLegs = LEG_BONES.some((b) => pose.bones[b]);
  if (hasLegs) mask |= MASK_LEGS;
  if (pose.hips) mask |= MASK_HIPS;
  const hasFace = pose.gaze !== undefined || EXPRESSIONS.some((e) => pose.expressions[e] !== undefined);
  if (hasFace) mask |= MASK_FACE;
  if (pose.fingers) mask |= MASK_FINGERS;
  if (opts.perfectSync && pose.blendshapes) mask |= MASK_BLEND;

  let size = 6;
  for (let i = 0; i < UPPER_BONES.length; i++) if (mask & (1 << i)) size += 4;
  if (mask & MASK_LEGS) size += 24;
  if (mask & MASK_HIPS) size += 6;
  if (mask & MASK_FACE) size += 14;
  if (mask & MASK_FINGERS) size += 10;
  if (mask & MASK_BLEND) size += 39;

  const buf = new Uint8Array(size);
  const dv = new DataView(buf.buffer);
  dv.setUint16(0, opts.actor & 0xffff, true);
  dv.setUint16(2, pose.seq & 0xffff, true);
  dv.setUint16(4, mask, true);
  let o = 6;
  UPPER_BONES.forEach((b, i) => {
    if (!(mask & (1 << i))) return;
    dv.setUint32(o, packQuat(pose.bones[b]!), true);
    o += 4;
  });
  if (mask & MASK_LEGS) {
    for (const b of LEG_BONES) {
      dv.setUint32(o, packQuat(pose.bones[b] ?? [0, 0, 0, 1]), true);
      o += 4;
    }
  }
  if (mask & MASK_HIPS) {
    for (let i = 0; i < 3; i++) {
      dv.setInt16(o, clamp(Math.round(pose.hips![i] * 1000), -32768, 32767), true);
      o += 2;
    }
  }
  if (mask & MASK_FACE) {
    for (const e of EXPRESSIONS) buf[o++] = u8(pose.expressions[e] ?? 0);
    dv.setInt8(o++, i8((pose.gaze?.yaw ?? 0) / GAZE_RANGE));
    dv.setInt8(o++, i8((pose.gaze?.pitch ?? 0) / GAZE_RANGE));
  }
  if (mask & MASK_FINGERS) {
    for (const f of FINGERS) buf[o++] = u8(pose.fingers![f] ?? 0);
  }
  if (mask & MASK_BLEND) {
    // 52 × 6 bits, packed LSB-first.
    let acc = 0;
    let bits = 0;
    for (const n of ARKIT_BLENDSHAPES) {
      acc |= Math.round(clamp(pose.blendshapes![n] ?? 0, 0, 1) * 63) << bits;
      bits += 6;
      while (bits >= 8) {
        buf[o++] = acc & 0xff;
        acc >>>= 8;
        bits -= 8;
      }
    }
    if (bits > 0) buf[o++] = acc & 0xff;
  }
  return buf;
}

export interface DecodedPose {
  actor: number;
  pose: LivePose;
}

/** Parses a frame. Throws on a truncated buffer. `t` is set to `now`. */
export function decodePose(bytes: Uint8Array, now = 0): DecodedPose {
  if (bytes.byteLength < 6) throw new Error('pose frame too short');
  const dv = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  const actor = dv.getUint16(0, true);
  const seq = dv.getUint16(2, true);
  const mask = dv.getUint16(4, true);
  const pose = emptyPose(seq, now);
  let o = 6;
  const need = (n: number) => {
    if (o + n > bytes.byteLength) throw new Error('pose frame truncated');
  };
  UPPER_BONES.forEach((b, i) => {
    if (!(mask & (1 << i))) return;
    need(4);
    pose.bones[b] = unpackQuat(dv.getUint32(o, true));
    o += 4;
  });
  if (mask & MASK_LEGS) {
    need(24);
    for (const b of LEG_BONES) {
      pose.bones[b] = unpackQuat(dv.getUint32(o, true));
      o += 4;
    }
  }
  if (mask & MASK_HIPS) {
    need(6);
    pose.hips = [0, 1, 2].map((i) => dv.getInt16(o + i * 2, true) / 1000) as Vec3;
    o += 6;
  }
  if (mask & MASK_FACE) {
    need(14);
    for (const e of EXPRESSIONS) pose.expressions[e] = bytes[o++] / 255;
    pose.gaze = {
      yaw: (dv.getInt8(o++) / 127) * GAZE_RANGE,
      pitch: (dv.getInt8(o++) / 127) * GAZE_RANGE,
    };
  }
  if (mask & MASK_FINGERS) {
    need(10);
    pose.fingers = {};
    for (const f of FINGERS) pose.fingers[f] = bytes[o++] / 255;
  }
  if (mask & MASK_BLEND) {
    need(39);
    pose.blendshapes = {};
    let acc = 0;
    let bits = 0;
    for (const n of ARKIT_BLENDSHAPES) {
      while (bits < 6) {
        acc |= bytes[o++] << bits;
        bits += 8;
      }
      pose.blendshapes[n] = (acc & 63) / 63;
      acc >>>= 6;
      bits -= 6;
    }
  }
  return { actor, pose };
}

/** True when u16 sequence `a` is newer than `b` (wrap-around aware). */
export function seqNewer(a: number, b: number): boolean {
  const d = (a - b) & 0xffff;
  return d !== 0 && d < 0x8000;
}

function mixRecord<K extends string>(
  a: Partial<Record<K, number>> | undefined,
  b: Partial<Record<K, number>> | undefined,
  t: number,
): Partial<Record<K, number>> | undefined {
  if (!a && !b) return undefined;
  const out: Partial<Record<K, number>> = {};
  const keys = new Set<K>([...(Object.keys(a ?? {}) as K[]), ...(Object.keys(b ?? {}) as K[])]);
  for (const k of keys) {
    const va = a?.[k];
    const vb = b?.[k];
    out[k] = va === undefined ? vb : vb === undefined ? va : lerp(va, vb, t);
  }
  return out;
}

/** Slerp on bones, lerp on weights. Bones present in only one side are kept. */
export function interpolatePose(a: LivePose, b: LivePose, t: number): LivePose {
  const bones: Partial<Record<PoseBone, Quat>> = {};
  for (const bone of POSE_BONES) {
    const qa = a.bones[bone];
    const qb = b.bones[bone];
    if (qa && qb) bones[bone] = qSlerp(qa, qb, t);
    else if (qa || qb) bones[bone] = (qa ?? qb)!;
  }
  const out: LivePose = {
    seq: t < 0.5 ? a.seq : b.seq,
    t: lerp(a.t, b.t, t),
    bones,
    expressions: mixRecord(a.expressions, b.expressions, t) ?? {},
  };
  if (a.hips && b.hips) out.hips = [0, 1, 2].map((i) => lerp(a.hips![i], b.hips![i], t)) as Vec3;
  else if (a.hips || b.hips) out.hips = a.hips ?? b.hips;
  if (a.gaze && b.gaze) out.gaze = { yaw: lerp(a.gaze.yaw, b.gaze.yaw, t), pitch: lerp(a.gaze.pitch, b.gaze.pitch, t) };
  else if (a.gaze || b.gaze) out.gaze = a.gaze ?? b.gaze;
  const fingers = mixRecord(a.fingers, b.fingers, t);
  if (fingers) out.fingers = fingers;
  const blend = mixRecord(a.blendshapes, b.blendshapes, t);
  if (blend) out.blendshapes = blend;
  return out;
}

/**
 * Receiver-side jitter buffer: keeps a short history per actor and samples it
 * `delayMs` in the past, so 15 Hz input plays smoothly at the render rate.
 * Frames that arrive with an older sequence than the newest one are dropped.
 */
export class PoseBuffer {
  private frames: LivePose[] = [];
  private newest: number | null = null;
  dropped = 0;

  constructor(
    public delayMs = 120,
    private capacity = 16,
  ) {}

  /** Returns false when the frame was stale and got dropped. */
  push(pose: LivePose): boolean {
    if (this.newest !== null && !seqNewer(pose.seq, this.newest)) {
      this.dropped++;
      return false;
    }
    this.newest = pose.seq;
    this.frames.push(pose);
    if (this.frames.length > this.capacity) this.frames.shift();
    return true;
  }

  latest(): LivePose | null {
    return this.frames[this.frames.length - 1] ?? null;
  }

  /** Pose at `now - delayMs`, interpolated between the two frames around it. */
  sample(now: number): LivePose | null {
    const f = this.frames;
    if (f.length === 0) return null;
    const target = now - this.delayMs;
    if (target <= f[0].t) return f[0];
    for (let i = f.length - 1; i > 0; i--) {
      const a = f[i - 1];
      const b = f[i];
      if (target >= a.t && target <= b.t) {
        const span = b.t - a.t;
        return interpolatePose(a, b, span > 0 ? (target - a.t) / span : 1);
      }
    }
    return f[f.length - 1];
  }

  clear(): void {
    this.frames = [];
    this.newest = null;
  }
}
