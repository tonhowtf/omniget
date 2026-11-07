<script lang="ts">
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import type { Item } from "$lib/tube/types";
  import ChannelRow from "./ChannelRow.svelte";
  import PlaylistCard from "./PlaylistCard.svelte";
  import ShortCard from "./ShortCard.svelte";
  import VideoCard from "./VideoCard.svelte";
  import { isVisible } from "./feedback.svelte";

  let {
    items,
    shorts = false,
    showAvatar = true,
    feedbackActions = true,
    skeleton = 0,
  }: {
    items: Item[];
    shorts?: boolean;
    showAvatar?: boolean;
    feedbackActions?: boolean;
    skeleton?: number;
  } = $props();

  let forced = $derived(!shorts ? tubePrefs().grid_columns : 0);
  let shown = $derived(items.filter(isVisible));
</script>

<div class="wrap">
  <div class="grid" class:shorts class:forced={forced > 0} style:--force={forced || undefined}>
    {#each shown as it, i (("id" in it ? it.id : "") + ":" + i)}
      {#if it.kind === "video"}
        <VideoCard item={it} {showAvatar} {feedbackActions} />
      {:else if it.kind === "short"}
        <ShortCard item={it} {feedbackActions} />
      {:else if it.kind === "playlist"}
        <PlaylistCard item={it} />
      {:else if it.kind === "channel"}
        <ChannelRow item={it} layout="card" />
      {/if}
    {/each}
    {#each Array(skeleton) as _, i (i)}
      <div class="sk" aria-hidden="true">
        <div class="yt-skel thumb"></div>
        <div class="sk-details">
          {#if showAvatar && !shorts}<div class="yt-skel av"></div>{/if}
          <div class="sk-lines">
            <div class="yt-skel l1"></div>
            <div class="yt-skel l2"></div>
          </div>
        </div>
      </div>
    {/each}
  </div>
</div>

<style>
  .wrap {
    container: ytgrid / inline-size;
  }
  .grid {
    --cols: 1;
    display: grid;
    grid-template-columns: repeat(var(--cols), minmax(0, 1fr));
    column-gap: var(--yt-grid-gap-x);
    row-gap: var(--yt-grid-gap-y);
  }
  @container ytgrid (min-width: 596px) {
    .grid {
      --cols: 2;
    }
  }
  @container ytgrid (min-width: 902px) {
    .grid {
      --cols: 3;
    }
  }
  @container ytgrid (min-width: 1208px) {
    .grid {
      --cols: 4;
    }
  }
  @container ytgrid (min-width: 2200px) {
    .grid {
      --cols: 5;
    }
  }
  @container ytgrid (min-width: 2600px) {
    .grid {
      --cols: 6;
    }
  }
  .grid.forced {
    --cols: var(--force);
  }
  @container ytgrid (max-width: 595px) {
    .grid.forced {
      --cols: 1;
    }
  }
  .grid.shorts {
    --cols: 2;
    row-gap: 24px;
  }
  @container ytgrid (min-width: 560px) {
    .grid.shorts {
      --cols: 3;
    }
  }
  @container ytgrid (min-width: 760px) {
    .grid.shorts {
      --cols: 4;
    }
  }
  @container ytgrid (min-width: 960px) {
    .grid.shorts {
      --cols: 5;
    }
  }
  @container ytgrid (min-width: 1160px) {
    .grid.shorts {
      --cols: 6;
    }
  }
  .sk {
    min-width: 0;
  }
  .thumb {
    aspect-ratio: 16 / 9;
    border-radius: var(--yt-radius-thumb-lg);
  }
  .shorts .thumb {
    aspect-ratio: 9 / 16;
  }
  .sk-details {
    display: flex;
    gap: 12px;
    margin-top: 12px;
  }
  .av {
    flex: none;
    width: 36px;
    height: 36px;
    border-radius: 50%;
  }
  .sk-lines {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 2px;
  }
  .l1 {
    height: 20px;
    width: 90%;
  }
  .l2 {
    height: 16px;
    width: 60%;
  }
</style>
