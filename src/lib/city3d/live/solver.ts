// Our own solver (estudo 13 §1): MediaPipe output in, LivePose out.
//
// - Face: the 52 blendshapes go straight into expressions (and, untouched,
//   into `blendshapes` for perfect-sync avatars); the head rotation comes from
//   the facial transformation matrix.
// - Body: world landmarks (metres, hip-centred) become per-bone rotations in
//   the parent's space. Each limb gets a full frame (bone axis plus bend plane),
//   so the elbow and the knee stay pure hinges and the upper bone gets its twist.
// - Every rotation passes a One Euro filter and a per-joint limit, the same
//   limits the LLM gestures are clamped to.

import { BODY_FILTER, FACE_FILTER, OneEuroBank, OneEuroQuatBank, WEIGHT_FILTER } from './filter';
import {
  ARKIT_BLENDSHAPES,
  type ArkitName,
  type ExpressionName,
  type FingerName,
  type LivePose,
  type PoseBone,
} from './pose';
import {
  DEG,
  QI,
  clamp,
  cross,
  dot,
  len,
  limitSwingTwist,
  mid,
  norm,
  qAxisAngle,
  qConj,
  qFromFrames,
  qFromMat4,
  qFromUnitVectors,
  qMirrorX,
  qMul,
  qRotate,
  qSlerp,
  sub,
  type Quat,
  type Vec3,
} from './math';

/** Face result, already flattened by the vision runner. */
export interface FaceFrame {
  /** Weights in `ARKIT_BLENDSHAPES` order. */
  blendshapes: ArrayLike<number>;
  /** Column-major 4×4 facial transformation matrix, if the model gave one. */
  matrix?: ArrayLike<number> | null;
}

/** Pose result: 33 world landmarks (x, y, z interleaved) and their visibility. */
export interface BodyFrame {
  world: ArrayLike<number>;
  visibility: ArrayLike<number>;
}

/** Hand result: 21 world landmarks per hand, keyed by the anatomical side. */
export interface HandsFrame {
  left?: ArrayLike<number>;
  right?: ArrayLike<number>;
}

export interface SolverOptions {
  /** Mirror left/right (selfie view). */
  mirror: boolean;
  /** Also solve the legs (only useful with the whole body in frame). */
  legs: boolean;
  /** Keep the hips still (desk webcam). */
  lockHips: boolean;
  /** Minimum landmark visibility to trust a limb. */
  minVisibility: number;
}

export const DEFAULT_SOLVER: SolverOptions = {
  mirror: false,
  legs: false,
  lockHips: true,
  minVisibility: 0.5,
};

/** Per-bone limits: twist axis (rest direction), max swing and max twist. */
interface Limit {
  axis: Vec3;
  swing: number;
  twist: number;
}

const X: Vec3 = [1, 0, 0];
const Y: Vec3 = [0, 1, 0];
const Z: Vec3 = [0, 0, 1];
const NX: Vec3 = [-1, 0, 0];
const NY: Vec3 = [0, -1, 0];

export const JOINT_LIMITS: Record<PoseBone, Limit> = {
  hips: { axis: Y, swing: 45 * DEG, twist: 60 * DEG },
  spine: { axis: Y, swing: 35 * DEG, twist: 30 * DEG },
  chest: { axis: Y, swing: 25 * DEG, twist: 25 * DEG },
  neck: { axis: Y, swing: 40 * DEG, twist: 60 * DEG },
  head: { axis: Y, swing: 35 * DEG, twist: 50 * DEG },
  leftUpperArm: { axis: X, swing: 170 * DEG, twist: 90 * DEG },
  leftLowerArm: { axis: X, swing: 155 * DEG, twist: 90 * DEG },
  leftHand: { axis: X, swing: 80 * DEG, twist: 20 * DEG },
  rightUpperArm: { axis: NX, swing: 170 * DEG, twist: 90 * DEG },
  rightLowerArm: { axis: NX, swing: 155 * DEG, twist: 90 * DEG },
  rightHand: { axis: NX, swing: 80 * DEG, twist: 20 * DEG },
  leftUpperLeg: { axis: NY, swing: 130 * DEG, twist: 45 * DEG },
  leftLowerLeg: { axis: NY, swing: 150 * DEG, twist: 5 * DEG },
  leftFoot: { axis: Z, swing: 50 * DEG, twist: 20 * DEG },
  rightUpperLeg: { axis: NY, swing: 130 * DEG, twist: 45 * DEG },
  rightLowerLeg: { axis: NY, swing: 150 * DEG, twist: 5 * DEG },
  rightFoot: { axis: Z, swing: 50 * DEG, twist: 20 * DEG },
};

export function applyLimit(bone: PoseBone, q: Quat): Quat {
  const l = JOINT_LIMITS[bone];
  return limitSwingTwist(q, l.axis, l.swing, l.twist);
}

// MediaPipe pose landmark indices ("left" = the subject's left).
const P = {
  nose: 0,
  lEar: 7,
  rEar: 8,
  lShoulder: 11,
  rShoulder: 12,
  lElbow: 13,
  rElbow: 14,
  lWrist: 15,
  rWrist: 16,
  lPinky: 17,
  rPinky: 18,
  lIndex: 19,
  rIndex: 20,
  lHip: 23,
  rHip: 24,
  lKnee: 25,
  rKnee: 26,
  lAnkle: 27,
  rAnkle: 28,
  lHeel: 29,
  rHeel: 30,
  lFoot: 31,
  rFoot: 32,
} as const;

const BS_INDEX: Record<string, number> = Object.fromEntries(ARKIT_BLENDSHAPES.map((n, i) => [n, i]));

/** Swaps Left/Right in an ARKit name. */
const swapSide = (n: string) =>
  n.includes('Left') ? n.replace('Left', 'Right') : n.includes('Right') ? n.replace('Right', 'Left') : n;

// Swaps a MediaPipe pose landmark index for its mirror twin.
const MIRROR_INDEX: number[] = (() => {
  const m = Array.from({ length: 33 }, (_, i) => i);
  const pairs = [
    [1, 4], [2, 5], [3, 6], [7, 8], [9, 10], [11, 12], [13, 14], [15, 16], [17, 18], [19, 20],
    [21, 22], [23, 24], [25, 26], [27, 28], [29, 30], [31, 32],
  ];
  for (const [a, b] of pairs) {
    m[a] = b;
    m[b] = a;
  }
  return m;
})();

export class LiveSolver {
  opts: SolverOptions;
  private qf = new OneEuroQuatBank(BODY_FILTER);
  private hf = new OneEuroQuatBank(FACE_FILTER);
  private wf = new OneEuroBank(WEIGHT_FILTER);
  private body: Partial<Record<PoseBone, Quat>> = {};
  private chestWorld: Quat = QI;
  private seq = 0;
  private hands: Partial<Record<FingerName, number>> | undefined;
  private face: Pick<LivePose, 'expressions' | 'gaze' | 'blendshapes' | 'headMatrix'> & { head?: Quat } = {
    expressions: {},
  };

  constructor(opts: Partial<SolverOptions> = {}) {
    this.opts = { ...DEFAULT_SOLVER, ...opts };
  }

  reset(): void {
    this.qf.reset();
    this.hf.reset();
    this.wf.reset();
    this.body = {};
    this.chestWorld = QI;
    this.face = { expressions: {} };
    this.hands = undefined;
  }

  /** Feeds a face result (30 Hz path). */
  updateFace(face: FaceFrame, tMs: number): void {
    const t = tMs / 1000;
    const raw: Partial<Record<ArkitName, number>> = {};
    for (let i = 0; i < ARKIT_BLENDSHAPES.length; i++) {
      let name: string = ARKIT_BLENDSHAPES[i];
      if (this.opts.mirror) name = swapSide(name);
      raw[name as ArkitName] = this.wf.filter(name, clamp(face.blendshapes[i] ?? 0, 0, 1), t);
    }
    this.face.blendshapes = raw;
    this.face.expressions = blendshapesToExpressions(raw);
    this.face.gaze = blendshapesToGaze(raw);
    if (face.matrix && face.matrix.length >= 16) {
      this.face.headMatrix = Array.from(face.matrix);
      let q = qFromMat4(face.matrix);
      if (this.opts.mirror) q = qMirrorX(q);
      this.face.head = this.hf.filter('head', q, t);
    }
  }

  /** Feeds a pose result (15 Hz path). */
  updateBody(body: BodyFrame, tMs: number): void {
    const t = tMs / 1000;
    const vis = (i: number) => body.visibility[this.side(i)] ?? 0;
    const p = (i: number): Vec3 => {
      const k = this.side(i) * 3;
      // MediaPipe: x to the image right, y down, z away from the camera.
      // Avatar: faces +Z toward the camera, its left is +X, up is +Y.
      const x = body.world[k];
      return [this.opts.mirror ? -x : x, -body.world[k + 1], -body.world[k + 2]];
    };
    const ok = (...ids: number[]) => ids.every((i) => vis(i) >= this.opts.minVisibility);
    const out: Partial<Record<PoseBone, Quat>> = {};
    const set = (bone: PoseBone, q: Quat) => {
      out[bone] = this.qf.filter(bone, applyLimit(bone, q), t);
    };

    // Torso. Desk webcams rarely see the hips well, so "up" falls back to +Y.
    if (!ok(P.lShoulder, P.rShoulder)) {
      this.body = out;
      return;
    }
    const ls = p(P.lShoulder);
    const rs = p(P.rShoulder);
    const hipsOk = ok(P.lHip, P.rHip);
    const hipMid = hipsOk ? mid(p(P.lHip), p(P.rHip)) : ([0, 0, 0] as Vec3);
    const up = hipsOk ? norm(sub(mid(ls, rs), hipMid)) : Y;
    const chestW = qFromFrames(X, Y, norm(sub(ls, rs)), up);
    let hipsW: Quat = QI;
    if (!this.opts.lockHips && hipsOk) {
      hipsW = qFromFrames(X, Y, norm(sub(p(P.lHip), p(P.rHip))), up);
      set('hips', hipsW);
      hipsW = out.hips!;
    }
    const torso = qMul(qConj(hipsW), chestW);
    const spineL = qSlerp(QI, torso, 0.5);
    set('spine', spineL);
    const spineW = qMul(hipsW, out.spine!);
    set('chest', qMul(qConj(spineW), qMul(hipsW, torso)));
    this.chestWorld = qMul(spineW, out.chest!);

    // Arms: rest axis ±X, the elbow bends toward +Z (hinge -Y left, +Y right).
    const arm = (side: 'left' | 'right') => {
      const L = side === 'left';
      const [s, e, w, pi, ix] = L
        ? [P.lShoulder, P.lElbow, P.lWrist, P.lPinky, P.lIndex]
        : [P.rShoulder, P.rElbow, P.rWrist, P.rPinky, P.rIndex];
      if (!ok(s, e)) return;
      const a0: Vec3 = L ? X : NX;
      const h0: Vec3 = L ? NY : Y;
      const parent = this.chestWorld;
      const a = norm(sub(p(e), p(s)));
      const wristOk = ok(w);
      const f = wristOk ? norm(sub(p(w), p(e))) : a;
      const bend = cross(a, f);
      let upperW: Quat;
      let elbow = 0;
      if (len(bend) > 0.05) {
        upperW = qFromFrames(a0, h0, a, norm(bend));
        elbow = Math.acos(clamp(dot(a, f), -1, 1));
      } else {
        upperW = qMul(parent, qFromUnitVectors(a0, qRotate(qConj(parent), a)));
        if (wristOk) elbow = Math.acos(clamp(dot(a, f), -1, 1));
      }
      const upperBone: PoseBone = L ? 'leftUpperArm' : 'rightUpperArm';
      const lowerBone: PoseBone = L ? 'leftLowerArm' : 'rightLowerArm';
      const handBone: PoseBone = L ? 'leftHand' : 'rightHand';
      set(upperBone, qMul(qConj(parent), upperW));
      const upperFW = qMul(parent, out[upperBone]!);
      set(lowerBone, qAxisAngle(h0, clamp(elbow, 0, 150 * DEG)));
      const lowerW = qMul(upperFW, out[lowerBone]!);
      if (wristOk && ok(pi, ix)) {
        const wp = p(w);
        const knuckles = mid(p(pi), p(ix));
        const dir = norm(sub(knuckles, wp));
        const normal = norm(cross(sub(p(ix), wp), sub(p(pi), wp)));
        const handW = qFromFrames(a0, L ? Y : NY, dir, normal);
        set(handBone, qMul(qConj(lowerW), handW));
      }
    };
    arm('left');
    arm('right');

    // Legs: rest axis -Y, the knee bends toward -Z (hinge +X).
    if (this.opts.legs && hipsOk) {
      const leg = (side: 'left' | 'right') => {
        const L = side === 'left';
        const [h, k, an, heel, toe] = L
          ? [P.lHip, P.lKnee, P.lAnkle, P.lHeel, P.lFoot]
          : [P.rHip, P.rKnee, P.rAnkle, P.rHeel, P.rFoot];
        if (!ok(h, k, an)) return;
        const a = norm(sub(p(k), p(h)));
        const f = norm(sub(p(an), p(k)));
        const bend = cross(a, f);
        const upperW =
          len(bend) > 0.05
            ? qFromFrames(NY, X, a, norm(bend))
            : qMul(hipsW, qFromUnitVectors(NY, qRotate(qConj(hipsW), a)));
        const knee = Math.acos(clamp(dot(a, f), -1, 1));
        const ub: PoseBone = L ? 'leftUpperLeg' : 'rightUpperLeg';
        const lb: PoseBone = L ? 'leftLowerLeg' : 'rightLowerLeg';
        const fb: PoseBone = L ? 'leftFoot' : 'rightFoot';
        set(ub, qMul(qConj(hipsW), upperW));
        set(lb, qAxisAngle(X, clamp(knee, 0, 150 * DEG)));
        const lowerW = qMul(qMul(hipsW, out[ub]!), out[lb]!);
        if (ok(heel, toe)) {
          const d = norm(sub(p(toe), p(heel)));
          set(fb, qFromUnitVectors(Z, qRotate(qConj(lowerW), d)));
        }
      };
      leg('left');
      leg('right');
    }

    // Head from the pose when there is no face result.
    if (!this.face.head && ok(P.lEar, P.rEar, P.nose)) {
      const le = p(P.lEar);
      const re = p(P.rEar);
      const fwd = sub(p(P.nose), mid(le, re));
      // The nose sits ~15° below the ear line when looking straight ahead.
      const headW = qMul(qFromFrames(X, Z, norm(sub(le, re)), norm(fwd)), qAxisAngle(X, -15 * DEG));
      this.bodyHead = this.hf.filter('pose-head', headW, t);
    } else {
      this.bodyHead = undefined;
    }
    this.body = out;
  }

  private bodyHead: Quat | undefined;

  /** Landmark index to read for an anatomical index (mirror-aware). */
  private side(i: number): number {
    return this.opts.mirror ? MIRROR_INDEX[i] : i;
  }

  /** Feeds hand landmarks (optional path). */
  updateHands(hands: HandsFrame, tMs: number): void {
    const t = tMs / 1000;
    const out: Partial<Record<FingerName, number>> = {};
    const one = (lm: ArrayLike<number> | undefined, side: 'left' | 'right') => {
      if (!lm) return;
      const v = (i: number): Vec3 => [lm[i * 3], lm[i * 3 + 1], lm[i * 3 + 2]];
      // Chains: thumb 1-4, index 5-8, middle 9-12, ring 13-16, little 17-20.
      const chains: [FingerName, number][] =
        side === 'left'
          ? [['leftThumb', 1], ['leftIndex', 5], ['leftMiddle', 9], ['leftRing', 13], ['leftLittle', 17]]
          : [['rightThumb', 1], ['rightIndex', 5], ['rightMiddle', 9], ['rightRing', 13], ['rightLittle', 17]];
      for (const [name, base] of chains) {
        let bendSum = 0;
        let prev = norm(sub(v(base), v(0)));
        for (let j = 0; j < 3; j++) {
          const seg = norm(sub(v(base + j + 1), v(base + j)));
          bendSum += Math.acos(clamp(dot(prev, seg), -1, 1));
          prev = seg;
        }
        const full = name.endsWith('Thumb') ? 1.6 : 2.6;
        out[name] = this.wf.filter(name, clamp(bendSum / full, 0, 1), t);
      }
    };
    const [l, r] = this.opts.mirror ? [hands.right, hands.left] : [hands.left, hands.right];
    one(l, 'left');
    one(r, 'right');
    this.hands = out;
  }

  /** Composes the latest face, body and hands into one pose. */
  pose(tMs: number): LivePose {
    const bones: Partial<Record<PoseBone, Quat>> = { ...this.body };
    const headW = this.face.head ?? this.bodyHead;
    if (headW) {
      // Head world → local under the chest, split 40/60 between neck and head.
      const total = qMul(qConj(this.chestWorld), headW);
      const neck = applyLimit('neck', qSlerp(QI, total, 0.4));
      bones.neck = neck;
      bones.head = applyLimit('head', qMul(qConj(neck), total));
    }
    const pose: LivePose = {
      seq: this.seq = (this.seq + 1) & 0xffff,
      t: tMs,
      bones,
      expressions: { ...this.face.expressions },
    };
    if (this.face.gaze) pose.gaze = this.face.gaze;
    if (this.face.blendshapes) pose.blendshapes = this.face.blendshapes;
    if (this.face.headMatrix) pose.headMatrix = this.face.headMatrix;
    if (this.hands) pose.fingers = this.hands;
    return pose;
  }
}

const bs = (b: Partial<Record<ArkitName, number>>, n: ArkitName) => b[n] ?? 0;

/** ARKit weights → the 12 VRM preset expressions. */
export function blendshapesToExpressions(b: Partial<Record<ArkitName, number>>): Partial<Record<ExpressionName, number>> {
  const jaw = bs(b, 'jawOpen');
  const funnel = bs(b, 'mouthFunnel');
  const pucker = bs(b, 'mouthPucker');
  const smile = (bs(b, 'mouthSmileLeft') + bs(b, 'mouthSmileRight')) / 2;
  const stretch = (bs(b, 'mouthStretchLeft') + bs(b, 'mouthStretchRight')) / 2;
  const frown = (bs(b, 'mouthFrownLeft') + bs(b, 'mouthFrownRight')) / 2;
  const browDown = (bs(b, 'browDownLeft') + bs(b, 'browDownRight')) / 2;
  const browUp = (bs(b, 'browOuterUpLeft') + bs(b, 'browOuterUpRight')) / 2;
  const eyeWide = (bs(b, 'eyeWideLeft') + bs(b, 'eyeWideRight')) / 2;
  const sneer = (bs(b, 'noseSneerLeft') + bs(b, 'noseSneerRight')) / 2;
  const inner = bs(b, 'browInnerUp');
  const c = (v: number) => clamp(v, 0, 1);
  // Visemes: the mouth shape decides the vowel, the jaw how open it is.
  const round = Math.max(funnel, pucker);
  const oh = c(funnel * 1.4 * (0.4 + jaw));
  const ou = c(pucker * 1.3 * (1 - jaw * 0.5));
  const aa = c(jaw * 1.6 * (1 - round * 0.8));
  const ee = c((stretch * 1.5 + smile * 0.6) * (1 - round) * (0.3 + jaw));
  const ih = c((stretch + smile) * 0.6 * (1 - jaw) * (1 - round));
  const happy = c((smile - 0.15) * 1.6);
  return {
    aa,
    ih,
    ou,
    ee,
    oh,
    blinkLeft: c((bs(b, 'eyeBlinkLeft') - 0.1) * 1.4),
    blinkRight: c((bs(b, 'eyeBlinkRight') - 0.1) * 1.4),
    happy,
    angry: c((browDown - 0.2) * 1.5 + sneer * 0.5),
    sad: c((frown - 0.1) * 1.5 + inner * 0.4 - happy),
    relaxed: c(smile * 0.5 * (1 - jaw) - browDown),
    surprised: c((browUp * 0.6 + eyeWide * 0.6 + jaw * 0.3 - 0.3) * 1.4),
  };
}

/** Eye-look blendshapes → gaze angles (±~25°). */
export function blendshapesToGaze(b: Partial<Record<ArkitName, number>>) {
  // The avatar's left is its +X. Looking to the left: the left eye looks out
  // and the right eye looks in.
  const yaw =
    ((bs(b, 'eyeLookOutLeft') - bs(b, 'eyeLookInLeft') + bs(b, 'eyeLookInRight') - bs(b, 'eyeLookOutRight')) / 2) *
    25 * DEG;
  const pitch =
    ((bs(b, 'eyeLookUpLeft') + bs(b, 'eyeLookUpRight') - bs(b, 'eyeLookDownLeft') - bs(b, 'eyeLookDownRight')) / 2) *
    20 * DEG;
  return { yaw, pitch };
}

export { BS_INDEX };

