<script lang="ts">
  /**
   * Busca sem IA e sem anúncio / pins parecidos (estudo 67): a mesma API que
   * o site usa, com os filtros que o Pinterest não dá (anúncio, IA em três
   * níveis, só vídeo/imagem/GIF, largura mínima). Seleciona e baixa.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onDestroy, onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errText, onToolProgress, openUrl, pct, pickDir, reveal, type ToolProgress } from "$lib/tools/rt";
  import { defaultDownload, defaultFilters, fmtCount, loadCookies, type Board, type ListOut, type ManyOut, type Pin } from "$lib/tools/pinterest";
  import PinCookies from "./PinCookies.svelte";
  import PinDownloadOptions from "./PinDownloadOptions.svelte";
  import PinFilters from "./PinFilters.svelte";
  import PinGrid from "./PinGrid.svelte";

  let { mode = "search" }: { mode?: "search" | "related" } = $props();

  let query = $state("");
  let scope = $state<"pins" | "videos" | "boards">("pins");
  let cookies = $state(loadCookies());
  let filters = $state(defaultFilters());
  let limit = $state(60);
  let busy = $state<string | null>(null);
  let list = $state<ListOut | null>(null);
  let boards = $state<Board[]>([]);
  let selected = $state<Set<string>>(new Set());
  let opts = $state(defaultDownload());
  let showOpts = $state(false);
  let showCookies = $state(false);
  let progress = $state<ToolProgress | null>(null);
  let result = $state<ManyOut | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(async () => { unlisten = await onToolProgress((p) => { if (p.id.startsWith("pinterest:")) progress = p; }); });
  onDestroy(() => unlisten?.());

  async function run() {
    if (!query.trim() || busy) return;
    busy = "list"; list = null; boards = []; selected = new Set(); result = null; progress = null;
    try {
      if (mode === "related") {
        list = await invoke<ListOut>("tool_pin_related", { url: query, cookies: cookies || null, limit, filters });
      } else if (scope === "boards") {
        boards = await invoke<Board[]>("tool_pin_boards_search", { query, cookies: cookies || null, limit });
      } else {
        const url = query.includes("pinterest.") || query.startsWith("pin.it") ? query : `https://www.pinterest.com/search/${scope}/?q=${encodeURIComponent(query)}`;
        list = await invoke<ListOut>("tool_pin_list", { url, cookies: cookies || null, limit, filters });
      }
    } catch (e) { showToast("error", errText(e)); } finally { busy = null; }
  }

  async function download(all: boolean) {
    if (!list || busy) return;
    const pins: Pin[] = all ? list.pins : list.pins.filter((p) => selected.has(p.id));
    if (!pins.length) return;
    if (!opts.dest) { const d = await pickDir(); if (!d) return; opts.dest = d; }
    busy = "download"; result = null; progress = null;
    try {
      result = await invoke<ManyOut>("tool_pin_download_many", { pins, opts: { ...opts, section_folders: false }, cookies: cookies || null });
      showToast(result.failed.length ? "info" : "success", `${result.downloaded} ${$t("tools.pinterest.downloaded")}`);
    } catch (e) { showToast("error", errText(e)); } finally { busy = null; }
  }

  function useGuide(g: string) { query = mode === "related" ? query : g; if (mode !== "related") run(); }
</script>

<div class="tool">
  <section>
    <div class="group">
      <div class="group-row">
        <div class="group-row-content"><input class="input" type="text" bind:value={query} placeholder={mode === "related" ? $t("tools.pinterest.pin_placeholder") : $t("tools.pinterest.query_placeholder")} onkeydown={(e) => e.key === "Enter" && run()} /></div>
        <div class="group-row-trailing btn-row">
          {#if mode === "search"}
            <div class="segmented">
              <button class="segmented-btn" class:active={scope === "pins"} type="button" onclick={() => (scope = "pins")}>{$t("tools.pinterest.scope_pins")}</button>
              <button class="segmented-btn" class:active={scope === "videos"} type="button" onclick={() => (scope = "videos")}>{$t("tools.pinterest.scope_videos")}</button>
              <button class="segmented-btn" class:active={scope === "boards"} type="button" onclick={() => (scope = "boards")}>{$t("tools.pinterest.scope_boards")}</button>
            </div>
          {/if}
          <button class="btn btn-ghost btn-sm" type="button" onclick={() => (showCookies = !showCookies)} title={$t("tools.pinterest.cookies_hint")}>🍪</button>
          <button class="btn btn-primary" type="button" disabled={busy !== null || !query.trim()} onclick={run}>{busy === "list" ? $t("tools.common.working") : $t("tools.pinterest.search")}</button>
        </div>
      </div>
      {#if showCookies}<PinCookies bind:value={cookies} />{/if}
      <PinFilters bind:filters />
      <div class="group-row">
        <div class="group-row-content"><div class="group-row-title">{$t("tools.pinterest.limit")}</div></div>
        <div class="group-row-trailing"><input class="input" type="number" min="10" max="1000" step="10" bind:value={limit} style:width="7em" /></div>
      </div>
      {#if busy === "list" && progress}<div class="group-row"><div class="group-row-sub">{$t("tools.pinterest.stage_list")} {progress.done}</div></div>{/if}
    </div>
  </section>

  {#if boards.length}
    <section>
      <span class="group-label">{boards.length} {$t("tools.pinterest.boards")}</span>
      <div class="group">
        {#each boards as b (b.id)}
          <div class="group-row">
            {#if b.cover}<img class="cover" src={b.cover} alt="" loading="lazy" />{/if}
            <div class="group-row-content">
              <div class="group-row-title">{b.name}</div>
              <div class="group-row-sub">{fmtCount(b.pin_count)} {$t("tools.pinterest.pins")} · {fmtCount(b.follower_count)} {$t("tools.pinterest.followers")}{#if b.owner?.username} · {b.owner.name ?? b.owner.username}{/if}{#if b.description} · {b.description}{/if}</div>
            </div>
            <div class="group-row-trailing btn-row">
              <button class="btn btn-secondary btn-sm" type="button" onclick={() => { query = b.url; scope = "pins"; run(); }}>{$t("tools.pinterest.load")}</button>
              <button class="btn btn-ghost btn-sm" type="button" onclick={() => openUrl(b.url)}>↗</button>
            </div>
          </div>
        {/each}
      </div>
    </section>
  {/if}

  {#if list}
    {#if list.guides.length}
      <section>
        <span class="group-label">{$t("tools.pinterest.guides")}</span>
        <div class="chips">{#each list.guides as g (g)}<button class="btn btn-secondary btn-sm" type="button" onclick={() => useGuide(g)}>{g}</button>{/each}</div>
      </section>
    {/if}
    <section>
      <span class="group-label">{list.pins.length} {$t("tools.pinterest.results")}{#if list.hidden} · {list.hidden} {$t("tools.pinterest.hidden")}{/if}{#if mode === "related"} · {list.title}{/if}</span>
      <div class="group">
        <div class="group-row">
          <div class="group-row-content"><div class="group-row-sub">{selected.size} {$t("tools.pinterest.selected")}{#if opts.dest} · <span class="mono">{opts.dest}</span>{/if}</div>
            {#if busy === "download" && progress}<div class="progress"><div class="progress-fill" style:width="{pct(progress) ?? 0}%"></div></div>{/if}
          </div>
          <div class="group-row-trailing btn-row">
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => (selected = new Set(list!.pins.map((p) => p.id)))}>{$t("tools.pinterest.select_all")}</button>
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => (selected = new Set())}>{$t("tools.pinterest.clear")}</button>
            <button class="btn btn-ghost btn-sm" type="button" onclick={() => (showOpts = !showOpts)}>⚙</button>
            <button class="btn btn-secondary btn-sm" type="button" disabled={busy !== null || !selected.size} onclick={() => download(false)}>{$t("tools.pinterest.download_selected")}</button>
            <button class="btn btn-primary btn-sm" type="button" disabled={busy !== null || !list.pins.length} onclick={() => download(true)}>{$t("tools.pinterest.download_all")}</button>
          </div>
        </div>
        {#if showOpts}<PinDownloadOptions bind:opts sections={false} />{/if}
        {#if result}
          <div class="group-row">
            <div class="group-row-content"><div class="group-row-sub">{result.downloaded} {$t("tools.pinterest.downloaded")} · {result.skipped} {$t("tools.pinterest.skipped")} · {result.failed.length} {$t("tools.pinterest.failed")}</div></div>
            <div class="group-row-trailing"><button class="btn btn-secondary btn-sm" type="button" onclick={() => reveal(result!.dest)}>{$t("tools.common.reveal")}</button></div>
          </div>
        {/if}
      </div>
      {#if list.pins.length}
        <div class="grid-wrap"><PinGrid pins={list.pins} bind:selected /></div>
      {:else}
        <div class="group"><div class="group-row"><div class="group-row-sub">{$t("tools.pinterest.no_results")}</div></div></div>
      {/if}
    </section>
  {/if}
</div>

<style>
  .tool { display: flex; flex-direction: column; gap: var(--space-5); }
  .mono { font-family: var(--font-mono); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .chips { display: flex; flex-wrap: wrap; gap: var(--space-1); }
  .cover { width: 56px; height: 56px; object-fit: cover; border-radius: var(--radius-md); flex-shrink: 0; }
  .grid-wrap { margin-top: var(--space-3); }
  .segmented-btn.active { background: var(--surface-hi); color: var(--text); }
</style>
