<script lang="ts">
  import { introLayout } from "$lib/studio/frame";
  import { BUNDLED_FONT } from "$lib/studio/presets";
  import { cssFontStack } from "$lib/studio/fonts";

  let {
    title,
    subtitle,
    background,
    color,
    outW,
    outH,
    scale,
  }: { title: string[]; subtitle: string[]; background: string; color: string; outW: number; outH: number; scale: number } = $props();

  let layout = $derived(introLayout(outW, outH, title.length, subtitle.length));
</script>

<div class="ic" style:background style:color style:font-family={cssFontStack(BUNDLED_FONT)} aria-hidden="true">
  {#each title as line, i (i)}
    <span class="ic-line" style:top="{layout.titleYs[i] * scale}px" style:font-size="{layout.titleSize * scale}px">{line}</span>
  {/each}
  {#each subtitle as line, i (i)}
    <span class="ic-line ic-sub" style:top="{layout.subtitleYs[i] * scale}px" style:font-size="{layout.subtitleSize * scale}px">{line}</span>
  {/each}
</div>

<style>
  .ic {
    position: absolute;
    inset: 0;
    overflow: hidden;
  }

  .ic-line {
    position: absolute;
    left: 0;
    right: 0;
    text-align: center;
    font-weight: 700;
    line-height: 1;
    white-space: pre;
  }
</style>
