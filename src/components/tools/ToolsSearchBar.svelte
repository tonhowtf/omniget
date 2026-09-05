<script lang="ts">
  /**
   * Busca + filtro de plataforma do hub e das páginas de categoria.
   * O filtro é persistido: quem está no Windows e escolheu "Multiplataforma"
   * não precisa escolher de novo a cada visita.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import type { PlatformFilter } from "$lib/tools/catalog";

  let {
    query = $bindable(""),
    filter = $bindable<PlatformFilter>("all"),
    autofocus = false,
    resultCount,
  }: { query?: string; filter?: PlatformFilter; autofocus?: boolean; resultCount?: number } = $props();

  const FILTER_KEY = "omniget.tools.platform_filter";
  const FILTERS: { id: PlatformFilter; key: string }[] = [
    { id: "all", key: "tools.hub.filter_all" },
    { id: "cross", key: "tools.hub.filter_cross" },
    { id: "windows", key: "tools.hub.filter_windows" },
    { id: "macos", key: "tools.hub.filter_macos" },
    { id: "linux", key: "tools.hub.filter_linux" },
  ];

  let input = $state<HTMLInputElement | null>(null);

  onMount(() => {
    try {
      const saved = localStorage.getItem(FILTER_KEY) as PlatformFilter | null;
      if (saved && FILTERS.some((f) => f.id === saved)) filter = saved;
    } catch {}
    if (autofocus) input?.focus();
  });

  function pick(id: PlatformFilter) {
    filter = id;
    try {
      localStorage.setItem(FILTER_KEY, id);
    } catch {}
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape" && query) {
      e.preventDefault();
      query = "";
    }
  }
</script>

<div class="tools-bar">
  <label class="tools-search">
    <span class="sr-only">{$t("tools.hub.search_placeholder")}</span>
    <input
      bind:this={input}
      class="input input-search input-lg tools-search-input"
      type="search"
      placeholder={$t("tools.hub.search_placeholder")}
      bind:value={query}
      onkeydown={onKey}
      autocomplete="off"
      spellcheck="false"
    />
    {#if query && resultCount !== undefined}
      <span class="tools-search-count">{resultCount}</span>
    {/if}
  </label>

  <div class="segmented tools-filter" role="radiogroup" aria-label={$t("tools.hub.filter_label")}>
    {#each FILTERS as f (f.id)}
      <button
        type="button"
        class="segmented-btn"
        class:active={filter === f.id}
        role="radio"
        aria-checked={filter === f.id}
        onclick={() => pick(f.id)}
      >
        {$t(f.key)}
      </button>
    {/each}
  </div>
</div>

<style>
  .tools-bar {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .tools-search {
    position: relative;
    flex: 1 1 280px;
    min-width: 0;
  }

  .tools-search-input {
    width: 100%;
  }

  .tools-search-count {
    position: absolute;
    right: 10px;
    top: 50%;
    transform: translateY(-50%);
    min-width: 20px;
    padding: 0 6px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    color: var(--text-muted);
    font-size: var(--text-xs);
    font-weight: 600;
    line-height: 20px;
    text-align: center;
    pointer-events: none;
  }

  .tools-filter {
    flex-shrink: 0;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
  }

  @media (max-width: 720px) {
    .tools-filter {
      width: 100%;
      overflow-x: auto;
    }
  }
</style>
