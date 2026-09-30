// Gestures asked for by an LLM (estudo 13 §3 and §4.1).
//
// Two modes: `clip` picks an id from a short catalog and tunes a few params;
// `compose` writes sparse joint targets in the posecode vocabulary (flex,
// extend, abduct, ...), in degrees, phase by phase. The vocabulary is closed:
// an unknown word is an error sent back to the LLM, never silently ignored,
// and every angle is clamped to the joint's range of motion. The LLM never
// sends quaternions; this file turns the validated JSON into LivePose frames
// deterministically, so every client plays the same motion.
//
// The joint names, actions, axis signs and ROM table follow posecode
// (posecode-dev/posecode, packages/posecode-parser, Apache-2.0). Keep them in
// sync with `validate_gesture` in `src-tauri/src/commands/city3d/live.rs`.

import { emptyPose, type ExpressionName, type FingerName, type LivePose, type PoseBone } from './pose';
import { applyLimit } from './solver';
import { DEG, QI, clamp, qConj, qFromEuler, qFromUnitVectors, qMul, qSlerp, type Quat, type Vec3 } from './math';

// ---------------------------------------------------------------- vocabulary

type Axis = 'x' | 'y' | 'z';

export const GESTURE_BONES = [
  'pelvis', 'spine', 'chest', 'neck', 'head',
  'shoulder_left', 'shoulder_right', 'elbow_left', 'elbow_right', 'wrist_left', 'wrist_right',
  'hip_left', 'hip_right', 'knee_left', 'knee_right', 'ankle_left', 'ankle_right',
  'thumb_left', 'index_left', 'middle_left', 'ring_left', 'pinky_left',
  'thumb_right', 'index_right', 'middle_right', 'ring_right', 'pinky_right',
] as const;
export type GestureBone = (typeof GESTURE_BONES)[number];

const FINGERS_L: GestureBone[] = ['thumb_left', 'index_left', 'middle_left', 'ring_left', 'pinky_left'];
const FINGERS_R: GestureBone[] = ['thumb_right', 'index_right', 'middle_right', 'ring_right', 'pinky_right'];

export const JOINT_GROUPS: Record<string, GestureBone[]> = {
  shoulders: ['shoulder_left', 'shoulder_right'],
  elbows: ['elbow_left', 'elbow_right'],
  forearms: ['elbow_left', 'elbow_right'],
  wrists: ['wrist_left', 'wrist_right'],
  hips: ['hip_left', 'hip_right'],
  knees: ['knee_left', 'knee_right'],
  ankles: ['ankle_left', 'ankle_right'],
  fingers_left: FINGERS_L,
  fingers_right: FINGERS_R,
  fingers: [...FINGERS_L, ...FINGERS_R],
};

export const JOINT_NAMES = [...Object.keys(JOINT_GROUPS), ...GESTURE_BONES];

export const ACTIONS: Record<string, { axis: Axis; sign: number }> = {
  flex: { axis: 'x', sign: 1 },
  extend: { axis: 'x', sign: -1 },
  abduct: { axis: 'z', sign: -1 },
  adduct: { axis: 'z', sign: 1 },
  'rotate-in': { axis: 'y', sign: 1 },
  'rotate-out': { axis: 'y', sign: -1 },
  'twist-left': { axis: 'y', sign: 1 },
  'twist-right': { axis: 'y', sign: -1 },
  supinate: { axis: 'y', sign: -1 },
  pronate: { axis: 'y', sign: 1 },
  dorsiflex: { axis: 'x', sign: -1 },
  plantarflex: { axis: 'x', sign: 1 },
  hinge: { axis: 'x', sign: 1 },
};

type Rom = Record<string, number>;
/** Max degrees per joint type and action. */
export const ROM: Record<string, Rom> = {
  shoulder: { flex: 180, extend: 60, abduct: 180, adduct: 50, 'rotate-in': 70, 'rotate-out': 90 },
  elbow: { flex: 154, extend: 10, supinate: 92, pronate: 84 },
  wrist: { flex: 80, extend: 70, abduct: 20, adduct: 30 },
  hip: { flex: 135, extend: 20, abduct: 45, adduct: 30, 'rotate-in': 40, 'rotate-out': 45 },
  knee: { flex: 144, extend: 5 },
  ankle: { dorsiflex: 15, plantarflex: 50 },
  pelvis: { hinge: 120 },
  spine: { flex: 90, extend: 30, abduct: 35, adduct: 35, 'twist-left': 45, 'twist-right': 45 },
  chest: { flex: 30, extend: 20, 'twist-left': 35, 'twist-right': 35 },
  neck: { flex: 50, extend: 60, abduct: 45, adduct: 45, 'twist-left': 80, 'twist-right': 80 },
  head: { flex: 25, extend: 25, abduct: 20, adduct: 20, 'twist-left': 40, 'twist-right': 40 },
  thumb: { flex: 80, extend: 20, abduct: 50, adduct: 30 },
  index: { flex: 100, extend: 20 },
  middle: { flex: 100, extend: 20 },
  ring: { flex: 100, extend: 20 },
  pinky: { flex: 100, extend: 20 },
};

const FLEX_UP = new Set(['knee', 'spine', 'chest', 'neck', 'head']);
const boneType = (b: string) => b.replace(/_(left|right)$/, '');
const isLeft = (b: string) => b.endsWith('_left');

export const EXPRESSION_NAMES = [
  'neutral', 'happy', 'angry', 'sad', 'relaxed', 'surprised',
  'aa', 'ih', 'ou', 'ee', 'oh', 'blink', 'blinkLeft', 'blinkRight',
] as const;
export type GestureExpression = (typeof EXPRESSION_NAMES)[number];

export const MOODS = ['neutral', 'tired', 'happy', 'sad', 'angry', 'confident', 'shy', 'bored'] as const;
export type Mood = (typeof MOODS)[number];

export const TIMINGS = ['flow', 'settle', 'drive', 'snap', 'linear'] as const;
export type Timing = (typeof TIMINGS)[number];

export const STARTS = ['standing', 'seated'] as const;
export type StartPose = (typeof STARTS)[number];

export const LAYERS = ['full', 'upper', 'face'] as const;

const ENTITY_RE = /^(user|npc|obj|seat|self):[A-Za-z0-9_.:\-]{1,64}$/;
const isEntity = (s: string) => ENTITY_RE.test(s) || s === 'camera' || s === 'none';

// ------------------------------------------------------------------ catalog

export interface ClipSpec {
  id: string;
  description: string;
  /** Whether `side` matters. */
  sided: boolean;
  /** Whether `loops` matters. */
  loops: boolean;
}

export const CLIP_CATALOG: ClipSpec[] = [
  { id: 'wave', description: 'wave hello with one hand', sided: true, loops: true },
  { id: 'nod', description: 'nod yes', sided: false, loops: true },
  { id: 'shake_head', description: 'shake the head no', sided: false, loops: true },
  { id: 'shrug', description: 'shrug, palms up', sided: false, loops: false },
  { id: 'point_at', description: 'point with one arm (use target)', sided: true, loops: false },
  { id: 'clap', description: 'clap the hands', sided: false, loops: true },
  { id: 'bow', description: 'bow from the hips', sided: false, loops: false },
  { id: 'cheer', description: 'both arms up, celebrating', sided: false, loops: true },
  { id: 'think', description: 'hand to chin, thinking', sided: true, loops: false },
  { id: 'facepalm', description: 'hand over the face', sided: true, loops: false },
  { id: 'thumbs_up', description: 'thumbs up', sided: true, loops: false },
  { id: 'cross_arms', description: 'cross the arms', sided: false, loops: false },
  { id: 'stretch', description: 'stretch the arms overhead', sided: false, loops: false },
  { id: 'yawn', description: 'yawn with a stretch', sided: false, loops: false },
  { id: 'laugh', description: 'laugh, body shaking a little', sided: false, loops: true },
  { id: 'sit_tired', description: 'sit down, slumped and tired', sided: false, loops: false },
];

const CLIP_IDS = new Set(CLIP_CATALOG.map((c) => c.id));

/** One line per clip, for the LLM prompt. */
export function clipCatalogPrompt(): string {
  return CLIP_CATALOG.map(
    (c) => `${c.id}: ${c.description} (params: ${c.sided ? 'side, ' : ''}intensity 0-1, speed 0.25-3${c.loops ? ', loops 1-10' : ''})`,
  ).join('\n');
}

// -------------------------------------------------------------------- types

/** `{ bone: { action: degrees } }`, or `"neutral"` to reset a joint. */
export type JointTargets = Record<string, Record<string, number> | 'neutral'>;

export interface Phase {
  dur_s: number;
  timing: Timing;
  /** Joint targets, already expanded to bones and clamped. */
  joints: Partial<Record<GestureBone, Partial<Record<string, number>> | 'neutral'>>;
  /** Hips offset (metres), only from built-in clips. */
  hips?: Vec3;
  /** Expression weights reached at the end of this phase. */
  face?: Partial<Record<GestureExpression, number>>;
}

export interface ClipParams {
  side: 'left' | 'right';
  intensity: number;
  speed: number;
  loops: number;
}

export type GestureSpec =
  | { mode: 'clip'; clip: string; params: ClipParams; target?: string }
  | { mode: 'compose'; name: string; start: StartPose; phases: Phase[]; hold: boolean; target?: string };

export interface GestureAction {
  v: 1;
  actor?: string;
  gesture?: GestureSpec;
  face?: { expression: GestureExpression; weight: number; hold_s: number };
  look_at?: string;
  posture?: { mood: Mood; weight: number };
  blend: { in_s: number; out_s: number; layer: (typeof LAYERS)[number] };
}

export type GestureValidation =
  | { ok: true; action: GestureAction; warnings: string[] }
  | { ok: false; errors: string[] };

// --------------------------------------------------------------- validation

const isObj = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

class Ctx {
  errors: string[] = [];
  warnings: string[] = [];
  err(path: string, msg: string) {
    this.errors.push(`${path}: ${msg}`);
  }
  num(v: unknown, path: string, lo: number, hi: number, dflt: number): number {
    if (v === undefined) return dflt;
    if (typeof v !== 'number' || !Number.isFinite(v)) {
      this.err(path, 'must be a number');
      return dflt;
    }
    if (v < lo || v > hi) {
      this.warnings.push(`${path}: ${v} clamped to [${lo}, ${hi}]`);
      return clamp(v, lo, hi);
    }
    return v;
  }
  oneOf<T extends string>(v: unknown, path: string, list: readonly T[], dflt: T): T {
    if (v === undefined) return dflt;
    if (typeof v !== 'string' || !list.includes(v as T)) {
      this.err(path, `unknown value ${JSON.stringify(v)}; expected one of ${list.join(', ')}`);
      return dflt;
    }
    return v as T;
  }
  keys(o: Record<string, unknown>, path: string, allowed: string[]) {
    for (const k of Object.keys(o)) if (!allowed.includes(k)) this.err(path, `unknown field "${k}"`);
  }
}

function expandJoint(name: string): GestureBone[] {
  if (JOINT_GROUPS[name]) return JOINT_GROUPS[name];
  return (GESTURE_BONES as readonly string[]).includes(name) ? [name as GestureBone] : [];
}

function validateJoints(raw: unknown, path: string, c: Ctx): Phase['joints'] {
  const out: Phase['joints'] = {};
  if (!isObj(raw)) {
    c.err(path, 'must be an object of joint → { action: degrees }');
    return out;
  }
  for (const [joint, spec] of Object.entries(raw)) {
    const p = `${path}.${joint}`;
    const bones = expandJoint(joint);
    if (bones.length === 0) {
      c.err(p, `unknown joint; expected one of ${JOINT_NAMES.join(', ')}`);
      continue;
    }
    if (spec === 'neutral') {
      for (const b of bones) out[b] = 'neutral';
      continue;
    }
    if (!isObj(spec)) {
      c.err(p, 'must be { action: degrees } or "neutral"');
      continue;
    }
    for (const b of bones) {
      const acts: Partial<Record<string, number>> = {};
      for (const [action, deg] of Object.entries(spec)) {
        const ap = `${p}.${action}`;
        if (!ACTIONS[action]) {
          c.err(ap, `unknown action; expected one of ${Object.keys(ACTIONS).join(', ')}`);
          continue;
        }
        const max = ROM[boneType(b)]?.[action];
        if (max === undefined) {
          c.err(ap, `"${action}" is not allowed on ${b}; allowed: ${Object.keys(ROM[boneType(b)] ?? {}).join(', ')}`);
          continue;
        }
        acts[action] = c.num(deg, ap, 0, max, 0);
      }
      const prev = out[b];
      out[b] = prev && prev !== 'neutral' ? { ...prev, ...acts } : acts;
    }
  }
  return out;
}

function validateFaceTargets(raw: unknown, path: string, c: Ctx): Phase['face'] {
  if (raw === undefined) return undefined;
  if (!isObj(raw)) {
    c.err(path, 'must be { expression: weight }');
    return undefined;
  }
  const out: Phase['face'] = {};
  for (const [k, v] of Object.entries(raw)) {
    const e = c.oneOf(k, `${path}.${k}`, EXPRESSION_NAMES, 'neutral');
    out[e] = c.num(v, `${path}.${k}`, 0, 1, 0);
  }
  return out;
}

function validateTarget(v: unknown, path: string, c: Ctx): string | undefined {
  if (v === undefined) return undefined;
  if (typeof v !== 'string' || !isEntity(v)) {
    c.err(path, 'must be "user:<id>", "npc:<id>", "obj:<id>", "seat:<id>", "self:<id>", "camera" or "none"');
    return undefined;
  }
  return v;
}

function validateSpec(raw: unknown, c: Ctx): GestureSpec | undefined {
  if (!isObj(raw)) {
    c.err('gesture', 'must be an object');
    return undefined;
  }
  const mode = raw.mode;
  if (mode === 'clip') {
    c.keys(raw, 'gesture', ['mode', 'clip', 'params', 'target']);
    const clip = raw.clip;
    if (typeof clip !== 'string' || !CLIP_IDS.has(clip)) {
      c.err('gesture.clip', `unknown clip ${JSON.stringify(clip)}; the catalog is: ${[...CLIP_IDS].join(', ')}`);
      return undefined;
    }
    const pr = raw.params === undefined ? {} : raw.params;
    if (!isObj(pr)) {
      c.err('gesture.params', 'must be an object');
      return undefined;
    }
    c.keys(pr, 'gesture.params', ['side', 'intensity', 'speed', 'loops']);
    return {
      mode: 'clip',
      clip,
      params: {
        side: c.oneOf(pr.side, 'gesture.params.side', ['left', 'right'] as const, 'right'),
        intensity: c.num(pr.intensity, 'gesture.params.intensity', 0, 1, 0.8),
        speed: c.num(pr.speed, 'gesture.params.speed', 0.25, 3, 1),
        loops: Math.round(c.num(pr.loops, 'gesture.params.loops', 1, 10, 2)),
      },
      target: validateTarget(raw.target, 'gesture.target', c),
    };
  }
  if (mode === 'compose') {
    c.keys(raw, 'gesture', ['mode', 'name', 'start', 'phases', 'joints', 'dur_s', 'timing', 'hold', 'target']);
    let phasesRaw: unknown[];
    if (raw.phases !== undefined) {
      if (!Array.isArray(raw.phases) || raw.phases.length === 0) {
        c.err('gesture.phases', 'must be a non-empty array');
        return undefined;
      }
      if (raw.phases.length > 16) c.err('gesture.phases', 'at most 16 phases');
      phasesRaw = raw.phases.slice(0, 16);
    } else if (raw.joints !== undefined) {
      phasesRaw = [{ joints: raw.joints, dur_s: raw.dur_s, timing: raw.timing }];
    } else {
      c.err('gesture', 'compose needs "phases" or "joints"');
      return undefined;
    }
    const phases: Phase[] = phasesRaw.map((ph, i) => {
      const p = `gesture.phases[${i}]`;
      if (!isObj(ph)) {
        c.err(p, 'must be an object');
        return { dur_s: 0.5, timing: 'flow', joints: {} };
      }
      c.keys(ph, p, ['dur_s', 'timing', 'joints', 'face', 'contacts']);
      if (ph.contacts !== undefined) c.warnings.push(`${p}.contacts: ignored (contacts are resolved by the world)`);
      return {
        dur_s: c.num(ph.dur_s, `${p}.dur_s`, 0.05, 10, 0.6),
        timing: c.oneOf(ph.timing, `${p}.timing`, TIMINGS, 'flow'),
        joints: ph.joints === undefined ? {} : validateJoints(ph.joints, `${p}.joints`, c),
        face: validateFaceTargets(ph.face, `${p}.face`, c),
      };
    });
    const name = raw.name === undefined ? 'composed' : String(raw.name).slice(0, 64);
    return {
      mode: 'compose',
      name,
      start: c.oneOf(raw.start, 'gesture.start', STARTS, 'standing'),
      phases,
      hold: raw.hold === true,
      target: validateTarget(raw.target, 'gesture.target', c),
    };
  }
  c.err('gesture.mode', `must be "clip" or "compose", got ${JSON.stringify(mode)}`);
  return undefined;
}

/**
 * Validates an avatar action. Accepts the full envelope (`{v, actor, gesture,
 * face, look_at, posture, blend}`) or a bare gesture (`{mode, ...}`), as an
 * object or as JSON text (a ```json fence around it is fine).
 */
export function validateGesture(input: unknown): GestureValidation {
  const c = new Ctx();
  let raw = input;
  if (typeof raw === 'string') {
    const text = raw.trim();
    const fenced = /```(?:json)?\s*([\s\S]*?)```/.exec(text);
    const body = fenced ? fenced[1] : text.slice(Math.max(0, text.indexOf('{')), text.lastIndexOf('}') + 1);
    try {
      raw = JSON.parse(body);
    } catch (e) {
      return { ok: false, errors: [`not valid JSON: ${(e as Error).message}`] };
    }
  }
  if (!isObj(raw)) return { ok: false, errors: ['the action must be a JSON object'] };
  if ('mode' in raw) raw = { gesture: raw };
  const env = raw as Record<string, unknown>;
  c.keys(env, 'action', ['v', 'actor', 'gesture', 'face', 'look_at', 'posture', 'blend']);
  if (env.v !== undefined && env.v !== 1) c.err('v', 'only version 1 is supported');
  const action: GestureAction = { v: 1, blend: { in_s: 0.25, out_s: 0.4, layer: 'full' } };
  if (env.actor !== undefined) {
    if (typeof env.actor !== 'string' || !isEntity(env.actor)) c.err('actor', 'must be an entity like "npc:<id>"');
    else action.actor = env.actor;
  }
  if (env.gesture !== undefined) action.gesture = validateSpec(env.gesture, c);
  if (env.face !== undefined) {
    if (!isObj(env.face)) c.err('face', 'must be an object');
    else {
      c.keys(env.face, 'face', ['expression', 'weight', 'hold_s']);
      action.face = {
        expression: c.oneOf(env.face.expression, 'face.expression', EXPRESSION_NAMES, 'neutral'),
        weight: c.num(env.face.weight, 'face.weight', 0, 1, 0.7),
        hold_s: c.num(env.face.hold_s, 'face.hold_s', 0, 30, 2),
      };
    }
  }
  action.look_at = validateTarget(env.look_at, 'look_at', c);
  if (env.posture !== undefined) {
    if (!isObj(env.posture)) c.err('posture', 'must be an object');
    else {
      c.keys(env.posture, 'posture', ['mood', 'weight']);
      action.posture = {
        mood: c.oneOf(env.posture.mood, 'posture.mood', MOODS, 'neutral'),
        weight: c.num(env.posture.weight, 'posture.weight', 0, 1, 0.6),
      };
    }
  }
  if (env.blend !== undefined) {
    if (!isObj(env.blend)) c.err('blend', 'must be an object');
    else {
      c.keys(env.blend, 'blend', ['in_s', 'out_s', 'layer']);
      action.blend = {
        in_s: c.num(env.blend.in_s, 'blend.in_s', 0, 3, 0.25),
        out_s: c.num(env.blend.out_s, 'blend.out_s', 0, 3, 0.4),
        layer: c.oneOf(env.blend.layer, 'blend.layer', LAYERS, 'full'),
      };
    }
  }
  if (!action.gesture && !action.face && !action.look_at && !action.posture) {
    c.err('action', 'needs at least one of gesture, face, look_at or posture');
  }
  if (c.errors.length) return { ok: false, errors: c.errors };
  return { ok: true, action, warnings: c.warnings };
}

// ------------------------------------------------------------ built-in clips

const side = (p: ClipParams, b: string) => b.replace('_side', `_${p.side}`) as GestureBone;
const opp = (p: ClipParams) => (p.side === 'left' ? 'right' : 'left');

function repeat(n: number, phases: Phase[]): Phase[] {
  const out: Phase[] = [];
  for (let i = 0; i < n; i++) out.push(...phases);
  return out;
}

/**
 * Procedural stand-ins for the catalog. The character system may play a
 * `.vrma` of the same id instead; these keep every id working without one.
 */
export function clipPhases(clip: string, p: ClipParams): Phase[] {
  const k = p.intensity;
  const J = (o: Record<string, Record<string, number> | 'neutral'>): Phase['joints'] => {
    const out: Phase['joints'] = {};
    for (const [name, v] of Object.entries(o)) {
      for (const b of expandJoint(name.includes('_side') ? side(p, name) : name.replace('_opp', `_${opp(p)}`))) {
        out[b] = v === 'neutral' ? 'neutral' : Object.fromEntries(Object.entries(v).map(([a, d]) => [a, d]));
      }
    }
    return out;
  };
  const ph = (dur_s: number, timing: Timing, joints: Phase['joints'], extra: Partial<Phase> = {}): Phase => ({
    dur_s,
    timing,
    joints,
    ...extra,
  });
  switch (clip) {
    case 'wave':
      return [
        ph(0.35, 'settle', J({ shoulder_side: { abduct: 100 + 40 * k, 'rotate-out': 60 }, elbow_side: { flex: 70 } }), { face: { happy: 0.5 * k } }),
        ...repeat(p.loops, [
          ph(0.22, 'flow', J({ shoulder_side: { abduct: 90 + 30 * k, 'rotate-out': 60 }, elbow_side: { flex: 40 + 30 * k } })),
          ph(0.22, 'flow', J({ shoulder_side: { abduct: 110 + 40 * k, 'rotate-out': 60 }, elbow_side: { flex: 90 } })),
        ]),
        ph(0.4, 'settle', J({ shoulder_side: 'neutral', elbow_side: 'neutral' })),
      ];
    case 'nod':
      return [
        ...repeat(p.loops, [
          ph(0.2, 'flow', J({ neck: { flex: 12 + 10 * k }, head: { flex: 5 } })),
          ph(0.2, 'flow', J({ neck: { extend: 4 }, head: 'neutral' })),
        ]),
        ph(0.2, 'settle', J({ neck: 'neutral', head: 'neutral' })),
      ];
    case 'shake_head':
      return [
        ...repeat(p.loops, [
          ph(0.18, 'flow', J({ neck: { 'twist-left': 15 + 15 * k } })),
          ph(0.18, 'flow', J({ neck: { 'twist-right': 15 + 15 * k } })),
        ]),
        ph(0.2, 'settle', J({ neck: 'neutral' })),
      ];
    case 'shrug':
      return [
        ph(0.3, 'snap', J({ shoulders: { abduct: 15 + 10 * k }, elbows: { flex: 90 }, forearms: { supinate: 80 }, head: { abduct: 10 * k } }), {
          face: { surprised: 0.3 * k },
        }),
        ph(0.6, 'linear', {}),
        ph(0.4, 'settle', J({ shoulders: 'neutral', elbows: 'neutral', head: 'neutral' })),
      ];
    case 'point_at':
      return [
        ph(0.35, 'drive', J({
          shoulder_side: { flex: 80 + 10 * k }, elbow_side: { flex: 5 },
          thumb_side: { flex: 60 }, middle_side: { flex: 95 }, ring_side: { flex: 95 }, pinky_side: { flex: 95 },
        })),
        ph(1.0, 'linear', {}),
        ph(0.4, 'settle', J({ shoulder_side: 'neutral', elbow_side: 'neutral', [`fingers_${p.side}`]: 'neutral' })),
      ];
    case 'clap':
      return [
        ph(0.3, 'settle', J({ shoulders: { flex: 45, adduct: 10 }, elbows: { flex: 80 } })),
        ...repeat(p.loops + 1, [
          ph(0.12, 'snap', J({ shoulders: { flex: 45, adduct: 25 }, elbows: { flex: 75 } })),
          ph(0.15, 'flow', J({ shoulders: { flex: 45, abduct: 10 + 10 * k }, elbows: { flex: 85 } })),
        ]),
        ph(0.4, 'settle', J({ shoulders: 'neutral', elbows: 'neutral' })),
      ];
    case 'bow':
      return [
        ph(0.6, 'settle', J({ pelvis: { hinge: 20 + 30 * k }, neck: { flex: 15 } })),
        ph(0.5, 'linear', {}),
        ph(0.6, 'flow', J({ pelvis: 'neutral', neck: 'neutral' })),
      ];
    case 'cheer':
      return [
        ph(0.3, 'snap', J({ shoulders: { abduct: 150 + 20 * k }, elbows: { flex: 15 } }), { face: { happy: k } }),
        ...repeat(p.loops, [
          ph(0.2, 'flow', J({ shoulders: { abduct: 140 + 20 * k }, elbows: { flex: 30 } })),
          ph(0.2, 'flow', J({ shoulders: { abduct: 165 }, elbows: { flex: 10 } })),
        ]),
        ph(0.5, 'settle', J({ shoulders: 'neutral', elbows: 'neutral' }), { face: { happy: 0 } }),
      ];
    case 'think':
      return [
        ph(0.5, 'settle', J({ shoulder_side: { flex: 30, adduct: 15 }, elbow_side: { flex: 140 }, wrist_side: { flex: 20 }, head: { abduct: 10 * k }, neck: { flex: 8 } })),
        ph(1.5, 'linear', {}),
        ph(0.5, 'settle', J({ shoulder_side: 'neutral', elbow_side: 'neutral', wrist_side: 'neutral', head: 'neutral', neck: 'neutral' })),
      ];
    case 'facepalm':
      return [
        ph(0.3, 'snap', J({ shoulder_side: { flex: 60, adduct: 20 }, elbow_side: { flex: 145 }, neck: { flex: 25 * k } })),
        ph(1.2, 'linear', {}),
        ph(0.5, 'settle', J({ shoulder_side: 'neutral', elbow_side: 'neutral', neck: 'neutral' })),
      ];
    case 'thumbs_up':
      return [
        ph(0.35, 'snap', J({
          shoulder_side: { flex: 40 }, elbow_side: { flex: 90 },
          thumb_side: { extend: 20 }, index_side: { flex: 95 }, middle_side: { flex: 95 }, ring_side: { flex: 95 }, pinky_side: { flex: 95 },
        }), { face: { happy: 0.6 * k } }),
        ph(0.9, 'linear', {}),
        ph(0.4, 'settle', J({ shoulder_side: 'neutral', elbow_side: 'neutral', [`fingers_${p.side}`]: 'neutral' })),
      ];
    case 'cross_arms':
      return [
        ph(0.6, 'settle', J({ shoulders: { flex: 35, 'rotate-in': 50, adduct: 20 }, elbows: { flex: 110 } })),
        ph(2.0, 'linear', {}),
        ph(0.6, 'settle', J({ shoulders: 'neutral', elbows: 'neutral' })),
      ];
    case 'stretch':
      return [
        ph(0.8, 'drive', J({ shoulders: { abduct: 165 }, elbows: { flex: 10 }, spine: { extend: 10 * k }, neck: { extend: 15 } })),
        ph(0.6, 'linear', {}),
        ph(0.7, 'settle', J({ shoulders: 'neutral', elbows: 'neutral', spine: 'neutral', neck: 'neutral' })),
      ];
    case 'yawn':
      return [
        ph(0.8, 'drive', J({ shoulders: { abduct: 150 }, elbows: { flex: 40 }, neck: { extend: 20 } }), { face: { oh: 0.9 * k, blink: 0.7 } }),
        ph(0.8, 'linear', {}),
        ph(0.7, 'settle', J({ shoulders: 'neutral', elbows: 'neutral', neck: 'neutral' }), { face: { oh: 0, blink: 0 } }),
      ];
    case 'laugh':
      return [
        ph(0.2, 'snap', J({ spine: { flex: 8 * k }, neck: { extend: 10 } }), { face: { happy: 1, aa: 0.6 * k } }),
        ...repeat(p.loops + 1, [
          ph(0.14, 'flow', J({ spine: { flex: 12 * k } })),
          ph(0.14, 'flow', J({ spine: { flex: 4 } })),
        ]),
        ph(0.4, 'settle', J({ spine: 'neutral', neck: 'neutral' }), { face: { happy: 0.3, aa: 0 } }),
      ];
    case 'sit_tired':
      return [
        ph(1.0, 'settle', J({ hips: { flex: 90 }, knees: { flex: 90 }, spine: { flex: 15 + 10 * k }, neck: { flex: 15 + 10 * k }, shoulders: { adduct: 5 } }), {
          hips: [0, -0.42, 0],
          face: { relaxed: 0.4, blink: 0.3 * k },
        }),
      ];
    default:
      return [];
  }
}

const MOOD_POSTURE: Record<Mood, JointTargets> = {
  neutral: {},
  tired: { spine: { flex: 12 }, neck: { flex: 15 }, shoulders: { adduct: 6 } },
  happy: { chest: { extend: 5 }, neck: { extend: 4 } },
  sad: { spine: { flex: 10 }, neck: { flex: 22 } },
  angry: { chest: { extend: 6 }, neck: { flex: 6 }, elbows: { flex: 15 } },
  confident: { chest: { extend: 8 }, neck: { extend: 6 } },
  shy: { neck: { flex: 15 }, shoulders: { 'rotate-in': 15, adduct: 6 } },
  bored: { spine: { flex: 6 }, head: { abduct: 10 }, neck: { flex: 8 } },
};

const MOOD_FACE: Partial<Record<Mood, [GestureExpression, number]>> = {
  tired: ['relaxed', 0.5],
  happy: ['happy', 0.6],
  sad: ['sad', 0.6],
  angry: ['angry', 0.6],
  confident: ['happy', 0.25],
  shy: ['relaxed', 0.3],
  bored: ['relaxed', 0.3],
};

/** Speed and intensity multipliers a mood puts on clips. */
const MOOD_TEMPO: Partial<Record<Mood, [number, number]>> = {
  tired: [0.7, 0.6],
  sad: [0.8, 0.7],
  happy: [1.15, 1],
  angry: [1.2, 1],
  bored: [0.8, 0.6],
};

// ------------------------------------------------------- joints → VRM bones

const VRM_OF: Partial<Record<GestureBone, PoseBone>> = {
  pelvis: 'hips',
  spine: 'spine',
  chest: 'chest',
  neck: 'neck',
  head: 'head',
  shoulder_left: 'leftUpperArm',
  shoulder_right: 'rightUpperArm',
  elbow_left: 'leftLowerArm',
  elbow_right: 'rightLowerArm',
  wrist_left: 'leftHand',
  wrist_right: 'rightHand',
  hip_left: 'leftUpperLeg',
  hip_right: 'rightUpperLeg',
  knee_left: 'leftLowerLeg',
  knee_right: 'rightLowerLeg',
  ankle_left: 'leftFoot',
  ankle_right: 'rightFoot',
};

const FINGER_OF: Partial<Record<GestureBone, FingerName>> = {
  thumb_left: 'leftThumb',
  index_left: 'leftIndex',
  middle_left: 'leftMiddle',
  ring_left: 'leftRing',
  pinky_left: 'leftLittle',
  thumb_right: 'rightThumb',
  index_right: 'rightIndex',
  middle_right: 'rightMiddle',
  ring_right: 'rightRing',
  pinky_right: 'rightLittle',
};

// posecode's rest has the arms down; the VRM rest is the T-pose. `ARM_DOWN`
// takes the T-pose arm to the side, and the arm chain is conjugated by it.
const ARM_DOWN_L = qFromUnitVectors([1, 0, 0], [0, -1, 0]);
const ARM_DOWN_R = qFromUnitVectors([-1, 0, 0], [0, -1, 0]);

type Euler = { x: number; y: number; z: number };
type JointState = Partial<Record<GestureBone, Partial<Record<string, number>>>>;

function eulerOf(bone: GestureBone, actions: Partial<Record<string, number>>): Euler {
  const e: Euler = { x: 0, y: 0, z: 0 };
  const type = boneType(bone);
  for (const [action, deg] of Object.entries(actions)) {
    const a = ACTIONS[action];
    if (!a || deg === undefined) continue;
    const flexFlip = action === 'flex' || action === 'extend' ? (FLEX_UP.has(type) ? 1 : -1) : 1;
    const mirror = isLeft(bone) && a.axis !== 'x' ? -1 : 1;
    e[a.axis] += a.sign * flexFlip * mirror * deg * DEG;
  }
  return e;
}

/** Joint state (degrees per action) → LivePose bones and finger curls. */
export function jointsToPose(state: JointState, pose: LivePose = emptyPose()): LivePose {
  const q = (b: GestureBone): Quat => {
    const a = state[b];
    if (!a) return QI;
    const e = eulerOf(b, a);
    return qFromEuler(e.x, e.y, e.z);
  };
  const hinge = q('pelvis');
  for (const side of ['left', 'right'] as const) {
    const down = side === 'left' ? ARM_DOWN_L : ARM_DOWN_R;
    const up = qConj(down);
    const s = `shoulder_${side}` as GestureBone;
    const e = `elbow_${side}` as GestureBone;
    const w = `wrist_${side}` as GestureBone;
    pose.bones[VRM_OF[s]!] = applyLimit(VRM_OF[s]!, qMul(q(s), down));
    pose.bones[VRM_OF[e]!] = applyLimit(VRM_OF[e]!, qMul(up, qMul(q(e), down)));
    pose.bones[VRM_OF[w]!] = applyLimit(VRM_OF[w]!, qMul(up, qMul(q(w), down)));
    const h = `hip_${side}` as GestureBone;
    const k = `knee_${side}` as GestureBone;
    const an = `ankle_${side}` as GestureBone;
    // The hinge tips the torso; the legs counter-rotate to stay planted.
    if (state[h] || state.pelvis) pose.bones[VRM_OF[h]!] = qMul(qConj(hinge), q(h));
    if (state[k]) pose.bones[VRM_OF[k]!] = q(k);
    if (state[an]) pose.bones[VRM_OF[an]!] = q(an);
  }
  for (const b of ['pelvis', 'spine', 'chest', 'neck', 'head'] as const) {
    if (state[b]) pose.bones[VRM_OF[b]!] = applyLimit(VRM_OF[b]!, q(b));
  }
  for (const [b, f] of Object.entries(FINGER_OF) as [GestureBone, FingerName][]) {
    const a = state[b];
    if (!a) continue;
    const curl = ((a.flex ?? 0) - (a.extend ?? 0) * 0.5) / (b.startsWith('thumb') ? 80 : 100);
    (pose.fingers ??= {})[f] = clamp(curl, 0, 1);
  }
  return pose;
}

// ------------------------------------------------------------------- player

const EASE: Record<Timing, (u: number) => number> = {
  linear: (u) => u,
  flow: (u) => 0.5 - 0.5 * Math.cos(Math.PI * u),
  settle: (u) => 1 - Math.pow(1 - u, 3),
  drive: (u) => u * u * u,
  snap: (u) => 1 - Math.pow(1 - u, 5),
};

interface Keyframe {
  t: number;
  timing: Timing;
  pose: LivePose;
  face: Partial<Record<GestureExpression, number>>;
}

export interface GesturePlayer {
  action: GestureAction;
  /** Seconds; `Infinity` when the gesture holds its last pose. */
  duration: number;
  /** Pose at `t` seconds from the start (already weighted by the blend). */
  sample(t: number): LivePose;
  /** Blend weight at `t` (in, hold, out). */
  weight(t: number): number;
  done(t: number): boolean;
}

function toExpressions(face: Partial<Record<GestureExpression, number>>): Partial<Record<ExpressionName, number>> {
  const out: Partial<Record<ExpressionName, number>> = {};
  for (const [k, v] of Object.entries(face) as [GestureExpression, number][]) {
    if (k === 'neutral') continue;
    if (k === 'blink') {
      out.blinkLeft = v;
      out.blinkRight = v;
    } else out[k] = v;
  }
  return out;
}

/** Builds a deterministic player for a validated action. */
export function createGesturePlayer(action: GestureAction): GesturePlayer {
  const mood = action.posture?.mood ?? 'neutral';
  const moodW = action.posture?.weight ?? 0;
  const [speedK, intensityK] = MOOD_TEMPO[mood] ?? [1, 1];

  // Start state: the posture, scaled by its weight.
  const state: JointState = {};
  const mergeTargets = (targets: Phase['joints'], scale = 1) => {
    for (const [b, v] of Object.entries(targets) as [GestureBone, Phase['joints'][GestureBone]][]) {
      if (v === 'neutral') delete state[b];
      else if (v) state[b] = { ...(state[b] ?? {}), ...Object.fromEntries(Object.entries(v).map(([a, d]) => [a, (d ?? 0) * scale])) };
    }
  };
  const c = new Ctx();
  mergeTargets(validateJoints(MOOD_POSTURE[mood], 'posture', c), moodW);

  let phases: Phase[] = [];
  let hold = false;
  let speed = 1;
  const g = action.gesture;
  if (g?.mode === 'clip') {
    phases = clipPhases(g.clip, { ...g.params, intensity: g.params.intensity * intensityK });
    speed = g.params.speed * speedK;
    hold = g.clip === 'sit_tired';
  } else if (g?.mode === 'compose') {
    if (g.start === 'seated') mergeTargets({ hip_left: { flex: 90 }, hip_right: { flex: 90 }, knee_left: { flex: 90 }, knee_right: { flex: 90 } });
    phases = g.phases;
    hold = g.hold;
  }

  // Face baseline: explicit face, else the mood's.
  const baseFace: Partial<Record<GestureExpression, number>> = {};
  if (action.face && action.face.expression !== 'neutral') baseFace[action.face.expression] = action.face.weight;
  else if (MOOD_FACE[mood]) baseFace[MOOD_FACE[mood]![0]] = MOOD_FACE[mood]![1] * moodW;

  const frames: Keyframe[] = [];
  let hips: Vec3 | undefined = g?.mode === 'compose' && g.start === 'seated' ? [0, -0.42, 0] : undefined;
  let face = { ...baseFace };
  const snapshot = (t: number, timing: Timing) => {
    const pose = jointsToPose(state);
    if (hips) pose.hips = hips;
    frames.push({ t, timing, pose, face: { ...face } });
  };
  snapshot(0, 'linear');
  let t = 0;
  for (const ph of phases) {
    mergeTargets(ph.joints);
    if (ph.hips) hips = ph.hips;
    if (ph.face) face = { ...face, ...ph.face };
    t += ph.dur_s / Math.max(0.25, speed);
    snapshot(t, ph.timing);
  }
  const body = t;
  const faceHold = action.face?.hold_s ?? 0;
  const active = Math.max(body, faceHold);
  const duration = hold ? Infinity : active + action.blend.out_s;

  const bonesAt = (time: number): LivePose => {
    if (frames.length === 1 || time <= 0) return frames[0].pose;
    for (let i = 1; i < frames.length; i++) {
      const a = frames[i - 1];
      const b = frames[i];
      if (time <= b.t) {
        const u = b.t > a.t ? (time - a.t) / (b.t - a.t) : 1;
        const e = EASE[b.timing](clamp(u, 0, 1));
        const out = emptyPose(0, time * 1000);
        const keys = new Set([...Object.keys(a.pose.bones), ...Object.keys(b.pose.bones)] as PoseBone[]);
        for (const k of keys) out.bones[k] = qSlerp(a.pose.bones[k] ?? QI, b.pose.bones[k] ?? QI, e);
        const fk = new Set([...Object.keys(a.pose.fingers ?? {}), ...Object.keys(b.pose.fingers ?? {})] as FingerName[]);
        if (fk.size) {
          out.fingers = {};
          for (const f of fk) out.fingers[f] = (a.pose.fingers?.[f] ?? 0) * (1 - e) + (b.pose.fingers?.[f] ?? 0) * e;
        }
        if (a.pose.hips || b.pose.hips) {
          const ha = a.pose.hips ?? [0, 0, 0];
          const hb = b.pose.hips ?? [0, 0, 0];
          out.hips = [0, 1, 2].map((j) => ha[j] * (1 - e) + hb[j] * e) as Vec3;
        }
        const ek = new Set([...Object.keys(a.face), ...Object.keys(b.face)] as GestureExpression[]);
        const f: Partial<Record<GestureExpression, number>> = {};
        for (const k of ek) f[k] = (a.face[k] ?? 0) * (1 - e) + (b.face[k] ?? 0) * e;
        out.expressions = toExpressions(f);
        return out;
      }
    }
    const last = frames[frames.length - 1];
    return { ...last.pose, expressions: toExpressions(last.face) };
  };

  const weight = (time: number): number => {
    const wIn = action.blend.in_s > 0 ? clamp(time / action.blend.in_s, 0, 1) : 1;
    if (hold) return wIn;
    const wOut = action.blend.out_s > 0 ? clamp((duration - time) / action.blend.out_s, 0, 1) : time < active ? 1 : 0;
    return Math.min(wIn, wOut);
  };

  return {
    action,
    duration,
    weight,
    done: (time) => time >= duration,
    sample(time: number): LivePose {
      const p = bonesAt(time);
      const w = weight(time);
      if (w >= 1) return p;
      const out = emptyPose(p.seq, p.t);
      for (const [k, q] of Object.entries(p.bones) as [PoseBone, Quat][]) out.bones[k] = qSlerp(QI, q, w);
      for (const [k, v] of Object.entries(p.expressions) as [ExpressionName, number][]) out.expressions[k] = v * w;
      if (p.fingers) out.fingers = Object.fromEntries(Object.entries(p.fingers).map(([k, v]) => [k, (v ?? 0) * w]));
      if (p.hips) out.hips = p.hips.map((v) => v * w) as Vec3;
      return out;
    },
  };
}

/** Validates then builds the player; the errors go back to the LLM as they are. */
export function gestureFromText(text: string): { player: GesturePlayer; warnings: string[] } | { errors: string[] } {
  const v = validateGesture(text);
  if (!v.ok) return { errors: v.errors };
  return { player: createGesturePlayer(v.action), warnings: v.warnings };
}
