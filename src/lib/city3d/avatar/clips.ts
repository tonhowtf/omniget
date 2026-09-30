// Animation clips on the VRM rig: a compact JSON (quaternions per bone per
// frame, hips offset in units of the hips rest height), a sampler and the
// procedural definitions of the city clips and gestures.
//
// The Quaternius Universal Animation Library (CC0) was the planned source, but
// it is only distributed through Patreon/itch pages; these clips are authored
// here, in the joint vocabulary of posecode.ts, and baked to
// static/city3d/clips/clips.json by src-tauri/omniget-avatar/tools/gen_clips.ts.
import * as THREE from 'three';
import { ease, poseToBones, STANDING, type JointPose, type Timing } from './posecode';
import { VRM_BONES, type VrmBone } from './rig';

export interface ClipJson {
  name: string; fps: number; frames: number; loop: boolean;
  bones: string[];
  /** per bone, frames*4 quaternion components (x, y, z, w), 4 decimals;
   * just 4 when the bone holds still */
  rot: number[][];
  /** frames*3 hips offset, in hips rest heights */
  hips: number[];
  /** what an LLM sees in the gesture catalog */
  about?: string;
  params?: string[];
}

export interface Clip {
  name: string; fps: number; frames: number; loop: boolean; duration: number;
  boneIdx: Int16Array;
  rot: Float32Array;
  hips: Float32Array;
  about?: string;
}

export const CITY_ACTIONS = ['idle', 'walk', 'sit', 'talk', 'water', 'plant', 'harvest', 'carry', 'sleep', 'work'] as const;
export const GESTURES = ['wave', 'point', 'gesture', 'nod', 'shrug', 'clap', 'think', 'cheer'] as const;

export function clipFromJson(j: ClipJson): Clip {
  const boneIdx = Int16Array.from(j.bones, (b) => VRM_BONES.indexOf(b as VrmBone));
  const rot = new Float32Array(j.bones.length * j.frames * 4);
  j.rot.forEach((r, i) => {
    // a constant bone is stored as a single quaternion
    if (r.length === 4) for (let f = 0; f < j.frames; f++) rot.set(r, (i * j.frames + f) * 4);
    else rot.set(r, i * j.frames * 4);
  });
  return {
    name: j.name, fps: j.fps, frames: j.frames, loop: j.loop,
    duration: j.loop ? j.frames / j.fps : (j.frames - 1) / j.fps,
    boneIdx, rot, hips: Float32Array.from(j.hips), about: j.about,
  };
}

let cached: Promise<Map<string, Clip>> | null = null;

/** Loads the baked clips, falling back to generating them in place. */
export function loadClips(url = '/city3d/clips/clips.json'): Promise<Map<string, Clip>> {
  if (cached) return cached;
  cached = (async () => {
    let list: ClipJson[];
    try {
      const r = await fetch(url);
      if (!r.ok) throw new Error(String(r.status));
      list = (await r.json()).clips as ClipJson[];
    } catch {
      list = generateClips();
    }
    return new Map(list.map((c) => [c.name, clipFromJson(c)]));
  })();
  return cached;
}

const _qa = new THREE.Quaternion();
const _qb = new THREE.Quaternion();

/** Writes the clip pose at time `t` (seconds) into `quats` (52*4) blended by
 * `weight` over what is there, and adds weight * hips offset into `hips`. */
export function sampleClip(clip: Clip, t: number, quats: Float32Array, hips: Float32Array, weight = 1, mask?: Uint8Array, mirror = false) {
  const n = clip.frames;
  let f = t * clip.fps;
  if (clip.loop) f = ((f % n) + n) % n;
  else f = Math.max(0, Math.min(n - 1, f));
  const i0 = Math.floor(f);
  const i1 = clip.loop ? (i0 + 1) % n : Math.min(n - 1, i0 + 1);
  const a = f - i0;
  for (let b = 0; b < clip.boneIdx.length; b++) {
    let bone = clip.boneIdx[b];
    if (bone < 0) continue;
    if (mirror) bone = MIRROR[bone];
    if (mask && !mask[bone]) continue;
    const o = b * n * 4;
    _qa.fromArray(clip.rot, o + i0 * 4);
    _qb.fromArray(clip.rot, o + i1 * 4);
    _qa.slerp(_qb, a);
    if (mirror) { _qa.y = -_qa.y; _qa.z = -_qa.z; }
    if (weight < 1) {
      _qb.fromArray(quats, bone * 4);
      _qb.slerp(_qa, weight);
      _qb.toArray(quats, bone * 4);
    } else {
      _qa.toArray(quats, bone * 4);
    }
  }
  for (let k = 0; k < 3; k++) {
    const h = clip.hips[i0 * 3 + k] * (1 - a) + clip.hips[i1 * 3 + k] * a;
    hips[k] += (mirror && k === 0 ? -h : h) * weight;
  }
}

/** Bone index -> its left/right mirror. */
export const MIRROR = Int16Array.from(VRM_BONES, (b, i) => {
  const m = b.startsWith('left') ? 'right' + b.slice(4) : b.startsWith('right') ? 'left' + b.slice(5) : b;
  const j = VRM_BONES.indexOf(m as VrmBone);
  return j < 0 ? i : j;
});

/** Upper-body mask (spine up, arms, head). */
export const UPPER_MASK = Uint8Array.from(VRM_BONES, (b) =>
  /Leg|Foot|^hips$/.test(b) ? 0 : 1,
);

// ---------------------------------------------------------------------------
// Authoring

interface Key { pose: JointPose; hips?: [number, number, number] }

function expand(p: JointPose): Record<string, JointPose[string]> {
  const out: Record<string, JointPose[string]> = {};
  const plural: Record<string, string> = {
    hips: 'hip', knees: 'knee', ankles: 'ankle', clavicles: 'clavicle', shoulders: 'shoulder',
    elbows: 'elbow', wrists: 'wrist', fingers: 'fingers', thumbs: 'thumb',
  };
  for (const [k, v] of Object.entries(p)) {
    if (plural[k]) {
      out[`${plural[k]}_left`] = { ...(out[`${plural[k]}_left`] ?? {}), ...v };
      out[`${plural[k]}_right`] = { ...(out[`${plural[k]}_right`] ?? {}), ...v };
    } else out[k] = { ...(out[k] ?? {}), ...v };
  }
  return out;
}

/** `over` replaces `base` joint by joint (not action by action). */
export function mergePose(base: JointPose, over: JointPose): JointPose {
  return { ...expand(base), ...expand(over) };
}

export function makeClip(
  name: string, seconds: number, loop: boolean, fps: number,
  at: (phase: number) => Key, about?: string,
): ClipJson {
  const frames = loop ? Math.max(2, Math.round(seconds * fps)) : Math.max(2, Math.round(seconds * fps) + 1);
  const perBone = new Map<string, number[]>();
  const hips: number[] = [];
  const keys: Map<VrmBone, THREE.Quaternion>[] = [];
  for (let f = 0; f < frames; f++) {
    const phase = loop ? f / frames : f / (frames - 1);
    const k = at(phase);
    const { bones, errors } = poseToBones(mergePose(STANDING, k.pose));
    if (errors.length) throw new Error(`${name}: ${errors.join(', ')}`);
    keys.push(bones);
    hips.push(...(k.hips ?? [0, 0, 0]).map(r4));
  }
  const names = new Set<VrmBone>();
  keys.forEach((m) => m.forEach((_, b) => names.add(b)));
  for (const b of VRM_BONES) {
    if (!names.has(b)) continue;
    const arr: number[] = [];
    let prev: THREE.Quaternion | null = null;
    for (const m of keys) {
      const q = (m.get(b) ?? new THREE.Quaternion()).clone();
      if (prev && prev.dot(q) < 0) q.set(-q.x, -q.y, -q.z, -q.w);
      arr.push(r4(q.x), r4(q.y), r4(q.z), r4(q.w));
      prev = q;
    }
    let still = true;
    for (let i = 4; i < arr.length && still; i++) still = Math.abs(arr[i] - arr[i % 4]) < 2e-4;
    if (still && Math.abs(arr[3]) > 0.99995) continue; // identity throughout
    perBone.set(b, still ? arr.slice(0, 4) : arr);
  }
  return { name, fps, frames, loop, bones: [...perBone.keys()], rot: [...perBone.values()], hips, about };
}

function r4(v: number) {
  return Math.round(v * 10000) / 10000;
}

/** Keyframed pose sequence (holds and eases between keys). */
function keyed(keys: { t: number; pose: JointPose; hips?: [number, number, number]; timing?: Timing }[]) {
  return (phase: number): Key => {
    let i = 0;
    while (i < keys.length - 1 && keys[i + 1].t <= phase) i++;
    const a = keys[i];
    const b = keys[Math.min(i + 1, keys.length - 1)];
    const span = b.t - a.t;
    const u = span > 0 ? ease((phase - a.t) / span, b.timing ?? 'flow') : 0;
    return { pose: lerpPose(a.pose, b.pose, u), hips: lerp3(a.hips ?? [0, 0, 0], b.hips ?? [0, 0, 0], u) };
  };
}

export function lerpPose(a: JointPose, b: JointPose, u: number): JointPose {
  const ea = expand(a), eb = expand(b);
  const out: JointPose = {};
  for (const k of new Set([...Object.keys(ea), ...Object.keys(eb)])) {
    const va = ea[k] ?? {}, vb = eb[k] ?? {};
    const acts: Record<string, number> = {};
    for (const act of new Set([...Object.keys(va), ...Object.keys(vb)])) {
      const x = (va as Record<string, number>)[act] ?? 0, y = (vb as Record<string, number>)[act] ?? 0;
      acts[act] = x + (y - x) * u;
    }
    out[k] = acts;
  }
  return out;
}

function lerp3(a: [number, number, number], b: [number, number, number], u: number): [number, number, number] {
  return [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u, a[2] + (b[2] - a[2]) * u];
}

const TAU = Math.PI * 2;
const DEG = Math.PI / 180;
const pos = (v: number) => Math.max(0, v);
/** posecode wants non-negative amounts: split a signed angle in two actions */
const fe = (v: number) => (v >= 0 ? { flex: v } : { extend: -v });
const tw = (v: number) => (v >= 0 ? { 'twist-left': v } : { 'twist-right': -v });
const tl = (v: number) => (v >= 0 ? { 'tilt-right': v } : { 'tilt-left': -v });
const ab = (v: number) => (v >= 0 ? { abduct: v } : { adduct: -v });
const ro = (v: number) => (v >= 0 ? { 'rotate-out': v } : { 'rotate-in': -v });
const smooth = (x: number) => { const t = Math.max(0, Math.min(1, x)); return t * t * (3 - 2 * t); };
const wrap = (p: number) => ((p % 1) + 1) % 1;

// ---------------------------------------------------------------------------
// Reference leg for the IK that plants the feet. Joint offsets of the baked
// Anny bodies averaged over gender and height, in hips rest heights (the unit
// of the clip hips offset), so the contacts hold for any body of average
// proportions. Measured from static/city3d/anny with buildBody.

const LEG = {
  /** left hip joint relative to the hips bone */
  hip: new THREE.Vector3(0.1108, -0.0127, -0.0075),
  /** knee relative to the hip joint, ankle relative to the knee (T-pose) */
  knee: new THREE.Vector3(0.0404, -0.4378, 0.0353),
  ankle: new THREE.Vector3(0.0443, -0.4712, -0.0315),
  /** ankle height over the sole, heel and toe tip along z from the ankle */
  ankleY: 0.0783, heel: -0.054, toe: 0.21,
};
const REACH = LEG.knee.clone().add(LEG.ankle).length();
const X = new THREE.Vector3(1, 0, 0), Y = new THREE.Vector3(0, 1, 0), Z = new THREE.Vector3(0, 0, 1);
const qAxis = (axis: THREE.Vector3, deg: number) => new THREE.Quaternion().setFromAxisAngle(axis, deg * DEG);

/** Pelvis orientation as posecode builds it (signed: tilt right, twist left, flex). */
function pelvisQuat(p: { tilt?: number; twist?: number; flex?: number }) {
  return qAxis(Z, p.tilt ?? 0).multiply(qAxis(Y, p.twist ?? 0)).multiply(qAxis(X, p.flex ?? 0));
}
/** Hip as posecode builds it: signed flex, abduct and rotate-out. */
function hipQuat(L: number, flex: number, abd: number, rot: number) {
  return qAxis(X, -flex).multiply(qAxis(Z, L * abd)).multiply(qAxis(Y, L * rot));
}

interface LegSolve { flex: number; abd: number; knee: number }

/** Two-bone leg IK on the reference leg: hip flex, abduction and knee flex
 * that put the ankle at `target` (root space, hips rest heights), with the
 * knee pointing along the foot. Starts from `guess` (the previous frame). */
function solveLeg(side: 1 | -1, hipsPos: THREE.Vector3, qp: THREE.Quaternion, rot: number, target: THREE.Vector3, guess: LegSolve): LegSolve {
  const off = (v: THREE.Vector3) => new THREE.Vector3(v.x * side, v.y, v.z);
  const hipW = off(LEG.hip).applyQuaternion(qp).add(hipsPos);
  const kneeO = off(LEG.knee), ankleO = off(LEG.ankle);
  const fk = (s: LegSolve) => {
    const qh = qp.clone().multiply(hipQuat(side, s.flex, s.abd, rot));
    const qk = qh.clone().multiply(qAxis(X, s.knee));
    return kneeO.clone().applyQuaternion(qh).add(ankleO.clone().applyQuaternion(qk)).add(hipW);
  };
  // out of reach: aim the straightest leg at the target instead
  const want = target.clone();
  const d = want.clone().sub(hipW);
  if (d.length() > REACH * 0.999) want.copy(hipW).add(d.setLength(REACH * 0.999));
  const s = { ...guess };
  const keys: (keyof LegSolve)[] = ['flex', 'abd', 'knee'];
  for (let it = 0; it < 40; it++) {
    const e = want.clone().sub(fk(s));
    if (e.length() < 1e-6) break;
    const Jc = keys.map((k) => {
      const t = { ...s, [k]: s[k] + 0.01 };
      return fk(t).sub(fk(s)).divideScalar(0.01);
    });
    const J = new THREE.Matrix3().set(
      Jc[0].x, Jc[1].x, Jc[2].x,
      Jc[0].y, Jc[1].y, Jc[2].y,
      Jc[0].z, Jc[1].z, Jc[2].z,
    );
    if (Math.abs(J.determinant()) < 1e-9) { s.knee += 1; continue; }
    const dx = e.applyMatrix3(J.invert());
    const lim = 15;
    s.flex += Math.max(-lim, Math.min(lim, dx.x));
    s.abd += Math.max(-lim, Math.min(lim, dx.y));
    s.knee = Math.max(0.5, s.knee + Math.max(-lim, Math.min(lim, dx.z)));
  }
  return s;
}

/** Ankle angle (signed flex = toes up) that gives the foot `pitch` degrees
 * (toes up) in root space, with the leg of `s`. */
function ankleFor(side: 1 | -1, qp: THREE.Quaternion, rot: number, s: LegSolve, pitch: number, yaw = 0): number {
  const qk = qp.clone().multiply(hipQuat(side, s.flex, s.abd, rot)).multiply(qAxis(X, s.knee));
  const want = qAxis(Y, yaw).multiply(qAxis(X, -pitch));
  const qa = qk.invert().multiply(want);
  // twist of qa about x
  return -2 * Math.atan2(qa.x, qa.w) / DEG;
}

/** Ankle position for a foot planted with its sole flat at `flat` (ankle
 * when flat) and pitched `pitch` degrees around the heel (> 0) or the toe (< 0). */
function plantedAnkle(flat: THREE.Vector3, pitch: number): THREE.Vector3 {
  if (Math.abs(pitch) < 1e-6) return flat.clone();
  const pivotZ = pitch > 0 ? LEG.heel : LEG.toe;
  const pivot = new THREE.Vector3(flat.x, flat.y - LEG.ankleY, flat.z + pivotZ);
  return new THREE.Vector3(0, LEG.ankleY, -pivotZ).applyQuaternion(qAxis(X, -pitch)).add(pivot);
}

interface LegFrame { hips: THREE.Vector3; pelvis: { tilt?: number; twist?: number; flex?: number }; feet: { ankle: THREE.Vector3; pitch: number; yaw?: number }[] }

/** Solves both legs for a sequence of frames (targets in root space) and
 * returns the posecode joints of pelvis, hips, knees and ankles per frame. */
function solveLegs(frames: LegFrame[], init: LegSolve = { flex: 5, abd: 0, knee: 8 }): JointPose[] {
  const guess: LegSolve[] = [{ ...init }, { ...init }];
  // two passes so the first frame starts from a settled guess
  const out: JointPose[] = [];
  for (let pass = 0; pass < 2; pass++) {
    out.length = 0;
    for (const f of frames) {
      const qp = pelvisQuat(f.pelvis);
      const pose: JointPose = {
        pelvis: { ...tl(f.pelvis.tilt ?? 0), ...tw(f.pelvis.twist ?? 0), ...fe(f.pelvis.flex ?? 0) },
      };
      ([1, -1] as const).forEach((side, i) => {
        const foot = f.feet[i];
        const rot = (foot.yaw ?? 0) - side * (f.pelvis.twist ?? 0);
        const s = solveLeg(side, f.hips.clone().add(new THREE.Vector3(0, 1, 0)), qp, rot, foot.ankle, guess[i]);
        guess[i] = s;
        const a = ankleFor(side, qp, rot, s, foot.pitch, (foot.yaw ?? 0) * side);
        const n = side === 1 ? 'left' : 'right';
        pose[`hip_${n}`] = { ...fe(s.flex), ...ab(s.abd), ...ro(rot) };
        pose[`knee_${n}`] = { flex: s.knee };
        pose[`ankle_${n}`] = fe(a);
      });
      out.push(pose);
    }
  }
  return out;
}

/** Highest pelvis (hips offset y) that keeps both ankles in reach, smoothed
 * without ever going above the limit. */
function pelvisHeights(ankles: THREE.Vector3[][], hipsXZ: [number, number][], twist: number[], tilt: number[], cap: number, loop: boolean): number[] {
  const n = ankles.length;
  const raw = ankles.map((pair, f) => {
    const qp = pelvisQuat({ tilt: tilt[f], twist: twist[f] });
    let y = cap;
    pair.forEach((a, i) => {
      const side = i === 0 ? 1 : -1;
      const h = new THREE.Vector3(LEG.hip.x * side, LEG.hip.y, LEG.hip.z).applyQuaternion(qp);
      const dx = a.x - (hipsXZ[f][0] + h.x), dz = a.z - (hipsXZ[f][1] + h.z);
      const r = REACH * 0.997;
      const up = Math.sqrt(Math.max(0, r * r - dx * dx - dz * dz));
      y = Math.min(y, a.y + up - h.y - 1);
    });
    return y;
  });
  // erode then average over the same window: smooth and never above raw
  const k = Math.max(1, Math.round(n / 18));
  const at = (arr: number[], i: number) => arr[loop ? ((i % n) + n) % n : Math.max(0, Math.min(n - 1, i))];
  const eroded = raw.map((_, i) => Math.min(...Array.from({ length: 2 * k + 1 }, (_, j) => at(raw, i + j - k))));
  const y = raw.map((_, i) => Array.from({ length: 2 * k + 1 }, (_, j) => at(eroded, i + j - k)).reduce((s, v) => s + v, 0) / (2 * k + 1));
  return y;
}

/** Arm swing of the walk, with the forearm trailing the upper arm. */
function walkArms(p: number, amp = 1): JointPose {
  const sw = (lag: number, side: number) => Math.cos(TAU * (p - lag) + (side > 0 ? Math.PI : 0));
  const l = sw(0.06, 1), r = sw(0.06, -1);
  const le = sw(0.12, 1), re = sw(0.12, -1);
  return {
    shoulder_left: { abduct: 7, ...fe(amp * (l > 0 ? 17 * l : 12 * l)) },
    shoulder_right: { abduct: 7, ...fe(amp * (r > 0 ? 17 * r : 12 * r)) },
    elbow_left: { flex: 14 + amp * 16 * pos(le) }, elbow_right: { flex: 14 + amp * 16 * pos(re) },
    wrist_left: fe(4 + 5 * sw(0.16, 1)), wrist_right: fe(4 + 5 * sw(0.16, -1)),
    fingers: { curl: 22 },
  };
}

/**
 * Walk cycle with planted feet. One cycle is two steps and covers `stride`
 * hips rest heights of ground; the system plays it at speed / (1.63 * hips
 * height) per second, so stride = 1.63 * seconds keeps the feet still on
 * the ground. Each foot: heel strike at +18 degrees, rolls flat, heel off
 * and toe off at -35 degrees, then swings on a Hermite arc.
 */
function walkCycle(frames: number, stride: number, o: { duty?: number; width?: number; clear?: number; sway?: number; dip?: number } = {}) {
  const duty = o.duty ?? 0.62;
  const width = o.width ?? 0.15;
  const clear = o.clear ?? 0.035;
  const strikeZ = (stride * duty) / 2 - 0.05; // the stance runs a bit behind the hip
  const stance = (u: number): { flat: number; pitch: number } => {
    // u in [0, 1] over the stance; flat-foot ankle z moves back with the ground
    const flat = strikeZ - stride * duty * u;
    let pitch = 0;
    if (u < 0.14) pitch = 18 * (1 - smooth(u / 0.14));
    else if (u > 0.5) pitch = -30 * ((u - 0.5) / 0.5) ** 1.6;
    return { flat, pitch };
  };
  const plantedAt = (u: number, x: number) => {
    const s = stance(u);
    return { ankle: plantedAnkle(new THREE.Vector3(x, LEG.ankleY, s.flat), s.pitch), pitch: s.pitch };
  };
  const foot = (p: number, side: 1 | -1) => {
    const ph = wrap(p + (side === 1 ? 0 : 0.5));
    const x = width * side;
    if (ph < duty) return plantedAt(ph / duty, x);
    const u = (ph - duty) / (1 - duty);
    const a = plantedAt(1, x), b = plantedAt(0, x);
    // Hermite in z: leaves the ground gently and lands at the ground speed
    const v = -stride * (1 - duty);
    const h00 = 2 * u ** 3 - 3 * u ** 2 + 1, h10 = u ** 3 - 2 * u ** 2 + u, h01 = -2 * u ** 3 + 3 * u ** 2, h11 = u ** 3 - u ** 2;
    const z = h00 * a.ankle.z + h10 * v * 0.4 + h01 * b.ankle.z + h11 * v;
    const y = a.ankle.y + (b.ankle.y - a.ankle.y) * smooth(u) + clear * Math.sin(Math.PI * u) ** 0.9;
    const pitch = a.pitch + (b.pitch - a.pitch) * smooth(Math.min(1, u / 0.85));
    return { ankle: new THREE.Vector3(x, y, z), pitch };
  };
  const ps = Array.from({ length: frames }, (_, f) => f / frames);
  const sway = o.sway ?? 0.018;
  const twist = ps.map((p) => -6 * Math.cos(TAU * p));
  // pelvis drops on the swing side: left swings in the second half
  const tilt = ps.map((p) => 3.5 * Math.sin(TAU * p - 0.3));
  const xz = ps.map((p) => [sway * Math.cos(TAU * (p - 0.12)), 0] as [number, number]);
  const feet = ps.map((p) => [foot(p, 1), foot(p, -1)]);
  const ys = pelvisHeights(feet.map((f) => f.map((x) => x.ankle)), xz, twist, tilt, -0.012, true).map((y) => y - (o.dip ?? 0.006));
  const legs = solveLegs(ps.map((_, f) => ({
    hips: new THREE.Vector3(xz[f][0], ys[f], xz[f][1]),
    pelvis: { tilt: tilt[f], twist: twist[f], flex: 2 },
    feet: feet[f],
  })));
  return { legs, hips: ps.map((_, f) => [xz[f][0], ys[f], 0] as [number, number, number]), twist };
}

/** Standing stance: feet planted, the pelvis shifts weight from `shift`
 * (-1 right .. 1 left), sinking the unloaded hip. Returns legs + hips. */
function stanceLegs(shifts: number[], bob: number[] = []) {
  const feet = [
    { ankle: new THREE.Vector3(0.16, LEG.ankleY, 0.01), pitch: 0, yaw: 7 },
    { ankle: new THREE.Vector3(-0.16, LEG.ankleY, -0.02), pitch: 0, yaw: 7 },
  ];
  const tilt = shifts.map((s) => -4 * s);
  const xz = shifts.map((s) => [0.035 * s, 0] as [number, number]);
  const ys = pelvisHeights(shifts.map(() => feet.map((f) => f.ankle)), xz, shifts.map(() => 0), tilt, -0.008, true)
    .map((y, i) => y - 0.004 - 0.006 * Math.abs(shifts[i]) + (bob[i] ?? 0));
  const legs = solveLegs(shifts.map((_, f) => ({
    hips: new THREE.Vector3(xz[f][0], ys[f], 0), pelvis: { tilt: tilt[f] }, feet,
  })));
  return { legs, hips: shifts.map((_, f) => [xz[f][0], ys[f], 0] as [number, number, number]) };
}

/** Sampled keyframes with wrap-around and per-joint lag (overlap). */
function keyedLoop(keys: { t: number; pose: JointPose; timing?: Timing }[]) {
  const ks = [...keys, { ...keys[0], t: 1 + keys[0].t }];
  return (phase: number): JointPose => {
    const p = wrap(phase - keys[0].t) + keys[0].t;
    let i = 0;
    while (i < ks.length - 2 && ks[i + 1].t <= p) i++;
    const a = ks[i], b = ks[i + 1];
    return lerpPose(a.pose, b.pose, ease((p - a.t) / Math.max(1e-6, b.t - a.t), b.timing ?? 'flow'));
  };
}

/** Picks joints of a pose. */
function only(p: JointPose, names: string[]): JointPose {
  const e = expand(p);
  return Object.fromEntries(names.filter((n) => e[n]).map((n) => [n, e[n]]));
}

/** Envelope for one-shot gestures: eases in over `a`, holds, eases out over `r`. */
function envelope(p: number, a: number, r: number, timingIn: Timing = 'settle', timingOut: Timing = 'flow') {
  if (p < a) return ease(p / a, timingIn);
  if (p > 1 - r) return 1 - ease((p - (1 - r)) / r, timingOut);
  return 1;
}

/** Damped overshoot: 0 -> 1 with a small bounce past 1 (follow-through). */
function overshoot(x: number, amount = 0.08) {
  const t = Math.max(0, Math.min(1, x));
  return 1 - (1 - t) ** 3 + amount * Math.sin(Math.PI * t) * t ** 2 * 2.2 * (1 - t) * 3;
}

function lerpN(a: number, b: number, u: number) {
  return a + (b - a) * u;
}

export function generateClips(): ClipJson[] {
  const F = 20;
  const clips: ClipJson[] = [];

  // idle: 8 s, two breaths per 4 s, weight moves to the left leg and back to
  // the right; arms trail the pelvis and the head drifts on its own period
  {
    const IF = 8, secs = 8, n = secs * IF;
    const shift = Array.from({ length: n }, (_, f) => 0.25 + 0.75 * Math.sin(TAU * f / n));
    const breath = (p: number) => 0.5 - 0.5 * Math.cos(TAU * 4 * p); // 4 breaths per loop
    const st = stanceLegs(shift, Array.from({ length: n }, (_, f) => 0.003 * breath(f / n)));
    clips.push(makeClip('idle', secs, true, IF, (p) => {
      const i = Math.round(p * n) % n;
      const b = breath(p);
      const lag = shift[(i - 4 + n) % n];
      return {
        pose: {
          ...st.legs[i],
          spine: { ...fe(2 - 1 * b), ...tl(1.6 * lag) },
          chest: { ...fe(-1.5 * b), ...tl(1.2 * lag) },
          clavicles: { elevate: 1.5 * b },
          neck: { ...fe(3), ...tl(-1.5 * lag) },
          head: { ...tw(5 * Math.sin(TAU * p + 1)), ...fe(2 * Math.sin(TAU * 2 * p)) },
          shoulder_left: { abduct: 7 + 1.2 * b + 1.5 * pos(lag), ...fe(2 * Math.sin(TAU * p - 0.9)) },
          shoulder_right: { abduct: 7 + 1.2 * b + 1.5 * pos(-lag), ...fe(2 * Math.sin(TAU * p - 0.9)) },
          elbows: { flex: 10 + 2 * b }, wrists: { flex: 6 }, fingers: { curl: 22 },
        },
        hips: st.hips[i],
      };
    }, 'standing, breathing, shifting weight'));
  }

  // walk: 0.9 s per cycle = 1.47 hips heights of ground (the system plays
  // clips at speed / (1.63 * hips height) per clip second); at 1.4 m/s an
  // average adult takes 0.7 m steps at about 120 steps a minute
  {
    const secs = 0.9, n = Math.round(secs * F);
    const wc = walkCycle(n, 1.63 * secs);
    clips.push(makeClip('walk', secs, true, F, (p) => {
      const i = Math.round(p * n) % n;
      return {
        pose: {
          ...wc.legs[i],
          spine: tw(-wc.twist[i] * 0.5),
          chest: { ...tw(-wc.twist[i] * 0.7), ...fe(-2 + 1.5 * Math.cos(2 * TAU * p)) },
          neck: { ...fe(1), ...tw(wc.twist[i] * 0.3) },
          head: fe(-1.5 * Math.cos(2 * TAU * (p - 0.08))),
          ...walkArms(p),
        },
        hips: wc.hips[i],
      };
    }, 'walking cycle, 2 steps'));
  }

  // sit: hip joints 0.56 hips heights up (a 0.45 m seat plus the flesh under
  // the sitting bones on an average adult), thighs on the seat, feet flat
  {
    const hipsZ = -0.09;
    const qp = pelvisQuat({ flex: -8 });
    const hipJointY = 0.556;
    const hipsY = hipJointY - LEG.hip.clone().applyQuaternion(qp).y - 1;
    const feet = [
      { ankle: new THREE.Vector3(0.14, LEG.ankleY, 0.4), pitch: 0, yaw: 5 },
      { ankle: new THREE.Vector3(-0.14, LEG.ankleY, 0.37), pitch: 0, yaw: 5 },
    ];
    const legs = solveLegs([{ hips: new THREE.Vector3(0, hipsY, hipsZ), pelvis: { flex: -8 }, feet }], { flex: 85, abd: 0, knee: 85 })[0];
    clips.push(makeClip('sit', 3, true, 10, (p) => {
      const b = 0.5 - 0.5 * Math.cos(TAU * p);
      return {
        pose: {
          ...legs,
          spine: fe(9 + 1.2 * b), chest: fe(3 - 1.5 * b), clavicles: { elevate: 1.5 * b },
          neck: fe(4), head: { ...fe(3), ...tw(4 * Math.sin(TAU * p)) },
          shoulders: { abduct: 10, flex: 6 + 1 * b }, elbows: { flex: 64 }, wrists: { extend: 12 }, fingers: { curl: 25 },
        },
        hips: [0, hipsY, hipsZ],
      };
    }, 'sitting on a seat'));
  }

  clips.push(makeClip('talk', 3, true, F, (p) => {
    const s = Math.sin(TAU * p), s2 = Math.sin(TAU * 2 * p + 0.7);
    return {
      pose: {
        shoulder_right: { abduct: 18, flex: 28 + 10 * s }, elbow_right: { flex: 70 + 18 * s2 },
        shoulder_left: { abduct: 12, flex: 16 + 8 * pos(-s) }, elbow_left: { flex: 40 + 15 * pos(-s2) },
        neck: fe(4 * Math.sin(TAU * 3 * p)), head: tw(6 * s),
        chest: tw(3 * s),
      },
    };
  }, 'talking with hand gestures'));
  clips.push(makeClip('water', 3, true, F, (p) => ({
    pose: {
      spine: { flex: 14 }, neck: { flex: 22 },
      shoulder_right: { abduct: 14, flex: 48 }, elbow_right: { flex: 30 }, wrist_right: { flex: 20 + 18 * pos(Math.sin(TAU * p)) },
      shoulder_left: { abduct: 10, flex: 8 }, elbow_left: { flex: 20 },
      knees: { flex: 6 },
    },
    hips: [0, -0.01, 0],
  }), 'watering plants with a can'));
  clips.push(makeClip('plant', 2.4, true, F, (p) => {
    const d = Math.sin(TAU * p);
    return {
      pose: {
        hips: { flex: 100 }, knees: { flex: 118 }, ankles: { flex: 18 },
        spine: { flex: 30 }, chest: { flex: 12 }, neck: { flex: 18 },
        shoulders: { abduct: 12, flex: 72 + 12 * d }, elbows: { flex: 22 + 20 * pos(d) }, wrists: { flex: 20 },
      },
      hips: [0, -0.42, -0.08],
    };
  }, 'kneeling, planting seeds'));
  clips.push(makeClip('harvest', 2.6, true, F, keyed([
    { t: 0, pose: { spine: { flex: 10 }, shoulders: { abduct: 10, flex: 30 }, elbows: { flex: 60 } } },
    { t: 0.3, pose: { pelvis: { flex: 35 }, spine: { flex: 30 }, knees: { flex: 25 }, hips: { flex: 10 }, shoulders: { abduct: 12, flex: 85 }, elbows: { flex: 12 }, fingers: { curl: 50 } }, hips: [0, -0.06, -0.05] },
    { t: 0.55, pose: { pelvis: { flex: 35 }, spine: { flex: 30 }, knees: { flex: 25 }, hips: { flex: 10 }, shoulders: { abduct: 12, flex: 80 }, elbows: { flex: 20 }, fingers: { curl: 80 } }, hips: [0, -0.06, -0.05] },
    { t: 0.85, pose: { spine: { flex: 8 }, shoulders: { abduct: 14, flex: 40 }, elbows: { flex: 80 }, fingers: { curl: 80 } }, timing: 'settle' },
    { t: 1, pose: { spine: { flex: 10 }, shoulders: { abduct: 10, flex: 30 }, elbows: { flex: 60 } } },
  ]), 'bending to pick crops'));

  // carry: shorter, quicker steps under the load
  {
    const secs = 0.8, n = Math.round(secs * F);
    const wc = walkCycle(n, 1.63 * secs, { duty: 0.64, clear: 0.03, sway: 0.024, dip: 0.012 });
    clips.push(makeClip('carry', secs, true, F, (p) => {
      const i = Math.round(p * n) % n;
      return {
        pose: {
          ...wc.legs[i],
          spine: { extend: 5, ...tw(-wc.twist[i] * 0.8) }, chest: tw(-wc.twist[i] * 0.2),
          neck: fe(6),
          shoulders: { abduct: 16, flex: 22 + 2 * Math.cos(2 * TAU * (p - 0.1)), 'rotate-in': 12 }, elbows: { flex: 72 - 3 * Math.cos(2 * TAU * (p - 0.15)) },
          wrists: { extend: 8 }, fingers: { curl: 40 },
        },
        hips: wc.hips[i],
      };
    }, 'walking while carrying a box'));
  }

  clips.push(makeClip('sleep', 5, true, F, (p) => ({
    pose: {
      pelvis: { extend: 90 }, hips: { flex: 6 }, knees: { flex: 12 }, ankles: { extend: 25 },
      chest: fe(2 * Math.sin(TAU * p)), neck: { flex: 12 }, head: { 'twist-left': 12 },
      shoulders: { abduct: 14 }, elbows: { flex: 4 }, fingers: { curl: 30 },
    },
    hips: [0, -0.86, 0],
  }), 'lying asleep on the back'));

  // work: hammering. Slow wind-up (anticipation), a fast strike that drives
  // into the blow, a small recoil after the impact (follow-through) and a
  // settle back; the trunk and head follow the arm a few frames late.
  {
    const arm = keyedLoop([
      { t: 0, pose: { shoulder_right: { abduct: 22, flex: 55 }, elbow_right: { flex: 70 }, wrist_right: { extend: 5 } } },
      { t: 0.42, pose: { shoulder_right: { abduct: 32, flex: 84 }, elbow_right: { flex: 96 }, wrist_right: { extend: 35 } }, timing: 'flow' },
      { t: 0.5, pose: { shoulder_right: { abduct: 32, flex: 88 }, elbow_right: { flex: 100 }, wrist_right: { extend: 40 } }, timing: 'settle' },
      { t: 0.6, pose: { shoulder_right: { abduct: 20, flex: 42 }, elbow_right: { flex: 28 }, wrist_right: { flex: 20 } }, timing: 'drive' },
      { t: 0.67, pose: { shoulder_right: { abduct: 20, flex: 50 }, elbow_right: { flex: 42 }, wrist_right: { flex: 8 } }, timing: 'snap' },
      { t: 0.75, pose: { shoulder_right: { abduct: 20, flex: 46 }, elbow_right: { flex: 36 }, wrist_right: { flex: 12 } }, timing: 'settle' },
    ]);
    const trunk = keyedLoop([
      { t: 0, pose: { spine: { flex: 10 }, chest: { flex: 4 }, knees: { flex: 10 }, neck: { flex: 22 } } },
      { t: 0.45, pose: { spine: { flex: 6, 'twist-right': 4 }, chest: { flex: 0, 'twist-right': 6 }, knees: { flex: 8 }, neck: { flex: 18 } } },
      { t: 0.62, pose: { spine: { flex: 14 }, chest: { flex: 7 }, knees: { flex: 13 }, neck: { flex: 25 } }, timing: 'drive' },
      { t: 0.75, pose: { spine: { flex: 12 }, chest: { flex: 5 }, knees: { flex: 11 }, neck: { flex: 23 } }, timing: 'settle' },
    ]);
    const holdL = { shoulder_left: { abduct: 14, flex: 48 }, elbow_left: { flex: 62 }, wrist_left: { flex: 10 }, fingers_left: { curl: 55 }, fingers_right: { curl: 80 }, thumb_right: { curl: 50 } };
    clips.push(makeClip('work', 1.2, true, F, (p) => {
      const t = trunk(p - 0.04);
      const kneeF = (expand(t).knee_left?.flex ?? 12);
      return {
        pose: {
          ...arm(p), ...only(t, ['spine', 'chest', 'neck']),
          hip_left: { flex: kneeF * 0.55 }, hip_right: { flex: kneeF * 0.55 },
          knees: { flex: kneeF }, ankles: { flex: kneeF * 0.45 },
          ...holdL,
          shoulder_left: { abduct: 14, flex: 48 + 3 * Math.sin(TAU * (p - 0.66)) * pos(Math.cos(TAU * (p - 0.66))) },
        },
        hips: [0, -0.003 - 0.0013 * kneeF, -0.02],
      };
    }, 'hammering at a workbench'));
  }

  // gestures (played once by default; `loops` repeats them)

  // wave: the arm lifts with a settle, the forearm stands up in the frontal
  // plane and swings from the elbow; the hand trails the forearm by a
  // fraction of a swing, and the arm drops back with a small overshoot.
  clips.push(makeClip('wave', 1.8, false, F, (p) => {
    const up = envelope(p, 0.2, 0.22, 'settle', 'flow');
    const lift = p < 0.2 ? overshoot(p / 0.2, 0.06) : up;
    const swingT = Math.max(0, p - 0.14) / 0.64;
    const sw = Math.sin(TAU * 2.25 * swingT) * up;
    const swHand = Math.sin(TAU * 2.25 * swingT - 0.9) * up;
    return {
      pose: {
        clavicle_right: { elevate: 8 * lift },
        shoulder_right: { abduct: lerpN(8, 112, lift), flex: 12 * lift, 'rotate-out': 85 * lift },
        elbow_right: { flex: lerpN(10, 78, lift) + 20 * sw },
        wrist_right: { ...fe(8 * swHand), 'tilt-left': 6 * up },
        fingers_right: { curl: 12 * (1 - up) + 4 },
        chest: { ...tl(-2.5 * up), ...tw(-3 * up) },
        neck: tl(2 * up),
        head: { 'tilt-right': 5 * up, ...tw(-2 * sw) },
      },
    };
  }, 'waves hello with one hand; params: side, loops'));

  // point: a quick drive to the target, a small overshoot and settle, a
  // hold, then the arm relaxes down slower than it went up
  clips.push(makeClip('point', 1.6, false, F, (p) => {
    const reach = p < 0.3 ? overshoot(p / 0.3, 0.1) : p > 0.72 ? 1 - ease((p - 0.72) / 0.28, 'flow') : 1;
    const hand = p < 0.34 ? ease(Math.max(0, p - 0.04) / 0.3, 'settle') : p > 0.72 ? 1 - ease((p - 0.72) / 0.22, 'snap') : 1;
    const body = p < 0.36 ? ease(p / 0.36, 'flow') : p > 0.7 ? 1 - ease((p - 0.7) / 0.3, 'flow') : 1;
    return {
      pose: {
        shoulder_right: { abduct: 8 + 10 * reach, flex: 82 * reach, 'rotate-in': 10 * reach },
        elbow_right: { flex: 8 + 10 * (1 - reach) },
        wrist_right: { extend: 6 * hand },
        fingers_right: { curl: 12 + 70 * hand }, index_right: { curl: 4 + 8 * (1 - hand) }, thumb_right: { curl: 12 + 40 * hand },
        chest: tw(-6 * body), spine: { ...tw(-3 * body), ...fe(3 * body) },
        neck: tw(-4 * body), head: { 'twist-right': 6 * body, ...fe(2 * body) },
      },
    };
  }, 'points at a target; params: side'));

  clips.push(makeClip('gesture', 1.8, false, F, (p) => {
    const env = Math.min(1, p * 4, (1 - p) * 3);
    return {
      pose: {
        shoulders: { abduct: 8 + 22 * env, flex: 28 * env, 'rotate-out': 40 * env }, elbows: { flex: 8 + 72 * env },
        wrists: { extend: 25 * env }, fingers: { curl: 5 }, clavicles: { elevate: 6 * env },
        head: { 'tilt-left': 6 * env },
      },
    };
  }, 'open palms, explaining'));
  clips.push(makeClip('nod', 1.0, false, F, (p) => ({
    pose: { neck: fe(14 * Math.sin(TAU * 2 * p) * Math.min(1, (1 - p) * 4)) },
  }), 'nods yes'));
  clips.push(makeClip('shrug', 1.2, false, F, (p) => {
    const env = Math.min(1, p * 4, (1 - p) * 3);
    return {
      pose: {
        clavicles: { elevate: 18 * env }, shoulders: { abduct: 8 + 12 * env, 'rotate-out': 50 * env },
        elbows: { flex: 8 + 80 * env }, wrists: { extend: 20 * env }, head: { 'tilt-right': 8 * env },
      },
    };
  }, 'shrugs, not sure'));
  clips.push(makeClip('clap', 1.4, false, F, (p) => {
    const env = Math.min(1, p * 5, (1 - p) * 5);
    const c = 0.5 + 0.5 * Math.cos(TAU * 3 * p);
    return {
      pose: {
        shoulders: { abduct: 8 + 10 * env, flex: 45 * env, 'rotate-in': 30 * env },
        elbows: { flex: 8 + (60 + 20 * c) * env }, fingers: { curl: 5 },
      },
    };
  }, 'claps'));
  clips.push(makeClip('think', 2.2, false, F, (p) => {
    const env = Math.min(1, p * 3, (1 - p) * 3);
    return {
      pose: {
        shoulder_right: { abduct: 10, flex: 40 * env }, elbow_right: { flex: 8 + 130 * env },
        wrist_right: { extend: 20 * env }, fingers_right: { curl: 50 * env },
        shoulder_left: { abduct: 10, flex: 25 * env }, elbow_left: { flex: 8 + 80 * env },
        head: { 'tilt-left': 8 * env, 'twist-left': 6 * env }, neck: { extend: 6 * env },
      },
    };
  }, 'hand on chin, thinking'));
  clips.push(makeClip('cheer', 1.6, false, F, (p) => {
    const env = Math.min(1, p * 4, (1 - p) * 3);
    return {
      pose: {
        shoulders: { abduct: 8 + 150 * env, flex: 15 * env }, elbows: { flex: 8 + 20 * env + 15 * Math.sin(TAU * 3 * p) * env },
        fingers: { curl: 70 * env }, neck: { extend: 12 * env },
      },
      hips: [0, 0.03 * Math.max(0, Math.sin(TAU * 2 * p)) * env, 0],
    };
  }, 'arms up, celebrating'));
  return clips;
}
