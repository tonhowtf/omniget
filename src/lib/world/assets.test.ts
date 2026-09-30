// The pure half of the asset layer: merging the published atlases, turning the
// house map into baked chunk tiles, and choosing a sprite for an agent. The
// fixtures are the real files in `static/world` and the crate's own
// `house-v1.min.json`, read from disk, so a change in either breaks this test
// rather than the route.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { parseAtlas } from '$lib/world/render/atlas';
import { CHUNK_TILES } from '$lib/world/render/types';
import {
  ANIM_CLIPS,
  DIR_NAMES,
  EMPTY_TILE,
  agentSprite,
  buildChunkTiles,
  buildHudAtlasPart,
  marker,
  mergeAtlases,
  objectFrame,
  slotAt,
  zFromHeight,
  type MapDef,
} from './assets';

const read = (p: string) => JSON.parse(readFileSync(p, 'utf8'));
const omni = read('static/world/omni/atlas.json');
const casa = read('static/world/tiles/casa-v1/atlas.json');
const map = read('src-tauri/omniget-world/tests/fixtures/house-v1.min.json') as MapDef;

function mergedAtlas() {
  return parseAtlas(mergeAtlases([omni, casa, buildHudAtlasPart().part]).json);
}

describe('assets: merging the published atlases', () => {
  it('keeps every frame and shifts the page indices', () => {
    const merged = mergeAtlases([omni, casa, buildHudAtlasPart().part]);
    expect(merged.json.pages).toHaveLength(3);
    expect(Object.keys(merged.json.frames)).toHaveLength(
      Object.keys(omni.frames).length + Object.keys(casa.frames).length + 10,
    );
    expect(merged.json.frames['omni/idle/S/0'].page).toBe(0);
    expect(merged.json.frames['casa-v1/floor/wood'].page).toBe(1);
    expect(merged.json.frames['ui/bar/8'].page).toBe(2);
  });

  it("the merged atlas passes the renderer's own parser", () => {
    const atlas = mergedAtlas();
    expect(atlas.frames.size).toBeGreaterThan(100);
    expect(atlas.anims.has('omni/walk')).toBe(true);
    expect(atlas.tiles.get('object/workbench')?.frame).toBe('casa-v1/object/workbench');
  });

  it('the generated HUD page has a track and nine fills', () => {
    const { part } = buildHudAtlasPart();
    expect(part.frames['ui/bar/bg']).toBeTruthy();
    for (let i = 0; i <= 8; i++) expect(part.frames[`ui/bar/${i}`]).toBeTruthy();
    // Every fill starts at the same left edge, one pixel inside the track.
    const pivots = new Set(
      Array.from({ length: 9 }, (_, i) => String(part.frames[`ui/bar/${i}`].pivot)),
    );
    expect(pivots.size).toBe(1);
  });
});

describe('assets: the agent sprite', () => {
  it('resolves a clip and a direction into an atlas frame', () => {
    const atlas = mergedAtlas();
    const walkS = agentSprite(atlas, 1, 0, 0);
    expect(walkS.frame.startsWith('omni/walk/S/')).toBe(true);
    expect(walkS.flip).toBe(false);
  });

  it('the three mirrored directions come back flipped, never as their own art', () => {
    const atlas = mergedAtlas();
    for (const dir of [5, 6, 7]) {
      const s = agentSprite(atlas, 0, dir, 0);
      expect(s.flip).toBe(true);
    }
    for (const dir of [0, 1, 2, 3, 4]) {
      expect(agentSprite(atlas, 0, dir, 0).flip).toBe(false);
    }
  });

  it('the clip advances on its own clock', () => {
    const atlas = mergedAtlas();
    const a = agentSprite(atlas, 1, 0, 0).frame;
    const b = agentSprite(atlas, 1, 0, 400).frame;
    expect(a).not.toBe(b);
  });

  it('an animation with no art falls back instead of throwing', () => {
    const atlas = mergedAtlas();
    // Yawn (id 7) has no sheet yet; idle stands in for it.
    expect(agentSprite(atlas, 7, 0, 0).frame.startsWith('omni/idle/')).toBe(true);
    // And an id outside the table is still a frame, not an exception.
    expect(agentSprite(atlas, 99, 99, 0).frame.startsWith('omni/idle/')).toBe(true);
  });

  it("the clip and direction tables are the crate's, in the crate's order", () => {
    expect(ANIM_CLIPS).toEqual(['idle', 'walk', 'sit', 'sleep', 'work', 'talk', 'wave', 'yawn']);
    expect(DIR_NAMES).toEqual(['S', 'SW', 'W', 'NW', 'N', 'NE', 'E', 'SE']);
  });
});

describe('assets: the house', () => {
  it('bakes the map into chunk tiles the renderer understands', () => {
    const atlas = mergedAtlas();
    const chunks = buildChunkTiles(map, atlas);
    expect(chunks.size).toBe(map.chunks.length);
    const tiles = chunks.get('0,0')!;
    expect(tiles.length).toBeGreaterThan(CHUNK_TILES * CHUNK_TILES);
    for (const tile of tiles) {
      expect(atlas.frames.has(tile.frame)).toBe(true);
      expect(tile.lx).toBeGreaterThanOrEqual(0);
      expect(tile.lx).toBeLessThan(CHUNK_TILES);
      expect(tile.ly).toBeLessThan(CHUNK_TILES);
    }
    // The floor comes first, and the walls stand on their own elevation.
    expect(tiles[0].z).toBe(0);
    expect(tiles.some((t) => (t.z ?? 0) > 0)).toBe(true);
  });

  it('empty cells are skipped', () => {
    const atlas = mergedAtlas();
    const holes = map.chunks[0].object.filter((v) => v === EMPTY_TILE).length;
    expect(holes).toBeGreaterThan(0);
    const tiles = buildChunkTiles(map, atlas).get('0,0')!;
    const objectLayerTiles = tiles.filter((t) => t.frame.includes('/object/'));
    expect(objectLayerTiles.length).toBe(CHUNK_TILES * CHUNK_TILES - holes);
  });

  it('a wall 32 px high is one z unit', () => {
    expect(zFromHeight(32)).toBe(1);
    expect(zFromHeight(0)).toBe(0);
  });

  it('finds the markers and the slots', () => {
    expect(marker(map, 'spawn')).toEqual({ x: 8, y: 12 });
    expect(marker(map, 'nowhere')).toBeNull();
    expect(slotAt(map, 9, 3)?.id).toBe('study-bench');
    expect(slotAt(map, 0, 0)).toBeNull();
  });

  it('every object of the map has a frame in the atlas', () => {
    const atlas = mergedAtlas();
    for (const o of map.objects) expect(objectFrame(atlas, o.kind)).toBeTruthy();
    expect(objectFrame(atlas, 'object/nothing')).toBeNull();
  });
});

// A visual theme must not change the simulation's geometry or break pivots.
describe('handcrafted scenery content', () => {
  const craft = read('static/world/tiles/craft-v1/atlas.json');
  it('preserves every tile contract and frame geometry of the original house', () => {
    const themed = parseAtlas(craft);
    expect([...themed.tiles.keys()]).toEqual(expect.arrayContaining(Object.keys(casa.tiles)));
    for (const [key, tile] of Object.entries(casa.tiles) as [string, any][]) {
      const replacement = themed.tiles.get(key)!;
      expect({ ...replacement, frame: tile.frame }).toEqual(tile);
      const before = casa.frames[tile.frame];
      const after = themed.frames.get(replacement.frame)!;
      expect([after.w, after.h, after.pivotX, after.pivotY]).toEqual([
        before.w, before.h, ...before.pivot,
      ]);
    }
  });
  it('renders every farm prop and keeps its collision footprint on free ground', () => {
    const yard = read('static/world/yard-v1.json') as MapDef;
    const themed = parseAtlas(craft);
    const occupied = new Set<string>();
    const ground = new Set<string>();
    for (const chunk of yard.chunks) {
      chunk.floor.forEach((index, i) => {
        if (index !== EMPTY_TILE && yard.palette[index].walkable) {
          ground.add(`${chunk.cx * 16 + i % 16},${chunk.cy * 16 + Math.floor(i / 16)}`);
        }
      });
    }
    expect(yard.objects.filter((o) => o.kind === 'object/crop-bed')).toHaveLength(6);
    for (const object of yard.objects) {
      expect(objectFrame(themed, object.kind)).toBeTruthy();
      const tile = themed.tiles.get(object.kind)!;
      for (let dx = 0; dx < tile.footprint[0]; dx++) {
        for (let dy = 0; dy < tile.footprint[1]; dy++) {
          const cell = `${object.tile[0] + dx},${object.tile[1] + dy}`;
          expect(ground.has(cell), `${object.slot} outside the lawn`).toBe(true);
          expect(occupied.has(cell), `${object.slot} overlaps another prop`).toBe(false);
          occupied.add(cell);
        }
      }
    }
  });

});
