<script lang="ts">
  import { seriesCover } from "$lib/read/library-store.svelte";
  import type { Series } from "$lib/read/types";
  import Cover from "./Cover.svelte";

  let { series, onopen, onmenu }: { series: Series; onopen: () => void; onmenu?: (e: MouseEvent) => void } = $props();
</script>

<div
  class="sc"
  role="button"
  tabindex="0"
  onclick={onopen}
  onkeydown={(e) => e.key === "Enter" && onopen()}
  oncontextmenu={(e) => onmenu?.(e)}
  title={series.title}
>
  <div class="art">
    <Cover src={seriesCover(series)} title={series.title} ratio={1.45} />
    {#if series.unread}<span class="unread">{series.unread}</span>{/if}
    {#if series.downloaded}<span class="dl">{series.downloaded}</span>{/if}
  </div>
  <div class="t">{series.title}</div>
  <div class="a">{series.source.replace(/^decl:\d+/, "").replace(/^suwa:\d+/, "Suwayomi") || series.source}</div>
</div>

<style>
  .sc {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    cursor: pointer;
    outline: none;
  }
  .sc:focus-visible .art {
    outline: 2.5px solid var(--accent);
    outline-offset: 2px;
    border-radius: 6px;
  }
  .art {
    position: relative;
    transition: transform 0.15s;
  }
  .sc:hover .art {
    transform: translateY(-2px);
  }
  .unread,
  .dl {
    position: absolute;
    top: 6px;
    padding: 1px 7px;
    border-radius: var(--radius-full);
    font-size: 11px;
    font-weight: 750;
    color: #fff;
  }
  .unread {
    left: 6px;
    background: var(--accent);
  }
  .dl {
    right: 6px;
    background: rgb(0 0 0 / 0.62);
  }
  .t {
    font-size: var(--text-sm);
    font-weight: 600;
    line-height: 1.25;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .a {
    font-size: var(--text-xs);
    color: var(--text-muted);
    text-transform: capitalize;
  }
</style>
