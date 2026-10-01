/**
 * Clock-driven sprite player for the Omni pet.
 *
 * Two rules shape this file, both from the Phase 5 budget
 * (`docs/llm-world-execution-plan.md` §1.2):
 *
 *  1. **At rest there is no clock.** `nextDueMs === null` means the caller must
 *     not arm a timer at all — not a `setInterval`, not a `requestAnimationFrame`.
 *     The pet at rest is one `drawImage` that happened once.
 *  2. **While animating the clock is ours.** The caller chains `setTimeout`, so
 *     the frame rate is the clip's, capped at 12 fps, and not the display's.
 *
 * Nothing here touches the DOM or a canvas, so every rule above is a unit test.
 * `advance` mutates the state it is given instead of returning a new object:
 * the budget says no allocation per frame, and a player is owned by exactly one
 * window.
 *
 * The atlas parser is reused from `$lib/gfx/atlas` (pure: it pulls in
 * only the constant module `render/types`), so the pet and the world read the
 * same art through the same code.
 */

import {
  getFrame,
  resolveDir,
  type AtlasData,
  type AtlasFrame,
  type Dir8,
} from '$lib/gfx/atlas';

/** Frames per second the pet is never allowed to exceed. */
const MAX_FPS = 12;

/** How many times a non-resting clip loops before the pet settles down. */
const PLAY_CYCLES = 2;

/** The clip the pet falls back to, drawn as a single static frame. */
const REST_CLIP = 'idle';

/**
 * Clips that hold their last frame instead of returning to `idle`: a pet that
 * woke up two seconds after falling asleep reads as a bug.
 */
const HOLD_CLIPS: readonly string[] = ['sleep', 'sit'];

/** The pet is a front view; the world's other seven directions are unused. */
const PET_DIR: Dir8 = 'S';

const ERR_PET_CLIP = 'ERR_PET_CLIP';

class PetClipError extends Error {
  readonly code = ERR_PET_CLIP;
  constructor(message: string) {
    super(message);
    this.name = 'PetClipError';
  }
}

export interface PlayerState {
  /** Short clip name, e.g. `walk`. */
  clip: string;
  /** Atlas animation id, e.g. `omni/walk`. */
  animId: string;
  /** Frame ids of the current clip and direction. Never copied. */
  frames: readonly string[];
  /** The direction is mirrored art; the draw call flips horizontally. */
  flip: boolean;
  index: number;
  /** Milliseconds between frames, already capped at `MAX_FPS`. */
  delayMs: number;
  /** Absolute time of the next frame, or `null` when the pet is at rest. */
  nextDueMs: number | null;
  /** Loops still owed before the pet settles. */
  cyclesLeft: number;
  /** True while no timer should be armed. */
  resting: boolean;
}

/** `sheet` + clip -> the atlas animation id. */
export function animId(clip: string, sheet = 'omni'): string {
  return `${sheet}/${clip}`;
}

/** Milliseconds per frame for a clip's fps, capped at `MAX_FPS`. */
function delayFor(fps: number): number {
  const capped = Math.min(fps > 0 ? fps : MAX_FPS, MAX_FPS);
  return Math.ceil(1000 / capped);
}

function isHoldClip(clip: string): boolean {
  return HOLD_CLIPS.includes(clip);
}

function load(
  atlas: AtlasData,
  clip: string,
  sheet: string,
): { id: string; frames: string[]; flip: boolean; fps: number } {
  const id = animId(clip, sheet);
  if (!atlas.anims.has(id)) {
    throw new PetClipError(`${ERR_PET_CLIP}: the atlas has no clip ${id}`);
  }
  const resolved = resolveDir(atlas, id, PET_DIR);
  return { id, frames: resolved.frames, flip: resolved.flip, fps: resolved.fps };
}

/**
 * A player sitting at rest on `REST_CLIP`. No timer is owed: the caller draws
 * once and stops.
 */
export function createPlayer(atlas: AtlasData, sheet = 'omni'): PlayerState {
  const { id, frames, flip, fps } = load(atlas, REST_CLIP, sheet);
  return {
    clip: REST_CLIP,
    animId: id,
    frames,
    flip,
    index: 0,
    delayMs: delayFor(fps),
    nextDueMs: null,
    cyclesLeft: 0,
    resting: true,
  };
}

/**
 * Switch to `clip` and start its clock, or settle if it is the resting clip.
 *
 * Returns the state it was given, mutated. Asking for the clip already playing
 * restarts it: an intent that repeats means the pet should wave again.
 */
export function playClip(
  state: PlayerState,
  atlas: AtlasData,
  clip: string,
  nowMs: number,
  sheet = 'omni',
): PlayerState {
  const { id, frames, flip, fps } = load(atlas, clip, sheet);
  state.clip = clip;
  state.animId = id;
  state.frames = frames;
  state.flip = flip;
  state.index = 0;
  state.delayMs = delayFor(fps);
  if (clip === REST_CLIP) {
    // Rest is one frame and zero timers, whatever the atlas says its fps is.
    state.nextDueMs = null;
    state.cyclesLeft = 0;
    state.resting = true;
    return state;
  }
  state.nextDueMs = nowMs + state.delayMs;
  state.cyclesLeft = PLAY_CYCLES;
  state.resting = false;
  return state;
}

/**
 * Move the clock to `nowMs`. Returns true when the drawn frame changed, so the
 * caller redraws only then.
 *
 * At most one frame advances per call even if the call arrived late: a window
 * that was throttled for ten seconds must not burn through twenty loops to
 * catch up. When that happens the schedule is rebased on `nowMs` instead of
 * accumulating debt.
 */
export function advance(state: PlayerState, atlas: AtlasData, nowMs: number, sheet = 'omni'): boolean {
  if (state.nextDueMs === null || nowMs < state.nextDueMs) return false;

  const late = nowMs - state.nextDueMs > state.delayMs;
  state.index += 1;

  if (state.index >= state.frames.length) {
    state.cyclesLeft -= 1;
    if (state.cyclesLeft > 0) {
      state.index = 0;
    } else if (isHoldClip(state.clip)) {
      // Sleep and sit end on their last frame and stay there, no timer.
      state.index = state.frames.length - 1;
      state.nextDueMs = null;
      state.resting = true;
      return true;
    } else {
      playClip(state, atlas, REST_CLIP, nowMs, sheet);
      return true;
    }
  }

  state.nextDueMs = late ? nowMs + state.delayMs : state.nextDueMs + state.delayMs;
  return true;
}

/** Frame id currently on screen. */
function currentFrameId(state: PlayerState): string {
  return state.frames[state.index] ?? state.frames[0];
}

export function currentFrame(state: PlayerState, atlas: AtlasData): AtlasFrame {
  return getFrame(atlas, currentFrameId(state));
}

/**
 * Where to put a frame inside a `boxW` x `boxH` window at integer `scale`, so
 * that the pivot (the feet) lands on `groundY` from the bottom of the box.
 *
 * Integer scale and integer destination keep the pixel art crisp: a half-pixel
 * destination with `imageSmoothingEnabled = false` still shows a seam.
 */
export function placeFrame(
  frame: AtlasFrame,
  boxW: number,
  boxH: number,
  scale: number,
  groundInset: number,
): { dx: number; dy: number; dw: number; dh: number } {
  const dw = frame.w * scale;
  const dh = frame.h * scale;
  const groundY = boxH - groundInset;
  return {
    dx: Math.round(boxW / 2 - frame.pivotX * scale),
    dy: Math.round(groundY - frame.pivotY * scale),
    dw,
    dh,
  };
}

/** Largest integer scale at which the tallest frame still fits the box. */
export function fitScale(frameW: number, frameH: number, boxW: number, boxH: number): number {
  const s = Math.floor(Math.min(boxW / frameW, boxH / frameH));
  return Math.max(1, s);
}
