<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    readiness,
    label,
    hero,
    children,
  }: { readiness: "loading" | "ready" | "error" | "empty"; label: string; hero?: Snippet; children: Snippet } = $props();
</script>

<div class="ytm" data-agent-readiness={readiness} data-agent-surface="omnitube-music" aria-label={label} role="region">
  {@render hero?.()}
  <div class="ytm-inner">
    {@render children()}
  </div>
</div>

<style>
  .ytm {
    --ytm-pad: 16px;
    position: relative;
    min-height: 100%;
    background: var(--ytm-bg, var(--yt-bg));
    color: var(--ytm-text, var(--yt-text));
    padding-bottom: 48px;
    overflow-x: clip;
  }
  .ytm-inner {
    position: relative;
    max-width: 1050px;
    margin: 0 auto;
    padding: 24px var(--ytm-pad) 0;
  }
  @container omnitube (min-width: 700px) { .ytm { --ytm-pad: 32px; } }
  @container omnitube (min-width: 1150px) { .ytm-inner { max-width: calc(1050px + 2 * var(--ytm-pad)); } }
  @container omnitube (min-width: 1364px) { .ytm-inner { max-width: calc(1264px + 2 * var(--ytm-pad)); } }
  @container omnitube (min-width: 1578px) { .ytm-inner { max-width: calc(1478px + 2 * var(--ytm-pad)); } }
  @container omnitube (max-width: 1149px) { .ytm-inner { max-width: none; } }
</style>
