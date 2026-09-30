// Procedural layers on top of the clips: two-bone IK (feet on the ground,
// hand to an anchor) and look-at on neck, head and eyes.
import * as THREE from 'three';

const _a = new THREE.Vector3();
const _b = new THREE.Vector3();
const _c = new THREE.Vector3();
const _t = new THREE.Vector3();
const _v1 = new THREE.Vector3();
const _v2 = new THREE.Vector3();
const _n = new THREE.Vector3();
const _q = new THREE.Quaternion();
const _qw = new THREE.Quaternion();
const _qp = new THREE.Quaternion();

/** Rotates `bone` by a world-space delta rotation (keeps the parent). */
export function rotateWorld(bone: THREE.Object3D, delta: THREE.Quaternion) {
  bone.getWorldQuaternion(_qw);
  const newWorld = _qw.premultiply(delta);
  if (bone.parent) bone.parent.getWorldQuaternion(_qp);
  else _qp.identity();
  bone.quaternion.copy(_qp.invert().multiply(newWorld));
  bone.updateMatrixWorld(true);
}

/**
 * Analytic two-bone IK: moves the chain root->mid->end so `end` reaches
 * `target` (world), bending towards `pole` (world direction). `weight` blends
 * from the animated pose.
 */
export function solveTwoBone(
  root: THREE.Bone, mid: THREE.Bone, end: THREE.Bone,
  target: THREE.Vector3, pole: THREE.Vector3, weight = 1,
) {
  if (weight <= 0) return;
  root.getWorldPosition(_a);
  mid.getWorldPosition(_b);
  end.getWorldPosition(_c);
  const la = _a.distanceTo(_b);
  const lb = _b.distanceTo(_c);
  _t.copy(_c).lerp(target, Math.min(1, weight));
  const toT = _v1.subVectors(_t, _a);
  const d = THREE.MathUtils.clamp(toT.length(), Math.abs(la - lb) + 1e-4, la + lb - 1e-4);
  const dir = toT.normalize();
  // bend plane from the pole
  _n.crossVectors(dir, pole);
  if (_n.lengthSq() < 1e-8) _n.crossVectors(dir, _v2.set(0, 0, 1));
  _n.normalize();
  const cosA = THREE.MathUtils.clamp((la * la + d * d - lb * lb) / (2 * la * d), -1, 1);
  const angA = Math.acos(cosA);
  // desired mid position: rotate dir by angA around n (towards the pole side)
  const midDir = _v2.copy(dir).applyAxisAngle(_n, angA);
  const newMid = midDir.multiplyScalar(la).add(_a);
  // rotate root so the mid goes to newMid
  _q.setFromUnitVectors(_v1.subVectors(_b, _a).normalize(), _t.clone().subVectors(newMid, _a).normalize());
  rotateWorld(root, _q);
  mid.getWorldPosition(_b);
  end.getWorldPosition(_c);
  const targetEnd = _a.copy(_c).lerp(target, Math.min(1, weight));
  const cur = _v1.subVectors(_c, _b).normalize();
  const want = _v2.subVectors(targetEnd, _b).normalize();
  _q.setFromUnitVectors(cur, want);
  rotateWorld(mid, _q);
}

export interface LookState { yaw: number; pitch: number }

/** Turns neck, head and eyes towards `target` (world), limited and smoothed.
 * `bodyQuat` is the character root world rotation. */
export function applyLook(
  state: LookState, target: THREE.Vector3 | null, dt: number,
  neck: THREE.Bone, head: THREE.Bone, eyes: THREE.Bone[], bodyQuat: THREE.Quaternion,
) {
  let yaw = 0, pitch = 0;
  if (target) {
    head.getWorldPosition(_a);
    const dir = _b.subVectors(target, _a).applyQuaternion(_q.copy(bodyQuat).invert());
    yaw = Math.atan2(dir.x, dir.z);
    pitch = Math.atan2(dir.y, Math.hypot(dir.x, dir.z));
    if (Math.abs(yaw) > 2.2) yaw = 0; // behind: give up rather than snap
    yaw = THREE.MathUtils.clamp(yaw, -1.25, 1.25);
    pitch = THREE.MathUtils.clamp(pitch, -0.6, 0.5);
  }
  const k = 1 - Math.exp(-dt * 6);
  state.yaw += (yaw - state.yaw) * k;
  state.pitch += (pitch - state.pitch) * k;
  const add = (bone: THREE.Bone, f: number) => {
    const q = _qp.setFromEuler(new THREE.Euler(-state.pitch * f, state.yaw * f, 0, 'YXZ'));
    bone.quaternion.premultiply(q);
  };
  add(neck, 0.35);
  add(head, 0.45);
  const rest = 0.2;
  for (const e of eyes) {
    const q = _qp.setFromEuler(new THREE.Euler(
      THREE.MathUtils.clamp(-state.pitch * rest, -0.25, 0.25),
      THREE.MathUtils.clamp(state.yaw * rest, -0.35, 0.35), 0, 'YXZ'));
    e.quaternion.premultiply(q);
  }
}
