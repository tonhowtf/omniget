// Per-character animation state: a base clip with cross-fade, a gesture layer
// that overrides only the bones the gesture moves, and the sampled result as
// 52 quaternions plus a hips offset.
import * as THREE from 'three';
import { clipFromJson, lerpPose, makeClip, mergePose, sampleClip, UPPER_MASK, type Clip } from './clips';
import { ease, poseToBones, STANDING, type GestureAction, type JointPose, type Timing } from './posecode';
import { BONE_COUNT } from './anny';
import { VRM_BONES } from './rig';

const VRM_INDEX: Record<string, number> = Object.fromEntries(VRM_BONES.map((b, i) => [b, i]));

interface Play { clip: Clip; t: number; speed: number }
interface GesturePlay extends Play {
  mask: Uint8Array; mirror: boolean; intensity: number; loops: number;
  fadeIn: number; fadeOut: number; elapsed: number; hold: boolean;
}

const standingQuats = (() => {
  const q = new Float32Array(BONE_COUNT * 4);
  for (let i = 0; i < BONE_COUNT; i++) q[i * 4 + 3] = 1;
  return q;
})();
let standingReady = false;

/** Bones a gesture actually moves (differs from standing at some frame). */
function activeBones(clip: Clip, layerMask?: Uint8Array): Uint8Array {
  if (!standingReady) {
    const { bones } = poseToBones(STANDING);
    bones.forEach((q, name) => {
      const i = VRM_INDEX[name];
      if (i !== undefined) q.toArray(standingQuats, i * 4);
    });
    standingReady = true;
  }
  const mask = new Uint8Array(BONE_COUNT);
  const a = new THREE.Quaternion(), s = new THREE.Quaternion();
  for (let b = 0; b < clip.boneIdx.length; b++) {
    const bone = clip.boneIdx[b];
    if (bone < 0) continue;
    s.fromArray(standingQuats, bone * 4);
    for (let f = 0; f < clip.frames; f++) {
      a.fromArray(clip.rot, (b * clip.frames + f) * 4);
      if (Math.abs(a.dot(s)) < 0.9995) { mask[bone] = 1; break; }
    }
  }
  if (layerMask) for (let i = 0; i < BONE_COUNT; i++) mask[i] &= layerMask[i];
  return mask;
}

export class Animator {
  quats = new Float32Array(BONE_COUNT * 4);
  hips = new Float32Array(3);
  private a = new Float32Array(BONE_COUNT * 4);
  private b = new Float32Array(BONE_COUNT * 4);
  private ha = new Float32Array(3);
  private hb = new Float32Array(3);
  base: Play | null = null;
  prev: (Play & { w: number; fade: number }) | null = null;
  gesture: GesturePlay | null = null;
  private activeCache = new WeakMap<Clip, Uint8Array>();

  constructor(public clips: Map<string, Clip>, public phase = 0) {}

  play(name: string, speed = 1, fade = 0.3) {
    const clip = this.clips.get(name) ?? this.clips.get('idle');
    if (!clip) return;
    if (this.base?.clip === clip) { this.base.speed = speed; return; }
    if (this.base) this.prev = { ...this.base, w: 1, fade: Math.max(0.01, fade) };
    this.base = { clip, t: clip.loop ? this.phase * clip.duration : 0, speed };
  }

  get current(): string | null {
    return this.base?.clip.name ?? null;
  }

  /** Plays a gesture (catalog clip or composed); returns validation errors. */
  gestureAction(a: GestureAction): string[] {
    const g = a.gesture;
    if (!g) return [];
    const layer = a.blend?.layer === 'full' ? undefined : UPPER_MASK;
    if (g.mode === 'clip') {
      const clip = this.clips.get(g.clip);
      if (!clip) return [`unknown clip '${g.clip}'`];
      const p = g.params ?? {};
      let mask = this.activeCache.get(clip);
      if (!mask) this.activeCache.set(clip, (mask = activeBones(clip)));
      if (layer) mask = mask.map((v, i) => v & layer[i]);
      this.gesture = {
        clip, t: 0, speed: clamp(p.speed ?? 1, 0.25, 3), mask,
        mirror: p.side === 'left', intensity: clamp(p.intensity ?? 1, 0, 1),
        loops: Math.round(clamp(p.loops ?? 1, 1, 10)),
        fadeIn: a.blend?.in_s ?? 0.2, fadeOut: a.blend?.out_s ?? 0.3, elapsed: 0, hold: false,
      };
      return [];
    }
    if (g.mode === 'compose') {
      const { clip, errors } = composeClip(g.name ?? 'composed', g.phases, g.start);
      if (!clip) return errors;
      this.gesture = {
        clip, t: 0, speed: 1, mask: activeBones(clip, layer), mirror: false, intensity: 1, loops: 1,
        fadeIn: a.blend?.in_s ?? 0.2, fadeOut: a.blend?.out_s ?? 0.4, elapsed: 0, hold: !!g.hold,
      };
      return errors;
    }
    return ['gesture.mode must be clip or compose'];
  }

  stopGesture() {
    if (!this.gesture) return;
    this.gesture.hold = false;
    this.gesture.loops = 0;
  }

  step(dt: number) {
    const q = this.quats;
    this.hips.fill(0);
    if (!this.base) {
      fillIdentity(q);
      return;
    }
    this.base.t += dt * this.base.speed;
    fillIdentity(this.a);
    this.ha.fill(0);
    sampleClip(this.base.clip, this.base.t, this.a, this.ha);
    if (this.prev) {
      this.prev.t += dt * this.prev.speed;
      this.prev.w -= dt / this.prev.fade;
      if (this.prev.w <= 0) this.prev = null;
    }
    if (this.prev) {
      fillIdentity(this.b);
      this.hb.fill(0);
      sampleClip(this.prev.clip, this.prev.t, this.b, this.hb);
      const w = ease(1 - this.prev.w);
      blendInto(q, this.b, this.a, w);
      for (let k = 0; k < 3; k++) this.hips[k] = this.hb[k] * (1 - w) + this.ha[k] * w;
    } else {
      q.set(this.a);
      this.hips.set(this.ha);
    }
    const g = this.gesture;
    if (g) {
      g.elapsed += dt;
      g.t += dt * g.speed;
      const total = g.clip.duration * g.loops;
      const done = !g.hold && g.t >= total;
      let w = Math.min(1, g.elapsed / Math.max(0.01, g.fadeIn));
      if (!g.hold) w = Math.min(w, Math.max(0, (total - g.t) / Math.max(0.01, g.fadeOut)));
      w *= g.intensity;
      const t = g.hold ? Math.min(g.t, g.clip.duration) : g.clip.loop ? g.t : g.t % g.clip.duration;
      if (w > 0) {
        this.b.set(q);
        this.hb.fill(0);
        sampleClip(g.clip, t, this.b, this.hb, 1, g.mask, g.mirror);
        blendMasked(q, this.b, w, g.mask);
        for (let k = 0; k < 3; k++) this.hips[k] += this.hb[k] * w;
      }
      if (done) this.gesture = null;
    }
  }

  apply(bones: THREE.Bone[], hipsRest: THREE.Vector3) {
    for (let i = 0; i < bones.length; i++) bones[i].quaternion.fromArray(this.quats, i * 4);
    const h = hipsRest.y;
    bones[0].position.set(hipsRest.x + this.hips[0] * h, hipsRest.y + this.hips[1] * h, hipsRest.z + this.hips[2] * h);
  }
}

/** Builds a clip from composed phases (estudo 13 §4.1, mode 'compose'). */
export function composeClip(
  name: string,
  phases: { dur_s: number; timing?: Timing; joints: JointPose }[],
  start?: string,
): { clip: Clip | null; errors: string[] } {
  const errors: string[] = [];
  if (!Array.isArray(phases) || !phases.length) return { clip: null, errors: ['phases must be a non-empty list'] };
  if (start && !['standing', 'seated'].includes(start)) errors.push(`unknown start '${start}'`);
  const base: JointPose = start === 'seated' ? { hips: { flex: 88 }, knees: { flex: 88 } } : {};
  const keys: { t: number; pose: JointPose; timing?: Timing }[] = [{ t: 0, pose: base }];
  let t = 0;
  let acc = base;
  for (const ph of phases) {
    const dur = clamp(Number(ph.dur_s) || 0.5, 0.05, 10);
    t += dur;
    const check = poseToBones(ph.joints ?? {});
    errors.push(...check.errors);
    acc = mergePose(acc, ph.joints ?? {});
    keys.push({ t, pose: acc, timing: ph.timing });
  }
  if (errors.length) return { clip: null, errors };
  const total = t;
  const json = makeClip(name, total, false, 20, (phase) => {
    const time = phase * total;
    let i = 0;
    while (i < keys.length - 2 && keys[i + 1].t <= time) i++;
    const a = keys[i], b = keys[i + 1];
    const u = ease((time - a.t) / Math.max(1e-3, b.t - a.t), b.timing ?? 'flow');
    return { pose: lerpPose(a.pose, b.pose, u) };
  });
  return { clip: clipFromJson(json), errors };
}

function clamp(v: number, a: number, b: number) {
  return Math.max(a, Math.min(b, v));
}

function fillIdentity(q: Float32Array) {
  for (let i = 0; i < q.length; i += 4) { q[i] = 0; q[i + 1] = 0; q[i + 2] = 0; q[i + 3] = 1; }
}

const _x = new THREE.Quaternion();
const _y = new THREE.Quaternion();

function blendInto(out: Float32Array, from: Float32Array, to: Float32Array, w: number) {
  for (let i = 0; i < out.length; i += 4) {
    _x.fromArray(from, i);
    _y.fromArray(to, i);
    _x.slerp(_y, w).toArray(out, i);
  }
}

function blendMasked(out: Float32Array, over: Float32Array, w: number, mask: Uint8Array) {
  for (let b = 0; b < mask.length; b++) {
    if (!mask[b]) continue;
    const i = b * 4;
    _x.fromArray(out, i);
    _y.fromArray(over, i);
    _x.slerp(_y, w).toArray(out, i);
  }
}
