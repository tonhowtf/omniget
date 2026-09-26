// Characters and objects of the V2 kit (Codex C01), as the runtime reads them.
//
// Contract accepted in session 06 (GameChangeLLM/contratos/CLAUDE-DECISOES.md):
//   - `animation-set.v2`: eight real screen directions (S, SW, W, NW, N, NE,
//     E, SE), never mirrored; the crate's world direction d is drawn with
//     `directions[world_to_screen_direction_indices[d]]`.
//   - One cell per frame, 128x160, pivot (64,128) = the actor's origin on
//     the tile centre. The atlas stores trimmed frames with the pivot moved
//     accordingly, so a sprite is placed at the tile centre with z = 0: all
//     height (seat, bed, bench) is already in the pixels.
//   - The actor and the object it uses are separate entities on the same
//     tile. An object is drawn as up to two layers per action and direction:
//     `c01obj/<object>/<action>/<dir>/under` (z_base -1, before the actor)
//     and `…/over` (z_base +1, after it). The renderer's stable depth sort
//     does the rest; no protocol change.
//   - Clip events are presentation only: they never credit an item or finish
//     a task. The server's task state decides the action; this only draws it.
//
// Pure: no DOM, no GL. The atlas comes from the renderer's own parser.

import { resolveDir, type AtlasData, type Dir8 } from './render/atlas';
import { frameIndex } from './render/anim';

export const ANIMATION_SET_SCHEMA = 'animation-set.v2';
export const C01_DIRECTIONS: readonly Dir8[] = ['S', 'SW', 'W', 'NW', 'N', 'NE', 'E', 'SE'];
/** The ten V2 actions. */
export const V2_STATES = ['idle', 'walk', 'sit', 'talk', 'water', 'plant', 'harvest', 'carry', 'sleep', 'work'] as const;
export type V2State = (typeof V2_STATES)[number];

/**
 * The crate's animation ids (0..7: idle, walk, sit, sleep, work, talk, wave,
 * yawn) in V2 terms. Wave is a gesture of talk; yawn has no art and idles.
 */
export const CRATE_ANIM_TO_V2: readonly V2State[] = ['idle', 'walk', 'sit', 'sleep', 'work', 'talk', 'talk', 'idle'];

export interface ClipSpec {
  loop: boolean;
  durationSeconds: number;
  events: Array<{ frame: number; name: string }>;
}

export interface AnimationSet {
  schema: typeof ANIMATION_SET_SCHEMA;
  cell: [number, number];
  pivot: [number, number];
  fps: number;
  framesPerClip: number;
  directions: Dir8[];
  worldToScreen: number[];
  clips: Map<string, ClipSpec>;
}

export class ContractError extends Error {
  constructor(message: string) {
    super(`ERR_WORLD_C01_CONTRACT: ${message}`);
    this.name = 'ContractError';
  }
}

function pair(v: unknown, what: string): [number, number] {
  if (!Array.isArray(v) || v.length !== 2 || !v.every((n) => Number.isInteger(n))) throw new ContractError(`${what} must be two integers`);
  return [v[0], v[1]];
}

/** Validates `animation-set.json`; anything the runtime cannot honour is refused. */
export function parseAnimationSet(json: unknown): AnimationSet {
  if (!json || typeof json !== 'object') throw new ContractError('not an object');
  const raw = json as Record<string, unknown>;
  if (raw.schema !== ANIMATION_SET_SCHEMA) throw new ContractError(`schema ${String(raw.schema)}, expected ${ANIMATION_SET_SCHEMA}`);
  if (raw.mirroring !== false) throw new ContractError('mirroring must be false: every direction is drawn');
  if (raw.direction_space !== 'screen') throw new ContractError('directions must be in screen space');
  const cell = pair(raw.cell, 'cell');
  const pivot = pair(raw.pivot, 'pivot');
  if (pivot[0] < 0 || pivot[1] < 0 || pivot[0] > cell[0] || pivot[1] > cell[1]) throw new ContractError('pivot outside the cell');
  const dirs = raw.directions;
  if (!Array.isArray(dirs) || dirs.join() !== C01_DIRECTIONS.join()) throw new ContractError(`directions must be ${C01_DIRECTIONS.join(',')}`);
  const map = raw.world_to_screen_direction_indices;
  if (!Array.isArray(map) || map.length !== 8 || [...map].sort().join() !== '0,1,2,3,4,5,6,7') {
    throw new ContractError('world_to_screen_direction_indices must be a permutation of 0..7');
  }
  const fps = raw.fps;
  if (typeof fps !== 'number' || fps < 1 || fps > 30) throw new ContractError('fps out of range');
  const framesPerClip = raw.frames_per_clip;
  if (!Number.isInteger(framesPerClip) || (framesPerClip as number) < 1) throw new ContractError('frames_per_clip');
  const clipsRaw = raw.clips;
  if (!clipsRaw || typeof clipsRaw !== 'object') throw new ContractError('no clips');
  const clips = new Map<string, ClipSpec>();
  for (const [name, v] of Object.entries(clipsRaw as Record<string, Record<string, unknown>>)) {
    if (typeof v.loop !== 'boolean') throw new ContractError(`clip ${name} loop`);
    const events = Array.isArray(v.events) ? (v.events as Array<{ frame: number; name: string }>) : [];
    for (const e of events) {
      if (!Number.isInteger(e.frame) || e.frame < 0 || e.frame >= (framesPerClip as number)) throw new ContractError(`clip ${name} event frame ${e.frame}`);
    }
    clips.set(name, { loop: v.loop, durationSeconds: Number(v.duration_seconds) || 0, events });
  }
  for (const s of V2_STATES) if (!clips.has(s)) throw new ContractError(`missing clip ${s}`);
  return { schema: ANIMATION_SET_SCHEMA, cell, pivot, fps, framesPerClip: framesPerClip as number, directions: [...C01_DIRECTIONS], worldToScreen: map as number[], clips };
}

/** Screen direction a world direction (crate id 0..7) is drawn with. */
export function screenDir(set: AnimationSet, worldDir: number): Dir8 {
  const i = set.worldToScreen[((worldDir % 8) + 8) % 8];
  return set.directions[i];
}

/**
 * The actor's frame: clip `v2/<recipe>/<state>` in the screen direction, at
 * `elapsedMs` since the action started. Never flipped. A one-shot clip holds
 * its last frame; what comes after is the task's decision, not the clip's.
 */
export function actorFrame(atlas: AtlasData, set: AnimationSet, recipe: string, state: V2State, worldDir: number, elapsedMs: number): string {
  const dir = screenDir(set, worldDir);
  const r = resolveDir(atlas, `v2/${recipe}/${state}`, dir);
  if (r.flip) throw new ContractError(`v2/${recipe}/${state}/${dir} resolved through a mirror`);
  const loop = set.clips.get(state)?.loop ?? r.loop;
  return r.frames[frameIndex(elapsedMs, set.fps, r.frames.length, loop)];
}

/** The layers of an object used by an action, in the actor's screen direction. */
export function objectLayers(atlas: AtlasData, object: string, state: V2State, dir: Dir8): { under: string | null; over: string | null } {
  const base = `c01obj/${object}/${state}/${dir}`;
  return {
    under: atlas.frames.has(`${base}/under`) ? `${base}/under` : null,
    over: atlas.frames.has(`${base}/over`) ? `${base}/over` : null,
  };
}

/**
 * Checks an atlas against the set: every state has eight real directions of
 * `framesPerClip` frames, every frame's pivot sits where the cell pivot says
 * once trimmed (inside its rect), and object layers carry z_base -1/+1.
 * Returns the problems found; empty means accepted.
 */
export function checkPack(atlas: AtlasData, set: AnimationSet, recipe: string): string[] {
  const problems: string[] = [];
  for (const state of V2_STATES) {
    const id = `v2/${recipe}/${state}`;
    const clip = atlas.anims.get(id);
    if (!clip) {
      problems.push(`${id}: missing`);
      continue;
    }
    for (const dir of set.directions) {
      const frames = clip.dirs[dir];
      if (!frames) {
        problems.push(`${id}/${dir}: ${clip.mirrorOf[dir] ? 'mirrored' : 'missing'}`);
        continue;
      }
      if (frames.length !== set.framesPerClip) problems.push(`${id}/${dir}: ${frames.length} frames`);
      for (const f of frames) {
        const fr = atlas.frames.get(f)!;
        if (fr.w > set.cell[0] || fr.h > set.cell[1]) problems.push(`${f}: larger than the cell`);
        if (fr.pivotX < 0 || fr.pivotY < 0 || fr.pivotX > set.cell[0] || fr.pivotY > set.cell[1]) problems.push(`${f}: pivot outside the cell`);
        if (fr.zBase !== 0) problems.push(`${f}: actor z_base ${fr.zBase}`);
      }
    }
  }
  for (const [id, fr] of atlas.frames) {
    if (!id.startsWith('c01obj/')) continue;
    const layer = id.split('/').pop();
    if (layer === 'under' && fr.zBase !== -1) problems.push(`${id}: under needs z_base -1`);
    if (layer === 'over' && fr.zBase !== 1) problems.push(`${id}: over needs z_base +1`);
    if (layer !== 'under' && layer !== 'over') problems.push(`${id}: unknown layer`);
  }
  return problems;
}
