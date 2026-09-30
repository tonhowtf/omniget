<script lang="ts">
  // Loading placeholders (21st "Skeleton media") in the shape of what is
  // coming: grid cards, 9:16 shorts, search rows or compact "up next" rows.
  import Grid from "./Grid.svelte";

  let {
    layout = "grid",
    count = 12,
    showAvatar = true,
  }: { layout?: "grid" | "shorts" | "rows" | "compact" | "list"; count?: number; showAvatar?: boolean } = $props();
</script>

<div class="skel" data-agent-readiness="loading" aria-busy="true">
  {#if layout === "grid" || layout === "shorts"}
    <Grid items={[]} shorts={layout === "shorts"} skeleton={count} {showAvatar} />
  {:else}
    <div class="rows {layout}">
      {#each Array(count) as _, i (i)}
        <div class="row">
          <div class="yt-skel thumb"></div>
          <div class="lines">
            <div class="yt-skel l1"></div>
            <div class="yt-skel l2"></div>
            {#if layout === "rows"}
              <div class="yt-skel l3"></div>
              <div class="yt-skel l4"></div>
            {/if}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .rows {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .rows.compact {
    gap: 8px;
  }
  .row {
    display: flex;
    gap: 16px;
  }
  .compact .row {
    gap: 8px;
  }
  .thumb {
    flex: none;
    aspect-ratio: 16 / 9;
    width: min(360px, 45%);
    border-radius: var(--yt-radius-thumb-lg);
  }
  .compact .thumb {
    width: 168px;
    border-radius: var(--yt-radius-thumb-md);
  }
  .list .thumb {
    width: 160px;
    border-radius: var(--yt-radius-thumb-md);
  }
  .lines {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 4px;
  }
  .l1 {
    height: 20px;
    width: 85%;
  }
  .compact .l1 {
    height: 16px;
  }
  .l2 {
    height: 14px;
    width: 50%;
  }
  .l3 {
    height: 24px;
    width: 30%;
    border-radius: 12px;
    margin-top: 8px;
  }
  .l4 {
    height: 14px;
    width: 70%;
  }
</style>
