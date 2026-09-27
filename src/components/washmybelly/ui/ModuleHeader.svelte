<script lang="ts">
  import type { Snippet } from "svelte";

  /// Title row of a module. The shell already pads the content pane, so the
  /// header only spaces itself from what follows. `tile` repeats the
  /// sidebar's coloured icon next to the title; `meta` sits under the
  /// subtitle (a status line, a tag).
  let {
    title,
    subtitle = "",
    actions,
    tile,
    meta,
  }: {
    title: string;
    subtitle?: string;
    actions?: Snippet;
    tile?: { glyph: string; from: string; to: string };
    meta?: Snippet;
  } = $props();
</script>

<header class="wmb-mh">
  {#if tile}
    <span class="wmb-mh-tile" style:--from={tile.from} style:--to={tile.to} aria-hidden="true">
      <span class="wmb-mh-glyph" style:--glyph="url(/icons/{tile.glyph}.svg)"></span>
    </span>
  {/if}
  <div class="wmb-mh-text">
    <h2>{title}</h2>
    {#if subtitle}<p>{subtitle}</p>{/if}
    {#if meta}<div class="wmb-mh-meta">{@render meta()}</div>{/if}
  </div>
  {#if actions}<div class="wmb-mh-actions">{@render actions()}</div>{/if}
</header>

<style>
  .wmb-mh {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: 0 0 var(--space-4);
    min-width: 0;
  }
  .wmb-mh-tile {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 40px;
    height: 40px;
    border-radius: 10px;
    background: linear-gradient(180deg, var(--from), var(--to));
    box-shadow: inset 0 0 0 0.5px rgb(0 0 0 / 0.12), inset 0 1px 0 rgb(255 255 255 / 0.18);
  }
  .wmb-mh-glyph {
    width: 24px;
    height: 24px;
    background: #fff;
    -webkit-mask: var(--glyph) center / contain no-repeat;
    mask: var(--glyph) center / contain no-repeat;
  }
  .wmb-mh-text {
    flex: 1;
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--text-xl);
    font-weight: 700;
    letter-spacing: var(--track-tight);
    color: var(--text);
    line-height: 1.2;
  }
  p {
    margin: 2px 0 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
    line-height: 1.4;
  }
  .wmb-mh-meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
  .wmb-mh-actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
  }
</style>
