import { readFileSync } from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';
import { parseAtlas, type AtlasData } from '$lib/gfx/atlas';
import {
  advance,
  createPlayer,
  currentFrame,
  currentFrameId,
  delayFor,
  fitScale,
  isHoldClip,
  MAX_FPS,
  PET_CLIPS,
  PET_DIR,
  PLAY_CYCLES,
  PetClipError,
  placeFrame,
  playClip,
  REST_CLIP,
} from './sprite-player';

/**
 * The real shipped sheet, not a fixture: half the point of these tests is that
 * the art the app loads actually carries the seven clips the pet asks for.
 */
const ATLAS_PATH = path.resolve(__dirname, '../../../static/world/omni/atlas.json');
const atlas: AtlasData = parseAtlas(JSON.parse(readFileSync(ATLAS_PATH, 'utf8')));

describe('the shipped omni sheet', () => {
  it('carries every clip the pet can be asked to play', () => {
    for (const clip of PET_CLIPS) {
      const anim = atlas.anims.get(`omni/${clip}`);
      expect(anim, clip).toBeDefined();
      expect(anim!.dirs[PET_DIR].length, clip).toBeGreaterThan(0);
    }
  });

  it('is 48x64 at pivot (24, 60)', () => {
    const f = atlas.frames.get('omni/idle/S/0')!;
    expect([f.w, f.h, f.pivotX, f.pivotY]).toEqual([48, 64, 24, 60]);
  });
});

describe('delayFor', () => {
  it('caps the frame rate at 12 fps', () => {
    expect(delayFor(30)).toBe(Math.ceil(1000 / MAX_FPS));
    expect(delayFor(MAX_FPS)).toBe(84);
    expect(delayFor(6)).toBe(167);
    expect(delayFor(4)).toBe(250);
  });

  it('treats a nonsense fps as the cap instead of dividing by zero', () => {
    expect(Number.isFinite(delayFor(0))).toBe(true);
    expect(delayFor(0)).toBe(delayFor(MAX_FPS));
    expect(delayFor(-5)).toBe(delayFor(MAX_FPS));
  });
});

describe('createPlayer', () => {
  it('starts at rest with no timer owed', () => {
    const p = createPlayer(atlas);
    expect(p.clip).toBe(REST_CLIP);
    expect(p.resting).toBe(true);
    expect(p.nextDueMs).toBeNull();
    expect(p.index).toBe(0);
    expect(currentFrameId(p)).toBe('omni/idle/S/0');
  });

  it('never moves while nobody plays a clip', () => {
    const p = createPlayer(atlas);
    for (const t of [0, 1, 1000, 60_000, 3_600_000]) {
      expect(advance(p, atlas, t), `t=${t}`).toBe(false);
    }
    expect(p.index).toBe(0);
    expect(p.nextDueMs).toBeNull();
  });
});

describe('playClip', () => {
  it('arms the clock for an animated clip', () => {
    const p = playClip(createPlayer(atlas), atlas, 'walk', 1000);
    expect(p.clip).toBe('walk');
    expect(p.resting).toBe(false);
    expect(p.nextDueMs).toBe(1000 + delayFor(10));
    expect(p.cyclesLeft).toBe(PLAY_CYCLES);
    expect(currentFrameId(p)).toBe('omni/walk/S/0');
  });

  it('disarms the clock when the clip is the resting one', () => {
    const p = playClip(createPlayer(atlas), atlas, 'walk', 0);
    playClip(p, atlas, REST_CLIP, 500);
    expect(p.resting).toBe(true);
    expect(p.nextDueMs).toBeNull();
    expect(p.index).toBe(0);
  });

  it('restarts a clip that is already playing', () => {
    const p = playClip(createPlayer(atlas), atlas, 'wave', 0);
    advance(p, atlas, 10_000);
    expect(p.index).toBeGreaterThan(0);
    playClip(p, atlas, 'wave', 20_000);
    expect(p.index).toBe(0);
    expect(p.nextDueMs).toBe(20_000 + p.delayMs);
  });

  it('rejects a clip the atlas does not have, with a stable code', () => {
    expect(() => playClip(createPlayer(atlas), atlas, 'dance', 0)).toThrow(PetClipError);
    try {
      playClip(createPlayer(atlas), atlas, 'dance', 0);
    } catch (e) {
      expect((e as PetClipError).code).toBe('ERR_PET_CLIP');
    }
  });
});

describe('advance', () => {
  it('does nothing before the frame is due', () => {
    const p = playClip(createPlayer(atlas), atlas, 'walk', 0);
    const due = p.nextDueMs!;
    expect(advance(p, atlas, due - 1)).toBe(false);
    expect(p.index).toBe(0);
    expect(advance(p, atlas, due)).toBe(true);
    expect(p.index).toBe(1);
  });

  it('walks a whole cycle one frame at a time on the clip clock', () => {
    const p = playClip(createPlayer(atlas), atlas, 'walk', 0);
    const step = p.delayMs;
    const len = p.frames.length;
    expect(len).toBe(8);
    for (let i = 1; i < len; i++) {
      expect(advance(p, atlas, step * i), `frame ${i}`).toBe(true);
      expect(p.index).toBe(i);
      expect(p.nextDueMs).toBe(step * (i + 1));
    }
  });

  it('settles back to a static idle after PLAY_CYCLES loops', () => {
    const p = playClip(createPlayer(atlas), atlas, 'walk', 0);
    const step = p.delayMs;
    const frames = p.frames.length;
    // Every frame of every cycle, then the one step that ends the last cycle.
    for (let i = 1; i <= frames * PLAY_CYCLES; i++) advance(p, atlas, step * i);
    expect(p.clip).toBe(REST_CLIP);
    expect(p.resting).toBe(true);
    expect(p.nextDueMs).toBeNull();
    expect(currentFrameId(p)).toBe('omni/idle/S/0');
    // And from there it is silent forever.
    expect(advance(p, atlas, step * 10_000)).toBe(false);
  });

  it('holds the last frame of sleep instead of waking up', () => {
    expect(isHoldClip('sleep')).toBe(true);
    expect(isHoldClip('sit')).toBe(true);
    expect(isHoldClip('walk')).toBe(false);

    const p = playClip(createPlayer(atlas), atlas, 'sleep', 0);
    const step = p.delayMs;
    const frames = p.frames.length;
    for (let i = 1; i <= frames * PLAY_CYCLES; i++) advance(p, atlas, step * i);
    expect(p.clip).toBe('sleep');
    expect(p.resting).toBe(true);
    expect(p.nextDueMs).toBeNull();
    expect(p.index).toBe(frames - 1);
    expect(advance(p, atlas, 1e9)).toBe(false);
  });

  it('advances one frame at a time when the timer came back late', () => {
    const p = playClip(createPlayer(atlas), atlas, 'talk', 0);
    const step = p.delayMs;
    // The window was throttled for ten seconds.
    expect(advance(p, atlas, 10_000)).toBe(true);
    expect(p.index).toBe(1);
    // The schedule rebased on now instead of carrying ten seconds of debt.
    expect(p.nextDueMs).toBe(10_000 + step);
  });

  it('keeps the schedule when the timer was only a little late', () => {
    const p = playClip(createPlayer(atlas), atlas, 'talk', 0);
    const step = p.delayMs;
    advance(p, atlas, step + 1);
    expect(p.nextDueMs).toBe(step * 2);
  });

  it('never exceeds 12 fps on the fastest clip', () => {
    for (const clip of PET_CLIPS) {
      const p = playClip(createPlayer(atlas), atlas, clip, 0);
      expect(p.delayMs, clip).toBeGreaterThanOrEqual(Math.ceil(1000 / MAX_FPS));
    }
  });

  it('draws a real frame of the sheet at every index', () => {
    const p = playClip(createPlayer(atlas), atlas, 'work', 0);
    for (let i = 0; i < 40; i++) {
      expect(() => currentFrame(p, atlas)).not.toThrow();
      advance(p, atlas, p.delayMs * (i + 1));
    }
  });
});

describe('placeFrame / fitScale', () => {
  const frame = atlas.frames.get('omni/idle/S/0')!;

  it('picks the largest integer scale that fits the 200x200 window', () => {
    expect(fitScale(frame.w, frame.h, 200, 200)).toBe(3);
    expect(fitScale(frame.w, frame.h, 96, 128)).toBe(2);
    // Never zero, however small the box.
    expect(fitScale(frame.w, frame.h, 10, 10)).toBe(1);
  });

  it('centres on the pivot and lands the feet on the ground line', () => {
    const scale = 3;
    const inset = 24;
    const { dx, dy, dw, dh } = placeFrame(frame, 200, 200, scale, inset);
    expect(dw).toBe(frame.w * scale);
    expect(dh).toBe(frame.h * scale);
    // The pivot x sits at the horizontal centre of the box...
    expect(dx + frame.pivotX * scale).toBe(100);
    // ...and the pivot y on the ground line.
    expect(dy + frame.pivotY * scale).toBe(200 - inset);
    expect(Number.isInteger(dx) && Number.isInteger(dy)).toBe(true);
  });
});
