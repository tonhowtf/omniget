<script lang="ts" module>
  import type { PreviewFont } from "$lib/fonts/preview";
  import type { LicenseClass } from "$lib/fonts/types";

  export type ListItem = { font: PreviewFont; installed: boolean; licenseClass: LicenseClass | null; detail: string };
  export type Source = "catalog" | "installed";
</script>

<script lang="ts">
  import { pluralKey } from "$lib/fonts/text";
  import { t } from "$lib/i18n";
  import { CATEGORIES, CATEGORY_KEYS } from "$lib/fonts/catalog";
  import { ensureFont } from "$lib/fonts/loader";
  import { fontStack, MAX_COMPARE } from "$lib/fonts/preview";
  import StatusIcon from "#components/converter/StatusIcon.svelte";

  let {
    source,
    query = $bindable(),
    category = $bindable(),
    includeOther = $bindable(),
    items,
    total,
    hidden,
    status,
    error,
    selectedKey,
    compare,
    lang,
    catalogCount,
    installedCount,
    onsource,
    onselect,
    oncompare,
    onretry,
    onmore,
  }: {
    source: Source;
    query: string;
    category: string | null;
    includeOther: boolean;
    items: ListItem[];
    total: number;
    hidden: number;
    status: "loading" | "ready" | "error";
    error: string | null;
    selectedKey: string | null;
    compare: string[];
    lang: string;
    catalogCount: number | null;
    installedCount: number | null;
    onsource: (s: Source) => void;
    onselect: (key: string) => void;
    oncompare: (key: string) => void;
    onretry: () => void;
    onmore: () => void;
  } = $props();

  let compareFull = $derived(compare.length >= MAX_COMPARE);

  function lazy(node: HTMLElement, font: PreviewFont) {
    let current = font;
    if (current.source !== "catalog" || typeof IntersectionObserver === "undefined") return {};
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          void ensureFont(current, 400, false, lang);
          observer.disconnect();
        }
      },
      { rootMargin: "120px" },
    );
    observer.observe(node);
    return {
      update(next: PreviewFont) {
        current = next;
      },
      destroy() {
        observer.disconnect();
      },
    };
  }
</script>

<div class="ty-list">
  <div class="segmented ty-list-source" role="group" aria-label={$t("fonts.source_label") as string}>
    <button type="button" class="segmented-btn" class:active={source === "catalog"} aria-pressed={source === "catalog"} onclick={() => onsource("catalog")}>
      {$t("fonts.source_catalog")}
      {#if catalogCount !== null}<span class="ty-list-count">{catalogCount}</span>{/if}
    </button>
    <button type="button" class="segmented-btn" class:active={source === "installed"} aria-pressed={source === "installed"} onclick={() => onsource("installed")}>
      {$t("fonts.source_installed")}
      {#if installedCount !== null}<span class="ty-list-count">{installedCount}</span>{/if}
    </button>
  </div>

  <input
    class="input input-search ty-list-search"
    type="search"
    placeholder={(source === "catalog" ? $t("fonts.search_catalog") : $t("fonts.search_installed")) as string}
    aria-label={$t("fonts.search_label") as string}
    bind:value={query}
  />

  <div class="ty-list-cats" role="group" aria-label={$t("fonts.category_label") as string}>
    <button type="button" class="btn-pill" aria-pressed={category === null} onclick={() => (category = null)}>{$t("fonts.cat_all")}</button>
    {#each CATEGORIES as cat (cat)}
      {#if source === "catalog" || cat === "monospace" || cat === "serif" || cat === "sans-serif"}
        <button type="button" class="btn-pill" aria-pressed={category === cat} onclick={() => (category = category === cat ? null : cat)}>{$t(CATEGORY_KEYS[cat])}</button>
      {/if}
    {/each}
  </div>

  {#if source === "catalog"}
    <label class="ty-list-other">
      <input type="checkbox" class="checkbox" bind:checked={includeOther} />
      <span>{$t("fonts.include_other")}</span>
    </label>
  {/if}

  {#if status === "error"}
    <div class="ty-list-state" role="alert">
      <StatusIcon name="warning" />
      <p>{error}</p>
      <button type="button" class="btn btn-secondary btn-sm" onclick={onretry}>{$t("fonts.retry")}</button>
    </div>
  {/if}

  {#if status === "loading" && items.length === 0}
    <div class="ty-list-state">
      <span class="spinner" aria-hidden="true"></span>
      <p>{source === "catalog" ? $t("fonts.loading_catalog") : $t("fonts.loading_installed")}</p>
    </div>
  {:else if items.length === 0 && status !== "error"}
    <div class="ty-list-state">
      <p>{query ? $t("fonts.no_results", { query }) : $t("fonts.no_fonts")}</p>
      {#if hidden > 0}
        <button type="button" class="btn btn-ghost btn-sm" onclick={() => (includeOther = true)}>{$t(pluralKey("fonts.show_hidden", hidden), { count: hidden })}</button>
      {/if}
    </div>
  {:else}
    <ul class="ty-list-items" aria-busy={status === "loading"}>
      {#each items as item (item.font.key)}
        {@const inCompare = compare.includes(item.font.key)}
        <li class="ty-row" class:selected={selectedKey === item.font.key}>
          <button type="button" class="ty-row-main" aria-current={selectedKey === item.font.key ? "true" : undefined} onclick={() => onselect(item.font.key)}>
            <span class="ty-row-name" use:lazy={item.font} style:font-family={fontStack(item.font)}>{item.font.family}</span>
            <span class="ty-row-meta">
              <span>{item.detail}</span>
              {#if item.licenseClass === "other"}
                <span class="tag tag-warning">{$t("fonts.license_other_short")}</span>
              {/if}
              {#if item.installed && item.font.source === "catalog"}
                <span class="tag tag-success">{$t("fonts.installed_tag")}</span>
              {/if}
            </span>
          </button>
          <button
            type="button"
            class="btn-icon btn-sm ty-row-add"
            aria-pressed={inCompare}
            disabled={!inCompare && compareFull}
            title={(inCompare ? $t("fonts.compare_remove", { name: item.font.family }) : compareFull ? $t("fonts.compare_full") : $t("fonts.compare_add", { name: item.font.family })) as string}
            aria-label={(inCompare ? $t("fonts.compare_remove", { name: item.font.family }) : $t("fonts.compare_add", { name: item.font.family })) as string}
            onclick={() => oncompare(item.font.key)}
          >
            <svg viewBox="0 0 16 16" width="14" height="14" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true">
              {#if inCompare}<path d="m4 8.2 2.6 2.6L12 5.4" />{:else}<path d="M8 3.5v9M3.5 8h9" />{/if}
            </svg>
          </button>
        </li>
      {/each}
    </ul>
    {#if total > items.length}
      <button type="button" class="btn btn-ghost btn-sm ty-list-more" onclick={onmore}>{$t("fonts.show_more", { shown: items.length, total })}</button>
    {/if}
    {#if hidden > 0}
      <p class="ty-list-hint">{$t(pluralKey("fonts.hidden_other", hidden), { count: hidden })}</p>
    {/if}
  {/if}
</div>

<style>
  .ty-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }

  .ty-list-source {
    display: flex;
    align-self: stretch;
    min-width: 0;
  }

  .ty-list-source .segmented-btn {
    flex: 1 1 0;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .ty-list-count {
    font-size: var(--text-caption);
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
  }

  .ty-list-search {
    width: 100%;
  }

  .ty-list-cats {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .ty-list-other {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
    color: var(--text-muted);
  }

  .ty-list-state {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-6) var(--space-3);
    text-align: center;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ty-list-state p {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  .ty-list-items {
    margin: 0;
    padding: var(--space-1);
    list-style: none;
    max-height: min(62vh, 640px);
    overflow-y: auto;
    border-radius: var(--radius-lg);
    background: var(--surface);
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .ty-row {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    border-radius: var(--radius-md);
  }

  .ty-row.selected {
    background: var(--fill-2);
  }

  .ty-row-main {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border: 0;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text);
    text-align: left;
    cursor: pointer;
  }

  @media (hover: hover) {
    .ty-row:not(.selected) .ty-row-main:hover {
      background: var(--fill-1);
    }
  }

  .ty-row-main:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
  }

  .ty-row-name {
    overflow: hidden;
    font-size: var(--text-lg);
    line-height: 1.25;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .ty-row-meta {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .ty-row-add {
    flex-shrink: 0;
    margin-right: var(--space-1);
  }

  .ty-list-more {
    align-self: center;
  }

  .ty-list-hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
</style>
