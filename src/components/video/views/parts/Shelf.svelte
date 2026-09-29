<script lang="ts">
  // Horizontal row with round prev/next arrows (the "Apple Card Carousel" on
  // 21st.dev, redone with native scroll snapping instead of embla).
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import VIcon from "./VIcon.svelte";

  let {
    title,
    width = 260,
    actions,
    children,
  }: { title: string; width?: number; actions?: Snippet; children: Snippet } = $props();

  let track: HTMLDivElement | undefined = $state();
  let canPrev = $state(false);
  let canNext = $state(false);

  function update() {
    if (!track) return;
    canPrev = track.scrollLeft > 4;
    canNext = track.scrollLeft + track.clientWidth < track.scrollWidth - 4;
  }

  $effect(() => {
    if (!track) return;
    update();
    const ro = new ResizeObserver(update);
    ro.observe(track);
    return () => ro.disconnect();
  });

  function page(dir: 1 | -1) {
    track?.scrollBy({ left: dir * track.clientWidth * 0.85, behavior: "smooth" });
  }
</script>

<section class="shelf">
  <header>
    <h2>{title}</h2>
    <div class="hd-r">
      {#if actions}{@render actions()}{/if}
      <button class="arrow" onclick={() => page(-1)} disabled={!canPrev} aria-label={$t("video.common.previous") as string}><VIcon name="chevron-left" size={16} /></button>
      <button class="arrow" onclick={() => page(1)} disabled={!canNext} aria-label={$t("video.common.next") as string}><VIcon name="chevron-right" size={16} /></button>
    </div>
  </header>
  <div class="track" bind:this={track} onscroll={update} style:--w="{width}px">
    {@render children()}
  </div>
</section>

<style>
  .shelf {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    min-width: 0;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }
  h2 {
    margin: 0;
    font-size: var(--text-xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
  }
  .hd-r {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .arrow {
    all: unset;
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: 50%;
    background: var(--fill-1);
    color: var(--text);
    cursor: pointer;
    transition: background var(--duration-fast);
  }
  .arrow:hover:not(:disabled) {
    background: var(--fill-2);
  }
  .arrow:disabled {
    opacity: 0.35;
    cursor: default;
  }
  .arrow:focus-visible {
    outline: var(--focus-ring);
  }
  .track {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: var(--w);
    gap: var(--space-4);
    overflow-x: auto;
    scroll-snap-type: x mandatory;
    scroll-padding-inline: 2px;
    padding: 4px 2px var(--space-2);
    scrollbar-width: none;
  }
  .track::-webkit-scrollbar {
    display: none;
  }
  .track > :global(*) {
    scroll-snap-align: start;
  }
</style>
