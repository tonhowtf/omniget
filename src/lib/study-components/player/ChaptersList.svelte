<script lang="ts">
  import { t } from "$lib/i18n";
  import type { YoutubeChapter } from "$lib/study-bridge";

  type Props = {
    chapters: YoutubeChapter[];
    currentTimeMs: number;
    onJump: (toMs: number) => void;
    compact?: boolean;
  };

  let { chapters, currentTimeMs, onJump, compact = false }: Props = $props();

  const activeIndex = $derived.by(() => {
    for (let i = 0; i < chapters.length; i += 1) {
      const c = chapters[i];
      if (currentTimeMs >= c.start_ms && currentTimeMs < c.end_ms) return i;
    }
    return -1;
  });

  function formatTime(ms: number): string {
    const totalSec = Math.floor(ms / 1000);
    const h = Math.floor(totalSec / 3600);
    const m = Math.floor((totalSec % 3600) / 60);
    const s = totalSec % 60;
    if (h > 0) {
      return `${h}:${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`;
    }
    return `${m}:${s.toString().padStart(2, "0")}`;
  }

  function handleClick(c: YoutubeChapter) {
    onJump(c.start_ms);
  }
</script>

{#if chapters.length > 0}
  <ol class="chapters" class:compact aria-label={$t("study.player.shell.chapters")}>
    {#each chapters as chapter, i (chapter.start_ms + ":" + chapter.title)}
      <li>
        <button
          type="button"
          class="chapter"
          class:active={i === activeIndex}
          onclick={() => handleClick(chapter)}
          aria-current={i === activeIndex ? "true" : undefined}
        >
          <span class="time" aria-hidden="true">{formatTime(chapter.start_ms)}</span>
          <span class="title">{chapter.title}</span>
        </button>
      </li>
    {/each}
  </ol>
{/if}

<style>
  .chapters {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .chapter {
    display: grid;
    grid-template-columns: max-content 1fr;
    align-items: baseline;
    gap: 10px;
    width: 100%;
    padding: 8px 12px;
    background: transparent;
    border: none;
    border-radius: 6px;
    color: var(--text);
    text-align: left;
    cursor: pointer;
    font: inherit;
  }

  .chapter:hover {
    background: color-mix(in oklab, var(--accent) 12%, transparent);
  }

  .chapter.active {
    background: color-mix(in oklab, var(--accent) 18%, transparent);
    color: var(--accent);
  }

  .chapter.active .title {
    font-weight: 600;
  }

  .time {
    font-variant-numeric: tabular-nums;
    font-size: 12px;
    color: color-mix(in oklab, var(--text) 55%, transparent);
    min-width: 3.5ch;
  }

  .chapter.active .time {
    color: var(--accent);
  }

  .title {
    font-size: 13px;
    line-height: 1.35;
    overflow: hidden;
    text-overflow: ellipsis;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    -webkit-box-orient: vertical;
  }

  .compact .chapter {
    padding: 6px 10px;
  }

  .compact .title {
    font-size: 12px;
  }
</style>
