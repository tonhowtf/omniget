<script lang="ts">
  import type { Cue } from "$lib/studio/layout";
  import { ASS_SIZE_TO_CSS, CAPTION_SHADOW, geometry } from "$lib/studio/presets";
  import { cssFontStack } from "$lib/studio/fonts";
  import type { CaptionStyle } from "$lib/studio/types";

  let {
    cue,
    style,
    time,
    outW,
    outH,
    scale,
  }: { cue: Cue | null; style: CaptionStyle; time: number; outW: number; outH: number; scale: number } = $props();

  let g = $derived(geometry(style, outW, outH));
  let vertical = $derived(g.alignment >= 7 ? "top" : g.alignment >= 4 ? "middle" : "bottom");
  let left = $derived(g.alignment === 1 || g.alignment === 4 || g.alignment === 7);
  let fontPx = $derived(g.fontSize * scale * ASS_SIZE_TO_CSS);
  let linePx = $derived(g.fontSize * scale);
  let strokePx = $derived(style.box ? 0 : g.outline * scale * 2);
  let shadowPx = $derived(style.box ? 0 : g.shadow * scale);
  let padPx = $derived(style.box ? g.outline * scale : 0);

  function wordColor(start: number): string {
    return style.karaoke && time >= start ? style.highlight : style.color;
  }
</script>

{#if cue}
  <div
    class="co co-{vertical}"
    class:co-left={left}
    style:--co-mv="{g.marginV * scale}px"
    style:--co-mh="{g.marginH * scale}px"
    style:font-family={cssFontStack(style.font)}
    style:font-weight={style.bold ? 700 : 400}
    style:font-size="{fontPx}px"
    style:line-height="{linePx}px"
    aria-hidden="true"
  >
    {#each cue.lines as line, li (li)}
      <span
        class="co-line"
        class:co-box={style.box}
        style:background={style.box ? style.boxColor : null}
        style:padding="0 {padPx}px"
        style:-webkit-text-stroke={strokePx > 0 ? `${strokePx}px ${style.outlineColor}` : null}
        style:text-shadow={shadowPx > 0 ? `${shadowPx}px ${shadowPx}px 0 ${CAPTION_SHADOW}` : null}
      >
        {#each line as word, wi (wi)}<span style:color={wordColor(word.start_ms)}>{word.text}</span>{wi < line.length - 1 ? " " : ""}{/each}
      </span>
    {/each}
  </div>
{/if}

<style>
  .co {
    position: absolute;
    left: var(--co-mh);
    right: var(--co-mh);
    display: flex;
    flex-direction: column;
    align-items: center;
    text-align: center;
    white-space: pre;
    pointer-events: none;
  }

  .co-left {
    align-items: flex-start;
    text-align: left;
  }

  .co-bottom {
    bottom: var(--co-mv);
  }

  .co-top {
    top: var(--co-mv);
  }

  .co-middle {
    top: 50%;
    transform: translateY(-50%);
  }

  .co-line {
    display: inline-block;
    paint-order: stroke fill;
  }
</style>
