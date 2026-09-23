<script lang="ts">
  /** One facet (category, source, licence, origin tool): checkboxes with counts, first 8 then "more". */
  import { t } from "$lib/i18n";
  import type { FacetValue } from "$lib/central/catalog";

  let {
    title,
    values = [],
    selected = [],
    label = (v: string) => v,
    onchange,
    limit = 8,
  }: {
    title: string;
    values?: FacetValue[];
    selected?: string[];
    label?: (v: string) => string;
    onchange?: (next: string[]) => void;
    limit?: number;
  } = $props();

  let expanded = $state(false);
  let filter = $state("");
  let list = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    const base = q ? values.filter((v) => label(v.value).toLowerCase().includes(q)) : values;
    // keep selected values visible even with a zero count
    const extra = selected.filter((s) => !base.some((v) => v.value === s)).map((value) => ({ value, count: 0 }));
    return [...base, ...extra];
  });
  let shown = $derived(expanded ? list : list.slice(0, limit));

  function toggle(v: string) {
    onchange?.(selected.includes(v) ? selected.filter((x) => x !== v) : [...selected, v]);
  }
</script>

{#if values.length || selected.length}
  <fieldset class="facet">
    <legend>
      <span>{title}</span>
      {#if selected.length}<button type="button" class="clear" onclick={() => onchange?.([])}>{$t("llm.central.catalog.clear")}</button>{/if}
    </legend>
    {#if expanded && values.length > 12}
      <input class="filter" bind:value={filter} placeholder={$t("llm.central.catalog.filter")} aria-label={`${title}: ${$t("llm.central.catalog.filter")}`} />
    {/if}
    {#each shown as v (v.value)}
      <label class="opt" class:zero={v.count === 0}>
        <input type="checkbox" checked={selected.includes(v.value)} onchange={() => toggle(v.value)} />
        <span class="lb" title={label(v.value)}>{label(v.value)}</span>
        <span class="ct tabular">{v.count}</span>
      </label>
    {/each}
    {#if list.length > limit}
      <button type="button" class="more" onclick={() => (expanded = !expanded)}>
        {expanded ? $t("llm.central.catalog.show_less") : $t("llm.central.catalog.show_more", { count: String(list.length - limit) })}
      </button>
    {/if}
  </fieldset>
{/if}

<style>
  .facet {
    border: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  legend {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    padding: 0 0 var(--space-1);
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: var(--track-caps);
  }
  .clear,
  .more {
    border: none;
    background: none;
    padding: 0;
    color: var(--accent-hi);
    font-size: var(--text-xs);
    cursor: pointer;
    text-transform: none;
    letter-spacing: 0;
    text-align: left;
  }
  .more {
    padding: 2px var(--space-1);
  }
  .filter {
    height: 24px;
    margin-bottom: 4px;
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: var(--input-bg);
    color: var(--text);
    font-size: var(--text-xs);
  }
  .opt {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    padding: 2px var(--space-1);
    border-radius: var(--radius-xs);
    font-size: var(--text-sm);
    cursor: pointer;
  }
  .opt:hover {
    background: var(--fill-1);
  }
  .opt.zero {
    opacity: 0.5;
  }
  .opt input {
    accent-color: var(--accent);
    margin: 0;
  }
  .lb {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ct {
    color: var(--text-faint);
    font-size: var(--text-xs);
  }
</style>
