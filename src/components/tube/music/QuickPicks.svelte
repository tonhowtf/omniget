<script lang="ts">
  import SongRow from "./SongRow.svelte";
  import SquareCard from "./SquareCard.svelte";
  import { songFromItem } from "../views/music/shared.svelte";
  import type { Item } from "$lib/tube/types";

  let { items }: { items: Item[] } = $props();
</script>

<div class="qp" role="list">
  {#each items as it, i (i)}
    {@const s = songFromItem(it)}
    <div class="qp-cell" role="listitem">
      {#if s}
        <SongRow song={s} variant="pick" />
      {:else}
        <SquareCard item={it} width={56} />
      {/if}
    </div>
  {/each}
</div>

<style>
  .qp {
    display: grid;
    grid-auto-flow: column;
    grid-template-rows: repeat(4, var(--ytm-row-h, 56px));
    grid-auto-columns: min(var(--ytm-quickpick-col-w, 420px), 86cqi);
    column-gap: 24px;
    row-gap: 8px;
  }
  .qp-cell { min-width: 0; scroll-snap-align: start; }
</style>
