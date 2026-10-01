<script lang="ts">
  import type { Snippet } from "svelte";
  import { t } from "$lib/i18n";
  import TIcon from "../TIcon.svelte";

  let {
    children,
    gap = 16,
    step = 0.8,
    arrows = true,
    label = "",
    class: klass = "",
  }: { children: Snippet; gap?: number; step?: number; arrows?: boolean; label?: string; class?: string } = $props();

  let track = $state<HTMLDivElement>();
  let atStart = $state(true);
  let atEnd = $state(true);

  function measure() {
    if (!track) return;
    const max = track.scrollWidth - track.clientWidth;
    atStart = track.scrollLeft <= 2;
    atEnd = track.scrollLeft >= max - 2;
  }

  $effect(() => {
    if (!track) return;
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(track);
    const mo = new MutationObserver(measure);
    mo.observe(track, { childList: true, subtree: false });
    return () => {
      ro.disconnect();
      mo.disconnect();
    };
  });

  function by(dir: 1 | -1) {
    if (!track) return;
    track.scrollBy({ left: dir * track.clientWidth * step, behavior: "smooth" });
  }
</script>

<div class="scroller {klass}" class:fade-l={!atStart} class:fade-r={!atEnd}>
  <div class="track" bind:this={track} onscroll={measure} style:gap="{gap}px" role="group" aria-label={label || undefined}>
    {@render children()}
  </div>
  {#if arrows && !atStart}
    <button class="arrow left" onclick={() => by(-1)} aria-label={$t("tube.common.scroll_left") as string}>
      <TIcon name="chevron-left" />
    </button>
  {/if}
  {#if arrows && !atEnd}
    <button class="arrow right" onclick={() => by(1)} aria-label={$t("tube.common.scroll_right") as string}>
      <TIcon name="chevron-right" />
    </button>
  {/if}
</div>

<style>
  .scroller {
    position: relative;
    min-width: 0;
  }
  .track {
    display: flex;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
    scroll-behavior: smooth;
    overscroll-behavior-x: contain;
  }
  .track::-webkit-scrollbar {
    display: none;
  }
  .scroller::before,
  .scroller::after {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    width: 56px;
    pointer-events: none;
    opacity: 0;
    transition: opacity var(--yt-dur-base) var(--yt-ease-standard);
    z-index: 1;
  }
  .scroller::before {
    left: 0;
    background: linear-gradient(to right, var(--yt-bg) 20%, transparent);
  }
  .scroller::after {
    right: 0;
    background: linear-gradient(to left, var(--yt-bg) 20%, transparent);
  }
  .scroller.fade-l::before,
  .scroller.fade-r::after {
    opacity: 1;
  }
  .arrow {
    position: absolute;
    top: 50%;
    z-index: 2;
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    margin-top: -20px;
    border: 0;
    border-radius: 50%;
    color: var(--yt-text);
    background: var(--yt-bg-menu);
    box-shadow: 0 4px 4px var(--yt-shadow), 0 0 0 1px var(--yt-outline);
    cursor: pointer;
    transition: background var(--yt-dur-fast);
  }
  .arrow:hover {
    background: var(--yt-bg-raised);
    filter: brightness(1.15);
  }
  .arrow:focus-visible {
    outline: 2px solid var(--yt-link);
  }
  .left {
    left: 0;
  }
  .right {
    right: 0;
  }
  .scroller.chips .arrow {
    width: 32px;
    height: 32px;
    margin-top: -16px;
    background: var(--yt-bg);
    box-shadow: none;
  }
  .scroller.chips .arrow:hover {
    background: var(--yt-hover);
  }
</style>
