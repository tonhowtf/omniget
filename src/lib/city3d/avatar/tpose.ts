// The VRM T-pose of an Anny rig. The Anny rest keeps the forearm bent ~42
// degrees, a slight wrist and finger curl and soft knees; the VRM export bakes
// the T-pose into the mesh, and the character system uses the same rotations
// to apply live poses given on the normalized rig (identity = T-pose).

import * as THREE from 'three';
import type { CharacterRig } from './assemble';

const FINGER_CHAINS = ['Index', 'Middle', 'Ring', 'Little'].map((f) => [`${f}Proximal`, `${f}Intermediate`, `${f}Distal`]);

/** Rotates `bone` (in world space) so the direction to `child` becomes `dir`. */
function aim(bone: THREE.Object3D, child: THREE.Object3D, dir: THREE.Vector3) {
  const from = child.getWorldPosition(new THREE.Vector3()).sub(bone.getWorldPosition(new THREE.Vector3())).normalize();
  const q = new THREE.Quaternion().setFromUnitVectors(from, dir.clone().normalize());
  const world = bone.getWorldQuaternion(new THREE.Quaternion()).premultiply(q);
  const parent = bone.parent!.getWorldQuaternion(new THREE.Quaternion()).invert();
  bone.quaternion.copy(parent.multiply(world));
  bone.updateMatrixWorld(true);
}

/** Poses the arm, finger and leg chains of a rig into the VRM T-pose. */
export function aimTPose(rig: CharacterRig) {
  const b = (n: string) => rig.byName.get(n);
  for (const [side, sx] of [['left', 1], ['right', -1]] as const) {
    const x = new THREE.Vector3(sx, 0, 0);
    const chain = ['UpperArm', 'LowerArm', 'Hand', 'MiddleProximal'].map((n) => b(side + n));
    for (let i = 0; i + 1 < chain.length; i++) if (chain[i] && chain[i + 1]) aim(chain[i]!, chain[i + 1]!, x);
    for (const f of FINGER_CHAINS) {
      const [p, m, d] = f.map((n) => b(side + n));
      if (!p || !m || !d) continue;
      const dir = m.getWorldPosition(new THREE.Vector3()).sub(p.getWorldPosition(new THREE.Vector3()));
      aim(m, d, dir);
    }
    const [up, low, foot] = ['UpperLeg', 'LowerLeg', 'Foot'].map((n) => b(side + n));
    if (up && low && foot) {
      const dir = foot.getWorldPosition(new THREE.Vector3()).sub(up.getWorldPosition(new THREE.Vector3()));
      aim(up, low, dir);
      aim(low, foot, dir);
    }
  }
  rig.root.updateMatrixWorld(true);
}

/**
 * Local and world rotations of every bone in the VRM T-pose, measured on a
 * rig at rest and put back afterwards. Live poses (webcam, LLM gestures) are
 * rotations on the normalized rig, whose identity is the T-pose, while the
 * Anny rest keeps the elbows bent; the character system converts with these.
 */
export function tposeRotations(rig: CharacterRig): {
  local: Map<string, THREE.Quaternion>;
  world: Map<string, THREE.Quaternion>;
} {
  const saved = rig.bones.map((bone) => bone.quaternion.clone());
  const savedPos = rig.bones.map((bone) => bone.position.clone());
  // `aim` works in world space: face the root down +Z while measuring, or a
  // resident that stands turned gets arms aimed along the world X axis
  // instead of its own (the T-pose would carry its yaw at that moment).
  const savedRoot = rig.root.quaternion.clone();
  rig.root.quaternion.identity();
  // measure from the bind pose, not from whatever the animation left
  rig.bones.forEach((bone) => bone.quaternion.identity());
  rig.root.updateMatrixWorld(true);
  aimTPose(rig);
  const rootInv = rig.root.getWorldQuaternion(new THREE.Quaternion()).invert();
  const local = new Map<string, THREE.Quaternion>();
  const world = new Map<string, THREE.Quaternion>();
  for (const [name, bone] of rig.byName) {
    local.set(name, bone.quaternion.clone());
    world.set(name, rootInv.clone().multiply(bone.getWorldQuaternion(new THREE.Quaternion())));
  }
  rig.bones.forEach((bone, k) => {
    bone.quaternion.copy(saved[k]);
    bone.position.copy(savedPos[k]);
  });
  rig.root.quaternion.copy(savedRoot);
  rig.root.updateMatrixWorld(true);
  return { local, world };
}

