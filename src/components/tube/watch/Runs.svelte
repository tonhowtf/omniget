<script lang="ts">
  import type { TextRun } from "$lib/tube/types";
  import { followRun } from "../cards/util";

  let {
    runs,
    videoId = "",
    onseek,
  }: { runs: TextRun[]; videoId?: string; onseek?: (t: number) => void } = $props();

  function linky(r: TextRun): boolean {
    return !!(r.url || r.video_id || r.channel_id || r.t != null);
  }
</script>

{#each runs as r, i (i)}{#if linky(r)}<button
      class="run"
      class:ts={r.t != null && (!r.video_id || r.video_id === videoId)}
      onclick={(e) => {
        e.stopPropagation();
        void followRun(r, onseek, videoId);
      }}>{r.text}</button
    >{:else if r.bold}<b>{r.text}</b>{:else}{r.text}{/if}{/each}

<style>
  .run {
    display: inline;
    padding: 0;
    border: 0;
    font: inherit;
    color: var(--yt-link);
    background: none;
    text-align: left;
    cursor: pointer;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .run:hover {
    text-decoration: underline;
  }
  .run:focus-visible {
    outline: 2px solid var(--yt-link);
    border-radius: 2px;
  }
  b {
    font-weight: 500;
  }
</style>
