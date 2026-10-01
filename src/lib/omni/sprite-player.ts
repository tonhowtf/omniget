import {
  getFrame,
  resolveDir,
  type AtlasData,
  type AtlasFrame,
  type Dir8,
} from '$lib/gfx/atlas';

const MAX_FPS = 12;

const PLAY_CYCLES = 2;

const REST_CLIP = 'idle';

const HOLD_CLIPS: readonly string[] = ['sleep', 'sit'];

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
  clip: string;
  animId: string;
  frames: readonly string[];
  flip: boolean;
  index: number;
  delayMs: number;
  nextDueMs: number | null;
  cyclesLeft: number;
  resting: boolean;
}

export function animId(clip: string, sheet = 'omni'): string {
  return `${sheet}/${clip}`;
}

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

export function advance(state: PlayerState, atlas: AtlasData, nowMs: number, sheet = 'omni'): boolean {
  if (state.nextDueMs === null || nowMs < state.nextDueMs) return false;

  const late = nowMs - state.nextDueMs > state.delayMs;
  state.index += 1;

  if (state.index >= state.frames.length) {
    state.cyclesLeft -= 1;
    if (state.cyclesLeft > 0) {
      state.index = 0;
    } else if (isHoldClip(state.clip)) {
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

function currentFrameId(state: PlayerState): string {
  return state.frames[state.index] ?? state.frames[0];
}

export function currentFrame(state: PlayerState, atlas: AtlasData): AtlasFrame {
  return getFrame(atlas, currentFrameId(state));
}

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

export function fitScale(frameW: number, frameH: number, boxW: number, boxH: number): number {
  const s = Math.floor(Math.min(boxW / frameW, boxH / frameH));
  return Math.max(1, s);
}
