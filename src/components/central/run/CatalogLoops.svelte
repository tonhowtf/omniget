<script lang="ts">
  /** The catalog's loops (goal + interval + stop check + budget), each runnable as an OmniGet Loop. */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { catalogSearch, itemHref, type SearchHit } from "$lib/central/catalog";
  import RunAsLoop from "./RunAsLoop.svelte";

  let { onstarted }: { onstarted?: (id: string) => void } = $props();

  let items = $state<SearchHit[]>([]);
  let query = $state("");
  let error = $state("");

  onMount(() => void load());

  async function load() {
    try {
      const r = await catalogSearch({ query, filters: { kinds: ["loop"] }, sort: "name", limit: 60 });
      items = r.items;
      error = "";
    } catch (e) {
      error = String(e);
    }
  }
</script>

<details class="creation-panel">
  <summary>{$t("llm.central.run.loop.from_catalog", { count: String(items.length) })}</summary>
  <div class="list">
    <input class="input" type="search" placeholder={$t("llm.central.run.loop.search")} bind:value={query} oninput={() => void load()} />
    {#if error}<p class="hint">{error}</p>{/if}
    {#each items as it (it.id)}
      <div class="item">
        <div class="text">
          <a class="name" href={itemHref(it.id)}>{it.name}</a>
          <span class="dim">{it.category}</span>
          <p class="desc">{it.description}</p>
        </div>
        <RunAsLoop itemId={it.id} itemName={it.name} {onstarted} />
      </div>
    {/each}
  </div>
</details>

<style>
  .creation-panel {
    margin-bottom: 24px;
  }
  .creation-panel summary {
    cursor: pointer;
    padding: 14px 16px;
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    font-weight: 600;
  }
  .creation-panel[open] summary {
    margin-bottom: 12px;
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .item {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }
  .text {
    flex: 1;
    min-width: 0;
  }
  .name {
    font-weight: 600;
    color: var(--text);
    text-decoration: none;
  }
  .dim {
    margin-left: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .desc {
    margin: 2px 0 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
</style>
