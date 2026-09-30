// The 3D city on top of the `three3d` backend: kit, region streaming around
// the camera, distance LOD, picking, people, the frame loop with on-demand
// drawing and the watchdog. `CityView.svelte` is a thin wrapper around this;
// no 3D object is a Svelte component (estudo 01 §10).

import { Raycaster, Vector2, Vector3, type Intersection, type Object3D } from 'three';
import type { Blueprint, Building, Lot } from '$lib/city3d/types';
import { REGION_TILES, TILE_MM } from '$lib/city3d/types';
import { frameBudgetMs } from '../tier';
import { emitTierChanged, Watchdog } from '../watchdog';
import type { CameraMode3D, Caps3D, Tier } from '../types';
import { createCameraRig, type CameraRig } from './camera';
import { fallbackKit, parseKey, type Kit } from './kit';
import { PaletteBook } from './palette';
import { createPeople, type CharacterSystemLike, type People } from './people';
import { applyCellLod, buildRegion, regionKey, REGION_M, type BuiltRegion, type InstancePick } from './region';
import { createThreeRenderer, type ThreeRenderer } from './renderer';
import { tauriSource, type CityInfo, type RegionSource } from './source';

export interface CitySelection {
  kind: 'building' | 'lot' | 'prop' | 'module' | 'ground';
  region: [number, number];
  /** Building / lot / prop id; null for bare ground. */
  id: string | null;
  owner: string | null;
  key: string | null;
  tile: [number, number] | null;
  building: Building | null;
  lot: Lot | null;
}

export interface CityStats {
  fps: number;
  tier: Tier;
  cpuMs: number;
  drawCalls: number;
  triangles: number;
  instances: number;
  instancesLoaded: number;
  regions: number;
  people: number;
  rigged: boolean;
  kitReal: boolean;
  memoryBytes: number;
  compileMs: number;
  gpu: string;
}

export interface CitySceneOpts {
  canvas: HTMLCanvasElement;
  tier: Tier;
  camera: CameraMode3D;
  /** Lower the tier when p95 stays over budget (off when the tier is pinned). */
  watchdog?: boolean;
  source?: RegionSource;
  /** Server session: residents come from `city_join` instead of the local walkers. */
  live?: { city: string; server: string | null } | null;
  onSelect?: (sel: CitySelection | null) => void;
  onStats?: (s: CityStats) => void;
  onTierChanged?: (t: Tier) => void;
  onCanvasReplaced?: (c: HTMLCanvasElement) => void;
  onError?: (code: string) => void;
}

export interface CityScene {
  readonly renderer: ThreeRenderer;
  readonly rig: CameraRig;
  readonly info: CityInfo | null;
  readonly caps: Caps3D;
  /** Agent D's character system once loaded (null = capsules only). */
  readonly characters: CharacterSystemLike | null;
  /**
   * A live pose (agent I's `startLive`) on the viewer's avatar, which is
   * spawned at the camera target the first time. False when the character
   * system cannot take poses (capsules only).
   */
  applyPose(pose: unknown): boolean;
  /** Resolves when the first regions are built and the shaders compiled. */
  readonly ready: Promise<void>;
  setCameraMode(m: CameraMode3D): void;
  setTier(t: Tier): void;
  lookAtTile(region: [number, number], tile: [number, number]): void;
  reloadRegion(region: [number, number]): Promise<void>;
  resize(w: number, h: number, dpr: number): void;
  sleep(): void;
  wake(): Promise<void>;
  destroy(): void;
}

interface Loaded {
  built: BuiltRegion;
  blueprint: Blueprint | null;
}

const STREAM_EVERY_MS = 250;
const RETRY_FAILED_MS = 60_000;
/** Camera distance that frames the viewer's avatar while a live pose drives it. */
const SELF_VIEW_M = 7;

export async function createCityScene(opts: CitySceneOpts): Promise<CityScene> {
  const source = opts.source ?? tauriSource();
  let canvas = opts.canvas;
  const renderer = createThreeRenderer({
    onCanvasReplaced: (c) => {
      canvas = c;
      rig.connect(c);
      attachPointer(c);
      opts.onCanvasReplaced?.(c);
    },
    onLost: () => opts.onError?.('ERR_WORLD_CONTEXT_LOST'),
    onRestored: () => kick(),
  });
  const caps = await renderer.init(canvas, { tier: opts.tier, camera: opts.camera });
  const rig = createCameraRig(renderer.camera, canvas, opts.camera);

  const info = await source.info();
  const palettes = new PaletteBook(info?.palettes ?? [], info?.defaultPalette);
  let kit: Kit = fallbackKit(info?.style ?? 'default');
  const loaded = new Map<string, Loaded>();
  const loading = new Set<string>();
  const failed = new Map<string, number>();
  let destroyed = false;
  let sleeping = false;
  /** Regions wait for the kit, so none is built twice (stand-ins, then real). */
  let kitReady = false;

  let people: People | null = null;
  const peopleReady = createPeople(renderer.scene, renderer.gl!, renderer.materials, {
    people: renderer.budget.people,
    full: renderer.budget.fullCharacters,
    seed: info?.seed ?? 'city',
  })
    .then((p) => {
      people = p;
      if (destroyed) p.dispose();
      for (const l of loaded.values()) if (l.blueprint) p.setRegion(l.blueprint);
    })
    .catch(() => {});

  // ── regions ─────────────────────────────────────────────────────────────
  function candidates(target: Vector3): [number, number][] {
    const reach = renderer.budget.drawDistance + REGION_M * 0.72;
    const out: Array<{ r: [number, number]; d: number }> = [];
    const consider = (r: [number, number]) => {
      const cx = r[0] * REGION_M + REGION_M / 2;
      const cz = r[1] * REGION_M + REGION_M / 2;
      const d = Math.hypot(cx - target.x, cz - target.z);
      if (d <= reach) out.push({ r, d });
    };
    if (info && info.regions.length) {
      for (const r of info.regions) consider(r);
    } else {
      const rx = Math.floor(target.x / REGION_M);
      const rz = Math.floor(target.z / REGION_M);
      const span = Math.ceil(reach / REGION_M);
      for (let dz = -span; dz <= span; dz++) for (let dx = -span; dx <= span; dx++) consider([rx + dx, rz + dz]);
    }
    return out.sort((a, b) => a.d - b.d).map((o) => o.r);
  }

  function instancesLoaded(): number {
    let n = 0;
    for (const l of loaded.values()) n += l.built.instanceCount;
    return n;
  }

  async function loadRegion(r: [number, number], replace = false): Promise<void> {
    const key = regionKey(r);
    if (loading.has(key)) return;
    loading.add(key);
    try {
      const [mesh, blueprint] = await Promise.all([source.mesh(r), source.blueprint(r)]);
      if (destroyed) return;
      const built = buildRegion(mesh, kit, palettes, renderer.materials, {
        outline: renderer.budget.outline,
        shadows: renderer.budget.shadowMap > 0,
      });
      const old = loaded.get(key);
      if (old && !replace && old.built.hash === built.hash) {
        built.dispose();
        return;
      }
      old?.built.dispose();
      renderer.scene.add(built.group);
      loaded.set(key, { built, blueprint });
      failed.delete(key);
      if (blueprint) people?.setRegion(blueprint);
      renderer.invalidate();
      kick();
    } catch {
      failed.set(key, performance.now());
    } finally {
      loading.delete(key);
    }
  }

  function unloadRegion(key: string): void {
    const l = loaded.get(key);
    if (!l) return;
    l.built.dispose();
    loaded.delete(key);
    people?.dropRegion(l.built.region);
    renderer.invalidate();
  }

  let lastStream = 0;
  const target = new Vector3();
  let lastPoseAt = 0;
  function stream(now: number): void {
    if (!kitReady || now - lastStream < STREAM_EVERY_MS) return;
    lastStream = now;
    rig.target(target);
    const want = candidates(target);
    const wantKeys = new Set(want.map(regionKey));
    let budget = renderer.budget.instances - instancesLoaded();
    for (const r of want) {
      const key = regionKey(r);
      if (loaded.has(key) || loading.has(key)) continue;
      const f = failed.get(key);
      if (f !== undefined && now - f < RETRY_FAILED_MS) continue;
      if (budget <= 0) break;
      if (loading.size >= 2) break;
      void loadRegion(r);
      budget -= 2000; // a typical region; the real count lands with the mesh
    }
    const keepReach = renderer.budget.drawDistance * 1.5 + REGION_M;
    for (const [key, l] of loaded) {
      if (wantKeys.has(key)) continue;
      if (Math.hypot(l.built.center.x - target.x, l.built.center.z - target.z) > keepReach) unloadRegion(key);
    }
  }

  /**
   * Region culling by distance, then per cell: the kit's far variants past
   * the tier's `lodDistance`, detail chunks hidden past `detailDistance`,
   * outlines only on full-detail cells. Cheap enough per frame.
   */
  function applyLod(): void {
    const cam = renderer.camera.position;
    const b = renderer.budget;
    let changed = false;
    for (const { built } of loaded.values()) {
      const d = Math.hypot(built.center.x - cam.x, built.center.z - cam.z) - built.radius;
      const visible = d < b.drawDistance;
      if (built.group.visible !== visible) changed = true;
      built.group.visible = visible;
      if (!visible) continue;
      if (applyCellLod(built, cam, b)) changed = true;
    }
    if (changed) renderer.invalidate();
  }

  // ── picking ─────────────────────────────────────────────────────────────
  const raycaster = new Raycaster();
  const ndc = new Vector2();
  let down: { x: number; y: number } | null = null;
  const onDown = (e: PointerEvent) => {
    if (e.button === 0) down = { x: e.clientX, y: e.clientY };
    kick();
  };
  const onUp = (e: PointerEvent) => {
    if (e.button !== 0 || !down) return;
    const moved = Math.hypot(e.clientX - down.x, e.clientY - down.y);
    down = null;
    if (moved > 5) return;
    opts.onSelect?.(pick(e.clientX, e.clientY));
  };
  const onWheel = () => kick();
  const onKey = () => kick();
  let pointerHost: HTMLCanvasElement | null = null;
  function attachPointer(c: HTMLCanvasElement): void {
    if (pointerHost) {
      pointerHost.removeEventListener('pointerdown', onDown);
      pointerHost.removeEventListener('pointerup', onUp);
      pointerHost.removeEventListener('wheel', onWheel);
      pointerHost.removeEventListener('keydown', onKey);
    }
    pointerHost = c;
    c.addEventListener('pointerdown', onDown);
    c.addEventListener('pointerup', onUp);
    c.addEventListener('wheel', onWheel, { passive: true });
    c.addEventListener('keydown', onKey);
  }
  attachPointer(canvas);

  function pick(clientX: number, clientY: number): CitySelection | null {
    const rect = canvas.getBoundingClientRect();
    ndc.set(((clientX - rect.left) / rect.width) * 2 - 1, -((clientY - rect.top) / rect.height) * 2 + 1);
    raycaster.setFromCamera(ndc, renderer.camera);
    const groups: Object3D[] = [];
    for (const l of loaded.values()) if (l.built.group.visible) groups.push(l.built.group);
    const hits: Intersection[] = raycaster.intersectObjects(groups, true);
    for (const hit of hits) {
      const obj = hit.object;
      if (!obj.visible) continue;
      const region = (obj.parent?.userData.region ?? [0, 0]) as [number, number];
      const l = loaded.get(regionKey(region));
      const tile = tileOf(region, hit.point);
      if (obj.userData.kind === 'instances' && hit.instanceId !== undefined) {
        const p = (obj.userData.picks as InstancePick[])[hit.instanceId];
        if (!p) continue;
        return selectionFor(region, l?.blueprint ?? null, p, tile);
      }
      if (obj.userData.kind === 'terrain') {
        const lot = l?.blueprint ? lotAt(l.blueprint, tile) : null;
        const building = lot?.building ? (l?.blueprint?.buildings.find((b) => b.id === lot.building) ?? null) : null;
        return {
          kind: lot ? 'lot' : 'ground',
          region,
          id: lot?.id ?? null,
          owner: lot?.owner_ref ?? null,
          key: null,
          tile,
          building,
          lot,
        };
      }
    }
    return null;
  }

  function tileOf(region: [number, number], p: Vector3): [number, number] {
    const lx = Math.floor((p.x - region[0] * REGION_M) / (TILE_MM / 1000));
    const lz = Math.floor((p.z - region[1] * REGION_M) / (TILE_MM / 1000));
    return [Math.max(0, Math.min(REGION_TILES - 1, lx)), Math.max(0, Math.min(REGION_TILES - 1, lz))];
  }

  function lotAt(bp: Blueprint, [x, y]: [number, number]): Lot | null {
    for (const lot of bp.lots ?? []) {
      const [lx, ly, w, h] = lot.rect;
      if (x >= lx && x < lx + w && y >= ly && y < ly + h) return lot;
    }
    return null;
  }

  function selectionFor(region: [number, number], bp: Blueprint | null, p: InstancePick, tile: [number, number]): CitySelection {
    const { layer } = parseKey(p.key);
    const building = p.owner ? (bp?.buildings.find((b) => b.id === p.owner) ?? null) : null;
    const lot = building ? (bp?.lots.find((l) => l.id === building.lot) ?? null) : bp ? lotAt(bp, tile) : null;
    const kind: CitySelection['kind'] = building ? 'building' : layer === 'prop' || layer === 'arvore' || layer === 'enfeite' ? 'prop' : p.owner ? 'building' : 'module';
    return { kind, region, id: p.owner, owner: building?.owner_ref ?? lot?.owner_ref ?? null, key: p.key, tile, building, lot };
  }

  // ── live server residents ───────────────────────────────────────────────
  let stopLive: (() => void) | null = null;
  if (opts.live) {
    void (async () => {
      try {
        const { startServerResidents } = await import('./server');
        const stop = await startServerResidents(opts.live!, (agents) => {
          people?.setServerAgents(agents);
          kick();
        });
        if (destroyed) stop();
        else stopLive = stop;
      } catch (e) {
        opts.onError?.(e instanceof Error ? e.message : String(e));
      }
    })();
  }

  // ── loop ────────────────────────────────────────────────────────────────
  const watchdog = new Watchdog(renderer.tier);
  let raf = 0;
  let last = 0;
  let idleSince = 0;
  let frames = 0;
  let fpsStart = 0;
  let fps = 0;
  let lastStatsAt = 0;
  let lastCpu = 0;
  let lastCalls = 0;
  let lastTris = 0;
  let lastInstances = 0;

  function kick(): void {
    if (destroyed || sleeping || raf) return;
    last = 0;
    raf = requestAnimationFrame(tick);
  }

  function tick(now: number): void {
    raf = 0;
    if (destroyed || sleeping) return;
    const minGap = 1000 / renderer.budget.fps - 1.5;
    if (last !== 0 && now - last < minGap) {
      raf = requestAnimationFrame(tick);
      return;
    }
    const dtMs = last === 0 ? 16.7 : Math.min(250, now - last);
    const continuous = last !== 0;
    last = now;
    const dt = dtMs / 1000;

    const camMoved = rig.update(dt);
    if (camMoved) renderer.invalidate();
    const peopleMoved = people ? people.update(dt, renderer.camera) : false;
    if (peopleMoved) renderer.invalidate();
    stream(now);
    rig.target(target);
    renderer.followShadow(target);
    applyLod();
    const stats = renderer.frame(dtMs);
    if (stats.rendered) {
      lastCpu = stats.cpuMs;
      lastCalls = stats.drawCalls;
      lastTris = stats.triangles;
      lastInstances = stats.instances;
      frames++;
    }
    if (fpsStart === 0) fpsStart = now;
    else if (now - fpsStart >= 500) {
      fps = (frames * 1000) / (now - fpsStart);
      frames = 0;
      fpsStart = now;
    }
    if (continuous && stats.rendered && opts.watchdog !== false) {
      const from = watchdog.tier;
      const next = watchdog.push(dtMs, now);
      if (next !== null) {
        renderer.setTier(next);
        emitTierChanged({ from, to: next, p95Ms: watchdog.lastP95, budgetMs: frameBudgetMs(from), reason: 'watchdog' });
        opts.onTierChanged?.(next);
      }
    }
    if (now - lastStatsAt > 500) {
      lastStatsAt = now;
      opts.onStats?.(snapshot());
    }
    const busy = camMoved || peopleMoved || loading.size > 0 || stats.rendered;
    if (busy) idleSince = now;
    // Nothing moved for a while: stop asking for frames (0 rAF when idle).
    if (now - idleSince < 600) raf = requestAnimationFrame(tick);
    else fps = 0;
  }

  function snapshot(): CityStats {
    return {
      fps,
      tier: renderer.tier,
      cpuMs: lastCpu,
      drawCalls: lastCalls,
      triangles: lastTris,
      instances: lastInstances,
      instancesLoaded: instancesLoaded(),
      regions: loaded.size,
      people: people?.count ?? 0,
      rigged: people?.rigged ?? false,
      kitReal: kit.real,
      memoryBytes: renderer.memory().bytes,
      compileMs: renderer.caps?.compileMs ?? caps.compileMs,
      gpu: caps.gpu,
    };
  }

  rig.controls.addEventListener('update', () => kick());
  rig.controls.addEventListener('controlstart', () => kick());
  const offTier = renderer.onTier((t) => {
    people?.setBudget(renderer.budget.people, renderer.budget.fullCharacters);
    watchdog.setTier(t);
    kick();
  });
  const offChanged = await source.onChanged((r) => void loadRegion(r, true));

  // ── first view ──────────────────────────────────────────────────────────
  const first = info?.regions[0] ?? [0, 0];
  rig.lookAt(first[0] * REGION_M + REGION_M / 2, first[1] * REGION_M + REGION_M / 2, 110, false);

  const ready = (async () => {
    // The kit and the first ring of regions, then shaders, behind the loading state.
    kit = await source.kit(info?.style ?? 'default');
    kitReady = true;
    rig.target(target);
    const firstRing = candidates(target).slice(0, 4);
    await Promise.all(firstRing.map((r) => loadRegion(r)));
    await peopleReady;
    try {
      await renderer.compile();
    } catch {
      // Compiling on first draw is slower, not wrong.
    }
    lastStream = 0;
    kick();
  })();

  return {
    renderer,
    rig,
    info,
    caps,
    ready,
    get characters() {
      return people?.characters ?? null;
    },
    applyPose(pose: unknown): boolean {
      const sys = people?.characters;
      if (!people || !sys?.applyPose) return false;
      rig.target(target);
      // First pose after a pause: come close enough for the full rig (the
      // character system only poses full and reduced tiers).
      const now = performance.now();
      // Face the camera, like a mirror would.
      const id = people.ensureSelf(target, rig.controls.azimuthAngle);
      if (now - lastPoseAt > 2000) {
        // Look at the chest of the avatar standing on the terrain.
        void rig.controls.moveTo(target.x, people.groundAt(target.x, target.z) + 1.1, target.z, true);
        if (rig.controls.distance > SELF_VIEW_M) void rig.controls.dollyTo(SELF_VIEW_M, true);
      }
      lastPoseAt = now;
      try {
        sys.applyPose(id, pose);
      } catch {
        return false;
      }
      kick();
      return true;
    },
    setCameraMode(m) {
      rig.setMode(m);
      kick();
    },
    setTier(t) {
      renderer.setTier(t);
      watchdog.setTier(t);
      kick();
    },
    lookAtTile(region, tile) {
      rig.lookAt(region[0] * REGION_M + (tile[0] + 0.5) * 2, region[1] * REGION_M + (tile[1] + 0.5) * 2, 60, true);
      kick();
    },
    async reloadRegion(r) {
      await loadRegion(r, true);
    },
    resize(w, h, dpr) {
      renderer.resize(w, h, dpr);
      kick();
    },
    sleep() {
      if (sleeping) return;
      sleeping = true;
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
      renderer.sleep();
    },
    async wake() {
      if (!sleeping) return;
      await renderer.wake();
      sleeping = false;
      kick();
    },
    destroy() {
      destroyed = true;
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
      offTier();
      offChanged();
      stopLive?.();
      if (pointerHost) {
        pointerHost.removeEventListener('pointerdown', onDown);
        pointerHost.removeEventListener('pointerup', onUp);
        pointerHost.removeEventListener('wheel', onWheel);
        pointerHost.removeEventListener('keydown', onKey);
      }
      people?.dispose();
      for (const key of [...loaded.keys()]) unloadRegion(key);
      rig.dispose();
      kit.dispose();
      renderer.destroy();
    },
  };
}
