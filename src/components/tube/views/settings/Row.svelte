<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    title,
    desc = "",
    stack = false,
    dim = false,
    children,
  }: { title: string; desc?: string; stack?: boolean; dim?: boolean; children?: Snippet } = $props();
</script>

<div class="rw" class:stack class:dim>
  <div class="rw-text">
    <span class="rw-title">{title}</span>
    {#if desc}<span class="rw-desc">{desc}</span>{/if}
  </div>
  {#if children}<div class="rw-ctl">{@render children()}</div>{/if}
</div>

<style>
  .rw {
    display: flex; align-items: center; gap: 24px;
    min-height: 56px; padding: 12px 0;
    border-top: 1px solid var(--yt-divider);
  }
  .rw:first-child { border-top: 0; }
  .rw.dim { opacity: .5; }
  .rw.stack { flex-direction: column; align-items: stretch; gap: 12px; }
  .rw-text { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .rw-title { font: var(--yt-type-body-strong); color: var(--yt-text); }
  .rw-desc { font: 400 13px/18px var(--yt-font); color: var(--yt-text-2); }
  .rw-ctl { flex: none; display: flex; align-items: center; gap: 8px; max-width: 100%; }
  .rw.stack .rw-ctl { flex: 1; flex-wrap: wrap; }
  @container omnitube (max-width: 640px) {
    .rw:not(.stack) { flex-wrap: wrap; gap: 12px; }
  }
</style>
