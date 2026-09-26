import { existsSync, readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { mergeAtlases } from './assets';
import { parseAtlas } from './render/atlas';
import { sortByDepth } from './render/batcher';
import { ContractError, actorFrame, checkPack, objectLayers, parseAnimationSet, screenDir, V2_STATES } from './c01';

const DIRS = ['S', 'SW', 'W', 'NW', 'N', 'NE', 'E', 'SE'];
function set(over: Record<string, unknown> = {}) {
  return {
    schema: 'animation-set.v2',
    cell: [128, 160],
    pivot: [64, 128],
    fps: 12,
    frames_per_clip: 8,
    directions: DIRS,
    direction_space: 'screen',
    mirroring: false,
    world_to_screen_direction_indices: [1, 2, 3, 4, 5, 6, 7, 0],
    clips: Object.fromEntries(V2_STATES.map((s) => [s, { loop: !['water', 'plant', 'harvest'].includes(s), duration_seconds: 0.667, events: [] }])),
    ...over,
  };
}

function atlasFor(states: readonly string[], extra: Record<string, unknown> = {}) {
  const frames: Record<string, unknown> = {};
  const anims: Record<string, unknown> = {};
  for (const s of states) {
    const dirs: Record<string, string[]> = {};
    for (const d of DIRS) {
      dirs[d] = [];
      for (let i = 0; i < 8; i++) {
        const id = `v2/r/${s}/${d}/${i}`;
        frames[id] = { page: 0, x: 0, y: 0, w: 40, h: 80, pivot: [20, 76], z_base: 0 };
        dirs[d].push(id);
      }
    }
    anims[`v2/r/${s}`] = { fps: 12, loop: true, dirs };
  }
  return { version: 1, pages: ['p.png'], frames: { ...frames, ...extra }, anims, tiles: {} };
}

describe('animation-set.v2 contract', () => {
  it('accepts the candidate shape and maps world to screen directions', () => {
    const s = parseAnimationSet(set());
    expect(s.cell).toEqual([128, 160]);
    // Crate world S (0) is drawn with the screen SW sheet, per the mapping.
    expect(screenDir(s, 0)).toBe('SW');
    expect(screenDir(s, 7)).toBe('S');
  });

  it('refuses what the runtime cannot honour', () => {
    expect(() => parseAnimationSet(set({ schema: 'animation-set.v1' }))).toThrow(ContractError);
    expect(() => parseAnimationSet(set({ mirroring: true }))).toThrow(/mirroring/);
    expect(() => parseAnimationSet(set({ directions: [...DIRS].reverse() }))).toThrow(/directions/);
    expect(() => parseAnimationSet(set({ world_to_screen_direction_indices: [0, 0, 1, 2, 3, 4, 5, 6] }))).toThrow(/permutation/);
    expect(() => parseAnimationSet(set({ pivot: [200, 10] }))).toThrow(/pivot/);
    const clips = set().clips as Record<string, unknown>;
    delete clips.sleep;
    expect(() => parseAnimationSet(set({ clips }))).toThrow(/sleep/);
  });

  it('one-shot actions hold their last frame, loops wrap, nothing flips', () => {
    const s = parseAnimationSet(set());
    const atlas = parseAtlas(atlasFor(V2_STATES));
    expect(actorFrame(atlas, s, 'r', 'water', 0, 10_000)).toBe('v2/r/water/SW/7');
    expect(actorFrame(atlas, s, 'r', 'walk', 0, 9 * (1000 / 12) + 1)).toBe('v2/r/walk/SW/1');
    expect(checkPack(atlas, s, 'r')).toEqual([]);
  });

  it('checkPack names a mirrored or missing direction and a wrong layer depth', () => {
    const raw = atlasFor(V2_STATES, {
      'c01obj/chair/sit/N/under': { page: 0, x: 0, y: 0, w: 10, h: 10, pivot: [5, 9], z_base: 0 },
    }) as { anims: Record<string, { dirs: Record<string, unknown> }> };
    raw.anims['v2/r/sit'].dirs.E = { mirror_of: 'W' };
    const problems = checkPack(parseAtlas(raw), parseAnimationSet(set()), 'r');
    expect(problems).toContain('v2/r/sit/E: mirrored');
    expect(problems).toContain('c01obj/chair/sit/N/under: under needs z_base -1');
  });

  it('under/over layers sort around the actor on the same tile, whatever the input order', () => {
    const atlas = parseAtlas(
      atlasFor(['sit'], {
        'c01obj/chair/sit/N/under': { page: 0, x: 0, y: 0, w: 10, h: 10, pivot: [5, 9], z_base: -1 },
        'c01obj/chair/sit/N/over': { page: 0, x: 0, y: 0, w: 10, h: 10, pivot: [5, 9], z_base: 1 },
      }),
    );
    const layers = objectLayers(atlas, 'chair', 'sit', 'N');
    const sprites = [
      { frame: layers.over!, x: 3.5, y: 3.5, z: 0 },
      { frame: 'v2/r/sit/N/0', x: 3.5, y: 3.5, z: 0 },
      { frame: layers.under!, x: 3.5, y: 3.5, z: 0 },
    ];
    expect(sortByDepth(sprites, atlas).map((i) => sprites[i].frame)).toEqual([layers.under, 'v2/r/sit/N/0', layers.over]);
  });
});

// The real fixture, when this checkout sits next to the workshop's.
const FIXTURE = process.env.C01_FIXTURE ?? '../omnidisc-server/GameChangeLLM/production/c01-fixture/packs/3c593e292807f324';
const OBJPACK = process.env.C01_OBJPACK;
describe.skipIf(!existsSync(`${FIXTURE}/atlas.json`))('the C01 fixture pack', () => {
  it('passes the contract: ten states, eight real directions, eight frames, sane pivots', () => {
    const s = parseAnimationSet(JSON.parse(readFileSync(`${FIXTURE}/animation-set.json`, 'utf8')));
    const parts = [JSON.parse(readFileSync(`${FIXTURE}/atlas.json`, 'utf8'))];
    if (OBJPACK && existsSync(`${OBJPACK}/objects.atlas.json`)) parts.push(JSON.parse(readFileSync(`${OBJPACK}/objects.atlas.json`, 'utf8')));
    const atlas = parseAtlas(mergeAtlases(parts).json);
    expect(checkPack(atlas, s, 'resident-01')).toEqual([]);
  });
});
