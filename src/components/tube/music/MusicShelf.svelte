<script lang="ts">
  // One YT Music shelf: strapline (12 caps grey) + 24/700 title + ‹ › arrows
  // and "More", then the body picked by `shelf.kind`. Horizontal bodies scroll
  // with a hidden scrollbar, snap, and fade at the edges (21st Scroller).
  import { t } from "$lib/i18n";
  import { bestAvatar } from "$lib/tube/format";
  import { go } from "$lib/tube/ui.svelte";
  import TIcon from "../TIcon.svelte";
  import MoodGrid from "./MoodGrid.svelte";
  import QuickPicks from "./QuickPicks.svelte";
  import SongRow from "./SongRow.svelte";
  import SquareCard from "./SquareCard.svelte";
  import { itemKey, songFromItem } from "../views/music/shared.svelte";
  import type { Shelf } from "$lib/tube/types";

  let {
    shelf,
    /** Force a layout (search/library use "grid" and "list"). */
    layout,
    /** Rows shown for list shelves before "Show all". */
    listLimit = 0,
  }: { shelf: Shelf; layout?: "carousel" | "grid" | "list" | "quick_picks" | "moods"; listLimit?: number } = $props();

  let mode = $derived.by(() => {
    if (layout) return layout;
    switch (shelf.kind) {
      case "quick_picks":
        return "quick_picks";
      case "moods":
        return "moods";
      case "list":
        return "list";
      case "grid":
        return "grid";
      default:
        return "carousel";
    }
  });
  let horizontal = $derived(mode === "carousel" || mode === "quick_picks" || (mode === "moods" && shelf.items.length > 12));
  let scroller = $state<HTMLDivElement>();
  let atStart = $state(true);
  let atEnd = $state(true);
  let expanded = $state(false);
  let listItems = $derived(listLimit > 0 && !expanded ? shelf.items.slice(0, listLimit) : shelf.items);
  let allSongs = $derived(shelf.items.every((i) => songFromItem(i)));

  function measure() {
    if (!scroller) return;
    atStart = scroller.scrollLeft <= 2;
    atEnd = scroller.scrollLeft + scroller.clientWidth >= scroller.scrollWidth - 2;
  }

  function page(dir: 1 | -1) {
    if (!scroller) return;
    scroller.scrollBy({ left: dir * Math.max(200, scroller.clientWidth - 80), behavior: "smooth" });
  }

  $effect(() => {
    if (!scroller) return;
    void shelf.items.length;
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(scroller);
    return () => ro.disconnect();
  });

  function openMore() {
    if (shelf.more) go({ name: "m_browse", id: shelf.more.browse_id, params: shelf.more.params ?? null, title: shelf.title ?? null });
  }
</script>

<section class="ms" aria-label={shelf.title ?? undefined}>
  {#if shelf.title || shelf.strapline}
    <header class="ms-head">
      {#if shelf.thumb}
        <img class="ms-thumb yt-avatar" src={bestAvatar(shelf.thumb, 56)} alt="" width="56" height="56" loading="lazy" decoding="async" referrerpolicy="no-referrer" />
      {/if}
      <div class="ms-titles">
        {#if shelf.strapline}<p class="ms-strap">{shelf.strapline}</p>{/if}
        {#if shelf.title}
          {#if shelf.more}
            <h2 class="ms-title"><button class="ms-title-btn" onclick={openMore}>{shelf.title}</button></h2>
          {:else}
            <h2 class="ms-title">{shelf.title}</h2>
          {/if}
        {/if}
        {#if shelf.subtitle}<p class="ms-subtitle">{shelf.subtitle}</p>{/if}
      </div>
      <div class="ms-actions">
        {#if shelf.more}
          <button class="yt-btn yt-btn--s ms-more" onclick={openMore}>{$t("tube.music.more")}</button>
        {/if}
        {#if horizontal && !(atStart && atEnd)}
          <button class="ms-arrow" onclick={() => page(-1)} disabled={atStart} aria-label={$t("tube.common.previous") as string}><TIcon name="chevron-left" size={20} /></button>
          <button class="ms-arrow" onclick={() => page(1)} disabled={atEnd} aria-label={$t("tube.common.next") as string}><TIcon name="chevron-right" size={20} /></button>
        {/if}
      </div>
    </header>
  {/if}

  {#if horizontal}
    <div class="ms-scroller" class:fade-l={!atStart} class:fade-r={!atEnd} bind:this={scroller} onscroll={measure}>
      {#if mode === "quick_picks"}
        <QuickPicks items={shelf.items} />
      {:else if mode === "moods"}
        <MoodGrid items={shelf.items} rows={4} />
      {:else}
        <div class="ms-row" role="list">
          {#each shelf.items as it, i (itemKey(it, i))}
            <div role="listitem"><SquareCard item={it} /></div>
          {/each}
        </div>
      {/if}
    </div>
  {:else if mode === "moods"}
    <MoodGrid items={shelf.items} />
  {:else if mode === "list" && allSongs}
    <div class="ms-list" role="list">
      {#each listItems as it, i (itemKey(it, i))}
        {@const s = songFromItem(it)}
        {#if s}<div role="listitem"><SongRow song={s} /></div>{/if}
      {/each}
    </div>
    {#if listLimit > 0 && shelf.items.length > listLimit}
      <button class="yt-btn yt-btn--s ms-showall" onclick={() => (expanded = !expanded)}>
        {expanded ? $t("tube.music.show_less") : $t("tube.music.show_all")}
      </button>
    {/if}
  {:else}
    <div class="ms-grid" role="list">
      {#each shelf.items as it, i (itemKey(it, i))}
        {@const s = mode === "list" ? songFromItem(it) : null}
        <div role="listitem" class:span={!!s}>
          {#if s}<SongRow song={s} />{:else}<SquareCard item={it} fluid />{/if}
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .ms { margin: 0 0 48px; color: var(--ytm-text, var(--yt-text)); }
  .ms-head { display: flex; align-items: flex-end; gap: 16px; margin-bottom: 16px; min-height: 40px; }
  .ms-thumb { width: 56px; height: 56px; }
  .ms-titles { flex: 1; min-width: 0; }
  .ms-strap { margin: 0 0 2px; font: 400 12px/16px var(--yt-font); letter-spacing: .04em; text-transform: uppercase; color: var(--ytm-text-2, var(--yt-text-2)); }
  .ms-title { margin: 0; font: var(--yt-type-headline-m); color: var(--ytm-text, var(--yt-text)); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ms-title-btn { all: unset; cursor: pointer; }
  .ms-title-btn:hover { text-decoration: underline; }
  .ms-title-btn:focus-visible { outline: 2px solid var(--yt-link); border-radius: 2px; }
  .ms-subtitle { margin: 2px 0 0; font: var(--yt-type-meta); color: var(--ytm-text-2, var(--yt-text-2)); }
  .ms-actions { display: flex; align-items: center; gap: 8px; flex: none; }
  .ms-more { background: transparent; box-shadow: inset 0 0 0 1px var(--ytm-divider, var(--yt-outline)); color: var(--ytm-text, var(--yt-text)); }
  .ms-more:hover { background: var(--ytm-chip, var(--yt-tonal)); }
  .ms-arrow {
    display: grid; place-items: center; width: 36px; height: 36px; border-radius: 50%;
    border: 1px solid rgba(255, 255, 255, .2); padding: 0; background: transparent;
    color: var(--ytm-text, var(--yt-text)); cursor: pointer;
    transition: background var(--yt-dur-fast), opacity var(--yt-dur-fast);
  }
  .ms-arrow:hover:not(:disabled) { background: var(--ytm-chip, var(--yt-tonal)); }
  .ms-arrow:disabled { opacity: .3; cursor: default; }
  .ms-arrow:focus-visible { outline: 2px solid var(--yt-link); outline-offset: 2px; }

  .ms-scroller {
    --fade: 48px;
    overflow-x: auto; overflow-y: hidden;
    scroll-snap-type: x mandatory;
    scroll-behavior: smooth;
    scrollbar-width: none;
    overscroll-behavior-x: contain;
    padding-bottom: 4px;
  }
  .ms-scroller::-webkit-scrollbar { display: none; }
  .ms-scroller.fade-r { mask-image: linear-gradient(to right, #000 calc(100% - var(--fade)), transparent); }
  .ms-scroller.fade-l { mask-image: linear-gradient(to left, #000 calc(100% - var(--fade)), transparent); }
  .ms-scroller.fade-l.fade-r { mask-image: linear-gradient(to right, transparent, #000 var(--fade), #000 calc(100% - var(--fade)), transparent); }
  .ms-row { display: flex; gap: var(--ytm-card-gap, 16px); width: max-content; }

  .ms-list { display: flex; flex-direction: column; }
  .ms-showall { margin-top: 16px; background: transparent; box-shadow: inset 0 0 0 1px var(--ytm-divider, var(--yt-outline)); }
  .ms-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(160px, 1fr));
    gap: 32px var(--ytm-card-gap, 16px);
  }
  .ms-grid .span { grid-column: 1 / -1; }

  @media (prefers-reduced-motion: reduce) { .ms-scroller { scroll-behavior: auto; } }
</style>
