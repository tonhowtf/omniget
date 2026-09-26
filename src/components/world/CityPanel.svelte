<script lang="ts">
  // Enter the city, claim a plot, talk to whoever is near. Nothing here talks
  // to the network until a button is pressed.
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import type { CityChat } from "$lib/world/city";
  import { Editor, HOUSE_FINISHES, finishTint, isEmpty, publishBody, type AssetUse, type EditorStatus, type HouseFinish, type Placement, type Published } from "$lib/world/editor";
  import { PICTURE_ASSET } from "$lib/world/pictures";

  /** What the canvas needs while a draft is open. */
  export interface EditorHandoff {
    editor: Editor;
    tool: string | null;
    published: Published | null;
    plotRect: [number, number, number, number] | null;
    /** The image a frame placed now shows. */
    picture: string | null;
    /** The finish the draft previews on my house. */
    finishPreview: { house: [number, number]; tint: number } | null;
  }

  /** The last city this device entered, to offer "continue". */
  export interface LastCity {
    city: string;
    server: string | null;
  }
  const LAST_CITY_KEY = "omniget.world.lastCity";
  export function readLastCity(): LastCity | null {
    try {
      const v = JSON.parse(localStorage.getItem(LAST_CITY_KEY) ?? "null") as LastCity | null;
      return v && typeof v.city === "string" && v.city ? v : null;
    } catch {
      return null;
    }
  }
  function writeLastCity(v: LastCity): void {
    try {
      localStorage.setItem(LAST_CITY_KEY, JSON.stringify(v));
    } catch {
      // storage unavailable
    }
  }

  interface Props {
    /** The city the canvas is showing, or null when at home. */
    city: { city: string; server: string | null } | null;
    where: { region: string; ent: number; tick: number; interior: boolean } | null;
    onenter: (city: { city: string; server: string | null }) => void;
    onleave: () => void;
    onsay?: (ent: number, text: string) => void;
    ongoto?: (tile: [number, number]) => void;
    /** The crop a click on an empty bed plants; the canvas reads it. */
    crop?: string;
    oncrop?: (crop: string) => void;
    /** The editor session for the canvas, or null when not editing. */
    oneditor?: (s: EditorHandoff | null) => void;
    /** A finish was published: the canvas re-reads the colours. */
    onfinish?: () => void;
    /** For the diagnostics area only. */
    diag?: { fps: number; tier: number; backend: string; connection: string; error: string } | null;
  }
  let { city, where, onenter, onleave, onsay, ongoto, crop = "carrot", oncrop, oneditor, onfinish, diag = null }: Props = $props();

  type Home = { home: { id: string; name: string; plot: string; interior_region: string; revision?: number; inventory?: Record<string, number> }; plot: { id: string; address: string; entrance: [number, number]; region: string; kind?: string; rect?: [number, number, number, number]; house?: [number, number] } | null };
  type Cell = { id: string; x: number; y: number; crop: string | null; stage: string; kind: string; stage_now: string; needs_water: boolean; next_at: string | null; next_stage: string | null; watered_at: string | null };
  type Asset = { hash: string; bytes: number; width: number; height: number; visibility: string; url: string; homes: number; reports: number; deleted: boolean };
  const CROPS = ["carrot", "tomato", "wheat"];

  let cityId = $state("cidade-piloto");
  let server = $state(getSettings()?.world?.city_server ?? "");
  let signedIn = $state<boolean | null>(null);
  let username = $state("");
  let password = $state("");
  let creating = $state(false);
  let busy = $state(false);
  let home = $state<Home | null>(null);
  /** Loaded once the city answered: null = not yet, false = no home. */
  let homeLoaded = $state(false);
  const lastCity = readLastCity();
  let chat = $state<CityChat[]>([]);
  let draft = $state("");

  // --- the home editor -----------------------------------------------------
  let editing = $state(false);
  let pub = $state<Published | null>(null);
  let tool = $state<string | null>(null);
  let selectedId = $state<string | null>(null);
  let editStatus = $state<EditorStatus>("clean");
  let editError = $state("");
  /** The revision the last publish produced, shown until the next edit. */
  let publishedRevision = $state<number | null>(null);
  /** The picture a frame placed now shows. */
  let pictureChoice = $state<string | null>(null);
  /** Object URLs of my pictures' thumbnails, by hash. */
  let thumbs = $state<Record<string, string>>({});
  /** Bumped on every draft change so the counts below re-render. */
  let editRev = $state(0);
  const editor = new Editor(() => {
    editRev++;
    if (home) editor.save(home.home.id);
    if (editStatus !== "publishing") editStatus = isEmpty(editor.draft) ? "clean" : "draft";
    if (!isEmpty(editor.draft)) publishedRevision = null;
    if (editing) emitEditor();
  });
  const pendingCount = $derived.by(() => {
    void editRev;
    return Object.keys(editor.draft.placed).length + editor.draft.removed.length + (editor.draft.houseVariant !== null ? 1 : 0);
  });
  const catalogue = $derived(pub ? pub.catalogue[where?.interior ? "inside" : "outside"] ?? [] : []);
  const usable = $derived(catalogue.filter((a) => pub?.uses[a] && pub.uses[a] !== "picture"));
  const decorative = $derived(catalogue.filter((a) => !pub?.uses[a]));
  const hasFrame = $derived(catalogue.includes(PICTURE_ASSET));
  const finishes = $derived<HouseFinish[]>(pub?.finishes?.length ? pub.finishes : HOUSE_FINISHES);
  /** The finish on screen: the draft's, or the published one. */
  const currentFinish = $derived.by(() => {
    void editRev;
    return editor.draft.houseVariant ?? pub?.houseVariant ?? 0;
  });
  /** The selected object as the editor sees it (for the selection bar). */
  const selectedPlacement = $derived.by(() => {
    void editRev;
    if (!selectedId) return null;
    return editor.draft.placed[selectedId] ?? pub?.objects.find((o) => o.id === selectedId) ?? null;
  });

  function assetName(asset: string): string {
    const leaf = asset.split("/").pop() ?? asset;
    const v = $t(`world.city.asset_${leaf}`) as string;
    return v && !v.startsWith("world.city.") ? v : leaf;
  }

  function emitEditor(): void {
    const house = home?.plot?.house;
    const preview = editing && house && editor.draft.houseVariant !== null ? { house, tint: finishTint(finishes, editor.draft.houseVariant) } : null;
    oneditor?.(editing ? { editor, tool, published: pub, plotRect: home?.plot?.rect ?? null, picture: pictureChoice, finishPreview: preview } : null);
  }

  function pickFinish(id: number): void {
    editor.setHouseVariant(id === pub?.houseVariant ? null : id);
    emitEditor();
  }

  function choosePicture(hash: string): void {
    pictureChoice = hash;
    // A selected draft frame takes the new picture; otherwise the next one placed does.
    if (selectedId && editor.draft.placed[selectedId]?.asset === PICTURE_ASSET) editor.setPicture(selectedId, hash);
    if (tool !== PICTURE_ASSET) tool = PICTURE_ASSET;
    emitEditor();
  }

  async function fetchPublished(): Promise<void> {
    if (!city || !home) return;
    const r = (await invoke("city_api", { method: "GET", path: `/api/world/homes/${home.home.id}/objects`, body: null, server: city.server })) as {
      home: { revision: number };
      objects: Placement[];
      package_hash: string | null;
      house_variant: number | null;
      house_variants?: HouseFinish[];
      catalogue: Record<"outside" | "inside", string[]>;
      uses?: Record<string, AssetUse>;
    };
    pub = {
      revision: r.home.revision,
      packageHash: r.package_hash ?? "",
      houseVariant: r.house_variant ?? 0,
      objects: r.objects,
      catalogue: r.catalogue ?? { outside: [], inside: [] },
      finishes: r.house_variants ?? HOUSE_FINISHES,
      uses: r.uses ?? {},
    };
  }

  async function startEdit(): Promise<void> {
    if (!home) return;
    busy = true;
    try {
      await fetchPublished();
      editor.restore(home.home.id);
      editStatus = isEmpty(editor.draft) ? "clean" : "draft";
      editing = true;
      tool = null;
      selectedId = null;
      emitEditor();
    } catch (error) {
      showToast("error", String(error));
    } finally {
      busy = false;
    }
  }

  function stopEdit(): void {
    editing = false;
    tool = null;
    selectedId = null;
    emitEditor();
  }

  function pickTool(asset: string): void {
    tool = tool === asset ? null : asset;
    emitEditor();
  }

  /** The canvas asked for the tool to go (Escape). */
  export function clearTool(): void {
    tool = null;
    emitEditor();
  }

  export function selectionChanged(id: string | null): void {
    selectedId = id;
  }

  export function editorChanged(): void {
    editRev++;
  }

  function removeSelected(): void {
    if (!selectedId) return;
    const published = pub?.objects.some((o) => o.id === selectedId) ?? false;
    editor.remove(selectedId, published);
    selectedId = null;
  }

  async function publish(): Promise<void> {
    if (!city || !home || !pub || isEmpty(editor.draft)) return;
    const key = editor.pendingKey ?? crypto.randomUUID();
    editor.pendingKey = key;
    editor.save(home.home.id);
    editStatus = "publishing";
    editError = "";
    try {
      const finishChanged = editor.draft.houseVariant !== null;
      const r = (await invoke("city_api", { method: "POST", path: `/api/world/homes/${home.home.id}/publish`, body: publishBody(pub, editor.draft, key), server: city.server })) as { revision: number; duplicate?: boolean };
      publishedRevision = r.revision;
      if (finishChanged) onfinish?.();
      editor.published();
      editor.save(home.home.id);
      await fetchPublished();
      editStatus = "published";
      showToast("success", $t("world.city.edit_published", { revision: r.revision }) as string);
      emitEditor();
      void loadHome();
    } catch (error) {
      const msg = String(error);
      editError = msg.split(":").pop() ?? msg;
      editStatus = msg.includes("stale_revision") ? "conflict" : "error";
      if (editStatus === "error") showToast("error", $t("world.city.edit_refused", { code: editError }) as string);
    }
  }

  /** After a conflict: take the other side's revision, keep the draft. */
  async function reloadPublished(): Promise<void> {
    try {
      await fetchPublished();
      editStatus = isEmpty(editor.draft) ? "clean" : "draft";
      editError = "";
      emitEditor();
    } catch (error) {
      showToast("error", String(error));
    }
  }

  function discard(): void {
    editor.discard();
    if (home) editor.save(home.home.id);
    editStatus = "clean";
    editError = "";
    selectedId = null;
    emitEditor();
  }

  // --- the inspector: what a resident is doing, and delegating the beds -----
  type TaskLine = { tick: number; task: number; action: string; target: number; state: string; revision: number; reason?: string };
  type Inspect = {
    agent_id: string;
    name: string;
    kind: string;
    energy: number | null;
    autonomous: boolean;
    delegated: number[];
    current: { action: string; state: string; reason: string; progress: number; duration: number; target: number } | null;
    trace: TaskLine[];
  };
  let inspectEnt = $state<number | null>(null);
  let inspected = $state<Inspect | null>(null);
  let inspectErr = $state("");
  let myDelegations = $state<string[]>([]);
  let inspectTimer: ReturnType<typeof setInterval> | null = null;

  /** The canvas picked a figure (or none). */
  export function inspect(ent: number | null): void {
    inspectEnt = ent;
    inspected = null;
    inspectErr = "";
    if (inspectTimer) clearInterval(inspectTimer);
    inspectTimer = null;
    if (ent === null) return;
    void loadInspect();
    void loadDelegations();
    inspectTimer = setInterval(() => void loadInspect(), 2000);
  }

  async function loadInspect(): Promise<void> {
    if (!city || inspectEnt === null) return;
    try {
      inspected = (await invoke("city_api", { method: "GET", path: `/api/world/cities/${encodeURIComponent(city.city)}/ents/${inspectEnt}/tasks?trace=6`, body: null, server: city.server })) as Inspect;
      inspectErr = "";
    } catch (error) {
      inspectErr = String(error).includes("FORBIDDEN") ? "private" : "unavailable";
    }
  }

  async function loadDelegations(): Promise<void> {
    if (!city || !home) return;
    try {
      const r = (await invoke("city_api", { method: "GET", path: `/api/world/homes/${home.home.id}/delegations`, body: null, server: city.server })) as { delegations: Array<{ agent_id: string }> };
      myDelegations = r.delegations.map((d) => d.agent_id);
    } catch {
      myDelegations = [];
    }
  }

  async function toggleDelegation(): Promise<void> {
    if (!city || !home || !inspected) return;
    const on = myDelegations.includes(inspected.agent_id);
    try {
      if (on) {
        await invoke("city_api", { method: "DELETE", path: `/api/world/homes/${home.home.id}/delegations/${inspected.agent_id}`, body: null, server: city.server });
        showToast("success", $t("world.city.inspect_revoked", { name: inspected.name }) as string);
      } else {
        await invoke("city_api", { method: "POST", path: `/api/world/homes/${home.home.id}/delegations`, body: { agent_id: inspected.agent_id, crop }, server: city.server });
        showToast("success", $t("world.city.inspect_delegated", { name: inspected.name }) as string);
      }
      await Promise.all([loadDelegations(), loadInspect()]);
    } catch (error) {
      showToast("error", String(error));
    }
  }

  function taskText(action: string, state: string, reason: string): string {
    const a = $t(`world.city.task_${action}`) as string;
    const s = $t(`world.city.task_state_${state}`) as string;
    const r = reason ? ` · ${$t(`world.city.reason_${reason.split(":")[0]}`)}` : "";
    return `${a} · ${s}${r}`;
  }

  // --- the beds, from the server clock --------------------------------------
  let cells = $state<Cell[]>([]);
  let farmPending = $state<string[]>([]);
  let farmTimer: ReturnType<typeof setInterval> | null = null;

  async function loadFarm(): Promise<void> {
    if (!city || !home?.plot) {
      cells = [];
      return;
    }
    try {
      const r = (await invoke("city_api", { method: "GET", path: `/api/world/cities/${encodeURIComponent(city.city)}/plots/${encodeURIComponent(home.plot.id)}/farm`, body: null, server: city.server })) as { cells: Cell[] };
      cells = r.cells;
    } catch {
      cells = [];
    }
  }

  function naturalAction(c: Cell): "plant" | "water" | "harvest" | "clear" | null {
    if (c.stage === "empty" || !c.crop) return "plant";
    if (c.stage_now === "withered") return "clear";
    if (c.stage_now === "ripe") return "harvest";
    if (c.needs_water) return "water";
    return null;
  }

  function cellLabel(c: Cell): string {
    if (c.stage === "empty" || !c.crop) return $t("world.city.farm_empty_bed") as string;
    const cropName = $t(`world.city.crop_${c.crop}`) as string;
    if (c.stage_now === "withered") return `${cropName} · ${$t("world.city.farm_withered")}`;
    if (c.stage_now === "ripe") return `${cropName} · ${$t("world.city.farm_ripe")}`;
    if (c.needs_water) return `${cropName} · ${c.stage_now} · ${$t("world.city.farm_needs_water")}`;
    if (c.next_at && c.next_stage) {
      const ms = new Date(c.next_at).getTime() - Date.now();
      const secs = Math.max(0, Math.round(ms / 1000));
      const time = secs >= 60 ? `${Math.ceil(secs / 60)} min` : `${secs} s`;
      return `${cropName} · ${$t("world.city.farm_next_in", { stage: c.next_stage, time })}`;
    }
    return `${cropName} · ${c.stage_now}`;
  }

  async function farmAct(c: Cell, action: "plant" | "water" | "harvest" | "clear"): Promise<void> {
    if (!city || farmPending.includes(c.id)) return;
    farmPending = [...farmPending, c.id];
    try {
      await invoke("city_api", { method: "POST", path: "/api/world/farm", body: { city: city.city, cell: c.id, action, crop: action === "plant" ? crop : undefined, op_id: crypto.randomUUID() }, server: city.server });
      await Promise.all([loadFarm(), loadHome()]);
    } catch (error) {
      const msg = String(error);
      if (msg.includes("farm_conflict")) {
        showToast("error", $t("world.city.farm_conflict") as string);
        await loadFarm();
      } else {
        const code = (msg.split(":").pop() ?? "").toLowerCase().replace(/^err_world_(farm_)?/, "");
        showToast("error", ($t(`world.city.farm_${code}`) as string) || msg);
      }
    } finally {
      farmPending = farmPending.filter((id) => id !== c.id);
    }
  }

  // --- pictures attached to the home ----------------------------------------
  let assets = $state<Asset[]>([]);
  let uploading = $state(false);
  let fileInput = $state<HTMLInputElement | null>(null);

  async function loadAssets(): Promise<void> {
    if (!city || !home) {
      assets = [];
      return;
    }
    try {
      const r = (await invoke("city_api", { method: "GET", path: `/api/world/assets?home_id=${home.home.id}`, body: null, server: city.server })) as { assets: Asset[] };
      assets = r.assets;
      void loadThumbs();
    } catch {
      assets = [];
    }
  }

  async function loadThumbs(): Promise<void> {
    if (!city) return;
    const next: Record<string, string> = {};
    for (const a of assets) {
      if (a.deleted) continue;
      if (thumbs[a.hash]) {
        next[a.hash] = thumbs[a.hash];
        continue;
      }
      try {
        const bytes = (await invoke("city_asset", { hash: a.hash, server: city.server })) as ArrayBuffer;
        next[a.hash] = URL.createObjectURL(new Blob([bytes]));
      } catch {
        // No thumbnail; the row still shows the size.
      }
    }
    for (const [h, url] of Object.entries(thumbs)) if (!next[h]) URL.revokeObjectURL(url);
    thumbs = next;
  }

  async function uploadPicture(): Promise<void> {
    const file = fileInput?.files?.[0];
    if (!file || !city || !home) return;
    if (file.size > 512 * 1024) {
      showToast("error", $t("world.city.assets_too_large") as string);
      return;
    }
    uploading = true;
    try {
      const dataUrl = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result));
        reader.onerror = () => reject(reader.error);
        reader.readAsDataURL(file);
      });
      const b64 = dataUrl.slice(dataUrl.indexOf(",") + 1);
      const r = (await invoke("city_api", { method: "POST", path: "/api/world/assets", body: { content_type: file.type, data_b64: b64, home_id: home.home.id }, server: city.server })) as Asset;
      showToast("success", $t("world.city.assets_uploaded", { width: r.width, height: r.height, bytes: r.bytes }) as string);
      await loadAssets();
    } catch (error) {
      showToast("error", String(error));
    } finally {
      uploading = false;
      if (fileInput) fileInput.value = "";
    }
  }

  async function setVisibility(a: Asset, visibility: string): Promise<void> {
    if (!city) return;
    try {
      await invoke("city_api", { method: "PATCH", path: `/api/world/assets/${a.hash}`, body: { visibility }, server: city.server });
      await loadAssets();
    } catch (error) {
      showToast("error", String(error));
    }
  }

  async function detachAsset(a: Asset): Promise<void> {
    if (!city || !home) return;
    try {
      await invoke("city_api", { method: "DELETE", path: `/api/world/homes/${home.home.id}/assets/${a.hash}`, body: null, server: city.server });
      await loadAssets();
    } catch (error) {
      showToast("error", String(error));
    }
  }

  async function reportAsset(a: Asset): Promise<void> {
    if (!city) return;
    try {
      await invoke("city_api", { method: "POST", path: `/api/world/assets/${a.hash}/report`, body: { reason: "reported from the panel" }, server: city.server });
      showToast("success", $t("world.city.assets_reported") as string);
      await loadAssets();
    } catch (error) {
      showToast("error", String(error));
    }
  }

  async function checkSession(): Promise<void> {
    try {
      const r = (await invoke("city_session", { server: server.trim() || null })) as { signed_in: boolean };
      signedIn = r.signed_in;
    } catch {
      signedIn = false;
    }
  }

  async function signIn(): Promise<void> {
    const u = username.trim();
    if (!u || !password) return;
    busy = true;
    try {
      await invoke(creating ? "city_register" : "city_login", { username: u, password, server: server.trim() || null });
      password = "";
      signedIn = true;
    } catch (error) {
      showToast("error", String(error));
    } finally {
      busy = false;
    }
  }

  async function signOut(): Promise<void> {
    try {
      await invoke("city_logout", { server: server.trim() || null });
    } catch {
      // Nothing to forget.
    }
    signedIn = false;
    onleave();
  }

  onMount(() => {
    const stops: Array<() => void> = [];
    void checkSession();
    void (async () => {
      stops.push(
        await listen<CityChat>("city://chat", (e) => {
          chat = [...chat.slice(-49), e.payload];
          onsay?.(e.payload.from_ent, e.payload.text);
        }),
      );
    })();
    return () => stops.forEach((stop) => stop());
  });

  $effect(() => {
    if (city) void loadHome();
    else {
      inspect(null);
      home = null;
      homeLoaded = false;
      chat = [];
      cells = [];
      assets = [];
      if (editing) stopEdit();
    }
  });

  $effect(() => {
    if (home?.plot && city) {
      void loadFarm();
      void loadAssets();
      farmTimer = setInterval(() => void loadFarm(), 20_000);
    }
    return () => {
      if (farmTimer) clearInterval(farmTimer);
      farmTimer = null;
    };
  });

  /** The canvas tells the page a farm action landed; refresh inventory and beds. */
  export function refreshHome(): void {
    void loadHome();
    void loadFarm();
  }

  async function loadHome(): Promise<void> {
    if (!city) return;
    try {
      home = (await invoke("city_api", { method: "GET", path: `/api/world/cities/${encodeURIComponent(city.city)}/homes/@me`, body: null, server: city.server })) as Home;
    } catch {
      home = null;
    }
    homeLoaded = true;
  }

  function enter(): void {
    const id = cityId.trim();
    if (!id) return;
    const target = { city: id, server: server.trim() || null };
    writeLastCity(target);
    onenter(target);
  }

  function resume(): void {
    if (!lastCity) return;
    onenter(lastCity);
  }

  function leave(): void {
    // Leave on screen first: a session that never opened (a refused sign-in,
    // a server that went away) must not keep the user waiting on the close.
    onleave();
    void invoke("city_leave").catch(() => {
      // Already gone.
    });
  }

  async function claim(): Promise<void> {
    if (!city) return;
    busy = true;
    try {
      home = (await invoke("city_api", { method: "POST", path: `/api/world/cities/${encodeURIComponent(city.city)}/claim`, body: {}, server: city.server })) as Home;
      showToast("success", $t("world.city.claimed", { address: home.plot?.address ?? "" }) as string);
    } catch (error) {
      showToast("error", String(error));
    } finally {
      busy = false;
    }
  }

  function goHome(): void {
    const e = home?.plot?.entrance;
    if (!e) return;
    ongoto?.([e[0], e[1] + 1]);
  }

  async function sendChat(): Promise<void> {
    const text = draft.trim();
    if (!text) return;
    draft = "";
    try {
      await invoke("city_chat", { text });
    } catch (error) {
      showToast("error", String(error));
    }
  }
</script>

<section class="city">
  {#if !city}
    <div class="lobby">
      {#if signedIn === null}
        <p class="hint">{$t("world.city.checking")}</p>
      {:else if signedIn === false}
        <p class="lede">{$t("world.city.sign_in_lede")}</p>
        <form class="row" onsubmit={(e) => { e.preventDefault(); void signIn(); }}>
          <input type="text" bind:value={username} maxlength="32" autocomplete="username" aria-label={$t("world.city.username") as string} placeholder={$t("world.city.username") as string} />
          <input type="password" bind:value={password} maxlength="256" autocomplete="current-password" aria-label={$t("world.city.password") as string} placeholder={$t("world.city.password") as string} />
          <button type="submit" class="btn primary" disabled={busy || !username.trim() || !password}>{creating ? $t("world.city.create_account") : $t("world.city.sign_in")}</button>
          <button type="button" class="link" onclick={() => (creating = !creating)}>{creating ? $t("world.city.have_account") : $t("world.city.no_account")}</button>
        </form>
      {:else}
        {#if lastCity}
          <div class="row">
            <button type="button" class="btn primary" onclick={resume}>{$t("world.city.continue_in", { city: lastCity.city })}</button>
            <span class="hint">{$t("world.city.continue_hint")}</span>
          </div>
        {/if}
        <form class="row" onsubmit={(e) => { e.preventDefault(); enter(); }}>
          <label class="label" for="city-id">{lastCity ? $t("world.city.other_city") : $t("world.city.title")}</label>
          <input id="city-id" type="text" bind:value={cityId} maxlength="64" placeholder="cidade-piloto" />
          <button type="submit" class="btn" class:primary={!lastCity}>{$t("world.city.enter")}</button>
          <button type="button" class="link" onclick={signOut}>{$t("world.city.sign_out")}</button>
        </form>
      {/if}
      <p class="hint">{$t("world.city.hint")}</p>
      <details class="diag">
        <summary>{$t("world.city.advanced")}</summary>
        <div class="row">
          <label class="label" for="city-server">{$t("world.city.server")}</label>
          <input id="city-server" type="text" class="server" bind:value={server} placeholder="https://chat.tonho.wtf" title={$t("world.city.server_hint") as string} onchange={() => void checkSession()} />
        </div>
        <p class="hint">{$t("world.city.server_hint")}</p>
      </details>
    </div>
  {:else}
    <div class="row">
      <span class="dot" class:warn={diag?.connection === "reconnecting"} aria-hidden="true"></span>
      <span class="label strong">{$t("world.city.in", { city: city.city })}</span>
      {#if where}<span class="chip">{where.interior ? $t("world.city.inside_home") : $t("world.city.on_the_street")}</span>{/if}
      {#if diag?.connection === "reconnecting"}<span class="chip warn" role="status">{$t("world.city.reconnecting")}</span>{/if}
      <button type="button" class="btn" onclick={leave}>{$t("world.city.leave")}</button>
    </div>
    <div class="row">
      {#if home?.plot}
        <span class="label">{$t("world.city.address")}</span>
        <span class="chip host">{home.plot.address}</span>
        <button type="button" class="btn" onclick={goHome}>{$t("world.city.go_home")}</button>
        {#if home.home.inventory && Object.keys(home.home.inventory).length > 0}
          <span class="chip" title={$t("world.city.inventory") as string}>{Object.entries(home.home.inventory).map(([k, v]) => `${$t(`world.city.crop_${k}`)} ×${v}`).join(" · ")}</span>
        {/if}
        <label class="label">{$t("world.city.plant")}
          <select value={crop} onchange={(e) => oncrop?.((e.currentTarget as HTMLSelectElement).value)}>
            {#each CROPS as c}<option value={c}>{$t(`world.city.crop_${c}`)}</option>{/each}
          </select>
        </label>
      {:else if homeLoaded}
        <button type="button" class="btn primary" onclick={claim} disabled={busy}>{$t("world.city.claim")}</button>
        <span class="hint">{$t("world.city.claim_hint")}</span>
      {:else}
        <span class="hint">{$t("world.city.entering")}</span>
      {/if}
    </div>
    {#if home?.plot}
      <div class="row">
        {#if !editing}
          <button type="button" class="btn" onclick={() => void startEdit()} disabled={busy}>{$t("world.city.edit")}</button>
        {:else}
          <button type="button" class="btn" onclick={stopEdit}>{$t("world.city.edit_stop")}</button>
          <span class="chip status {editStatus}" data-status={editStatus} role="status" aria-live="polite">
            {#if editStatus === "published" && publishedRevision !== null}{$t("world.city.edit_published_rev", { revision: publishedRevision })}{:else}{$t(`world.city.edit_status_${editStatus}`)}{/if}{#if editError} · {editError}{/if}
          </span>
          {#if pendingCount > 0}<span class="chip">{$t("world.city.edit_pending", { count: pendingCount })}</span>{/if}
        {/if}
      </div>
      {#if editing && pub}
        {#if selectedPlacement}
          <div class="row selbar" role="group" aria-label={$t("world.city.edit_selection") as string}>
            <span class="label strong">{assetName(selectedPlacement.asset)}</span>
            <span class="hint">{$t("world.city.edit_selected")}</span>
            <button type="button" class="btn" onclick={removeSelected}>{$t("world.city.edit_remove")}</button>
          </div>
        {/if}
        {#if !where?.interior}
          <div class="row finishes" role="radiogroup" aria-label={$t("world.city.edit_house_variant") as string}>
            <span class="label">{$t("world.city.edit_house_variant")}</span>
            {#each finishes as f (f.id)}
              <button type="button" class="chip pick finish" role="radio" aria-checked={currentFinish === f.id} class:active={currentFinish === f.id} onclick={() => pickFinish(f.id)}>
                <span class="swatch" style={`background:${f.tint}`}></span>{$t(`world.city.finish_${f.key}`)}
              </button>
            {/each}
          </div>
        {/if}
        {#if usable.length > 0}
          <div class="row catalogue" data-space={where?.interior ? "inside" : "outside"}>
            <span class="label">{$t("world.city.edit_usable")}</span>
            {#each usable as asset (asset)}
              <button type="button" class="chip pick" aria-pressed={tool === asset} class:active={tool === asset} onclick={() => pickTool(asset)} title={$t(`world.city.use_${pub.uses[asset]}`) as string}>{assetName(asset)} <small>{$t(`world.city.use_${pub.uses[asset]}`)}</small></button>
            {/each}
          </div>
        {/if}
        <div class="row catalogue" data-space={where?.interior ? "inside" : "outside"}>
          <span class="label">{$t("world.city.edit_decorative")}</span>
          {#each decorative as asset (asset)}
            <button type="button" class="chip pick" aria-pressed={tool === asset} class:active={tool === asset} onclick={() => pickTool(asset)}>{assetName(asset)}</button>
          {/each}
        </div>
        {#if hasFrame}
          <div class="row pictures">
            <span class="label">{$t("world.city.edit_frame")}</span>
            {#if assets.filter((a) => !a.deleted).length === 0}
              <span class="hint">{$t("world.city.edit_frame_empty")}</span>
            {:else}
              {#each assets.filter((a) => !a.deleted) as a (a.hash)}
                <button type="button" class="thumb" aria-pressed={pictureChoice === a.hash} class:active={pictureChoice === a.hash} onclick={() => choosePicture(a.hash)} title={`${a.width}×${a.height}`}>
                  {#if thumbs[a.hash]}<img src={thumbs[a.hash]} alt={$t("world.city.picture_alt", { width: a.width, height: a.height }) as string} />{:else}<span>{a.width}×{a.height}</span>{/if}
                </button>
              {/each}
            {/if}
            {#if tool === PICTURE_ASSET && !pictureChoice}<span class="hint warn-text">{$t("world.city.edit_frame_pick")}</span>{/if}
          </div>
        {/if}
        <div class="row">
          {#key editRev}
            <button type="button" class="btn" onclick={() => editor.undo()} disabled={!editor.canUndo}>{$t("world.city.edit_undo")}</button>
            <button type="button" class="btn" onclick={() => editor.redo()} disabled={!editor.canRedo}>{$t("world.city.edit_redo")}</button>
          {/key}
          <button type="button" class="btn" onclick={discard} disabled={pendingCount === 0 || editStatus === "publishing"}>{$t("world.city.edit_discard")}</button>
          {#if editStatus === "conflict"}
            <button type="button" class="btn" onclick={() => void reloadPublished()}>{$t("world.city.edit_reload")}</button>
          {/if}
          <button type="button" class="btn primary" onclick={() => void publish()} disabled={pendingCount === 0 || editStatus === "publishing" || editStatus === "conflict"}>{$t("world.city.edit_publish")}</button>
        </div>
        <p class="hint">{$t("world.city.edit_keys")}</p>
      {/if}
      {#if cells.length > 0}
        <div class="row farm">
          <span class="label">{$t("world.city.farm_title")}</span>
          {#each cells as c, i (c.id)}
            {@const action = naturalAction(c)}
            {@const pending = farmPending.includes(c.id)}
            <span class="chip bed" class:pending title={c.id}>
              <span class="bed-name">{$t("world.city.farm_bed", { index: i + 1 })}</span>
              <span class="bed-state">{cellLabel(c)}</span>
              {#if action}
                <button type="button" class="mini" onclick={() => void farmAct(c, action)} disabled={pending}>{pending ? "…" : $t(`world.city.farm_action_${action}`)}</button>
              {/if}
            </span>
          {/each}
        </div>
      {/if}
      <details class="assets">
        <summary>{$t("world.city.assets_title")} ({assets.length})</summary>
        <div class="row">
          <input type="file" accept="image/png,image/jpeg,image/webp" bind:this={fileInput} onchange={() => void uploadPicture()} disabled={uploading} aria-label={$t("world.city.assets_upload") as string} />
          <span class="hint">{$t("world.city.assets_limits")}</span>
        </div>
        {#if assets.length === 0}
          <p class="hint">{$t("world.city.assets_none")}</p>
        {:else}
          {#each assets as a (a.hash)}
            <div class="row asset">
              {#if thumbs[a.hash]}<img class="mini-thumb" src={thumbs[a.hash]} alt="" />{/if}
              <span class="hint">{a.width}×{a.height} · {Math.round(a.bytes / 1024)} KB{#if a.deleted} · {$t("world.city.assets_removed")}{/if}</span>
              <button type="button" class="link" onclick={() => void setVisibility(a, a.visibility === "public" ? "private" : "public")} title={$t("world.city.assets_visibility_hint") as string}>{a.visibility === "public" ? $t("world.city.assets_public") : $t("world.city.assets_private")}</button>
              <button type="button" class="link" onclick={() => void detachAsset(a)}>{$t("world.city.assets_detach")}</button>
              <button type="button" class="link" onclick={() => void reportAsset(a)}>{$t("world.city.assets_report")}</button>
            </div>
          {/each}
        {/if}
        <p class="hint">{$t("world.city.assets_scope")}</p>
      </details>
    {/if}
    <div class="chat">
      <div class="chat-log" aria-live="polite">
        {#each chat as line (line.ts + ":" + line.from_ent)}
          <p><strong>{line.name}</strong> {line.text}</p>
        {/each}
      </div>
      <form class="chat-form" onsubmit={(e) => { e.preventDefault(); void sendChat(); }}>
        <input type="text" bind:value={draft} maxlength="500" aria-label={$t("world.city.chat_placeholder") as string} placeholder={$t("world.city.chat_placeholder") as string} />
        <button type="submit" class="btn" disabled={!draft.trim()}>{$t("world.house.send")}</button>
      </form>
    </div>
    {#if inspectEnt !== null}
      <div class="inspect" role="region" aria-label={$t("world.city.inspect_title") as string} aria-live="polite">
        {#if inspected}
          <div class="row">
            <span class="label strong">{inspected.name}</span>
            <span class="chip">{$t(`world.city.kind_${inspected.kind}`)}</span>
            {#if inspected.energy !== null}<span class="chip" title={$t("world.city.inspect_energy") as string}>⚡ {Math.round(((inspected.energy ?? 0) / 255) * 100)}%</span>{/if}
            <button type="button" class="link" onclick={() => inspect(null)}>{$t("world.city.inspect_close")}</button>
          </div>
          {#if inspected.current}
            <p class="now"><strong>{$t("world.city.inspect_now")}</strong> {taskText(inspected.current.action, inspected.current.state, inspected.current.reason)}{#if inspected.current.duration > 0 && inspected.current.state === "executing"} ({Math.min(100, Math.round((inspected.current.progress / inspected.current.duration) * 100))}%){/if}</p>
          {:else}
            <p class="now hint">{inspected.autonomous ? $t("world.city.inspect_choosing") : $t("world.city.inspect_no_tasks")}</p>
          {/if}
          {#if inspected.trace.length > 0}
            <ol class="trace">
              {#each [...inspected.trace].reverse() as l (l.tick + ":" + l.task + ":" + l.state)}
                <li class:bad={!!l.reason && l.state !== "queued"}>{taskText(l.action, l.state, l.reason ?? "")}</li>
              {/each}
            </ol>
          {/if}
          {#if inspected.kind === "resident" && home?.plot}
            <div class="row">
              <button type="button" class="btn" onclick={() => void toggleDelegation()}>{myDelegations.includes(inspected.agent_id) ? $t("world.city.inspect_revoke") : $t("world.city.inspect_delegate", { name: inspected.name })}</button>
              <span class="hint">{$t("world.city.inspect_delegate_hint")}</span>
            </div>
          {/if}
        {:else if inspectErr}
          <p class="hint">{$t(`world.city.inspect_${inspectErr}`)}</p>
        {:else}
          <p class="hint">…</p>
        {/if}
      </div>
    {/if}
    <details class="diag">
      <summary>{$t("world.city.diagnostics")}</summary>
      <dl>
        <dt>{$t("world.city.diag_region")}</dt><dd><code>{where?.region ?? "—"}</code></dd>
        <dt>{$t("world.city.diag_tick")}</dt><dd>{where?.tick ?? "—"}</dd>
        <dt>{$t("world.city.diag_render")}</dt><dd>{diag ? `${diag.backend} · tier ${diag.tier} · ${Math.round(diag.fps)} fps` : "—"}</dd>
        <dt>{$t("world.city.diag_connection")}</dt><dd>{diag?.connection ?? "—"}</dd>
        <dt>{$t("world.city.diag_server")}</dt><dd><code>{city.server ?? $t("world.city.diag_default_server")}</code></dd>
        {#if diag?.error}<dt>{$t("world.city.diag_error")}</dt><dd><code>{diag.error}</code></dd>{/if}
      </dl>
    </details>
  {/if}
</section>

<style>
  .city {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
    max-width: 960px;
  }
  .row,
  .chat-form {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
  }
  .btn {
    padding: 0.35rem 0.8rem;
    border-radius: 8px;
    border: 1px solid rgba(127, 127, 127, 0.3);
    background: rgba(127, 127, 127, 0.12);
    color: inherit;
    font: inherit;
    font-size: 0.85rem;
    cursor: pointer;
  }
  .btn.primary {
    background: var(--accent, #0a84ff);
    border-color: transparent;
    color: #fff;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  select {
    padding: 0.3rem 0.5rem;
    border-radius: 8px;
    border: 1px solid rgba(127, 127, 127, 0.3);
    background: rgba(127, 127, 127, 0.08);
    color: inherit;
    font: inherit;
    font-size: 0.85rem;
    margin-left: 0.3rem;
  }
  input {
    padding: 0.35rem 0.6rem;
    border-radius: 8px;
    border: 1px solid rgba(127, 127, 127, 0.3);
    background: rgba(127, 127, 127, 0.08);
    color: inherit;
    font: inherit;
    font-size: 0.85rem;
  }
  .server {
    width: 18rem;
    font-family: ui-monospace, monospace;
  }
  .chat-form input {
    flex: 1;
    min-width: 0;
  }
  .link {
    background: none;
    border: 0;
    color: inherit;
    opacity: 0.6;
    font-size: 0.8rem;
    cursor: pointer;
    text-decoration: underline;
  }
  .hint,
  .label {
    font-size: 0.85rem;
    opacity: 0.7;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #30d158;
    display: inline-block;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.15rem 0.6rem;
    border-radius: 999px;
    background: rgba(127, 127, 127, 0.14);
    font-size: 0.8rem;
  }
  .chip.host {
    font-weight: 600;
  }
  .chip.pick {
    cursor: pointer;
    border: 1px solid transparent;
    font: inherit;
    font-size: 0.8rem;
    color: inherit;
  }
  .chip.pick.active {
    background: var(--accent, #0a84ff);
    color: #fff;
  }
  .chip.status.conflict,
  .chip.status.error {
    background: rgba(255, 69, 58, 0.18);
  }
  .chip.status.published {
    background: rgba(48, 209, 88, 0.18);
  }
  .chip.bed {
    gap: 0.5rem;
  }
  .chip.bed.pending {
    opacity: 0.6;
  }
  .bed-name {
    font-weight: 600;
  }
  .mini {
    padding: 0.1rem 0.5rem;
    border-radius: 999px;
    border: 1px solid rgba(127, 127, 127, 0.3);
    background: rgba(127, 127, 127, 0.15);
    color: inherit;
    font: inherit;
    font-size: 0.75rem;
    cursor: pointer;
  }
  .mini:disabled {
    cursor: default;
  }
  .assets summary {
    cursor: pointer;
    font-size: 0.85rem;
    opacity: 0.8;
  }
  .assets .row {
    margin-top: 0.4rem;
  }
  .inspect {
    padding: 0.6rem 0.75rem;
    border-radius: 12px;
    background: rgba(127, 127, 127, 0.1);
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }
  .inspect .now {
    margin: 0;
    font-size: 0.85rem;
  }
  .trace {
    margin: 0;
    padding-left: 1.1rem;
    font-size: 0.78rem;
    opacity: 0.8;
  }
  .trace li.bad {
    color: #ff9f0a;
  }
  .lobby {
    display: flex;
    flex-direction: column;
    gap: 0.6rem;
  }
  .lede {
    margin: 0;
    font-size: 0.9rem;
  }
  .strong {
    font-weight: 600;
    opacity: 0.9;
  }
  .dot.warn {
    background: #ff9f0a;
  }
  .chip.warn {
    background: rgba(255, 159, 10, 0.2);
  }
  .warn-text {
    color: #ff9f0a;
    opacity: 1;
  }
  .chip.pick small {
    opacity: 0.7;
    font-size: 0.7rem;
  }
  .chip.pick:focus-visible,
  .thumb:focus-visible,
  .btn:focus-visible {
    outline: 2px solid var(--accent, #0a84ff);
    outline-offset: 2px;
  }
  .swatch {
    width: 0.8rem;
    height: 0.8rem;
    border-radius: 50%;
    border: 1px solid rgba(0, 0, 0, 0.25);
    display: inline-block;
  }
  .selbar {
    padding: 0.35rem 0.5rem;
    border-radius: 10px;
    background: rgba(255, 230, 128, 0.14);
  }
  .thumb {
    width: 2.6rem;
    height: 2.6rem;
    padding: 0;
    border-radius: 8px;
    border: 2px solid transparent;
    background: rgba(127, 127, 127, 0.12);
    color: inherit;
    font-size: 0.6rem;
    cursor: pointer;
    overflow: hidden;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }
  .thumb.active {
    border-color: var(--accent, #0a84ff);
  }
  .thumb img,
  .mini-thumb {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .mini-thumb {
    width: 1.6rem;
    height: 1.6rem;
    border-radius: 4px;
  }
  .diag summary {
    cursor: pointer;
    font-size: 0.8rem;
    opacity: 0.6;
  }
  .diag dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 0.2rem 0.8rem;
    font-size: 0.78rem;
    margin: 0.4rem 0 0;
  }
  .diag dt {
    opacity: 0.6;
  }
  .diag dd {
    margin: 0;
    word-break: break-all;
  }
  .chat-log {
    max-height: 9rem;
    overflow-y: auto;
    font-size: 0.85rem;
  }
  .chat-log p {
    margin: 0.15rem 0;
  }
</style>
