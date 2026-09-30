// People of the 3D city. Two sources, one drawing path:
// - local residents: presentational walkers on the blueprint's public tiles
//   (street, sidewalk, plaza, bridge) that now and then sit on a bench anchor;
// - the server: agents of the existing city session (`$lib/world/city`),
//   interpolated like the 2D canvas does and converted to metres.
// Drawing goes through agent D's `createCharacterSystem` (loaded lazily; the
// tier's `fullCharacters` caps how many are rigged). Everyone past the cap, or
// everyone when the module is missing or fails, is a coloured capsule in one
// InstancedMesh.

import {
  CapsuleGeometry,
  Color,
  InstancedMesh,
  Matrix4,
  Quaternion,
  Vector3,
  type Camera,
  type Scene,
  type WebGLRenderer,
} from 'three';
import { REGION_TILES, TILE_MM, type Anchor, type Blueprint, type Surface } from '$lib/city3d/types';
import type { MaterialSet } from './materials';
import { REGION_M } from './region';

/** The slice of agent D's character system this module uses. */
export interface CharacterSystemLike {
  spawn(id: string, genome: string, at?: { pos?: Vector3; yaw?: number }): unknown;
  setTarget(id: string, pos: Vector3, yaw: number, action: string): void;
  update(dt: number, camera: Camera): void;
  dispose(): void;
  despawn?(id: string): void;
  remove?(id: string): void;
  /** Agent I's live poses (face/body from the camera), implemented by D. */
  applyPose?(id: string, pose: unknown): void;
}

/** Id of the viewer's own avatar in the scene. */
export const SELF_ID = 'me';

type CreateCharacterSystem = (
  scene: Scene,
  renderer: WebGLRenderer,
  opts?: Record<string, unknown>,
) => CharacterSystemLike | Promise<CharacterSystemLike>;

// A glob instead of a static import: the module is agent D's and may not
// exist yet; an empty glob is a capsule-only city, not a build error.
const avatarModules = import.meta.glob('../../../city3d/avatar/index.ts');

async function loadCharacterSystem(): Promise<CreateCharacterSystem | null> {
  const load = Object.values(avatarModules)[0];
  if (!load) return null;
  try {
    const mod = (await load()) as { createCharacterSystem?: CreateCharacterSystem };
    return typeof mod.createCharacterSystem === 'function' ? mod.createCharacterSystem : null;
  } catch {
    return null;
  }
}

const PUBLIC: ReadonlySet<Surface> = new Set<Surface>(['street', 'sidewalk', 'plaza', 'bridge']);
const WALK_SPEED = 1.3; // m/s
const TILE_M = TILE_MM / 1000;

function hashStr(s: string): number {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  return h >>> 0;
}

function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Walkable grid and anchors of one region, from its blueprint. */
interface Streets {
  region: [number, number];
  walk: Uint8Array; // 64×64, 1 = public tile
  tiles: number[]; // indices of walkable tiles
  heights: number[]; // 65×65 corner heights, mm
  anchors: Array<Anchor & { taken: boolean }>;
}

function streetsOf(bp: Blueprint): Streets {
  const walk = new Uint8Array(REGION_TILES * REGION_TILES);
  const tiles: number[] = [];
  for (let i = 0; i < walk.length; i++) {
    if (PUBLIC.has(bp.surface?.[i] as Surface)) {
      walk[i] = 1;
      tiles.push(i);
    }
  }
  const anchors: Streets['anchors'] = [];
  for (const p of bp.props ?? []) for (const a of p.anchors ?? []) anchors.push({ ...a, taken: false });
  return { region: bp.region, walk, tiles, heights: bp.corner_height_mm ?? [], anchors };
}

/** Terrain height (m) at region-local tile coordinates, bilinear over corners. */
function heightAt(s: Streets, tx: number, ty: number): number {
  const h = s.heights;
  if (h.length < 65 * 65) return 0;
  const x0 = Math.max(0, Math.min(63, Math.floor(tx)));
  const y0 = Math.max(0, Math.min(63, Math.floor(ty)));
  const fx = Math.max(0, Math.min(1, tx - x0));
  const fy = Math.max(0, Math.min(1, ty - y0));
  const c = (x: number, y: number) => h[y * 65 + x] ?? 0;
  const a = c(x0, y0) + (c(x0 + 1, y0) - c(x0, y0)) * fx;
  const b = c(x0, y0 + 1) + (c(x0 + 1, y0 + 1) - c(x0, y0 + 1)) * fx;
  return (a + (b - a) * fy) / 1000;
}

/** BFS over walkable tiles; returns tile indices from start (exclusive) to goal. */
function bfs(s: Streets, from: number, goal: number): number[] | null {
  if (from === goal) return [];
  const prev = new Int32Array(REGION_TILES * REGION_TILES).fill(-1);
  prev[from] = from;
  const q = [from];
  for (let qi = 0; qi < q.length; qi++) {
    const cur = q[qi];
    const x = cur % REGION_TILES;
    const y = (cur / REGION_TILES) | 0;
    const next = [
      x > 0 ? cur - 1 : -1,
      x < REGION_TILES - 1 ? cur + 1 : -1,
      y > 0 ? cur - REGION_TILES : -1,
      y < REGION_TILES - 1 ? cur + REGION_TILES : -1,
    ];
    for (const n of next) {
      if (n < 0 || prev[n] !== -1) continue;
      if (!s.walk[n] && n !== goal) continue;
      prev[n] = cur;
      if (n === goal) {
        const path: number[] = [];
        for (let c = goal; c !== from; c = prev[c]) path.push(c);
        return path.reverse();
      }
      q.push(n);
    }
  }
  return null;
}

interface Walker {
  id: string;
  streets: Streets;
  tile: number;
  path: number[];
  pos: Vector3;
  from: Vector3;
  to: Vector3;
  t: number;
  legSec: number;
  yaw: number;
  action: 'walk' | 'idle' | 'sit';
  waitSec: number;
  anchor: (Anchor & { taken: boolean }) | null;
  rand: () => number;
  heading: number;
}

interface Figure {
  id: string;
  full: boolean;
  slot: number; // capsule instance index when !full
  color: Color;
  pos: Vector3;
  yaw: number;
  action: string;
  seen: boolean;
}

export interface ServerAgent {
  id: string;
  /** Tile units of the server map. */
  x: number;
  y: number;
  z: number;
  yaw: number;
  moving: boolean;
}

export interface People {
  /** Gives the local residents the streets of a region (replaces its data). */
  setRegion(bp: Blueprint): void;
  dropRegion(region: [number, number]): void;
  /** Budget of walkers and of rigged characters (tier). */
  setBudget(people: number, full: number): void;
  /** Server positions replace the local walkers while set. */
  setServerAgents(agents: ServerAgent[] | null): void;
  /** Puts the viewer's avatar at `pos` (spawned once), turned to `yaw` when given, and returns its id. */
  ensureSelf(pos: Vector3, yaw?: number): string;
  /** Terrain height (m) at a world position, 0 where no region is loaded. */
  groundAt(x: number, z: number): number;
  /** Advances walkers and characters; true when anything moved. */
  update(dtSec: number, camera: Camera): boolean;
  readonly count: number;
  readonly rigged: boolean;
  /** Agent D's system when it loaded (agent I applies live poses through it). */
  readonly characters: CharacterSystemLike | null;
  dispose(): void;
}

const _m = new Matrix4();
const _q = new Quaternion();
const _s = new Vector3(1, 1, 1);
const _up = new Vector3(0, 1, 0);
const _p = new Vector3();

export async function createPeople(
  scene: Scene,
  gl: WebGLRenderer,
  mats: MaterialSet,
  opts: { people: number; full: number; seed?: string },
): Promise<People> {
  let peopleBudget = opts.people;
  let fullBudget = opts.full;
  const seed = opts.seed ?? 'city';
  const streets = new Map<string, Streets>();
  const walkers = new Map<string, Walker>();
  const figures = new Map<string, Figure>();
  let serverMode = false;

  const MAX_CAPSULES = 256;
  const capsuleGeo = new CapsuleGeometry(0.28, 1.0, 4, 8);
  capsuleGeo.translate(0, 0.78, 0);
  const capsules = new InstancedMesh(capsuleGeo, mats.plain, MAX_CAPSULES);
  capsules.count = 0;
  capsules.castShadow = true;
  capsules.frustumCulled = false;
  capsules.name = 'people:capsules';
  scene.add(capsules);
  const freeSlots: number[] = [];
  let usedSlots = 0;

  let system: CharacterSystemLike | null = null;
  const factory = await loadCharacterSystem();
  if (factory) {
    try {
      // D's system has its own LOD (full / reduced / crowd); the tier caps the full rigs.
      system = await Promise.resolve(factory(scene, gl, { maxFull: fullBudget, maxReduced: Math.max(peopleBudget, 8) }));
    } catch {
      system = null;
    }
  }

  let genomes: string[] | null = null;
  /** A genome string for D's `spawn`, or null (the figure stays a capsule). */
  async function genomeFor(id: string, index: number): Promise<string | null> {
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      if (id === SELF_ID) {
        try {
          const mine = (await invoke('city3d_avatar_get')) as { genome?: string } | null;
          if (mine?.genome) return mine.genome;
        } catch {
          // No saved avatar: a random one below.
        }
      }
      if (genomes === null) {
        try {
          const list = (await invoke('city3d_npc_identities', {
            cityId: seed,
            count: Math.max(8, peopleBudget),
            seed: hashStr(seed),
          })) as Array<{ genome?: string }>;
          genomes = (Array.isArray(list) ? list : []).map((r) => r?.genome).filter((g): g is string => typeof g === 'string');
        } catch {
          genomes = [];
        }
      }
      if (id !== SELF_ID && genomes.length) return genomes[index % genomes.length];
      const r = (await invoke('city3d_genome_random', { seed: hashStr(`${seed}:${id}`) })) as { genome?: string };
      return r?.genome ?? null;
    } catch {
      return null;
    }
  }

  function addFigure(id: string): Figure {
    const r = rng(hashStr(id));
    const color = new Color().setHSL(r(), 0.45 + r() * 0.3, 0.45 + r() * 0.2);
    const f: Figure = { id, full: false, slot: -1, color, pos: new Vector3(), yaw: 0, action: 'idle', seen: true };
    // A capsule until D's figure exists; then D draws it (and its LOD).
    f.slot = takeSlot();
    if (system) {
      const sys = system;
      void genomeFor(id, figures.size).then((genome) => {
        if (!genome || figures.get(id) !== f) return;
        try {
          sys.spawn(id, genome, { pos: f.pos, yaw: f.yaw });
          releaseSlot(f);
          f.full = true;
        } catch {
          // D's system refused this one: it stays a capsule.
        }
      });
    }
    figures.set(id, f);
    return f;
  }

  function releaseSlot(f: Figure): void {
    if (f.slot < 0) return;
    _m.makeScale(0, 0, 0);
    capsules.setMatrixAt(f.slot, _m);
    capsules.instanceMatrix.needsUpdate = true;
    freeSlots.push(f.slot);
    f.slot = -1;
  }

  function takeSlot(): number {
    if (freeSlots.length) return freeSlots.pop()!;
    if (usedSlots >= MAX_CAPSULES) return -1;
    return usedSlots++;
  }

  function removeFigure(id: string): void {
    const f = figures.get(id);
    if (!f) return;
    if (f.full) {
      try {
        if (system?.despawn) system.despawn(id);
        else system?.remove?.(id);
      } catch {
        // Nothing to clean on our side.
      }
    }
    releaseSlot(f);
    figures.delete(id);
  }

  function tileCenter(s: Streets, tile: number, out: Vector3): Vector3 {
    const tx = (tile % REGION_TILES) + 0.5;
    const ty = Math.floor(tile / REGION_TILES) + 0.5;
    return out.set(s.region[0] * REGION_M + tx * TILE_M, heightAt(s, tx, ty), s.region[1] * REGION_M + ty * TILE_M);
  }

  function spawnWalkers(): void {
    if (serverMode) return;
    const all = [...streets.values()].filter((s) => s.tiles.length > 0);
    if (!all.length) return;
    let i = 0;
    while (walkers.size < peopleBudget && i < peopleBudget * 4) {
      const id = `npc:${seed}:${i++}`;
      if (walkers.has(id)) continue;
      const r = rng(hashStr(id));
      const s = all[Math.floor(r() * all.length)];
      const tile = s.tiles[Math.floor(r() * s.tiles.length)];
      const pos = tileCenter(s, tile, new Vector3());
      walkers.set(id, {
        id,
        streets: s,
        tile,
        path: [],
        pos: pos.clone(),
        from: pos.clone(),
        to: pos.clone(),
        t: 1,
        legSec: 0,
        yaw: 0,
        action: 'idle',
        waitSec: r() * 2,
        anchor: null,
        rand: r,
        heading: Math.floor(r() * 4),
      });
      addFigure(id);
    }
    // Over budget after a tier drop: the newest go first.
    const ids = [...walkers.keys()];
    for (let k = ids.length - 1; k >= 0 && walkers.size > peopleBudget; k--) {
      const w = walkers.get(ids[k])!;
      if (w.anchor) w.anchor.taken = false;
      walkers.delete(ids[k]);
      removeFigure(ids[k]);
    }
  }

  function nextLeg(w: Walker): void {
    const s = w.streets;
    if (!w.path.length) {
      // Sometimes head for a free bench; otherwise wander, preferring straight.
      const free = s.anchors.filter((a) => !a.taken && a.action.startsWith('sit'));
      if (free.length && w.rand() < 0.08) {
        const a = free[Math.floor(w.rand() * free.length)];
        const goal = a.cell[1] * REGION_TILES + a.cell[0];
        const path = bfs(s, w.tile, goal);
        if (path) {
          a.taken = true;
          w.anchor = a;
          w.path = path;
        }
      }
      if (!w.path.length) {
        const x = w.tile % REGION_TILES;
        const y = (w.tile / REGION_TILES) | 0;
        const dirs = [
          [0, -1],
          [1, 0],
          [0, 1],
          [-1, 0],
        ];
        const options: number[] = [];
        for (let d = 0; d < 4; d++) {
          const nx = x + dirs[d][0];
          const ny = y + dirs[d][1];
          if (nx < 0 || ny < 0 || nx >= REGION_TILES || ny >= REGION_TILES) continue;
          const n = ny * REGION_TILES + nx;
          if (!s.walk[n]) continue;
          // Straight on counts three times, turning once, going back never unless stuck.
          const weight = d === w.heading ? 3 : d === (w.heading + 2) % 4 ? 0 : 1;
          for (let k = 0; k < weight; k++) options.push(d);
        }
        if (!options.length) {
          for (let d = 0; d < 4; d++) {
            const nx = x + dirs[d][0];
            const ny = y + dirs[d][1];
            if (nx >= 0 && ny >= 0 && nx < REGION_TILES && ny < REGION_TILES && s.walk[ny * REGION_TILES + nx]) options.push(d);
          }
        }
        if (!options.length) {
          w.action = 'idle';
          w.waitSec = 3;
          return;
        }
        const d = options[Math.floor(w.rand() * options.length)];
        w.heading = d;
        w.path = [(y + dirs[d][1]) * REGION_TILES + (x + dirs[d][0])];
      }
    }
    const next = w.path.shift()!;
    w.from.copy(w.pos);
    tileCenter(s, next, w.to);
    w.tile = next;
    w.t = 0;
    w.legSec = Math.max(0.2, w.from.distanceTo(w.to) / WALK_SPEED);
    w.yaw = Math.atan2(w.to.x - w.from.x, w.to.z - w.from.z);
    w.action = 'walk';
  }

  function stepWalker(w: Walker, dt: number): void {
    if (w.waitSec > 0) {
      w.waitSec -= dt;
      if (w.waitSec > 0) return;
      if (w.action === 'sit' && w.anchor) {
        w.anchor.taken = false;
        w.anchor = null;
      }
      w.action = 'idle';
    }
    if (w.t >= 1) {
      if (w.anchor && !w.path.length && w.action === 'walk') {
        // Arrived at the bench: sit facing the anchor's yaw.
        const a = w.anchor;
        // Anchor yaw: 0..=255 maps to 0..360°, clockwise from north (types.rs).
        const yaw = (-a.yaw / 256) * Math.PI * 2;
        w.yaw = yaw + Math.PI;
        w.pos.y += a.level * 3;
        w.action = 'sit';
        w.waitSec = 8 + w.rand() * 10;
        return;
      }
      nextLeg(w);
      if (w.action !== 'walk') return;
    }
    w.t = Math.min(1, w.t + dt / w.legSec);
    w.pos.lerpVectors(w.from, w.to, w.t);
  }

  function writeCapsule(f: Figure): void {
    if (f.slot < 0) return;
    _q.setFromAxisAngle(_up, f.yaw);
    _p.copy(f.pos);
    if (f.action === 'sit') _p.y -= 0.35;
    _m.compose(_p, _q, _s);
    capsules.setMatrixAt(f.slot, _m);
    capsules.setColorAt(f.slot, f.color);
  }

  const people: People = {
    setRegion(bp: Blueprint) {
      const key = `${bp.region[0]},${bp.region[1]}`;
      const s = streetsOf(bp);
      streets.set(key, s);
      // Walkers of the old data keep their position but take the new grid.
      for (const w of walkers.values()) {
        if (w.streets.region[0] === bp.region[0] && w.streets.region[1] === bp.region[1]) {
          w.streets = s;
          w.path = [];
          w.anchor = null;
          if (!s.walk[w.tile] && s.tiles.length) w.tile = s.tiles[0];
        }
      }
      spawnWalkers();
    },
    dropRegion(region) {
      streets.delete(`${region[0]},${region[1]}`);
      for (const [id, w] of walkers) {
        if (w.streets.region[0] === region[0] && w.streets.region[1] === region[1]) {
          walkers.delete(id);
          removeFigure(id);
        }
      }
      spawnWalkers();
    },
    setBudget(p: number, full: number) {
      peopleBudget = p;
      fullBudget = full;
      spawnWalkers();
    },
    setServerAgents(agents) {
      if (agents === null) {
        if (serverMode) {
          serverMode = false;
          for (const id of [...figures.keys()]) if (id.startsWith('srv:')) removeFigure(id);
          spawnWalkers();
        }
        return;
      }
      if (!serverMode) {
        serverMode = true;
        for (const id of [...walkers.keys()]) {
          walkers.delete(id);
          removeFigure(id);
        }
      }
      for (const f of figures.values()) f.seen = false;
      for (const a of agents.slice(0, Math.max(peopleBudget, 8))) {
        const id = `srv:${a.id}`;
        const f = figures.get(id) ?? addFigure(id);
        f.seen = true;
        f.pos.set(a.x * TILE_M, a.z * 3, a.y * TILE_M);
        f.yaw = a.yaw;
        f.action = a.moving ? 'walk' : 'idle';
      }
      for (const [id, f] of figures) if (!f.seen && id !== SELF_ID) removeFigure(id);
    },
    ensureSelf(pos: Vector3, yaw?: number) {
      const f = figures.get(SELF_ID) ?? addFigure(SELF_ID);
      f.pos.copy(pos);
      // The camera target sits at y = 0; stand on the terrain instead.
      f.pos.y = people.groundAt(pos.x, pos.z);
      if (yaw !== undefined) f.yaw = yaw;
      f.action = 'idle';
      return SELF_ID;
    },
    groundAt(x: number, z: number) {
      const rx = Math.floor(x / REGION_M);
      const rz = Math.floor(z / REGION_M);
      const st = streets.get(`${rx},${rz}`);
      return st ? heightAt(st, (x - rx * REGION_M) / TILE_M, (z - rz * REGION_M) / TILE_M) : 0;
    },
    update(dt: number, camera: Camera): boolean {
      let moved = false;
      if (!serverMode) {
        for (const w of walkers.values()) {
          stepWalker(w, dt);
          const f = figures.get(w.id);
          if (!f) continue;
          f.pos.copy(w.pos);
          f.yaw = w.yaw;
          f.action = w.action;
        }
      }
      let top = 0;
      for (const f of figures.values()) {
        if (f.full) {
          try {
            system?.setTarget(f.id, f.pos, f.yaw, f.action);
          } catch {
            // A figure D does not know yet (genome still loading).
          }
        } else {
          writeCapsule(f);
          top = Math.max(top, f.slot + 1);
        }
        if (f.action === 'walk') moved = true;
      }
      capsules.count = top;
      capsules.instanceMatrix.needsUpdate = true;
      if (capsules.instanceColor) capsules.instanceColor.needsUpdate = true;
      if (system) {
        system.update(dt, camera);
        if (figures.size) moved = true;
      }
      return moved;
    },
    get count() {
      return figures.size;
    },
    get rigged() {
      return !!system;
    },
    get characters() {
      return system;
    },
    dispose() {
      try {
        system?.dispose();
      } catch {
        // Nothing left to do.
      }
      system = null;
      capsules.removeFromParent();
      capsules.dispose();
      capsuleGeo.dispose();
      figures.clear();
      walkers.clear();
    },
  };
  return people;
}
