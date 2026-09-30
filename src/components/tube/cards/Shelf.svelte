<script lang="ts">
  // One `Shelf` of a Page, drawn by its kind: grid, horizontal row (with
  // arrows and fades), Shorts row, or a vertical list.
  import { t } from "$lib/i18n";
  import { bestAvatar } from "$lib/tube/format";
  import { go } from "$lib/tube/ui.svelte";
  import { patchTubePrefs } from "$lib/tube/prefs.svelte";
  import type { Shelf } from "$lib/tube/types";
  import TIcon from "../TIcon.svelte";
  import ChannelRow from "./ChannelRow.svelte";
  import Grid from "./Grid.svelte";
  import PlaylistCard from "./PlaylistCard.svelte";
  import Scroller from "./Scroller.svelte";
  import ShortCard from "./ShortCard.svelte";
  import VideoCard from "./VideoCard.svelte";
  import VideoRow from "./VideoRow.svelte";
  import { feedback, isVisible } from "./feedback.svelte";
  import { routeForNav } from "./util";

  let {
    shelf,
    showAvatar = true,
    feedbackActions = true,
    listSize = "search",
    first = false,
  }: {
    shelf: Shelf;
    showAvatar?: boolean;
    feedbackActions?: boolean;
    listSize?: "search" | "list" | "compact";
    /** First shelf of the page: no top divider. */
    first?: boolean;
  } = $props();

  let items = $derived(shelf.items.filter(isVisible));
  let more = $derived(routeForNav(shelf.more));
  let isShorts = $derived(shelf.kind === "shorts" || (items.length > 0 && items.every((i) => i.kind === "short")));
  let kind = $derived(isShorts ? "shorts" : shelf.kind === "carousel" || shelf.kind === "quick_picks" ? "row" : shelf.kind);
  let titled = $derived(!!shelf.title || isShorts);
  // Shelves built on our side (local library, home without an account) carry an i18n key.
  let title = $derived(shelf.title?.startsWith("tube.") ? ($t(shelf.title) as string) : shelf.title);

  function hideShorts() {
    void patchTubePrefs({ hide_shorts: true });
    feedback.undo = {
      id: Date.now(),
      text: $t("tube.shelf.shorts_hidden") as string,
      undo: () => patchTubePrefs({ hide_shorts: false }),
      commit: () => {},
    };
  }
</script>

{#if items.length}
  <section class="shelf {kind}" class:titled class:first>
    {#if titled}
      <header>
        {#if shelf.thumb}
          <img class="yt-avatar" src={bestAvatar(shelf.thumb, 32)} width="32" height="32" alt="" loading="lazy" referrerpolicy="no-referrer" />
        {:else if isShorts}
          <span class="shorts-ic"><TIcon name="shorts-fill" size={24} /></span>
        {/if}
        <div class="ht">
          {#if shelf.strapline}<div class="strap">{shelf.strapline}</div>{/if}
          <h2 class="yt-shelf-title">{title || $t("tube.shelf.shorts")}</h2>
          {#if shelf.subtitle}<div class="sub">{shelf.subtitle}</div>{/if}
        </div>
        <div class="hact">
          {#if more}
            <button class="yt-btn yt-btn--ghost" onclick={() => go(more!)}>{$t("tube.shelf.see_all")}</button>
          {/if}
          {#if isShorts && feedbackActions}
            <button class="yt-btn yt-btn--icon" onclick={hideShorts} title={$t("tube.shelf.hide_shorts") as string} aria-label={$t("tube.shelf.hide_shorts") as string}>
              <TIcon name="close" />
            </button>
          {/if}
        </div>
      </header>
    {/if}

    {#if kind === "grid"}
      <Grid {items} {showAvatar} {feedbackActions} />
    {:else if kind === "shorts"}
      <Scroller label={title ?? ""}>
        {#each items as it, i (("id" in it ? it.id : "") + i)}
          <div class="cell short-cell">
            {#if it.kind === "short"}
              <ShortCard item={it} {feedbackActions} />
            {:else if it.kind === "video"}
              <VideoCard item={it} {showAvatar} {feedbackActions} />
            {/if}
          </div>
        {/each}
      </Scroller>
    {:else if kind === "row"}
      <Scroller label={title ?? ""}>
        {#each items as it, i (("id" in it ? it.id : "") + i)}
          <div class="cell" class:ch-cell={it.kind === "channel"}>
            {#if it.kind === "video"}
              <VideoCard item={it} {showAvatar} {feedbackActions} />
            {:else if it.kind === "short"}
              <ShortCard item={it} {feedbackActions} />
            {:else if it.kind === "playlist"}
              <PlaylistCard item={it} />
            {:else if it.kind === "channel"}
              <ChannelRow item={it} layout="card" />
            {/if}
          </div>
        {/each}
      </Scroller>
    {:else}
      <div class="list">
        {#each items as it, i (("id" in it ? it.id : "") + i)}
          {#if it.kind === "video"}
            <VideoRow item={it} size={listSize} {feedbackActions} />
          {:else if it.kind === "playlist"}
            <PlaylistCard item={it} layout={listSize === "compact" ? "compact" : "row"} />
          {:else if it.kind === "channel"}
            <ChannelRow item={it} layout="row" />
          {:else if it.kind === "short"}
            <div class="short-inline"><ShortCard item={it} {feedbackActions} /></div>
          {/if}
        {/each}
      </div>
    {/if}
  </section>
{/if}

<style>
  .shelf {
    min-width: 0;
  }
  .shelf.titled:not(.first) {
    padding-top: 24px;
    border-top: 1px solid var(--yt-divider);
  }
  .shelf + :global(.shelf) {
    margin-top: 32px;
  }
  header {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-bottom: 16px;
    min-width: 0;
  }
  .shorts-ic {
    display: inline-flex;
    color: var(--yt-accent);
  }
  .ht {
    flex: 1;
    min-width: 0;
  }
  .strap {
    font: var(--yt-type-caption);
    color: var(--yt-text-2);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .sub {
    font: var(--yt-type-meta);
    color: var(--yt-text-2);
  }
  .hact {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .cell {
    flex: none;
    width: 290px;
    padding: 8px 0 8px 8px;
  }
  .cell:first-child {
    padding-left: 0;
  }
  .short-cell {
    width: 210px;
  }
  .ch-cell {
    width: 200px;
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .short-inline {
    width: 210px;
  }
  @container omnitube (max-width: 640px) {
    .cell {
      width: 240px;
    }
    .short-cell {
      width: 160px;
    }
  }
</style>
