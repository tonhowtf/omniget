<script lang="ts">
  /**
   * The world, drawn.
   *
   * Owns the renderer, the session and the frame loop. The rules it exists to
   * keep:
   *   - nothing is imported until this component mounts, and everything is
   *     released when it goes away (0 rAF, no GL context, no channel);
   *   - the simulation is authoritative: this file interpolates between the
   *     ticks it is sent and never invents a position;
   *   - the HUD is inside the canvas (names, balloons, energy bars, clock);
   *     the only DOM over it is the slot menu, which is a menu, not a HUD.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
  import { codecErrorCode, type Blob as WorldBlob } from "$lib/world/codec";
  import {
    agentSprite,
    buildChunkTiles,
    loadHouseMap,
    loadWorldAtlas,
    marker,
    objectFrame,
    slotAt,
    type MapDef,
    type SlotDef,
  } from "$lib/world/assets";
  import { buildHud, capAgents, gameClockLabel, type AgentFrame } from "$lib/world/hud";
  import { attachInput, followCanvas, type Pickable } from "$lib/world/input";
  import { sample } from "$lib/world/interp";
  import { STATE_ERR, WorldState } from "$lib/world/state";
  import { VISITOR_ENT_BASE, type AgentRow } from "$lib/world/activity";
  import {
    createFakeSession,
    createTauriSession,
    createVisitSession,
    inTauri,
    type WorldInput,
    type WorldSession,
  } from "$lib/world/session";
  import type { AtlasData } from "$lib/world/render/atlas";
  import type {
    Camera,
    ChunkId,
    ChunkTile,
    FrameStats,
    Renderer,
    Scene,
    SpriteInst,
    TextInst,
    Tier,
  } from "$lib/world/render/types";

  interface Props {
    /** Fixture blobs instead of Tauri: the tests and the storybook path. */
    fakeBlobs?: Uint8Array[];
    /** Read-only scenery review; never opens or replaces a saved world. */
    previewMap?: string;
    onstats?: (stats: FrameStats, fps: number, tier: Tier) => void;
    /** Set when this canvas shows someone else's open house. */
    visit?: { code: string; server?: string | null } | null;
    onvisitfailed?: (error: string) => void;
    /** The residents, about twice a second: what the activity panel lists. */
    onagents?: (agents: AgentRow[]) => void;
    /** The session is open and the first snapshot is on screen. */
    onready?: () => void;
    /** Keep every resident in the shot, as tight as they allow (the demo). */
    followAll?: boolean;
  }
  let { fakeBlobs, previewMap, onstats, visit = null, onvisitfailed, onagents, onready, followAll = false }: Props = $props();

  /** Zoom out until every room is on screen: the view a demo is watched from. */
  export function frameHouse(): void {
    if (!map || map.rooms.length === 0 || !projectFn || !clampZoomFn) return;
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity;
    for (const r of map.rooms) {
      x0 = Math.min(x0, r.rect[0]);
      y0 = Math.min(y0, r.rect[1]);
      x1 = Math.max(x1, r.rect[0] + r.rect[2]);
      y1 = Math.max(y1, r.rect[1] + r.rect[3]);
    }
    const corners = [projectFn(x0, y0), projectFn(x1, y0), projectFn(x0, y1), projectFn(x1, y1)];
    const spanX = Math.max(...corners.map((c) => c.px)) - Math.min(...corners.map((c) => c.px));
    const spanY = Math.max(...corners.map((c) => c.py)) - Math.min(...corners.map((c) => c.py));
    camera.x = (x0 + x1) / 2;
    camera.y = (y0 + y1) / 2;
    camera.zoom = clampZoomFn(Math.min(camera.width / spanX, camera.height / spanY) * 0.92);
    selected = null;
  }

  /**
   * Ease the camera towards the box around every resident. Agents that split
   * up between two rooms pull the shot wide; agents at their posts let it close
   * in. Runs once a frame on numbers the frame already has.
   */
  function followResidents(frames: AgentFrame[]): void {
    if (!projectFn || !clampZoomFn) return;
    let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity, n = 0;
    for (const f of frames) {
      if (f.agent.id >= VISITOR_ENT_BASE) continue;
      x0 = Math.min(x0, f.x);
      y0 = Math.min(y0, f.y);
      x1 = Math.max(x1, f.x);
      y1 = Math.max(y1, f.y);
      n++;
    }
    if (n === 0) return;
    const pad = 3.5;
    const corners = [
      projectFn(x0 - pad, y0 - pad),
      projectFn(x1 + pad, y0 - pad),
      projectFn(x0 - pad, y1 + pad),
      projectFn(x1 + pad, y1 + pad),
    ];
    const spanX = Math.max(...corners.map((c) => c.px)) - Math.min(...corners.map((c) => c.px));
    const spanY = Math.max(...corners.map((c) => c.py)) - Math.min(...corners.map((c) => c.py));
    const zoom = clampZoomFn(Math.min(camera.width / spanX, camera.height / spanY, 2.5));
    const ease = 0.04;
    camera.x += ((x0 + x1) / 2 - camera.x) * ease;
    camera.y += ((y0 + y1) / 2 - camera.y) * ease;
    camera.zoom += (zoom - camera.zoom) * ease;
  }

  /** Put the camera on an agent and pin its name, as a click on it would. */
  export function focus(ent: number): void {
    const a = world.agents.get(ent);
    if (!a) return;
    const p = sample(a.track, performance.now());
    camera.x = p.x;
    camera.y = p.y;
    selected = ent;
  }

  /** A chat line over the head of whoever said it (entity id of the speaker). */
  export function say(ent: number, text: string): void {
    const a = world.agents.get(ent);
    if (!a) return;
    a.saying = text.length > 80 ? `${text.slice(0, 79)}…` : text;
    a.sayingUntilMs = performance.now() + 6000;
  }

  type Status = "loading" | "absent" | "creating" | "ready" | "error";

  let canvas = $state<HTMLCanvasElement | null>(null);
  let status = $state<Status>("loading");
  let errorCode = $state("");
  let tier = $state<Tier>(1);
  let fps = $state(0);
  let stats = $state<FrameStats>({ cpuMs: 0, drawCalls: 0, sprites: 0, chunksBaked: 0 });
  // Engine counters are for diagnosing, not for living in the house: off
  // until asked for, remembered on this device.
  let showDiag = $state((() => {
    try {
      return localStorage.getItem("omniget.world.diagnostics") === "1";
    } catch {
      return false;
    }
  })());
  function toggleDiag(): void {
    showDiag = !showDiag;
    try {
      localStorage.setItem("omniget.world.diagnostics", showDiag ? "1" : "0");
    } catch {
      // storage unavailable
    }
  }
  let paused = $state(false);
  let hovered = $state<number | null>(null);
  let selected = $state<number | null>(null);
  let picker = $state<SlotDef | null>(null);
  let sleepState = $state("active");
  let clockLine = $state("");
  let dark = $state(false);
  // Mirrors of the replica, refreshed once a frame: `WorldState` is a plain
  // class on purpose (it is applied ten times a second and must not be a
  // reactive proxy), so the panel reads these instead.
  let agentCount = $state(0);
  let tickShown = $state(0);

  const world = new WorldState();
  let renderer: Renderer | null = null;
  let loop: { start(): void; stop(): void; readonly running: boolean } | null = null;
  let session: WorldSession | null = null;
  let atlas: AtlasData | null = null;
  let map: MapDef | null = null;
  let chunkTiles = new Map<ChunkId, ChunkTile[]>();
  let detachInput: (() => void) | null = null;
  let stopSleepEvents: (() => void) | null = null;
  let camera: Camera = { x: 8, y: 8, zoom: 1, width: 960, height: 540 };
  let objectsDrawn = $state(0);
  /** Rebuilt every frame and read by the hit test, so a click uses what is on screen. */
  let pickables: Pickable[] = [];
  let lastObjectsVersion = -1;
  let lastDark = false;
  let agentsSentMs = 0;

  function isTier(v: unknown): v is Tier {
    return v === 0 || v === 1 || v === 2 || v === 3;
  }

  /** Plan §1.3: a pinned tier wins, a fresh measurement is reused, otherwise measure. */
  async function resolveTier(mod: typeof import("$lib/world/render")): Promise<Tier> {
    const settings = getSettings()?.world;
    if (isTier(settings?.tier_override)) return settings.tier_override;
    let appVersion = "";
    try {
      appVersion = await (await import("@tauri-apps/api/app")).getVersion();
    } catch {
      appVersion = "";
    }
    if (
      isTier(settings?.tier_measured) &&
      appVersion &&
      settings?.measured_app_version === appVersion
    ) {
      return settings.tier_measured;
    }
    const cal = await mod.calibrateDetailed(null);
    try {
      await updateSettings({
        world: {
          tier_measured: cal.tier,
          measured_median_ms: cal.medianMs,
          measured_app_version: appVersion || null,
        },
      });
    } catch {
      // A settings write that fails must not stop the world from opening.
    }
    return cal.tier;
  }

  /**
   * Diffs that came down the channel before the opening snapshot was applied.
   * The channel is live from the moment `world_open` is called, and its first
   * messages can overtake the command's own answer.
   */
  let early: WorldBlob[] | null = null;
  /** One resync in flight at a time: every refused diff would ask again. */
  let resyncAsked = false;

  function onBlob(blob: WorldBlob, nowMs = performance.now()): void {
    if (early && blob.kind === "diff" && !world.ready) {
      early.push(blob);
      return;
    }
    // A diff the replica is already past (it was queued behind a snapshot).
    if (blob.kind === "diff" && world.ready && blob.diff.to <= world.tick) return;
    const result =
      blob.kind === "snapshot"
        ? world.applySnapshot(blob.snapshot, nowMs)
        : world.applyDiff(blob.diff, nowMs);
    if (result.ok) {
      if (blob.kind === "snapshot") {
        resyncAsked = false;
        if (errorCode === STATE_ERR.DIFF_GAP || errorCode === STATE_ERR.MAP_MISMATCH) errorCode = "";
      }
      return;
    }
    if (world.needsResync && !resyncAsked) {
      // Not an error the owner has to read: the next blob is a snapshot and
      // the house carries on. It is shown only if that snapshot never comes.
      resyncAsked = true;
      const code = result.code ?? "";
      void session?.resync().catch(() => {
        errorCode = code;
        resyncAsked = false;
      });
      // A request that got lost on the way must not silence the next one.
      setTimeout(() => (resyncAsked = false), 3000);
    }
  }

  async function boot(): Promise<void> {
    if (!canvas) return;
    status = "loading";
    try {
      if (previewMap) {
        await bootRender();
        centreOnHouse();
        status = "ready";
        onready?.();
        return;
      }
      session = fakeBlobs ? createFakeSession(fakeBlobs) : visit ? createVisitSession(visit) : createTauriSession();
      if (!fakeBlobs && !inTauri()) {
        // Outside the app there is no tick thread: the house still draws, so
        // the route is useful in `pnpm dev`, but nobody lives in it.
        await bootRender();
        centreOnHouse();
        status = "ready";
        return;
      }
      if (!(await session.exists())) {
        // First visit: the house and its first residents are made without
        // asking. The "absent" card stays as the fallback when that fails.
        status = "creating";
        try {
          await session.create({ house: "casa-v1" });
        } catch {
          status = "absent";
          return;
        }
      }
      await bootRender();
      early = [];
      const opened = await session.open(
        (blob) => onBlob(blob),
        (code) => {
          errorCode = code;
        },
      );
      const queued = early;
      early = null;
      onBlob(opened.blob);
      for (const blob of queued) onBlob(blob);
      stopSleepEvents = session.onSleepState((s) => (sleepState = s));
      await session.setVisible(true);
      centreOnHouse();
      status = "ready";
      onready?.();
    } catch (e) {
      errorCode = visit ? String(e) : codecErrorCode(e);
      status = "error";
      if (visit) onvisitfailed?.(String(e));
    }
  }

  /** Renderer, atlas, map and the frame loop; no session involved. */
  async function bootRender(): Promise<void> {
    const mod = await import("$lib/world/render");
    visibleChunksOf = (cam) => mod.visibleChunks(cam);
    projectFn = mod.project;
    clampZoomFn = mod.clampZoom;
    const resolvedTier = await resolveTier(mod);
    tier = resolvedTier;
    const r = mod.createRenderer();
    const caps = await r.init(canvas!, { tier: resolvedTier });
    tier = caps.tier;
    const loaded = await loadWorldAtlas();
    const handle = r.loadAtlas(loaded.json, loaded.pages);
    atlas = mod.parseAtlas(loaded.json);
    void handle;
    try {
      map = await loadHouseMap(previewMap);
      chunkTiles = buildChunkTiles(map, atlas);
      for (const [id, tiles] of chunkTiles) r.bakeChunk(id, tiles);
    } catch (e) {
      // A missing house is a named error on screen, not a crash: the agents
      // still draw, which is exactly what tells the owner what is missing.
      errorCode = codecErrorCode(e);
    }
    const rect = canvas!.getBoundingClientRect();
    r.resize(rect.width, rect.height, window.devicePixelRatio || 1);
    camera.width = canvas!.width;
    camera.height = canvas!.height;
    renderer = r;
    loop = mod.createLoop(r, (dt) => buildScene(dt), {
      watchdog: !isTier(getSettings()?.world?.tier_override),
      onStats: (s, f) => {
        stats = s;
        fps = f;
        agentCount = world.agents.size;
        tickShown = world.tick;
        const nowMs = performance.now();
        if (onagents && nowMs - agentsSentMs >= 500) {
          agentsSentMs = nowMs;
          onagents(
            world
              .list()
              .filter((a) => a.id < VISITOR_ENT_BASE)
              .map((a): AgentRow => ({ id: a.id, name: a.name, activity: a.activity.name, energy: a.energy })),
          );
        }
        clockLine = world.ready ? gameClockLabel(world.tick) : "";
        onstats?.(s, f, tier);
        if (import.meta.env.DEV) {
          (window as unknown as Record<string, unknown>).__omnigetWorldStats = {
            rafTicks: mod.getRafTicks(),
            fps: f,
            cpuMs: s.cpuMs,
            drawCalls: s.drawCalls,
            sprites: s.sprites,
            agents: world.agents.size,
            applyMs: world.stats.lastApplyMs,
            maxApplyMs: world.stats.maxApplyMs,
            tick: world.tick,
          };
        }
      },
      onTierChanged: (next) => (tier = next),
    });
    // The renderer may swap the canvas on wake (lost GL context): input and
    // the probes follow it to the new element.
    detachInput = followCanvas(canvas!, (el) => attachInput(
      el,
      { camera, pickables: () => pickables, dpr: () => (canvas!.width || 1) / (canvas!.getBoundingClientRect().width || 1) },
      {
        onTile: (tile) => {
          if (previewMap || !world.ready) return;
          if (visit) {
            // A guest walks; the furniture is the host's.
            // The host decides which entity is this guest; `ent` is ignored there.
            void send({ type: "move", ent: 0, to: [tile.x, tile.y] });
            return;
          }
          const slot = map ? slotAt(map, tile.x, tile.y) : null;
          if (slot) {
            picker = slot;
            return;
          }
          picker = null;
          if (selected !== null) {
            void send({ type: "move", ent: selected, to: [tile.x, tile.y] });
          }
        },
        onAgent: (id) => {
          selected = selected === id ? null : id;
          picker = null;
        },
        onHover: (id) => (hovered = id),
        onEscape: () => (picker = null),
      },
    ), (next) => (canvas = next));
    loop.start();
  }

  function centreOnHouse(): void {
    const spawn = map ? (previewMap ? marker(map, "yard-view") : null) ?? marker(map, "spawn") : null;
    if (spawn) {
      camera.x = spawn.x + 0.5;
      camera.y = spawn.y + 0.5;
      if (previewMap && clampZoomFn) {
        frameYard();
      }
      return;
    }
    const first = world.list()[0];
    if (first) {
      camera.x = first.track.toX;
      camera.y = first.track.toY;
    }
  }

  /** Include tall farm props when fitting the preview, so roofs are not cut off. */
  function frameYard(): void {
    if (!map || !atlas || !projectFn || !clampZoomFn) return;
    const entrance = marker(map, "house-door");
    if (!entrance) return;
    let left = Infinity, top = Infinity, right = -Infinity, bottom = -Infinity;
    const include = (x: number, y: number, w: number, h: number) => {
      left = Math.min(left, x); top = Math.min(top, y);
      right = Math.max(right, x + w); bottom = Math.max(bottom, y + h);
    };
    for (const chunk of map.chunks) {
      for (let i = 0; i < chunk.floor.length; i++) {
        const y = chunk.cy * 16 + Math.floor(i / 16);
        if (y < entrance.y || chunk.floor[i] === 255) continue;
        const point = projectFn(chunk.cx * 16 + i % 16, y);
        include(point.px - 32, point.py - 32, 64, 32);
      }
    }
    for (const object of map.objects) {
      if (object.tile[1] < entrance.y) continue;
      const key = objectFrame(atlas, object.kind);
      const frame = key ? atlas.frames.get(key) : null;
      if (!frame) continue;
      const point = projectFn(object.tile[0] + 0.5, object.tile[1] + 0.5);
      include(point.px - frame.pivotX, point.py - frame.pivotY, frame.w, frame.h);
    }
    if (!Number.isFinite(left)) return;
    const px = (left + right) / 2, py = (top + bottom) / 2;
    camera.x = (px / 32 + py / 16) / 2;
    camera.y = (py / 16 - px / 32) / 2;
    camera.zoom = clampZoomFn(Math.min(camera.width / (right - left + 100), camera.height / (bottom - top + 100)));
  }

  /** One frame: interpolate, cap by tier, add the HUD. */
  function buildScene(_dt: number): Scene {
    const now = performance.now();
    camera.width = canvas?.width ?? camera.width;
    camera.height = canvas?.height ?? camera.height;

    const frames: AgentFrame[] = [];
    pickables = [];
    for (const agent of world.list()) {
      const p = sample(agent.track, now);
      frames.push({ agent, x: p.x, y: p.y, z: p.z });
      pickables.push({ id: agent.id, x: p.x, y: p.y });
    }
    if (followAll) followResidents(frames);
    const shown = capAgents(frames, tier, camera);

    // The house goes dark only when every account is spent (plan §9.1): 26 of
    // 255 is the 10% line.
    dark = world.agents.size > 0 && world.highestEnergy() < 26;
    if (dark !== lastDark) {
      lastDark = dark;
      retintChunks(dark);
    }

    const sprites: SpriteInst[] = [];
    objectsDrawn = 0;
    if (atlas) {
      // Map objects provide a truthful scenery preview before any simulation
      // exists. Once a snapshot arrives it is the sole furniture authority.
      const objects = world.ready ? world.objects.values() : (map?.objects ?? []).map((o) => ({ ...o, tx: o.tile[0], ty: o.tile[1] }));
      for (const o of objects) {
        const frame = objectFrame(atlas, o.kind);
        if (!frame) continue;
        sprites.push({
          frame,
          x: o.tx + 0.5,
          y: o.ty + 0.5,
          z: 0,
          tint: dark ? 0x5a6070 : undefined,
        });
        objectsDrawn++;
      }
      for (const f of shown) {
        const sprite = agentSprite(atlas, f.agent.anim, f.agent.dir, now - f.agent.animStartedMs);
        sprites.push({
          frame: sprite.frame,
          x: f.x,
          y: f.y,
          z: f.z,
          flip: sprite.flip,
          tint: dark ? 0x7b8190 : undefined,
        });
      }
    }

    let texts: TextInst[] = [];
    if (renderer && atlas) {
      const hud = buildHud(world, shown, {
        tier,
        nowMs: now,
        camera,
        hovered,
        selected,
        dark,
        clockLine,
        scale: canvas ? camera.width / (canvas.clientWidth || camera.width) : 1,
        text: (str, style) => renderer!.text(str, style),
      });
      sprites.push(...hud.sprites);
      texts = hud.texts;
    }

    return { camera, chunksVisible: visibleChunkIds(), sprites, texts };
  }

  let visibleCache: ChunkId[] = [];
  let visibleChunksOf: ((cam: Camera) => ChunkId[]) | null = null;
  let projectFn: ((x: number, y: number, z?: number) => { px: number; py: number }) | null = null;
  let clampZoomFn: ((zoom: number) => number) | null = null;
  /** Chunks the camera touches that the map actually has. */
  function visibleChunkIds(): ChunkId[] {
    visibleCache.length = 0;
    if (!visibleChunksOf) {
      for (const id of chunkTiles.keys()) visibleCache.push(id);
      return visibleCache;
    }
    for (const id of visibleChunksOf(camera)) {
      if (chunkTiles.has(id)) visibleCache.push(id);
    }
    return visibleCache;
  }

  /** The house goes dark by re-baking its chunks once, not per frame. */
  function retintChunks(isDark: boolean): void {
    if (!renderer) return;
    for (const [id, tiles] of chunkTiles) {
      renderer.bakeChunk(
        id,
        isDark ? tiles.map((tile) => ({ ...tile, tint: 0x5a6070 })) : tiles,
      );
    }
  }

  async function send(input: WorldInput): Promise<void> {
    try {
      await session?.send(input);
    } catch (e) {
      errorCode = codecErrorCode(e);
    }
  }

  async function createWorld(): Promise<void> {
    status = "creating";
    try {
      await session?.create({ house: "casa-v1" });
      await boot();
    } catch (e) {
      errorCode = codecErrorCode(e);
      status = "error";
    }
  }

  function placeInSlot(slot: SlotDef, kind: string): void {
    const existing = [...world.objects.values()].find((o) => o.slot === slot.id);
    const id = existing?.id ?? nextObjectId();
    void send({
      type: "place_object",
      object: id,
      kind,
      tile: [slot.tile[0], slot.tile[1]],
      dir: slot.dir ?? 0,
      slot: slot.id,
    });
    picker = null;
  }

  function clearSlot(slot: SlotDef): void {
    const existing = [...world.objects.values()].find((o) => o.slot === slot.id);
    if (existing) void send({ type: "remove_object", object: existing.id });
    picker = null;
  }

  function nextObjectId(): number {
    let max = 0;
    for (const id of world.objects.keys()) if (id > max) max = id;
    return max + 1;
  }

  function recentre(): void {
    centreOnHouse();
  }

  /** The route asleep means zero callbacks and no GL context, not a slower loop. */
  function goToSleep(): void {
    if (paused) return;
    paused = true;
    loop?.stop();
    renderer?.sleep();
    void session?.setVisible(false);
  }

  async function wakeUp(): Promise<void> {
    if (!paused) return;
    paused = false;
    await renderer?.wake();
    await session?.setVisible(true);
    loop?.start();
  }

  onMount(() => {
    void boot();
    const onVisibility = () => {
      if (document.hidden) goToSleep();
      else void wakeUp();
    };
    document.addEventListener("visibilitychange", onVisibility);
    const onResize = () => {
      if (!canvas || !renderer) return;
      const rect = canvas.getBoundingClientRect();
      renderer.resize(rect.width, rect.height, window.devicePixelRatio || 1);
      camera.width = canvas.width;
      camera.height = canvas.height;
    };
    window.addEventListener("resize", onResize);
    return () => {
      document.removeEventListener("visibilitychange", onVisibility);
      window.removeEventListener("resize", onResize);
      detachInput?.();
      stopSleepEvents?.();
      loop?.stop();
      renderer?.destroy();
      renderer = null;
      loop = null;
      void session?.close().catch(() => {});
      session?.dispose();
      session = null;
      if (import.meta.env.DEV) {
        delete (window as unknown as Record<string, unknown>).__omnigetWorldStats;
      }
    };
  });
</script>

<div class="stage">
  <!-- The canvas takes focus so the arrow keys pan without a DOM overlay. -->
  <canvas
    bind:this={canvas}
    class="canvas"
    width="960"
    height="540"
    tabindex="0"
    aria-label={$t("world.canvas_label") as string}
  ></canvas>

  {#if status === "ready"}
    {#if !previewMap && showDiag}
    <div class="overlay" aria-live="off">
      <span>{$t("world.stats.tier")}: {tier}</span>
      <span>{$t("world.stats.fps")}: {fps.toFixed(0)}</span>
      <span>{$t("world.stats.cpu")}: {stats.cpuMs.toFixed(2)} ms</span>
      <span>{$t("world.stats.draws")}: {stats.drawCalls}</span>
      <span>{$t("world.hud.agents")}: {agentCount}</span>
      <span>{$t("world.hud.objects")}: {objectsDrawn}</span>
      <span>{$t("world.hud.tick")}: {tickShown}</span>
      <span>{$t("world.hud.sleep_state")}: {sleepState}</span>
    </div>
    {/if}

    <div class="actions">
      <button class="btn" onclick={recentre}>{$t("world.actions.recentre")}</button>
      {#if !previewMap}
        <button class="btn" aria-pressed={showDiag} onclick={toggleDiag}>{$t("world.city.diagnostics")}</button>
      {/if}
      {#if !previewMap}
        <button class="btn" onclick={() => (paused ? wakeUp() : goToSleep())}>
          {paused ? $t("world.actions.resume") : $t("world.actions.pause")}
        </button>
      {/if}
    </div>
  {/if}

  {#if picker}
    <div class="picker" role="dialog" aria-label={$t("world.slot.title") as string}>
      <p class="picker-title">{picker.id}</p>
      {#each picker.accepts as kind (kind)}
        <button class="picker-item" onclick={() => placeInSlot(picker!, kind)}>{kind}</button>
      {/each}
      <button class="picker-item picker-clear" onclick={() => clearSlot(picker!)}>
        {$t("world.slot.clear")}
      </button>
      <button class="picker-item" onclick={() => (picker = null)}>{$t("world.slot.close")}</button>
    </div>
  {/if}

  {#if status === "loading"}
    <p class="note">{$t("world.loading")}</p>
  {:else if status === "creating"}
    <p class="note">{$t("world.creating")}</p>
  {:else if status === "absent"}
    <div class="note absent">
      <p>{$t("world.create.desc")}</p>
      <button class="btn" onclick={createWorld}>{$t("world.create.button")}</button>
    </div>
  {:else if status === "error"}
    <p class="note error">{$t("world.error")} {errorCode}</p>
  {:else if errorCode}
    <p class="note warn">{errorCode}</p>
  {/if}
</div>

<style>
  .stage {
    position: relative;
    width: 100%;
    max-width: 1280px;
    aspect-ratio: 16 / 9;
    border-radius: 12px;
    overflow: hidden;
    background: #10131a;
  }
  .canvas {
    width: 100%;
    height: 100%;
    display: block;
    touch-action: none;
    outline: none;
  }
  .overlay {
    position: absolute;
    top: 8px;
    left: 8px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 8px;
    border-radius: 8px;
    background: rgba(0, 0, 0, 0.5);
    color: #e8eaf0;
    font-family: ui-monospace, monospace;
    font-size: 11px;
    pointer-events: none;
  }
  .actions {
    position: absolute;
    right: 8px;
    top: 8px;
    display: flex;
    gap: 6px;
  }
  .picker {
    position: absolute;
    left: 50%;
    bottom: 16px;
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 200px;
    padding: 8px;
    border-radius: 10px;
    background: var(--bg-elevated, #1b1f28);
    border: 1px solid var(--border-color, rgba(255, 255, 255, 0.12));
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.35);
  }
  .picker-title {
    margin: 0 0 4px;
    font-size: 12px;
    opacity: 0.7;
  }
  .picker-item {
    text-align: left;
    padding: 6px 8px;
    border-radius: 6px;
    border: none;
    background: transparent;
    color: inherit;
    font-size: 13px;
    cursor: pointer;
  }
  .picker-item:hover {
    background: rgba(255, 255, 255, 0.08);
  }
  .picker-clear {
    opacity: 0.75;
  }
  .note {
    position: absolute;
    bottom: 8px;
    left: 8px;
    margin: 0;
    color: #e8eaf0;
    font-size: 12px;
  }
  .absent {
    left: 50%;
    bottom: 50%;
    transform: translate(-50%, 50%);
    text-align: center;
    display: flex;
    flex-direction: column;
    gap: 8px;
    align-items: center;
  }
  .error {
    color: #ff8f7a;
  }
  .warn {
    color: #ffc46b;
  }
</style>
