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
  import { Editor, isEmpty, publishBody, type EditorStatus, type Placement, type Published } from "$lib/world/editor";

  /** What the canvas needs while a draft is open. */
  export interface EditorHandoff {
    editor: Editor;
    tool: string | null;
    published: Published | null;
    plotRect: [number, number, number, number] | null;
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
  }
  let { city, where, onenter, onleave, onsay, ongoto, crop = "carrot", oncrop, oneditor }: Props = $props();

  type Home = { home: { id: string; name: string; plot: string; interior_region: string; revision?: number; inventory?: Record<string, number> }; plot: { id: string; address: string; entrance: [number, number]; region: string; kind?: string; rect?: [number, number, number, number] } | null };
  type Cell = { id: string; x: number; y: number; crop: string | null; stage: string; kind: string; stage_now: string; needs_water: boolean; next_at: string | null; next_stage: string | null; watered_at: string | null };
  type Asset = { hash: string; bytes: number; width: number; height: number; visibility: string; url: string; homes: number; reports: number; deleted: boolean };
  const CROPS = ["carrot", "tomato", "wheat"];

  let cityId = $state("cidade-piloto");
  let server = $state(getSettings()?.world?.city_server ?? "");
  let showServer = $state(false);
  let signedIn = $state<boolean | null>(null);
  let username = $state("");
  let password = $state("");
  let creating = $state(false);
  let busy = $state(false);
  let home = $state<Home | null>(null);
  let chat = $state<CityChat[]>([]);
  let draft = $state("");

  // --- the home editor -----------------------------------------------------
  let editing = $state(false);
  let pub = $state<Published | null>(null);
  let tool = $state<string | null>(null);
  let selectedId = $state<string | null>(null);
  let editStatus = $state<EditorStatus>("clean");
  let editError = $state("");
  /** Bumped on every draft change so the counts below re-render. */
  let editRev = $state(0);
  const editor = new Editor(() => {
    editRev++;
    if (home) editor.save(home.home.id);
    if (editStatus !== "publishing") editStatus = isEmpty(editor.draft) ? "clean" : "draft";
  });
  const pendingCount = $derived.by(() => {
    void editRev;
    return Object.keys(editor.draft.placed).length + editor.draft.removed.length + (editor.draft.houseVariant !== null ? 1 : 0);
  });
  const catalogue = $derived(pub ? pub.catalogue[where?.interior ? "inside" : "outside"] ?? [] : []);

  function emitEditor(): void {
    oneditor?.(editing ? { editor, tool, published: pub, plotRect: home?.plot?.rect ?? null } : null);
  }

  async function fetchPublished(): Promise<void> {
    if (!city || !home) return;
    const r = (await invoke("city_api", { method: "GET", path: `/api/world/homes/${home.home.id}/objects`, body: null, server: city.server })) as {
      home: { revision: number };
      objects: Placement[];
      package_hash: string | null;
      house_variant: number | null;
      catalogue: Record<"outside" | "inside", string[]>;
    };
    pub = {
      revision: r.home.revision,
      packageHash: r.package_hash ?? "",
      houseVariant: r.house_variant ?? 1,
      objects: r.objects,
      catalogue: r.catalogue ?? { outside: [], inside: [] },
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
      const r = (await invoke("city_api", { method: "POST", path: `/api/world/homes/${home.home.id}/publish`, body: publishBody(pub, editor.draft, key), server: city.server })) as { revision: number; duplicate?: boolean };
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
    } catch {
      assets = [];
    }
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
      home = null;
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
  }

  function enter(): void {
    const id = cityId.trim();
    if (!id) return;
    onenter({ city: id, server: server.trim() || null });
  }

  async function leave(): Promise<void> {
    try {
      await invoke("city_leave");
    } catch {
      // Already gone.
    }
    onleave();
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
    {#if signedIn === false}
      <form class="row" onsubmit={(e) => { e.preventDefault(); void signIn(); }}>
        <span class="label">{$t("world.city.sign_in")}</span>
        <input type="text" bind:value={username} maxlength="32" autocomplete="username" placeholder={$t("world.city.username") as string} />
        <input type="password" bind:value={password} maxlength="256" autocomplete="current-password" placeholder={$t("world.city.password") as string} />
        <button type="submit" class="btn primary" disabled={busy || !username.trim() || !password}>{creating ? $t("world.city.create_account") : $t("world.city.sign_in")}</button>
        <button type="button" class="link" onclick={() => (creating = !creating)}>{creating ? $t("world.city.have_account") : $t("world.city.no_account")}</button>
        <button type="button" class="link" onclick={() => (showServer = !showServer)}>{$t("world.city.server")}</button>
        {#if showServer}
          <input type="text" class="server" bind:value={server} placeholder="https://chat.tonho.wtf" title={$t("world.city.server_hint") as string} onchange={() => void checkSession()} />
        {/if}
      </form>
    {:else}
      <form class="row" onsubmit={(e) => { e.preventDefault(); enter(); }}>
        <span class="label">{$t("world.city.title")}</span>
        <input type="text" bind:value={cityId} maxlength="64" placeholder="cidade-piloto" />
        <button type="submit" class="btn primary" disabled={signedIn !== true}>{$t("world.city.enter")}</button>
        <button type="button" class="link" onclick={() => (showServer = !showServer)}>{$t("world.city.server")}</button>
        {#if showServer}
          <input type="text" class="server" bind:value={server} placeholder="https://chat.tonho.wtf" title={$t("world.city.server_hint") as string} onchange={() => void checkSession()} />
        {/if}
        {#if signedIn}<button type="button" class="link" onclick={signOut}>{$t("world.city.sign_out")}</button>{/if}
      </form>
    {/if}
    <p class="hint">{$t("world.city.hint")}</p>
  {:else}
    <div class="row">
      <span class="dot on" aria-hidden="true"></span>
      <span class="label">{$t("world.city.in", { city: city.city })}</span>
      {#if where}<span class="chip">{where.interior ? $t("world.city.inside") : $t("world.city.outside")} · {where.region.split("/").pop()}</span>{/if}
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
      {:else}
        <button type="button" class="btn primary" onclick={claim} disabled={busy}>{$t("world.city.claim")}</button>
        <span class="hint">{$t("world.city.claim_hint")}</span>
      {/if}
    </div>
    {#if home?.plot}
      <div class="row">
        {#if !editing}
          <button type="button" class="btn" onclick={() => void startEdit()} disabled={busy}>{$t("world.city.edit")}</button>
        {:else}
          <button type="button" class="btn" onclick={stopEdit}>{$t("world.city.edit_stop")}</button>
          <span class="chip status {editStatus}" data-status={editStatus}>{$t(`world.city.edit_status_${editStatus}`)}{#if editError} · {editError}{/if}</span>
          {#if pendingCount > 0}<span class="chip">{$t("world.city.edit_pending", { count: pendingCount })}</span>{/if}
        {/if}
      </div>
      {#if editing && pub}
        <div class="row catalogue" data-space={where?.interior ? "inside" : "outside"}>
          <span class="label">{$t("world.city.edit_catalogue")}</span>
          {#each catalogue as asset (asset)}
            <button type="button" class="chip pick" class:active={tool === asset} onclick={() => pickTool(asset)}>{asset.split("/").pop()}</button>
          {/each}
        </div>
        <div class="row">
          {#key editRev}
            <button type="button" class="btn" onclick={() => editor.undo()} disabled={!editor.canUndo}>{$t("world.city.edit_undo")}</button>
            <button type="button" class="btn" onclick={() => editor.redo()} disabled={!editor.canRedo}>{$t("world.city.edit_redo")}</button>
          {/key}
          {#if selectedId}<button type="button" class="btn" onclick={removeSelected}>{$t("world.city.edit_remove")}</button>{/if}
          <button type="button" class="btn" onclick={discard} disabled={pendingCount === 0 || editStatus === "publishing"}>{$t("world.city.edit_discard")}</button>
          {#if editStatus === "conflict"}
            <button type="button" class="btn" onclick={() => void reloadPublished()}>{$t("world.city.edit_reload")}</button>
          {/if}
          <button type="button" class="btn primary" onclick={() => void publish()} disabled={pendingCount === 0 || editStatus === "publishing" || editStatus === "conflict"}>{$t("world.city.edit_publish")}</button>
        </div>
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
          <input type="file" accept="image/png,image/jpeg,image/webp" bind:this={fileInput} onchange={() => void uploadPicture()} disabled={uploading} />
          <span class="hint">{$t("world.city.assets_limits")}</span>
        </div>
        {#if assets.length === 0}
          <p class="hint">{$t("world.city.assets_none")}</p>
        {:else}
          {#each assets as a (a.hash)}
            <div class="row asset">
              <code>{a.hash.slice(0, 12)}</code>
              <span class="hint">{a.width}×{a.height} · {Math.round(a.bytes / 1024)} KB{#if a.deleted} · removed{/if}</span>
              <button type="button" class="link" onclick={() => void setVisibility(a, a.visibility === "public" ? "private" : "public")}>{a.visibility === "public" ? $t("world.city.assets_public") : $t("world.city.assets_private")}</button>
              <button type="button" class="link" onclick={() => void detachAsset(a)}>{$t("world.city.assets_detach")}</button>
              <button type="button" class="link" onclick={() => void reportAsset(a)}>{$t("world.city.assets_report")}</button>
            </div>
          {/each}
        {/if}
        <p class="hint">{$t("world.city.assets_placeholder")}</p>
      </details>
    {/if}
    <div class="chat">
      <div class="chat-log">
        {#each chat as line (line.ts + ":" + line.from_ent)}
          <p><strong>{line.name}</strong> {line.text}</p>
        {/each}
      </div>
      <form class="chat-form" onsubmit={(e) => { e.preventDefault(); void sendChat(); }}>
        <input type="text" bind:value={draft} maxlength="500" placeholder={$t("world.city.chat_placeholder") as string} />
        <button type="submit" class="btn" disabled={!draft.trim()}>{$t("world.house.send")}</button>
      </form>
    </div>
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
  .asset code {
    font-size: 0.75rem;
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
