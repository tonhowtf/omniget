// Joint vocabulary for poses and gestures (estudo 13 §3-4): anatomical actions
// in degrees, the posecode idea, clamped per joint, turned into bone
// quaternions on the VRM rig. Rest is the VRM T-pose (identity rotations,
// facing +Z, left at +X); angles are measured from the anatomical neutral
// (standing, arms down), so `shoulders: { abduct: 90 }` is the T-pose arm.
import * as THREE from 'three';
import type { VrmBone } from './rig';

export const ACTIONS = [
  'flex', 'extend', 'abduct', 'adduct', 'rotate-in', 'rotate-out',
  'twist-left', 'twist-right', 'tilt-left', 'tilt-right', 'curl', 'elevate',
] as const;
export type Action = (typeof ACTIONS)[number];

type Limits = Partial<Record<Action, number>>;
/** Maximum angle per joint and action (degrees). */
export const JOINT_LIMITS: Record<string, Limits> = {
  pelvis: { flex: 100, extend: 95, 'twist-left': 180, 'twist-right': 180, 'tilt-left': 30, 'tilt-right': 30 },
  spine: { flex: 45, extend: 25, 'twist-left': 30, 'twist-right': 30, 'tilt-left': 25, 'tilt-right': 25 },
  chest: { flex: 30, extend: 20, 'twist-left': 25, 'twist-right': 25, 'tilt-left': 20, 'tilt-right': 20 },
  neck: { flex: 45, extend: 40, 'twist-left': 60, 'twist-right': 60, 'tilt-left': 35, 'tilt-right': 35 },
  head: { flex: 30, extend: 30, 'twist-left': 30, 'twist-right': 30, 'tilt-left': 20, 'tilt-right': 20 },
  hip: { flex: 125, extend: 30, abduct: 45, adduct: 30, 'rotate-in': 40, 'rotate-out': 45 },
  knee: { flex: 150 },
  ankle: { flex: 25, extend: 50 },
  clavicle: { elevate: 30 },
  shoulder: { flex: 180, extend: 50, abduct: 180, adduct: 30, 'rotate-in': 70, 'rotate-out': 90 },
  elbow: { flex: 150 },
  wrist: { flex: 80, extend: 70, 'tilt-left': 25, 'tilt-right': 25 },
  fingers: { curl: 90 },
  thumb: { curl: 60 },
  index: { curl: 90 }, middle: { curl: 90 }, ring: { curl: 90 }, little: { curl: 90 },
};

/** Accepted joint names: singular with side, or plural for both sides. */
const SIDED: Record<string, string> = {
  hip: 'hip', knee: 'knee', ankle: 'ankle', clavicle: 'clavicle', shoulder: 'shoulder',
  elbow: 'elbow', wrist: 'wrist', fingers: 'fingers', thumb: 'thumb', index: 'index',
  middle: 'middle', ring: 'ring', little: 'little',
};
const PLURAL: Record<string, string> = {
  hips: 'hip', knees: 'knee', ankles: 'ankle', clavicles: 'clavicle', shoulders: 'shoulder',
  elbows: 'elbow', wrists: 'wrist', thumbs: 'thumb',
};

export type JointPose = Record<string, Partial<Record<Action, number>>>;

interface Resolved { joint: string; side: 'left' | 'right' | null; action: Action; deg: number }

/** Validates and clamps a pose. Unknown words are errors, never ignored. */
export function resolvePose(pose: JointPose): { items: Resolved[]; errors: string[] } {
  const items: Resolved[] = [];
  const errors: string[] = [];
  for (const [name, acts] of Object.entries(pose)) {
    let joint: string | undefined;
    let sides: ('left' | 'right' | null)[] = [null];
    const m = /^([a-z]+)_(left|right)$/.exec(name);
    if (m && SIDED[m[1]]) { joint = SIDED[m[1]]; sides = [m[2] as 'left' | 'right']; }
    else if (PLURAL[name]) { joint = PLURAL[name]; sides = ['left', 'right']; }
    else if (name === 'fingers') { joint = 'fingers'; sides = ['left', 'right']; }
    else if (JOINT_LIMITS[name] && !SIDED[name]) joint = name;
    if (!joint) { errors.push(`unknown joint '${name}'`); continue; }
    for (const [a, v] of Object.entries(acts ?? {})) {
      const lim = JOINT_LIMITS[joint][a as Action];
      if (lim === undefined) { errors.push(`'${a}' is not an action of ${joint}`); continue; }
      if (typeof v !== 'number' || !isFinite(v)) { errors.push(`${name}.${a} must be a number`); continue; }
      const deg = Math.max(0, Math.min(lim, v));
      for (const side of sides) items.push({ joint, side, action: a as Action, deg });
    }
  }
  return { items, errors };
}

const D = Math.PI / 180;
const qx = (deg: number) => new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(1, 0, 0), deg * D);
const qy = (deg: number) => new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 1, 0), deg * D);
const qz = (deg: number) => new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 0, 1), deg * D);

function sum(items: Resolved[], joint: string, side: 'left' | 'right' | null) {
  const a: Partial<Record<Action, number>> = {};
  for (const it of items) if (it.joint === joint && it.side === side) a[it.action] = (a[it.action] ?? 0) + it.deg;
  const g = (k: Action) => a[k] ?? 0;
  return { a, g, any: Object.keys(a).length > 0 };
}

/** The baked Anny rest pose keeps the forearm bent ~42 degrees forward
 * (measured over the phenotype range: 38-45); elbow angles count from a
 * straight arm, so the rest bend is taken out. */
export const ELBOW_REST_BEND = 42;

const FINGERS: [string, string][] = [
  ['thumb', 'Thumb'], ['index', 'Index'], ['middle', 'Middle'], ['ring', 'Ring'], ['little', 'Little'],
];

/** Bone rotations for a pose (bones not mentioned are left out). Arms and
 * legs mentioned without an abduct angle get the anatomical neutral. */
export function poseToBones(pose: JointPose): { bones: Map<VrmBone, THREE.Quaternion>; errors: string[] } {
  const { items, errors } = resolvePose(pose);
  const out = new Map<VrmBone, THREE.Quaternion>();
  const axial = (joint: string, bone: VrmBone) => {
    const s = sum(items, joint, null);
    if (!s.any) return;
    const q = qx(s.g('flex') - s.g('extend'))
      .premultiply(qy(s.g('twist-left') - s.g('twist-right')))
      .premultiply(qz(s.g('tilt-right') - s.g('tilt-left')));
    out.set(bone, q);
  };
  axial('pelvis', 'hips');
  axial('spine', 'spine');
  axial('chest', 'chest');
  axial('neck', 'neck');
  axial('head', 'head');
  for (const side of ['left', 'right'] as const) {
    const L = side === 'left' ? 1 : -1;
    const S = side === 'left' ? 'left' : 'right';
    const hip = sum(items, 'hip', side);
    if (hip.any) {
      const q = qy(L * (hip.g('rotate-out') - hip.g('rotate-in')))
        .premultiply(qz(L * (hip.g('abduct') - hip.g('adduct'))))
        .premultiply(qx(hip.g('extend') - hip.g('flex')));
      out.set(`${S}UpperLeg` as VrmBone, q);
    }
    const knee = sum(items, 'knee', side);
    if (knee.any) out.set(`${S}LowerLeg` as VrmBone, qx(knee.g('flex')));
    const ankle = sum(items, 'ankle', side);
    if (ankle.any) out.set(`${S}Foot` as VrmBone, qx(ankle.g('extend') - ankle.g('flex')));
    const cl = sum(items, 'clavicle', side);
    if (cl.any) out.set(`${S}Shoulder` as VrmBone, qz(L * cl.g('elevate')));
    const sh = sum(items, 'shoulder', side);
    if (sh.any) {
      const abd = sh.a.abduct !== undefined || sh.a.adduct !== undefined ? sh.g('abduct') - sh.g('adduct') : 0;
      // rotation about the arm's own axis mirrors without a sign flip
      const q = qx(sh.g('rotate-in') - sh.g('rotate-out'))
        .premultiply(qz(L * (abd - 90)))
        .premultiply(qx(sh.g('extend') - sh.g('flex')));
      out.set(`${S}UpperArm` as VrmBone, q);
    }
    const el = sum(items, 'elbow', side);
    if (el.any) out.set(`${S}LowerArm` as VrmBone, qy(-L * (el.g('flex') - ELBOW_REST_BEND)));
    const wr = sum(items, 'wrist', side);
    if (wr.any) {
      out.set(`${S}Hand` as VrmBone, qz(-L * (wr.g('flex') - wr.g('extend'))).premultiply(qy(L * (wr.g('tilt-left') - wr.g('tilt-right')))));
    }
    const all = sum(items, 'fingers', side);
    for (const [j, F] of FINGERS) {
      const f = sum(items, j, side);
      const curl = f.any ? f.g('curl') : all.any ? all.g('curl') * (j === 'thumb' ? 0.5 : 1) : null;
      if (curl === null) continue;
      const segs = j === 'thumb' ? ['Metacarpal', 'Proximal', 'Distal'] : ['Proximal', 'Intermediate', 'Distal'];
      segs.forEach((seg, k) => {
        const deg = curl * (k === 0 && j !== 'thumb' ? 0.9 : 1);
        const q = j === 'thumb' ? qy(-L * deg * 0.6) : qz(-L * deg);
        out.set(`${S}${F}${seg}` as VrmBone, q);
      });
    }
  }
  return { bones: out, errors };
}

/** The gesture message of estudo 13 §4.1. */
export interface GestureAction {
  v?: number;
  actor?: string;
  gesture?:
    | { mode: 'clip'; clip: string; params?: { side?: 'left' | 'right'; intensity?: number; speed?: number; loops?: number }; target?: string }
    | { mode: 'compose'; name?: string; start?: string; phases: { dur_s: number; timing?: Timing; joints: JointPose; contacts?: Record<string, string> }[]; hold?: boolean };
  face?: { expression: string; weight?: number; hold_s?: number };
  look_at?: string | { x: number; y: number; z: number } | null;
  posture?: { mood: string; weight?: number };
  blend?: { in_s?: number; out_s?: number; layer?: 'upper' | 'full' };
}

export type Timing = 'flow' | 'settle' | 'drive' | 'snap' | 'linear';

export function ease(t: number, timing: Timing = 'flow'): number {
  const x = Math.max(0, Math.min(1, t));
  switch (timing) {
    case 'linear': return x;
    case 'snap': return 1 - (1 - x) ** 4;
    case 'settle': return 1 - (1 - x) ** 2.2;
    case 'drive': return x ** 2;
    default: return x * x * (3 - 2 * x);
  }
}

export const STANDING: JointPose = {
  shoulders: { abduct: 8 }, elbows: { flex: 8 }, fingers: { curl: 12 },
};
