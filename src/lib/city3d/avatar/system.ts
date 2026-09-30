// The resident system the city view drives: spawn by genome, move with
// setTarget, and it picks the cost tier by distance to the camera:
// - full (nearest, up to `maxFull`): clothing geometry, expressions, IK, outline;
// - reduced: body with region colors, clips only;
// - crowd: GPU-skinned instances (crowd.ts).
import * as THREE from 'three';
import { buildBody, loadAnny, type AnnyBase } from './anny';
import { Animator } from './animator';
import { buildCharacter, phenotypesOf, type CharacterRig } from './assemble';
import { catalog } from './catalog';
import { loadClips, type Clip } from './clips';
import { Crowd, archetypeOf } from './crowd';
import { MOOD_FACE, toArkit, VRM_PRESETS } from './expressions';
import { decodeGenome, genomeKey, mulberry32, type Genome } from './genome';
import { applyLook, solveTwoBone, type LookState } from './ik';
import { tposeRotations } from './tpose';
import { fingerRotations, type LivePose } from './livepose';
import { srgbToLinear } from './materials';
import { coverBits, outfitColors } from './outfit';
import type { GestureAction } from './posecode';

export interface Vec3 { x: number; y: number; z: number }

export interface CharacterSystemOptions {
  /** base URL of the baked body (default `/city3d/anny/`) */
  annyUrl?: string;
  /** clips JSON (default `/city3d/clips/clips.json`) */
  clipsUrl?: string;
  /** most characters in the full tier (default 8) */
  maxFull?: number;
  /** most characters in the reduced tier (default 120) */
  maxReduced?: number;
  fullDistance?: number;
  reducedDistance?: number;
  /** beyond this nobody is drawn (default 400 m) */
  crowdDistance?: number;
  /** terrain height for feet IK; flat at the target y when absent */
  groundAt?: (x: number, z: number) => number;
  /** rigs built per frame when tiers change (default 1) */
  buildsPerFrame?: number;
  outline?: boolean;
}

export type TierName = 'full' | 'reduced' | 'crowd' | 'hidden';

export interface TargetExtra {
  /** world point the right hand reaches during the action */
  reach?: Vec3 | null;
  /** world point to look at */
  lookAt?: Vec3 | null;
  /** walking speed multiplier */
  speed?: number;
}

interface Resident {
  id: string;
  hex: string;
  genome: Genome;
  pos: THREE.Vector3;
  yaw: number;
  target: THREE.Vector3;
  targetYaw: number;
  action: string;
  moving: boolean;
  speed: number;
  height: number;
  archetype: string;
  scale: THREE.Vector3;
  tier: TierName;
  want: TierName;
  rig: CharacterRig | null;
  rigTier: 'full' | 'reduced' | null;
  /** T-pose rotations of the rig, measured lazily for live poses. */
  tpose: { local: Map<string, THREE.Quaternion>; world: Map<string, THREE.Quaternion>; parent: Map<string, string> } | null;
  anim: Animator | null;
  crowdTime: number;
  phase: number;
  look: LookState;
  lookAt: THREE.Vector3 | null;
  reach: THREE.Vector3 | null;
  reachW: number;
  expression: { weights: Record<string, number>; until: number } | null;
  pose: { data: LivePose; at: number } | null;
  blinkNext: number;
  blinkT: number;
  crowdColors: { skin: number[]; top: number[]; bottom: number[]; shoes: number[]; hair: number[] };
  cover: [number, number, number, number];
  dist: number;
}

export interface CharacterSystem {
  ready: Promise<void>;
  spawn(id: string, genome: string, at?: { pos?: Vec3; yaw?: number }): void;
  despawn(id: string): void;
  setTarget(id: string, pos: Vec3, yaw: number, action: string, extra?: TargetExtra): void;
  applyPose(id: string, pose: LivePose | null): void;
  playGesture(id: string, action: GestureAction): string[];
  setExpression(id: string, name: string, weight: number, holdSeconds?: number): void;
  lookAt(id: string, target: Vec3 | null): void;
  update(dt: number, camera: THREE.Camera): void;
  /** Object3D of a resident in the full or reduced tier (for picking). */
  object(id: string): THREE.Object3D | null;
  tierOf(id: string): TierName | null;
  ids(): string[];
  stats(): { residents: number; full: number; reduced: number; crowd: number; hidden: number };
  dispose(): void;
}

const GAIT_SPEED = [1.0, 1.25, 0.85, 0.7, 1.05, 0.8, 1.0, 0.75];
const WALK_SPEED = 1.4;

export function createCharacterSystem(
  scene: THREE.Scene,
  renderer: THREE.WebGLRenderer | null,
  opts: CharacterSystemOptions = {},
): CharacterSystem {
  void renderer;
  const o = {
    maxFull: 8, maxReduced: 120, fullDistance: 16, reducedDistance: 50, crowdDistance: 400,
    buildsPerFrame: 1, outline: true, ...opts,
  };
  const group = new THREE.Group();
  group.name = 'residents';
  scene.add(group);
  const residents = new Map<string, Resident>();
  let base: AnnyBase | null = null;
  let clips: Map<string, Clip> | null = null;
  let crowd: Crowd | null = null;
  let clock = 0;
  let disposed = false;

  const ready = (async () => {
    [base, clips] = await Promise.all([loadAnny(opts.annyUrl), loadClips(opts.clipsUrl)]);
    if (disposed) return;
    crowd = new Crowd(base, clips);
    group.add(crowd.group);
  })();

  function describe(r: Resident) {
    const colors = outfitColors(catalog, r.genome);
    const lin = (c: [number, number, number]) => srgbToLinear(c);
    r.crowdColors = {
      skin: lin(colors.skin), top: lin(colors.top), bottom: lin(colors.bottom), shoes: lin(colors.shoes), hair: lin(colors.hair),
    };
    const bits = coverBits(catalog, r.genome);
    r.cover = [bits.top, bits.bottom, bits.shoes, bits.hair];
    const ph = phenotypesOf(r.genome);
    r.archetype = archetypeOf(ph);
    if (base) {
      r.height = buildBody(base, '369', ph).height;
      const ah = crowd?.archetypeHeight(r.archetype) ?? r.height;
      const girth = 0.92 + ph[3] * 0.2 + ph[2] * 0.06;
      const s = r.height / ah;
      r.scale.set(s * girth, s, s * girth);
    }
    r.speed = WALK_SPEED * (GAIT_SPEED[r.genome.gait] ?? 1) * Math.sqrt(Math.max(0.4, r.height) / 1.75);
  }

  function spawn(id: string, hex: string, at: { pos?: Vec3; yaw?: number } = {}) {
    despawn(id);
    let genome: Genome;
    try {
      genome = decodeGenome(hex);
    } catch (e) {
      console.warn('[residents] bad genome for', id, e);
      return;
    }
    const p = at.pos ? new THREE.Vector3(at.pos.x, at.pos.y, at.pos.z) : new THREE.Vector3();
    const rnd = mulberry32(genomeKey(hex) ^ genomeKey(id));
    const r: Resident = {
      id, hex, genome, pos: p.clone(), yaw: at.yaw ?? 0, target: p.clone(), targetYaw: at.yaw ?? 0,
      action: 'idle', moving: false, speed: WALK_SPEED, height: 1.7, archetype: 'm-adult',
      scale: new THREE.Vector3(1, 1, 1), tier: 'hidden', want: 'crowd', rig: null, rigTier: null, tpose: null, anim: null,
      crowdTime: rnd() * 10, phase: rnd(), look: { yaw: 0, pitch: 0 }, lookAt: null, reach: null, reachW: 0,
      expression: null, pose: null, blinkNext: 1 + rnd() * 4, blinkT: -1,
      crowdColors: { skin: [1, 1, 1], top: [1, 1, 1], bottom: [1, 1, 1], shoes: [1, 1, 1], hair: [1, 1, 1] },
      cover: [0, 0, 0, 0], dist: Infinity,
    };
    residents.set(id, r);
    if (base) describe(r);
    else ready.then(() => residents.get(id) === r && describe(r));
  }

  function dropRig(r: Resident) {
    r.rig?.dispose();
    r.rig = null;
    r.rigTier = null;
    r.anim = null;
  }

  function despawn(id: string) {
    const r = residents.get(id);
    if (!r) return;
    dropRig(r);
    crowd?.remove(id);
    residents.delete(id);
  }

  function setTarget(id: string, pos: Vec3, yaw: number, action: string, extra: TargetExtra = {}) {
    const r = residents.get(id);
    if (!r) return;
    r.target.set(pos.x, pos.y, pos.z);
    r.targetYaw = yaw;
    r.action = action || 'idle';
    if (extra.reach !== undefined) r.reach = extra.reach ? new THREE.Vector3(extra.reach.x, extra.reach.y, extra.reach.z) : null;
    if (extra.lookAt !== undefined) r.lookAt = extra.lookAt ? new THREE.Vector3(extra.lookAt.x, extra.lookAt.y, extra.lookAt.z) : null;
    if (extra.speed) r.speed = r.speed * extra.speed;
    if (r.pos.distanceTo(r.target) > 20) {
      r.pos.copy(r.target);
      r.yaw = yaw;
    }
  }

  function ensureAnim(r: Resident) {
    if (!r.anim && clips) r.anim = new Animator(clips, r.phase);
    return r.anim;
  }

  function clipFor(r: Resident): string {
    if (r.moving) return r.action === 'carry' ? 'carry' : 'walk';
    if (r.action === 'walk') return 'idle';
    return clips?.has(r.action) ? r.action : 'idle';
  }

  function simulate(r: Resident, dt: number) {
    const dx = r.target.x - r.pos.x, dz = r.target.z - r.pos.z;
    const d = Math.hypot(dx, dz);
    r.moving = d > 0.06;
    if (r.moving) {
      const want = Math.atan2(dx, dz);
      r.yaw = turn(r.yaw, want, dt * 7);
      const step = Math.min(d, r.speed * dt);
      r.pos.x += (dx / d) * step;
      r.pos.z += (dz / d) * step;
    } else {
      r.yaw = turn(r.yaw, r.targetYaw, dt * 4);
    }
    const gy = opts.groundAt ? opts.groundAt(r.pos.x, r.pos.z) : r.target.y;
    r.pos.y += (gy - r.pos.y) * Math.min(1, dt * 10);
  }

  function assignTiers(cam: THREE.Vector3) {
    const list = [...residents.values()];
    for (const r of list) r.dist = r.pos.distanceTo(cam);
    // A live-posed resident (the viewer's own avatar) goes first and keeps a
    // rig at any visible distance: poses only apply to full and reduced rigs.
    const live = (r: Resident) => !!r.pose && clock - r.pose.at < 1.5;
    const posed = (r: Resident) => (live(r) ? 0 : 1);
    list.sort((a, b) => posed(a) - posed(b) || a.dist - b.dist);
    let full = 0, reduced = 0;
    for (const r of list) {
      const hyst = (t: TierName, limit: number) => (r.tier === t ? limit * 1.15 : limit);
      if (live(r) && r.dist <= o.crowdDistance) {
        if (full < o.maxFull) { r.want = 'full'; full++; }
        else { r.want = 'reduced'; reduced++; }
      } else if (r.dist > o.crowdDistance) r.want = 'hidden';
      else if (full < o.maxFull && r.dist < hyst('full', o.fullDistance)) { r.want = 'full'; full++; }
      else if (reduced < o.maxReduced && r.dist < hyst('reduced', o.reducedDistance)) { r.want = 'reduced'; reduced++; }
      else r.want = 'crowd';
    }
  }

  function updateFace(r: Resident, dt: number) {
    const rig = r.rig!;
    const infl = rig.body.morphTargetInfluences;
    const dict = rig.body.morphTargetDictionary;
    if (!infl || !dict) return;
    const w: Record<string, number> = { ...(MOOD_FACE[catalog.moods[r.genome.mood] ?? 'neutral'] ?? {}) };
    // blink
    r.blinkNext -= dt;
    if (r.blinkNext <= 0 && r.blinkT < 0) r.blinkT = 0;
    if (r.blinkT >= 0) {
      r.blinkT += dt;
      w.blink = Math.max(w.blink ?? 0, Math.sin(Math.min(1, r.blinkT / 0.16) * Math.PI));
      if (r.blinkT > 0.16) {
        r.blinkT = -1;
        r.blinkNext = 12 / Math.max(1, r.genome.blink) + Math.random() * 2;
      }
    }
    // talking mouth
    if (!r.moving && r.action === 'talk') {
      const t = clock * 9 + r.phase * 10;
      w.aa = Math.max(0, Math.sin(t)) * 0.6;
      w.oh = Math.max(0, Math.sin(t * 0.7 + 1)) * 0.3;
    }
    if (r.expression) {
      if (clock > r.expression.until) r.expression = null;
      else for (const [k, v] of Object.entries(r.expression.weights)) w[k] = v;
    }
    if (r.pose?.data.expressions) Object.assign(w, r.pose.data.expressions);
    const ark = toArkit(w);
    for (let i = 0; i < infl.length; i++) infl[i] *= Math.exp(-dt * 12);
    for (const [k, v] of Object.entries(ark)) {
      const i = dict[k];
      if (i !== undefined) infl[i] = Math.max(infl[i], Math.min(1, v));
    }
  }

  const _qa = new THREE.Quaternion();
  const _qb = new THREE.Quaternion();
  const _qIdentity = new THREE.Quaternion();
  const _v = new THREE.Vector3();
  const _v2 = new THREE.Vector3();

  function applyLive(r: Resident) {
    const p = r.pose;
    if (!p) return;
    const age = clock - p.at;
    if (age > 1.5) { r.pose = null; return; }
    const w = Math.min(1, Math.max(0, 1.5 - age) / 0.5);
    const rig = r.rig!;
    if (!r.tpose) {
      const t = tposeRotations(rig);
      const nameOf = new Map<THREE.Object3D, string>();
      for (const [n, b] of rig.byName) nameOf.set(b, n);
      const parent = new Map<string, string>();
      for (const [n, b] of rig.byName) {
        const pn = b.parent ? nameOf.get(b.parent) : undefined;
        if (pn) parent.set(n, pn);
      }
      r.tpose = { ...t, parent };
    }
    const tp = r.tpose;
    // Live rotations are on the normalized rig (identity = T-pose, frames
    // aligned with the world). On this rig: L = Wp^-1 * Q * Wp * T, where Wp
    // is the parent's world rotation and T the bone's local one in T-pose.
    for (const [name, q] of Object.entries(p.data.bones ?? {})) {
      const bone = rig.byName.get(name);
      if (!bone || !q) continue;
      const t = tp.local.get(name) ?? _qIdentity;
      const pn = tp.parent.get(name);
      const wp = (pn && tp.world.get(pn)) || _qIdentity;
      _qb.copy(wp).invert().multiply(_qa.fromArray(q)).multiply(wp).multiply(t);
      bone.quaternion.slerp(_qb, w);
    }
    if (p.data.fingers) {
      for (const [name, q] of fingerRotations(p.data.fingers)) rig.byName.get(name)?.quaternion.slerp(q, w);
    }
    if (p.data.hips) {
      const h = rig.bones[0];
      h.position.x += p.data.hips[0] * w;
      h.position.y += p.data.hips[1] * w;
      h.position.z += p.data.hips[2] * w;
    }
  }

  function poseRig(r: Resident, dt: number) {
    const rig = r.rig!;
    const anim = ensureAnim(r);
    if (!anim) return;
    const clip = clipFor(r);
    const walkScale = clip === 'walk' || clip === 'carry' ? r.speed / (1.63 * Math.max(0.3, rig.hipsRest.y)) : 1;
    anim.play(clip, walkScale);
    anim.step(dt);
    anim.apply(rig.bones, rig.hipsRest);
    rig.root.position.copy(r.pos);
    rig.root.rotation.set(0, r.yaw, 0);
    applyLive(r);
    rig.root.updateMatrixWorld(true);
    if (rig.tier !== 'full') return;
    const B = rig.byName;
    // feet on the ground
    if (opts.groundAt && !r.moving && (clip === 'idle' || clip === 'talk' || clip === 'work' || clip === 'water')) {
      for (const side of ['left', 'right'] as const) {
        const foot = B.get(`${side}Foot`)!;
        foot.getWorldPosition(_v);
        const dy = opts.groundAt(_v.x, _v.z) - r.pos.y;
        if (Math.abs(dy) < 0.005 || Math.abs(dy) > 0.4) continue;
        _v.y += dy;
        _v2.set(0, 0, 1).applyQuaternion(rig.root.quaternion);
        solveTwoBone(B.get(`${side}UpperLeg`)!, B.get(`${side}LowerLeg`)!, foot, _v, _v2, 1);
      }
    }
    // hand to the anchor
    const reaching = !!r.reach && !r.moving && ['water', 'plant', 'harvest', 'work', 'point'].includes(r.action);
    r.reachW += ((reaching ? 1 : 0) - r.reachW) * Math.min(1, dt * 4);
    if (r.reach && r.reachW > 0.01) {
      _v2.set(0, -1, 0);
      solveTwoBone(B.get('rightUpperArm')!, B.get('rightLowerArm')!, B.get('rightHand')!, r.reach, _v2, r.reachW);
    }
    // look
    const lookTarget = r.lookAt ?? (r.reach && reaching ? r.reach : null);
    rig.root.getWorldQuaternion(_qa);
    applyLook(r.look, lookTarget, dt, B.get('neck')!, B.get('head')!, [B.get('leftEye')!, B.get('rightEye')!], _qa);
    if (r.pose?.data.look) {
      const [yaw, pitch] = r.pose.data.look;
      for (const e of ['leftEye', 'rightEye']) {
        B.get(e)!.quaternion.premultiply(_qa.setFromEuler(new THREE.Euler(-pitch, yaw, 0, 'YXZ')));
      }
    }
    rig.root.updateMatrixWorld(true);
    updateFace(r, dt);
  }

  let builtThisFrame = 0;

  function realize(r: Resident) {
    if (!base || !clips || !crowd) return;
    const want = r.want;
    if (want === 'full' || want === 'reduced') {
      if (r.rigTier !== want) {
        if (builtThisFrame >= o.buildsPerFrame) return; // keep the old look one more frame
        builtThisFrame++;
        const old = r.rig;
        const rig = buildCharacter(base, r.genome, want, { outline: o.outline });
        old?.dispose();
        r.rig = rig;
        r.rigTier = want;
        r.tpose = null;
        group.add(rig.root);
      }
      crowd.remove(r.id);
      r.tier = want;
      return;
    }
    if (r.rig) dropRig(r);
    if (want === 'hidden') {
      crowd.remove(r.id);
      r.tier = 'hidden';
      return;
    }
    r.tier = 'crowd';
  }

  let tierClock = 0;
  const camPos = new THREE.Vector3();

  function update(dt: number, camera: THREE.Camera) {
    if (disposed) return;
    const step = Math.min(0.1, Math.max(0, dt));
    clock += step;
    builtThisFrame = 0;
    if (!base || !clips || !crowd) return;
    camera.getWorldPosition(camPos);
    tierClock -= step;
    if (tierClock <= 0) {
      assignTiers(camPos);
      tierClock = 0.25;
    }
    for (const r of residents.values()) {
      simulate(r, step);
      realize(r);
      if (r.tier === 'full' || r.tier === 'reduced') {
        if (r.rig) poseRig(r, step);
      } else if (r.tier === 'crowd') {
        const clip = clipFor(r);
        const scale = clip === 'walk' || clip === 'carry' ? r.speed / (1.63 * 0.52 * r.height) : 1;
        r.crowdTime += step * scale;
        crowd.set(r.id, {
          archetype: r.archetype, position: r.pos, yaw: r.yaw, scale: r.scale, clip, time: r.crowdTime + r.phase * 4,
          colors: r.crowdColors, cover: r.cover,
        });
      }
    }
  }

  return {
    ready,
    spawn,
    despawn,
    setTarget,
    applyPose(id, pose) {
      const r = residents.get(id);
      if (r) r.pose = pose ? { data: pose, at: clock } : null;
    },
    playGesture(id, action) {
      const r = residents.get(id);
      if (!r) return [`unknown resident '${id}'`];
      const anim = ensureAnim(r);
      if (!anim) return ['clips not loaded yet'];
      const errors = anim.gestureAction(action);
      if (action.face) {
        const name = action.face.expression;
        if (!VRM_PRESETS[name] && name !== 'neutral') errors.push(`unknown expression '${name}'`);
        else r.expression = { weights: { [name]: action.face.weight ?? 0.7 }, until: clock + (action.face.hold_s ?? 2) };
      }
      if (action.look_at && typeof action.look_at === 'object') {
        r.lookAt = new THREE.Vector3(action.look_at.x, action.look_at.y, action.look_at.z);
      } else if (action.look_at === null) r.lookAt = null;
      return errors;
    },
    setExpression(id, name, weight, hold = 2) {
      const r = residents.get(id);
      if (r) r.expression = { weights: { [name]: weight }, until: clock + hold };
    },
    lookAt(id, target) {
      const r = residents.get(id);
      if (r) r.lookAt = target ? new THREE.Vector3(target.x, target.y, target.z) : null;
    },
    update,
    object(id) {
      return residents.get(id)?.rig?.root ?? null;
    },
    tierOf(id) {
      return residents.get(id)?.tier ?? null;
    },
    ids() {
      return [...residents.keys()];
    },
    stats() {
      const s = { residents: residents.size, full: 0, reduced: 0, crowd: 0, hidden: 0 };
      for (const r of residents.values()) s[r.tier]++;
      return s;
    },
    dispose() {
      disposed = true;
      for (const r of residents.values()) dropRig(r);
      residents.clear();
      crowd?.dispose();
      group.removeFromParent();
    },
  };
}

function turn(from: number, to: number, maxStep: number): number {
  let d = to - from;
  while (d > Math.PI) d -= Math.PI * 2;
  while (d < -Math.PI) d += Math.PI * 2;
  return from + Math.max(-maxStep, Math.min(maxStep, d));
}
