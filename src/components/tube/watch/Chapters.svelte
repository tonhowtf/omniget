<script lang="ts">
  // Chapters as a horizontal row of cards (thumb + title + time); the one
  // playing is highlighted and clicking seeks.
  import { t } from "$lib/i18n";
  import { fmtDuration } from "$lib/tube/format";
  import { vplayer } from "$lib/tube/video.svelte";
  import type { Chapter } from "$lib/tube/types";
  import Scroller from "../cards/Scroller.svelte";

  let { chapters, onseek }: { chapters: Chapter[]; onseek: (t: number) => void } = $props();

  let current = $derived.by(() => {
    let idx = -1;
    for (let i = 0; i < chapters.length; i++) if (vplayer.position >= chapters[i].start_s) idx = i;
    return idx;
  });
</script>

<section class="chapters" aria-label={$t("tube.watch.chapters") as string}>
  <h2>{$t("tube.watch.chapters")}</h2>
  <Scroller gap={8}>
    {#each chapters as c, i (i)}
      <button class="ch" class:on={i === current} onclick={() => onseek(c.start_s)} aria-current={i === current ? "true" : undefined}>
        <span class="th">
          {#if c.thumb}<img src={c.thumb} alt="" loading="lazy" decoding="async" referrerpolicy="no-referrer" />{/if}
          <span class="time">{fmtDuration(c.start_s)}</span>
        </span>
        <span class="tt yt-clamp-2">{c.title}</span>
      </button>
    {/each}
  </Scroller>
</section>

<style>
  .chapters {
    margin-top: 16px;
  }
  h2 {
    margin: 0 0 8px;
    font: var(--yt-type-card-title);
  }
  .ch {
    flex: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
    width: 168px;
    padding: 6px;
    border: 0;
    border-radius: 12px;
    text-align: left;
    color: var(--yt-text);
    background: transparent;
    cursor: pointer;
    transition: background var(--yt-dur-fast);
  }
  .ch:hover {
    background: var(--yt-hover);
  }
  .ch.on {
    background: var(--yt-tonal);
  }
  .ch:focus-visible {
    outline: 2px solid var(--yt-link);
  }
  .th {
    position: relative;
    display: block;
    aspect-ratio: 16 / 9;
    border-radius: 8px;
    overflow: hidden;
    background: var(--yt-skeleton);
  }
  .th img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .time {
    position: absolute;
    left: 6px;
    bottom: 6px;
    padding: 0 4px;
    border-radius: 4px;
    font: 500 12px/18px var(--yt-font);
    color: #fff;
    background: var(--yt-ov-badge);
  }
  .on .time {
    background: var(--yt-accent);
    color: var(--yt-on-accent);
  }
  .tt {
    font: var(--yt-type-body-strong);
    font-size: 13px;
    line-height: 18px;
  }
</style>
