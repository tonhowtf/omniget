<script lang="ts">
  /**
   * Catalog sources: the built-in mirror, user sources (GitHub repo/path,
   * local folder, Claude plugin marketplace) with add / rescan / remove, and
   * the curated plugin marketplaces with "fetch now".
   */
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { openUrl } from "@tauri-apps/plugin-opener";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    compactNumber,
    errText,
    marketplaceLoad,
    marketplacesList,
    sourcesAdd,
    sourcesList,
    sourcesRemove,
    sourcesRescan,
    type CatalogMeta,
    type MarketplaceInfo,
    type UserSource,
  } from "$lib/central/catalog";

  let {
    meta = null,
    onclose,
    onchanged,
    onbrowse,
  }: {
    meta?: CatalogMeta | null;
    onclose?: () => void;
    onchanged?: () => void;
    /** Show the plugins of one marketplace in the catalog. */
    onbrowse?: (repo: string) => void;
  } = $props();

  let tab = $state<"sources" | "marketplaces">("sources");
  let sources = $state<UserSource[]>([]);
  let markets = $state<MarketplaceInfo[]>([]);
  let spec = $state("");
  let kind = $state<"" | "git" | "local" | "marketplace">("");
  let busy = $state<string | null>(null);
  let confirmRemove = $state<string | null>(null);
  let mfilter = $state("");

  let shownMarkets = $derived(
    mfilter.trim()
      ? markets.filter((m) => `${m.repo} ${m.name} ${m.description ?? ""}`.toLowerCase().includes(mfilter.trim().toLowerCase()))
      : markets,
  );

  async function load() {
    try {
      [sources, markets] = await Promise.all([sourcesList(), marketplacesList()]);
      markets = [...markets].sort((a, b) => (b.stars ?? 0) - (a.stars ?? 0));
    } catch (e) {
      showToast("error", errText(e));
    }
  }

  onMount(load);

  async function add() {
    const s = spec.trim();
    if (!s) return;
    busy = "add";
    try {
      const src = await sourcesAdd(s, kind || null);
      spec = "";
      showToast(
        src.error ? "error" : "success",
        src.error ?? $t("llm.central.catalog.sources.added", { count: String(src.items) }),
      );
      await load();
      onchanged?.();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
    }
  }

  async function pickFolder() {
    try {
      const r = await openDialog({ directory: true, multiple: false });
      if (typeof r === "string") {
        spec = r;
        kind = "local";
      }
    } catch {
      /* closed */
    }
  }

  async function rescan(id: string) {
    busy = id;
    try {
      const src = await sourcesRescan(id);
      showToast(src.error ? "error" : "success", src.error ?? $t("llm.central.catalog.sources.rescanned", { count: String(src.items) }));
      await load();
      onchanged?.();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
    }
  }

  async function remove(id: string) {
    confirmRemove = null;
    busy = id;
    try {
      await sourcesRemove(id);
      await load();
      onchanged?.();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
    }
  }

  async function fetchMarket(repo: string) {
    busy = repo;
    try {
      const r = await marketplaceLoad(repo);
      showToast("success", $t("llm.central.catalog.sources.market_loaded", { count: String(r.items.length), name: r.info.name || repo }));
      await load();
      onchanged?.();
    } catch (e) {
      showToast("error", errText(e));
    } finally {
      busy = null;
    }
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape") onclose?.();
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="overlay" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose?.()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="src-title">
    <header class="head">
      <h2 id="src-title">{$t("llm.central.catalog.sources.title")}</h2>
      <button type="button" class="x" onclick={() => onclose?.()} aria-label={$t("llm.central.catalog.close")}>✕</button>
    </header>
    <div class="tabs" role="tablist">
      <button type="button" role="tab" aria-selected={tab === "sources"} onclick={() => (tab = "sources")}>{$t("llm.central.catalog.sources.tab_sources")}</button>
      <button type="button" role="tab" aria-selected={tab === "marketplaces"} onclick={() => (tab = "marketplaces")}>{$t("llm.central.catalog.sources.tab_marketplaces")} ({markets.length})</button>
    </div>
    <div class="body">
      {#if tab === "sources"}
        {#if meta?.source}
          <div class="builtin">
            <strong>{meta.source.name}</strong>
            <span class="muted">{meta.source.repo} @ {meta.source.commit?.slice(0, 7)} · {meta.total} · {meta.source.license}</span>
            <p class="muted small">{meta.source.attribution}</p>
          </div>
        {/if}
        <form class="add" onsubmit={(e) => { e.preventDefault(); void add(); }}>
          <label class="lbl" for="src-spec">{$t("llm.central.catalog.sources.add_label")}</label>
          <div class="row">
            <input id="src-spec" bind:value={spec} placeholder={$t("llm.central.catalog.sources.add_placeholder")} />
            <select bind:value={kind} aria-label={$t("llm.central.catalog.sources.kind")}>
              <option value="">{$t("llm.central.catalog.sources.kind_auto")}</option>
              <option value="git">GitHub</option>
              <option value="local">{$t("llm.central.catalog.sources.kind_local")}</option>
              <option value="marketplace">{$t("llm.central.catalog.sources.kind_marketplace")}</option>
            </select>
            <button type="button" class="btn" onclick={pickFolder}>{$t("llm.central.catalog.project.browse")}</button>
            <button type="submit" class="btn primary" disabled={busy === "add" || !spec.trim()}>
              {busy === "add" ? $t("llm.central.catalog.sources.adding") : $t("llm.central.catalog.sources.add")}
            </button>
          </div>
          <p class="muted small">{$t("llm.central.catalog.sources.add_hint")}</p>
        </form>
        {#if sources.length}
          <ul class="list">
            {#each sources as s (s.id)}
              <li>
                <div class="info">
                  <strong>{s.repo ?? s.spec}</strong>
                  <span class="muted small">
                    {s.kind}{s.path ? ` · ${s.path}` : ""}{s.commit ? ` @ ${s.commit.slice(0, 7)}` : ""} ·
                    {$t("llm.central.catalog.sources.items", { count: String(s.items) })}
                    {#if s.scanned} · {new Date(s.scanned).toLocaleString()}{/if}
                  </span>
                  {#if s.error}<span class="err small">{s.error}</span>{/if}
                </div>
                {#if confirmRemove === s.id}
                  <button type="button" class="btn danger" onclick={() => remove(s.id)}>{$t("llm.central.catalog.sources.remove")}</button>
                  <button type="button" class="btn" onclick={() => (confirmRemove = null)}>{$t("llm.central.catalog.cancel")}</button>
                {:else}
                  <button type="button" class="btn" onclick={() => rescan(s.id)} disabled={busy === s.id}>{$t("llm.central.catalog.sources.rescan")}</button>
                  <button type="button" class="btn" onclick={() => (confirmRemove = s.id)} disabled={busy === s.id}>{$t("llm.central.catalog.sources.remove")}</button>
                {/if}
              </li>
            {/each}
          </ul>
        {:else}
          <p class="muted">{$t("llm.central.catalog.sources.none")}</p>
        {/if}
      {:else}
        <input class="mfilter" bind:value={mfilter} placeholder={$t("llm.central.catalog.filter")} aria-label={$t("llm.central.catalog.filter")} />
        <ul class="list">
          {#each shownMarkets as m (m.repo)}
            <li>
              <div class="info">
                <strong>{m.name || m.repo}</strong>
                <span class="muted small">
                  {m.repo}{m.stars ? ` · ★ ${compactNumber(m.stars)}` : ""} · {$t("llm.central.catalog.sources.plugins", { count: String(m.plugins) })}{m.license ? ` · ${m.license}` : ""}
                </span>
                {#if m.description}<span class="small">{m.description}</span>{/if}
              </div>
              {#if m.url}<button type="button" class="btn" onclick={() => m.url && openUrl(m.url)}>GitHub</button>{/if}
              <button type="button" class="btn" onclick={() => fetchMarket(m.repo)} disabled={busy === m.repo}>
                {busy === m.repo ? $t("llm.central.catalog.sources.fetching") : $t("llm.central.catalog.sources.fetch")}
              </button>
              {#if onbrowse}
                <button type="button" class="btn primary" onclick={() => onbrowse?.(m.repo)}>{$t("llm.central.catalog.sources.browse")}</button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  </div>
</div>

<style>
  .overlay {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--dialog-backdrop);
  }
  .dialog {
    width: min(820px, 95vw);
    max-height: 88vh;
    display: flex;
    flex-direction: column;
    background: var(--popup-bg);
    border-radius: var(--radius-xl);
    box-shadow: var(--elev-3);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-4) var(--space-5) var(--space-2);
  }
  h2 {
    margin: 0;
    font-size: var(--text-lg);
  }
  .x {
    border: none;
    background: var(--fill-1);
    width: 28px;
    height: 28px;
    border-radius: var(--radius-full);
    color: var(--text);
    cursor: pointer;
  }
  .tabs {
    display: flex;
    gap: var(--space-1);
    padding: 0 var(--space-5);
    border-bottom: 1px solid var(--separator);
  }
  .tabs button {
    padding: 6px 10px;
    border: none;
    background: none;
    color: var(--text-muted);
    font-size: var(--text-sm);
    border-bottom: 2px solid transparent;
    cursor: pointer;
  }
  .tabs button[aria-selected="true"] {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .body {
    flex: 1;
    overflow: auto;
    padding: var(--space-3) var(--space-5) var(--space-5);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .builtin {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
  }
  .muted {
    color: var(--text-muted);
  }
  .small {
    font-size: var(--text-xs);
    margin: 0;
  }
  .err {
    color: var(--error);
  }
  .add .row {
    display: flex;
    gap: var(--space-1);
    flex-wrap: wrap;
  }
  .lbl {
    display: block;
    margin-bottom: 4px;
    font-size: var(--text-sm);
    font-weight: 600;
  }
  input,
  select {
    height: var(--control-h-lg);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-md);
    background: var(--input-bg);
    color: var(--text);
    font-size: var(--text-base);
  }
  #src-spec {
    flex: 1;
    min-width: 220px;
  }
  .mfilter {
    width: 100%;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .list li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface);
  }
  .info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .btn {
    height: var(--control-h);
    padding: 0 var(--space-3);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--fill-1);
    color: var(--text);
    font-size: var(--text-sm);
    cursor: pointer;
    white-space: nowrap;
  }
  .add .btn {
    height: var(--control-h-lg);
  }
  .btn:hover:not(:disabled) {
    background: var(--fill-2);
  }
  .btn.primary {
    background: var(--cta);
    color: var(--on-cta);
  }
  .btn.danger {
    background: var(--error);
    color: var(--on-error);
  }
  .btn:disabled {
    opacity: 0.5;
  }
</style>
