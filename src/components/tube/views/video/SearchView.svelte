<script lang="ts">
  import { t } from "$lib/i18n";
  import * as api from "$lib/tube/api";
  import type { SearchFilter } from "$lib/tube/api";
  import { go, type Route } from "$lib/tube/ui.svelte";
  import ChipBar from "../../cards/ChipBar.svelte";
  import Sentinel from "../../cards/Sentinel.svelte";
  import Shelf from "../../cards/Shelf.svelte";
  import SkeletonGrid from "../../cards/SkeletonGrid.svelte";
  import State from "../../cards/State.svelte";
  import UndoPill from "../../cards/UndoPill.svelte";
  import { Pager } from "../../cards/pager.svelte";

  let { route }: { route: Extract<Route, { name: "search" }> } = $props();

  const FILTERS: SearchFilter[] = ["all", "video", "short", "channel", "playlist", "live"];

  let filter = $derived(route.filter ?? "all");
  const pager = new Pager("search", () => api.search(route.q, filter));

  $effect(() => {
    const q = route.q;
    const f = filter;
    void pager.load(() => api.search(q, f));
  });

  let chips = $derived(FILTERS.map((f) => ({ key: f, label: $t(`tube.search.filter_${f}`) as string, selected: f === filter })));

  function pick(key: string) {
    go({ name: "search", q: route.q, filter: key as SearchFilter }, { replace: true });
  }

  let empty = $derived(!!pager.page && pager.shelves.every((s) => s.items.length === 0));
</script>

<div class="search" data-agent-readiness={pager.loading ? "loading" : pager.error ? "error" : "ready"} data-agent-surface="omnitube-search">
  <div class="inner">
    <ChipBar {chips} onselect={pick} sticky label={$t("tube.search.filters") as string} />

    {#if pager.page?.corrected_query}
      <p class="corrected">
        {$t("tube.search.showing_for")}
        <button class="linkish" onclick={() => go({ name: "search", q: pager.page!.corrected_query!, filter }, { replace: true })}>
          <b><i>{pager.page.corrected_query}</i></b>
        </button>
      </p>
    {/if}

    <div class="results">
      {#if pager.loading && !pager.page}
        <SkeletonGrid layout="rows" count={8} />
      {:else if pager.error && !pager.page}
        <State icon="explore" error={pager.error} onretry={() => pager.load()} />
      {:else if empty}
        <State icon="explore" title={$t("tube.search.empty_title") as string} text={$t("tube.search.empty_text", { query: route.q }) as string} />
      {:else}
        {#each pager.shelves as s, i (s.id)}
          <Shelf shelf={s} first={i === 0} listSize="search" />
        {/each}
        {#if pager.hasMore}
          <Sentinel onmore={() => pager.more()} loading={pager.loadingMore} error={pager.moreError} />
        {/if}
      {/if}
    </div>
  </div>
  <UndoPill />
</div>

<style>
  .search {
    padding: 0 var(--yt-grid-gutter) 24px;
  }
  .inner {
    max-width: 1096px;
    margin: 0 auto;
  }
  .corrected {
    margin: 8px 0 0;
    font: var(--yt-type-meta);
    color: var(--yt-text-2);
  }
  .linkish {
    padding: 0;
    border: 0;
    font: inherit;
    color: var(--yt-link);
    background: none;
    cursor: pointer;
  }
  .results {
    display: flex;
    flex-direction: column;
    gap: 24px;
    padding-top: 16px;
  }
  .results > :global(.shelf.list + .shelf.list) {
    margin-top: 0;
  }
  @container omnitube (max-width: 640px) {
    .search {
      padding: 0 12px 24px;
    }
  }
</style>
