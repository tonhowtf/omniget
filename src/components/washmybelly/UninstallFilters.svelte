<script lang="ts">
  import { t } from "$lib/i18n";
  import type { AppSort } from "$lib/washmybelly/apps";

  /// Search, origin chips (only the origins present, each with its count)
  /// and the sort control above the app list.
  let {
    query = $bindable(""),
    sources = $bindable(new Set<string>()),
    sort = $bindable<AppSort>("name"),
    present,
    sourceLabel,
    shown,
    total,
  }: {
    query?: string;
    sources?: Set<string>;
    sort?: AppSort;
    present: { source: string; count: number }[];
    sourceLabel: (source: string) => string;
    shown: number;
    total: number;
  } = $props();

  function toggle(source: string) {
    const next = new Set(sources);
    if (next.has(source)) next.delete(source);
    else next.add(source);
    sources = next;
  }

  const SORTS: AppSort[] = ["name", "size", "last_used"];
</script>

<div class="wmb-uf">
  <input
    class="input input-search"
    type="search"
    placeholder={$t("washmybelly.uninstall.search") as string}
    aria-label={$t("washmybelly.uninstall.search") as string}
    bind:value={query}
  />

  {#if present.length > 1}
    <div class="chips" role="group" aria-label={$t("washmybelly.uninstall.filter_source") as string}>
      <button type="button" class="chip" class:on={sources.size === 0} aria-pressed={sources.size === 0} onclick={() => (sources = new Set())}>
        {$t("washmybelly.uninstall.all")}
        <span class="count">{total}</span>
      </button>
      {#each present as p (p.source)}
        <button type="button" class="chip" class:on={sources.has(p.source)} aria-pressed={sources.has(p.source)} onclick={() => toggle(p.source)}>
          {sourceLabel(p.source)}
          <span class="count">{p.count}</span>
        </button>
      {/each}
    </div>
  {/if}

  <div class="bar">
    <span class="shown">{$t("washmybelly.uninstall.shown", { count: shown })}</span>
    <div class="segmented" role="radiogroup" aria-label={$t("washmybelly.uninstall.sort") as string}>
      {#each SORTS as s (s)}
        <button type="button" class="segmented-btn" role="radio" aria-checked={sort === s} onclick={() => (sort = s)}>
          {$t(`washmybelly.uninstall.sort_${s}`)}
        </button>
      {/each}
    </div>
  </div>
</div>

<style>
  .wmb-uf {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .wmb-uf .input {
    width: 100%;
    box-sizing: border-box;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    padding: 0 6px 0 10px;
    border: none;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    color: var(--text-muted);
    font: inherit;
    font-size: var(--text-sm);
    font-weight: 500;
    cursor: pointer;
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
    transition: background var(--duration-fast) var(--ease-out), color var(--duration-fast) var(--ease-out);
  }
  @media (hover: hover) {
    .chip:hover:not(.on) {
      color: var(--text);
      background: var(--fill-2);
    }
  }
  .chip.on {
    background: var(--accent);
    color: var(--on-accent);
    box-shadow: none;
  }
  .chip:focus-visible {
    outline: var(--focus-ring);
    outline-offset: 1px;
  }
  .count {
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    box-sizing: border-box;
    display: inline-grid;
    place-items: center;
    border-radius: var(--radius-full);
    background: var(--fill-2);
    font-size: var(--text-xs);
    font-variant-numeric: tabular-nums;
  }
  .chip.on .count {
    background: rgb(255 255 255 / 0.22);
    color: inherit;
  }
  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .shown {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
</style>
