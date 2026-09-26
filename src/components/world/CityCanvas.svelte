<script lang="ts">
  /**
   * The city on screen: one canvas, the region the avatar is in as a live
   * replica, the neighbouring exterior blocks as static scenery, and the
   * server as the only authority. Where `WorldCanvas` owns a house,
   * this only looks at one.
   *
   * Regions arrive from `/api/world/regions/{id}/map` and are baked into the
   * renderer's chunk cache; exterior blocks share one coordinate space, so
   * walking from one to the next shows no seam. An interior lives at its own
   * origin and overlaps the exterior on paper, so entering a home swaps the
   * drawable chunk set and rebakes it.
   */
  import { onMount, untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { codecErrorCode } from "$lib/world/codec";
  import { agentSprite, buildChunkTiles, loadWorldAtlas, objectFrame, CRAFT_ATLAS_URL, VALE_ATLAS_URL, type MapDef } from "$lib/world/assets";
  import { buildHud, capAgents, gameClockLabel, type AgentFrame } from "$lib/world/hud";
  import { attachInput, followCanvas, type Pickable } from "$lib/world/input";
  import { sample } from "$lib/world/interp";
  import { STATE_ERR, WorldState } from "$lib/world/state";
  import { createCitySession, fetchRegionMap, type CityControl, type CityFrame, type CityInput, type CityReady, type CitySession } from "$lib/world/city";
  import { effective, footprintOf, runtimeKind, type Editor, type Placement, type Published, type Space } from "$lib/world/editor";
  import { PICTURE_ASSET, PAGE_SIZE, PictureStore, composePicturePage, decodePicture, frameForKind, pictureHash, type PictureFetch } from "$lib/world/pictures";
  import { worldToScreen } from "$lib/world/render/camera";
  import type { AtlasData } from "$lib/world/render/atlas";
  import { type Camera, type ChunkId, type ChunkTile, type FrameStats, type Renderer, type Scene, type SpriteInst, type TextInst, type Tier } from "$lib/world/render/types";

  interface Props {
    city: string;
    server?: string | null;
    /** What a click on an empty bed plants. */
    crop?: string;
    /** A farm action was acknowledged (or refused, with the code). */
    onfarm?: (result: { ok: boolean; code: string; action: string }) => void;
    onready?: (ready: CityReady) => void;
    onfailed?: (error: string) => void;
    onstate?: (state: { region: string; ent: number; tick: number; interior: boolean }) => void;
    onstats?: (stats: FrameStats, fps: number, tier: Tier, backend: string) => void;
    /** Edit mode: the draft being edited, or null. Clicks then place, select and move. */
    editor?: Editor | null;
    /** The catalogue asset a click places, or null to select/move. */
    tool?: string | null;
    published?: Published | null;
    /** The plot rectangle (outside); inside bounds come from the room. */
    plotRect?: [number, number, number, number] | null;
    /** The draft changed (place/move/rotate/remove/undo/redo). */
    onedit?: () => void;
    onselect?: (id: string | null) => void;
    /** The canvas wants the tool cleared (Escape). */
    oncleartool?: () => void;
    /** The image a `picture/frame` placed now shows (hash), chosen in the panel. */
    picture?: string | null;
    /** A finish the draft previews on a house: tile of the house and the colour. */
    finishPreview?: { house: [number, number]; tint: number } | null;
    /** Connection state for the panel (online, reconnecting). */
    onconnection?: (state: "online" | "reconnecting") => void;
    /** A figure was clicked: the panel shows what it is doing (inspector). */
    onpick?: (ent: number | null) => void;
  }
  let { city, server = null, crop = "carrot", onready, onfailed, onstate, onstats, onfarm, editor = null, tool = null, published = null, plotRect = null, onedit, onselect, oncleartool, picture = null, finishPreview = null, onconnection, onpick }: Props = $props();

  /** Re-read the house finishes (after publishing one). */
  export function refreshFinishes(): void {
    void loadFinishes();
  }

  /** A chat line over someone's head. */
  export function say(ent: number, text: string): void {
    const a = world.agents.get(ent);
    if (!a) return;
    a.saying = text.length > 80 ? `${text.slice(0, 79)}…` : text;
    a.sayingUntilMs = performance.now() + 6000;
  }

  /** Walk to my front door and go in (the panel's "go home"). */
  export async function goTo(tile: [number, number]): Promise<void> {
    await send({ type: "move", to: tile });
  }

  let canvas = $state<HTMLCanvasElement | null>(null);
  let status = $state<"loading" | "ready" | "error">("loading");
  let errorCode = $state("");
  let fps = $state(0);
  let tier = $state<Tier>(1);
  let region = $state("");
  let ent = $state(0);
  let connection = $state<"online" | "reconnecting">("online");
  $effect(() => {
    const c = connection;
    // The parent's handler reads its own state; untracked so it cannot loop.
    untrack(() => onconnection?.(c));
  });

  // --- house finishes -----------------------------------------------------
  /** Tile of a house ("x,y") → the finish colour its owner published. */
  let houseTints = new Map<string, number>();
  let finishTimer: ReturnType<typeof setInterval> | null = null;
  async function loadFinishes(): Promise<void> {
    if (!session) return;
    try {
      const r = await session.api<{ plots: Array<{ house: [number, number]; house_tint?: string | null }> }>("GET", `/api/world/cities/${encodeURIComponent(city)}/plots`);
      const next = new Map<string, number>();
      for (const p of r.plots) {
        const n = p.house_tint ? parseInt(p.house_tint.replace("#", ""), 16) : NaN;
        if (Number.isFinite(n) && n !== 0xffffff) next.set(`${p.house[0]},${p.house[1]}`, n);
      }
      houseTints = next;
    } catch {
      // Keep the colours we had; the next refresh tries again.
    }
  }
  function houseTint(kind: string, x: number, y: number): number | undefined {
    if (!kind.endsWith("/house")) return undefined;
    if (finishPreview && finishPreview.house[0] === x && finishPreview.house[1] === y) return finishPreview.tint === 0xffffff ? undefined : finishPreview.tint;
    return houseTints.get(`${x},${y}`);
  }

  // --- pictures -------------------------------------------------------------
  const pictures = new PictureStore({
    fetch: async (hash): Promise<PictureFetch> => {
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        const bytes = (await invoke("city_asset", { hash, server })) as ArrayBuffer;
        return { ok: true, bytes };
      } catch (e) {
        return { ok: false, code: String(e) };
      }
    },
    decode: decodePicture,
  });
  /** Hashes currently drawn on the renderer's dynamic page. */
  let onPage = new Set<string>();
  let pageVersion = -1;
  let composing = false;
  /** World object id → the hash its frame showed, to notice a swap to the placeholder. */
  const frameHashes = new Map<string, string>();
  async function recomposePictures(): Promise<void> {
    if (composing || !renderer) return;
    composing = true;
    const version = pictures.version;
    try {
      const page = typeof OffscreenCanvas !== "undefined" ? new OffscreenCanvas(PAGE_SIZE, PAGE_SIZE) : Object.assign(document.createElement("canvas"), { width: PAGE_SIZE, height: PAGE_SIZE });
      const c = (page as HTMLCanvasElement).getContext("2d") as CanvasRenderingContext2D | null;
      if (!c) return;
      const composed = composePicturePage(c, pictures);
      const bitmap = await createImageBitmap(page as unknown as CanvasImageSource);
      if (!renderer) {
        bitmap.close();
        return;
      }
      renderer.setDynamicPage(bitmap, composed.frames);
      pagePrev?.close();
      pagePrev = bitmap;
      onPage = composed.onPage;
      pageVersion = version;
    } catch (e) {
      errorCode = e instanceof Error ? e.message : String(e);
    } finally {
      composing = false;
    }
  }
  let pagePrev: ImageBitmap | null = null;
  /** Ask for the pictures on screen; notice frames the server just emptied. */
  function trackPictures(): void {
    const want = new Set<string>();
    const scan = (rid: string, st: WorldState) => {
      for (const o of st.objects.values()) {
        if (!o.kind.startsWith("picture/")) continue;
        const key = `${rid}:${o.id}`;
        const h = pictureHash(o.kind);
        if (h) {
          want.add(h);
          frameHashes.set(key, h);
        } else {
          const before = frameHashes.get(key);
          if (before) {
            pictures.drop(before, "gone");
            frameHashes.delete(key);
          }
        }
      }
    };
    scan(region, world);
    for (const [rid, st] of observed) scan(rid, st);
    if (editor) {
      for (const p of Object.values(editor.draft.placed)) if (p.picture) want.add(p.picture);
    }
    if (picture) want.add(picture);
    pictures.want(want);
    if (pictures.version !== pageVersion) void recomposePictures();
  }
  /** The atlas frame of a world object, pictures and house finishes included. */
  function objectSprite(kind: string, x: number, y: number): SpriteInst | null {
    if (!atlas) return null;
    const pic = frameForKind(kind, pictures, onPage);
    const frame = pic ?? objectFrame(atlas, kind);
    if (!frame) return null;
    return { frame, x: x + 0.5, y: y + 0.5, z: 0, tint: houseTint(kind, x, y) };
  }
  /** Frame of a catalogue asset as the editor draws it (a frame shows its picture). */
  function draftFrame(space: Space, asset: string, pictureHashOf?: string | null): string | null {
    if (!atlas) return null;
    if (asset === PICTURE_ASSET) return frameForKind(pictureHashOf ? `picture/${pictureHashOf}` : "picture/removed", pictures, onPage);
    return objectFrame(atlas, runtimeKind(space, asset));
  }

  /** The avatar's region: inputs, HUD and the clock come from here. */
  let world = new WorldState();
  /** Neighbouring blocks the server lets us watch: agents drawn, no input. */
  const observed = new Map<string, WorldState>();
  let renderer: Renderer | null = null;
  let loop: { start(): void; stop(): void } | null = null;
  let session: CitySession | null = null;
  let atlas: AtlasData | null = null;
  let camera: Camera = { x: 0, y: 0, zoom: 1, width: 960, height: 540 };
  let detachInput: (() => void) | null = null;
  let stops: Array<() => void> = [];
  let pickables: Pickable[] = [];
  let hovered = $state<number | null>(null);
  let selected = $state<number | null>(null);
  let clockLine = "";
  let projectFn: ((x: number, y: number, z?: number) => { px: number; py: number }) | null = null;
  let visibleChunksOf: ((cam: Camera) => ChunkId[]) | null = null;

  /** Region id → its map; `chunks` is the baked tile list per chunk id. */
  const maps = new Map<string, MapDef>();
  const regionChunks = new Map<string, Map<ChunkId, ChunkTile[]>>();
  /** Chunk ids the renderer may draw right now (exterior union, or one interior). */
  let drawable = new Set<ChunkId>();
  let interior = $state(false);
  let exteriorRegions: string[] = [];
  let lastInterestMs = 0;
  let lastInterestKey = "";

  function isInterior(id: string): boolean {
    return id.includes("/home:");
  }

  async function loadRegion(id: string): Promise<MapDef | null> {
    const cached = maps.get(id);
    if (cached) return cached;
    if (!session || !atlas) return null;
    try {
      const res = await fetchRegionMap(session, id);
      const map = res.map as MapDef;
      maps.set(id, map);
      regionChunks.set(id, buildChunkTiles(map, atlas));
      return map;
    } catch (e) {
      errorCode = codecErrorCode(e);
      return null;
    }
  }

  /** Make the set of chunks for the current mode and bake it. */
  function rebake(): void {
    if (!renderer) return;
    const next = new Set<ChunkId>();
    const ids = interior ? [region] : exteriorRegions.length ? exteriorRegions : [region];
    for (const rid of ids) {
      const chunks = regionChunks.get(rid);
      if (!chunks) continue;
      for (const [cid, tiles] of chunks) {
        renderer.bakeChunk(cid, tiles);
        next.add(cid);
      }
    }
    drawable = next;
  }

  async function enterRegion(id: string, at: [number, number]): Promise<void> {
    region = id;
    interior = isInterior(id);
    world = new WorldState();
    observed.clear();
    camera.x = at[0] + 0.5;
    camera.y = at[1] + 0.5;
    await loadRegion(id);
    if (!interior && exteriorRegions.length === 0 && session) {
      // The other blocks of the city, once: scenery for the horizon.
      try {
        const info = await session.api<{ regions: Array<{ id: string; kind: string }> }>("GET", `/api/world/cities/${encodeURIComponent(city)}`);
        exteriorRegions = info.regions.filter((r) => r.kind === "exterior").map((r) => r.id);
        await Promise.all(exteriorRegions.map((rid) => loadRegion(rid)));
      } catch {
        exteriorRegions = [id];
      }
    }
    rebake();
    onstate?.({ region, ent, tick: world.tick, interior });
  }

  function onFrame(frame: CityFrame): void {
    const nowMs = performance.now();
    if (frame.region !== region) {
      // A neighbouring block, or a straggler from a region we left. Only a
      // snapshot opens a replica; a diff for an unknown region is dropped.
      let state = observed.get(frame.region);
      if (!state) {
        if (frame.blob.kind !== "snapshot" || interior) return;
        state = new WorldState();
        observed.set(frame.region, state);
      }
      const r = frame.blob.kind === "snapshot" ? state.applySnapshot(frame.blob.snapshot, nowMs) : state.applyDiff(frame.blob.diff, nowMs);
      if (!r.ok && state.needsResync) observed.delete(frame.region); // the next snapshot reopens it
      return;
    }
    const result = frame.blob.kind === "snapshot" ? world.applySnapshot(frame.blob.snapshot, nowMs) : world.applyDiff(frame.blob.diff, nowMs);
    if (result.ok) {
      if (frame.blob.kind === "snapshot" && (errorCode === STATE_ERR.DIFF_GAP || errorCode === STATE_ERR.MAP_MISMATCH)) errorCode = "";
      return;
    }
    if (world.needsResync) void session?.resync().catch(() => (errorCode = result.code ?? ""));
  }

  function onControl(msg: CityControl): void {
    switch (msg.op) {
      case "region": {
        const m = msg as { region: string; at: [number, number] };
        void enterRegion(m.region, m.at);
        break;
      }
      case "reconnected": {
        const m = msg as CityReady;
        connection = "online";
        ent = m.ent;
        void enterRegion(m.region, m.spawn);
        // Something may have been removed or re-published while we were away.
        pictures.revalidate();
        void loadFinishes();
        break;
      }
      case "reconnecting":
        connection = "reconnecting";
        break;
      case "error": {
        const m = msg as { code: string; message: string };
        errorCode = m.code;
        status = "error";
        onfailed?.(`${m.code}: ${m.message}`);
        break;
      }
      default:
        break;
    }
  }

  async function send(input: CityInput): Promise<void> {
    try {
      await session?.send(input);
    } catch (e) {
      errorCode = codecErrorCode(e);
    }
  }

  /** The crop bed drawn on a tile, if any: its kind decides the action. */
  function bedAt(x: number, y: number): { kind: string } | null {
    for (const o of world.objects.values()) {
      if (o.tx === x && o.ty === y && (o.kind.startsWith("crop/") || o.kind === "prop/soil-empty")) return { kind: o.kind };
    }
    return null;
  }

  /** Water, harvest, clear or plant, by what the bed shows. */
  async function farm(x: number, y: number, kind: string): Promise<void> {
    const stage = kind.split("/")[2] ?? "";
    const action = kind === "prop/soil-empty" ? "plant" : stage === "ripe" ? "harvest" : stage === "withered" ? "clear" : "water";
    const seq = await session?.send({ type: "farm", tile: [x, y], action, crop: action === "plant" ? crop : undefined });
    if (seq !== undefined) pendingFarm.set(seq, action);
  }
  const pendingFarm = new Map<number, string>();

  // --- edit mode ---------------------------------------------------------
  let hoverTile: [number, number] | null = null;
  let selectedObj = $state<string | null>(null);

  function editSpace(): Space {
    return interior ? "inside" : "outside";
  }
  /** Draft coordinates of a tile: interior placements travel in room coordinates. */
  function toDraft(x: number, y: number): [number, number] {
    return interior ? [x - 1, y - 1] : [x, y];
  }
  function toTile(p: Placement): [number, number] {
    return p.space === "inside" ? [p.xy[0] + 1, p.xy[1] + 1] : [p.xy[0], p.xy[1]];
  }
  /** The rectangle a draft may use in the current space, in tiles. */
  function editBounds(): [number, number, number, number] | null {
    if (!interior) return plotRect;
    const room = maps.get(region)?.rooms?.[0]?.rect;
    return room ? [room[0], room[1], room[2], room[3]] : null;
  }
  function inBounds(x: number, y: number, asset: string): boolean {
    const b = editBounds();
    if (!b) return false;
    const [fw, fh] = footprintOf(asset);
    return x >= b[0] && y >= b[1] && x + fw <= b[0] + b[2] && y + fh <= b[1] + b[3];
  }
  /** Object ids the draft removed: those world objects are not drawn. */
  function hiddenObjectIds(): Set<number> {
    const out = new Set<number>();
    if (!editor || !published) return out;
    for (const p of published.objects) {
      if (editor.draft.removed.includes(p.id) && p.object_id !== undefined) out.add(p.object_id);
    }
    return out;
  }
  /** The draft or published placement on a tile of the current space. */
  function placementAt(x: number, y: number): { placement: Placement; pending: boolean } | null {
    if (!editor) return null;
    const space = editSpace();
    for (const e of effective(published?.objects ?? [], editor.draft)) {
      if (e.placement.space !== space) continue;
      const [tx, ty] = toTile(e.placement);
      const [fw, fh] = footprintOf(e.placement.asset);
      if (x >= tx && x < tx + fw && y >= ty && y < ty + fh) return e;
    }
    return null;
  }
  /** Tiles the static maps and the live objects already use, for the preview. */
  function staticBlocked(x: number, y: number): boolean {
    for (const map of maps.values()) {
      for (const o of map.objects) {
        const fp = map.palette.find((t) => t.key === o.kind)?.footprint ?? [1, 1];
        if (x >= o.tile[0] && x < o.tile[0] + Math.max(1, fp[0]) && y >= o.tile[1] && y < o.tile[1] + Math.max(1, fp[1])) return true;
      }
    }
    return false;
  }
  /**
   * Is a tile free of world objects (this region and the observed ones), the
   * static map objects and draft objects (ignoring `except`). The server
   * decides; this only keeps the preview honest.
   */
  function occupied(x: number, y: number, asset: string, except: string | null): boolean {
    const [fw, fh] = footprintOf(asset);
    const hidden = hiddenObjectIds();
    for (let dy = 0; dy < fh; dy++) {
      for (let dx = 0; dx < fw; dx++) {
        const tx = x + dx;
        const ty = y + dy;
        for (const o of world.objects.values()) {
          if (o.tx === tx && o.ty === ty && !hidden.has(o.id) && !isPublishedObject(o.id)) return true;
        }
        for (const st of observed.values()) {
          for (const o of st.objects.values()) {
            if (o.tx === tx && o.ty === ty && !hidden.has(o.id) && !isPublishedObject(o.id)) return true;
          }
        }
        if (staticBlocked(tx, ty)) return true;
        const p = placementAt(tx, ty);
        if (p && p.placement.id !== except) return true;
      }
    }
    return false;
  }
  function isPublishedObject(objectId: number): boolean {
    return published?.objects.some((p) => p.object_id === objectId) ?? false;
  }
  function editClick(x: number, y: number): void {
    if (!editor) return;
    const space = editSpace();
    if (tool) {
      if (!inBounds(x, y, tool) || occupied(x, y, tool, null)) return;
      if (tool === PICTURE_ASSET && !picture) return; // the panel asks for a picture first
      const id = editor.place(tool, toDraft(x, y), space, tool === PICTURE_ASSET ? picture : undefined);
      selectedObj = id;
      onselect?.(id);
      onedit?.();
      return;
    }
    const hit = placementAt(x, y);
    if (hit) {
      selectedObj = selectedObj === hit.placement.id ? null : hit.placement.id;
      onselect?.(selectedObj);
      return;
    }
    if (selectedObj) {
      moveSelected(x, y);
    }
  }
  /** Move the selection: a draft object moves; a published one is removed and re-placed. */
  function moveSelected(x: number, y: number): void {
    if (!editor || !selectedObj) return;
    const cur = effective(published?.objects ?? [], editor.draft).find((e) => e.placement.id === selectedObj);
    if (!cur) return;
    if (!inBounds(x, y, cur.placement.asset) || occupied(x, y, cur.placement.asset, selectedObj)) return;
    if (cur.pending) {
      editor.move(selectedObj, toDraft(x, y));
    } else {
      editor.remove(selectedObj, true);
      selectedObj = editor.place(cur.placement.asset, toDraft(x, y), cur.placement.space, cur.placement.picture);
      onselect?.(selectedObj);
    }
    onedit?.();
  }
  function editKey(ev: KeyboardEvent): boolean {
    if (!editor) return false;
    const mod = ev.metaKey || ev.ctrlKey;
    if (mod && ev.key.toLowerCase() === "z") {
      if (ev.shiftKey) editor.redo();
      else editor.undo();
      selectedObj = null;
      onedit?.();
      return true;
    }
    if (mod && ev.key.toLowerCase() === "y") {
      editor.redo();
      onedit?.();
      return true;
    }
    if (mod) return false;
    if ((ev.key === "Delete" || ev.key === "Backspace") && selectedObj) {
      const cur = effective(published?.objects ?? [], editor.draft).find((e) => e.placement.id === selectedObj);
      if (cur) editor.remove(selectedObj, !cur.pending);
      selectedObj = null;
      onselect?.(null);
      onedit?.();
      return true;
    }
    if (ev.key.toLowerCase() === "r" && selectedObj) {
      const cur = effective(published?.objects ?? [], editor.draft).find((e) => e.placement.id === selectedObj);
      if (!cur) return true;
      if (cur.pending) editor.rotate(selectedObj);
      else {
        editor.remove(selectedObj, true);
        const [tx, ty] = toTile(cur.placement);
        selectedObj = editor.place(cur.placement.asset, toDraft(tx, ty), cur.placement.space, cur.placement.picture);
        editor.rotate(selectedObj);
        onselect?.(selectedObj);
      }
      onedit?.();
      return true;
    }
    return false;
  }
  /** Draft objects and the ghost under the pointer, as sprites. */
  function editSprites(out: SpriteInst[]): void {
    if (!editor || !atlas) return;
    const space = editSpace();
    for (const e of effective(published?.objects ?? [], editor.draft)) {
      if (e.placement.space !== space || !e.pending) continue;
      const frame = draftFrame(space, e.placement.asset, e.placement.picture);
      if (!frame) continue;
      const [tx, ty] = toTile(e.placement);
      const sel = e.placement.id === selectedObj;
      out.push({ frame, x: tx + 0.5, y: ty + 0.5, z: 0, alpha: 0.85, flip: ((e.placement.dir ?? 0) & 1) === 1, tint: sel ? 0xffe680 : 0xd8ffd8 });
    }
    if (selectedObj) {
      const cur = effective(published?.objects ?? [], editor.draft).find((e) => e.placement.id === selectedObj);
      if (cur && !cur.pending) {
        const frame = draftFrame(space, cur.placement.asset, cur.placement.picture);
        const [tx, ty] = toTile(cur.placement);
        if (frame) out.push({ frame, x: tx + 0.5, y: ty + 0.5, z: 0.02, alpha: 0.5, tint: 0xffe680 });
      }
    }
    if (tool && hoverTile) {
      const frame = draftFrame(space, tool, picture);
      if (!frame) return;
      const [x, y] = hoverTile;
      const ok = inBounds(x, y, tool) && !occupied(x, y, tool, null);
      out.push({ frame, x: x + 0.5, y: y + 0.5, z: 0.01, alpha: 0.55, tint: ok ? 0xb0ffb0 : 0xff8080 });
    }
  }
  $effect(() => {
    if (!editor) {
      selectedObj = null;
      hoverTile = null;
    }
  });

  /** The door marker on a tile of the current region, if any. */
  function portalAt(x: number, y: number): string | null {
    const map = maps.get(region);
    if (!map) return null;
    const m = map.markers?.find((k) => k.tile[0] === x && k.tile[1] === y && (k.name.startsWith("house-door:") || k.name === "front-door"));
    return m ? m.name : null;
  }

  function pushInterest(): void {
    const now = performance.now();
    const key = `${Math.round(camera.x)}:${Math.round(camera.y)}`;
    if (key === lastInterestKey || now - lastInterestMs < 400) return;
    lastInterestKey = key;
    lastInterestMs = now;
    void session?.setInterest(camera.x, camera.y, 40).catch(() => {});
  }

  async function boot(): Promise<void> {
    if (!canvas) return;
    status = "loading";
    try {
      await bootRender();
      session = createCitySession({ city, server });
      stops.push(session.onAck((ack) => {
        const action = pendingFarm.get(ack.seq);
        if (!action) return;
        pendingFarm.delete(ack.seq);
        onfarm?.({ ok: ack.status === 0, code: ack.code, action });
      }));
      stops.push(session.onClosed((reason) => {
        if (reason !== "left") {
          errorCode = `ERR_CITY_${reason.toUpperCase()}`;
          status = "error";
          onfailed?.(errorCode);
        }
      }));
      const ready = await session.open(onFrame, onControl, (code) => (errorCode = code));
      ent = ready.ent;
      await enterRegion(ready.region, ready.spawn);
      await loadFinishes();
      finishTimer = setInterval(() => void loadFinishes(), 30_000);
      status = "ready";
      onready?.(ready);
    } catch (e) {
      errorCode = String(e);
      status = "error";
      onfailed?.(String(e));
    }
  }

  async function bootRender(): Promise<void> {
    const mod = await import("$lib/world/render");
    visibleChunksOf = (cam) => mod.visibleChunks(cam);
    projectFn = mod.project;
    const override = getSettings()?.world?.tier_override;
    const measured = getSettings()?.world?.tier_measured;
    const pick = (v: unknown): Tier | null => (v === 0 || v === 1 || v === 2 || v === 3 ? v : null);
    tier = pick(override) ?? pick(measured) ?? 1;
    const r = mod.createRenderer();
    const backend = getSettings()?.world?.backend_override ?? undefined;
    const caps = await r.init(canvas!, backend ? { tier, backend } : { tier });
    tier = caps.tier;
    // The house furniture (craft-v1) and the town (vale-v1) in one atlas.
    const loaded = await loadWorldAtlas(CRAFT_ATLAS_URL, [VALE_ATLAS_URL]);
    r.loadAtlas(loaded.json, loaded.pages);
    atlas = mod.parseAtlas(loaded.json);
    const rect = canvas!.getBoundingClientRect();
    r.resize(rect.width, rect.height, window.devicePixelRatio || 1);
    camera.width = canvas!.width;
    camera.height = canvas!.height;
    renderer = r;
    loop = mod.createLoop(r, (dt) => buildScene(dt), {
      watchdog: pick(override) === null,
      onStats: (s, f) => {
        fps = f;
        clockLine = world.ready ? gameClockLabel(world.tick) : "";
        onstats?.(s, f, tier, caps.backend);
        pushInterest();
        if (import.meta.env.DEV) {
          // A dev probe: `window.__omnigetCityProbe = {sx, sy}` (canvas pixels)
          // makes the next frame report the pixel there, read in the same
          // task as the draw so a GL back buffer is still intact.
          const w = window as unknown as Record<string, unknown>;
          const probe = w.__omnigetCityProbe as { sx: number; sy: number } | undefined;
          let pixel: number[] | null = null;
          if (probe && canvas) {
            const gl = (canvas.getContext("webgl2") ?? canvas.getContext("webgl")) as WebGLRenderingContext | null;
            if (gl && tier > 0) {
              const buf = new Uint8Array(4);
              gl.readPixels(Math.round(probe.sx), canvas.height - 1 - Math.round(probe.sy), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, buf);
              pixel = [...buf];
            } else {
              const c2 = canvas.getContext("2d");
              if (c2) pixel = [...c2.getImageData(Math.round(probe.sx), Math.round(probe.sy), 1, 1).data];
            }
          }
          w.__omnigetCityStats = {
            pixel,
            probeSeen: !!probe,
            hasCanvas: !!canvas,
            tier,
            backend: caps.backend,
            project: projectFn ? (x: number, y: number) => projectFn!(x, y, 0) : null,
            camera: { x: camera.x, y: camera.y, zoom: camera.zoom, width: camera.width, height: camera.height },
            fps: f,
            cpuMs: s.cpuMs,
            drawCalls: s.drawCalls,
            sprites: s.sprites,
            region,
            interior,
            ent,
            tick: world.tick,
            agents: world.agents.size,
            objects: world.objects.size,
            objectList: [...world.objects.values()].map((o) => [o.id, o.kind, o.tx, o.ty]),
            observedObjects: [...observed.entries()].flatMap(([rid, st]) => [...st.objects.values()].map((o) => [rid, o.id, o.kind, o.tx, o.ty])),
            screenOf: (x: number, y: number) => worldToScreen(camera, x, y, 0),
            lookAt: (x: number, y: number) => {
              camera.x = x + 0.5;
              camera.y = y + 0.5;
            },
            observed: [...observed.keys()],
            observedAgents: [...observed.values()].reduce((n, st) => n + st.agents.size, 0),
            drawable: drawable.size,
            status,
            errorCode,
            connection,
            memory: renderer?.memory() ?? null,
            // Figures on screen, for a driver to click one: [ent, name, x, y].
            figures: [...world.agents.values()].map((a) => [a.id, a.name, pickables.find((p) => p.id === a.id)?.x ?? null, pickables.find((p) => p.id === a.id)?.y ?? null]),
            pictures: [...pictures.entries.values()].map((e) => [e.hash.slice(0, 12), e.state]),
            onPage: [...onPage].map((h) => h.slice(0, 12)),
            pageVersion,
            houseTints: [...houseTints.entries()],
            // Context loss the way a GPU reset does it, then the browser's restore.
            loseContext: () => {
              const gl = (canvas?.getContext("webgl2") ?? canvas?.getContext("webgl")) as WebGLRenderingContext | null;
              const ext = gl?.getExtension("WEBGL_lose_context");
              ext?.loseContext();
              setTimeout(() => ext?.restoreContext(), 200);
              return !!ext;
            },
            sleepWake: async () => {
              renderer?.sleep();
              await renderer?.wake();
              return renderer?.memory() ?? null;
            },
          };
        }
      },
      onTierChanged: (next) => (tier = next),
    });
    bindInput(canvas!);
    loop.start();
  }

  /**
   * Input on the canvas element. A GL wake that had to swap the element for a
   * new one (macOS, occluded window) announces it; everything moves over.
   */
  function bindInput(first: HTMLCanvasElement): void {
    detachInput?.();
    detachInput = followCanvas(first, (el) => attachInput(
      el,
      { camera, pickables: () => pickables, dpr: () => (canvas!.width || 1) / (canvas!.getBoundingClientRect().width || 1) },
      {
        onTile: (tile) => {
          if (!world.ready) return;
          if (editor) {
            editClick(tile.x, tile.y);
            return;
          }
          const portal = portalAt(tile.x, tile.y);
          if (portal) {
            void send({ type: "enter", portal });
            return;
          }
          const bed = bedAt(tile.x, tile.y);
          if (bed) {
            void farm(tile.x, tile.y, bed.kind);
            return;
          }
          void send({ type: "move", to: [tile.x, tile.y] });
        },
        onAgent: (id) => {
          selected = selected === id ? null : id;
          onpick?.(selected !== null && selected !== ent ? selected : null);
          if (id !== ent) void send({ type: "wave", ent: id });
        },
        onHover: (id) => (hovered = id),
        onHoverTile: (tile) => (hoverTile = [tile.x, tile.y]),
        onKey: (ev) => editKey(ev),
        onEscape: () => {
          selected = null;
          if (editor) {
            if (tool) oncleartool?.();
            else {
              selectedObj = null;
              onselect?.(null);
            }
          }
        },
      },
    ), (next) => (canvas = next));
  }

  let visibleCache: ChunkId[] = [];
  function visibleChunkIds(): ChunkId[] {
    visibleCache.length = 0;
    if (!visibleChunksOf) return visibleCache;
    for (const id of visibleChunksOf(camera)) {
      if (drawable.has(id)) visibleCache.push(id);
    }
    return visibleCache;
  }

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
    // Neighbours: drawn and waved at, never the HUD's business.
    const farFrames: AgentFrame[] = [];
    for (const state of observed.values()) {
      for (const agent of state.list()) {
        const p = sample(agent.track, now);
        farFrames.push({ agent, x: p.x, y: p.y, z: p.z });
      }
    }
    const shown = capAgents(frames.concat(farFrames), tier, camera);
    const sprites: SpriteInst[] = [];
    if (atlas) {
      const hidden = editor ? hiddenObjectIds() : null;
      trackPictures();
      for (const o of world.objects.values()) {
        if (hidden?.has(o.id)) continue;
        const sp = objectSprite(o.kind, o.tx, o.ty);
        if (sp) sprites.push(sp);
      }
      for (const state of observed.values()) {
        for (const o of state.objects.values()) {
          const sp = objectSprite(o.kind, o.tx, o.ty);
          if (sp) sprites.push(sp);
        }
      }
      editSprites(sprites);
      for (const f of shown) {
        const sprite = agentSprite(atlas, f.agent.anim, f.agent.dir, now - f.agent.animStartedMs);
        sprites.push({ frame: sprite.frame, x: f.x, y: f.y, z: f.z, flip: sprite.flip, tint: f.agent.id === ent ? 0xfff2c8 : undefined });
      }
    }
    let texts: TextInst[] = [];
    if (renderer && atlas) {
      const hud = buildHud(world, shown, {
        tier,
        nowMs: now,
        camera,
        hovered,
        selected: selected ?? ent,
        dark: false,
        clockLine,
        scale: canvas ? camera.width / (canvas.clientWidth || camera.width) : 1,
        text: (str, style) => renderer!.text(str, style),
      });
      sprites.push(...hud.sprites);
      texts = hud.texts;
    }
    return { camera, chunksVisible: visibleChunkIds(), sprites, texts };
  }

  onMount(() => {
    void boot();
    const onResize = () => {
      if (!canvas || !renderer) return;
      const rect = canvas.getBoundingClientRect();
      renderer.resize(rect.width, rect.height, window.devicePixelRatio || 1);
    };
    window.addEventListener("resize", onResize);
    return () => {
      window.removeEventListener("resize", onResize);
      if (finishTimer) clearInterval(finishTimer);
      pictures.dispose();
      pagePrev?.close();
      detachInput?.();
      loop?.stop();
      for (const stop of stops.splice(0)) stop();
      void session?.close().catch(() => {});
      session?.dispose();
      renderer?.destroy();
    };
  });
  void projectFn;
</script>

<div class="stage">
  <canvas bind:this={canvas} class="canvas" class:editing={!!editor} width="960" height="540" tabindex="0" aria-label={$t("world.city.canvas_label") as string}></canvas>
  <div class="overlay">
    {#if status === "loading"}
      <span class="badge">{$t("world.city.entering")}</span>
    {:else if status === "error"}
      <span class="badge error">{$t("world.error")} {errorCode}</span>
    {:else}
      <span class="badge">{interior ? $t("world.city.inside_home") : $t("world.city.on_the_street")}</span>
      {#if connection === "reconnecting"}<span class="badge warn">{$t("world.city.reconnecting")}</span>{/if}
      {#if errorCode}<span class="badge warn">{errorCode}</span>{/if}
      {#if editor}<span class="badge edit">{tool ? $t("world.city.edit_placing", { asset: $t(`world.city.asset_${tool.split("/").pop() ?? tool}`) }) : selectedObj ? $t("world.city.edit_selected") : $t("world.city.edit_mode")}</span>{/if}
    {/if}
  </div>
</div>

<style>
  .stage {
    position: relative;
    width: 100%;
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
  }
  .overlay {
    position: absolute;
    left: 0.6rem;
    top: 0.6rem;
    display: flex;
    gap: 0.4rem;
    flex-wrap: wrap;
    pointer-events: none;
  }
  .badge {
    font-size: 0.75rem;
    padding: 0.2rem 0.55rem;
    border-radius: 999px;
    background: rgba(0, 0, 0, 0.55);
    color: #fff;
  }
  .badge.error {
    background: rgba(200, 40, 40, 0.8);
  }
  .badge.warn {
    background: rgba(200, 140, 20, 0.85);
  }
  .badge.edit {
    background: rgba(20, 120, 220, 0.85);
  }
  .canvas.editing {
    outline: 2px solid rgba(20, 120, 220, 0.6);
    outline-offset: -2px;
    cursor: crosshair;
  }
</style>
