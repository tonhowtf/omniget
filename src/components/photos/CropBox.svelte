<script lang="ts">
  import { t } from "$lib/i18n";
  import { resizeCrop, type Handle } from "$lib/photos/plan";
  import type { Crop } from "$lib/photos/types";

  let {
    crop,
    ratio,
    imageAspect,
    onchange,
  }: { crop: Crop; ratio: number | null; imageAspect: number; onchange: (crop: Crop) => void } = $props();

  let root: HTMLDivElement | undefined = $state();
  let drag: { handle: Handle; start: Crop; x: number; y: number; w: number; h: number } | null = null;

  const HANDLES: Handle[] = ["nw", "ne", "sw", "se"];
  const STEP = 0.01;

  function begin(e: PointerEvent, handle: Handle) {
    if (!root || e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const rect = root.getBoundingClientRect();
    drag = { handle, start: { ...crop }, x: e.clientX, y: e.clientY, w: rect.width, h: rect.height };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function moveTo(e: PointerEvent) {
    if (!drag) return;
    const dx = (e.clientX - drag.x) / drag.w;
    const dy = (e.clientY - drag.y) / drag.h;
    onchange(resizeCrop(drag.start, drag.handle, dx, dy, ratio, imageAspect));
  }

  function end() {
    drag = null;
  }

  function key(e: KeyboardEvent) {
    const moves: Record<string, [number, number]> = {
      ArrowLeft: [-STEP, 0],
      ArrowRight: [STEP, 0],
      ArrowUp: [0, -STEP],
      ArrowDown: [0, STEP],
    };
    const m = moves[e.key];
    if (!m) return;
    e.preventDefault();
    e.stopPropagation();
    const handle: Handle = e.shiftKey ? "se" : "move";
    onchange(resizeCrop(crop, handle, m[0], m[1], ratio, imageAspect));
  }
</script>

<div class="cb" bind:this={root}>
  <div
    class="cb-box"
    role="slider"
    tabindex="0"
    aria-label={$t("photos.editor.crop_area") as string}
    aria-valuetext={`${Math.round(crop.w * 100)}% × ${Math.round(crop.h * 100)}%`}
    aria-valuenow={Math.round(crop.w * 100)}
    style:left="{crop.x * 100}%"
    style:top="{crop.y * 100}%"
    style:width="{crop.w * 100}%"
    style:height="{crop.h * 100}%"
    onpointerdown={(e) => begin(e, "move")}
    onpointermove={moveTo}
    onpointerup={end}
    onpointercancel={end}
    onkeydown={key}
  >
    <span class="cb-grid cb-v1" aria-hidden="true"></span>
    <span class="cb-grid cb-v2" aria-hidden="true"></span>
    <span class="cb-grid cb-h1" aria-hidden="true"></span>
    <span class="cb-grid cb-h2" aria-hidden="true"></span>
    {#each HANDLES as h (h)}
      <span
        class="cb-handle cb-{h}"
        aria-hidden="true"
        onpointerdown={(e) => begin(e, h)}
        onpointermove={moveTo}
        onpointerup={end}
        onpointercancel={end}
      ></span>
    {/each}
  </div>
</div>

<style>
  .cb {
    position: absolute;
    inset: 0;
    overflow: hidden;
    touch-action: none;
  }

  .cb-box {
    position: absolute;
    box-sizing: border-box;
    border: 1.5px solid #fff;
    box-shadow: 0 0 0 9999px rgba(0, 0, 0, 0.55);
    cursor: move;
    outline: none;
  }

  .cb-box:focus-visible {
    border-color: var(--accent);
  }

  .cb-grid {
    position: absolute;
    background: rgba(255, 255, 255, 0.35);
    pointer-events: none;
  }

  .cb-v1,
  .cb-v2 {
    top: 0;
    bottom: 0;
    width: 1px;
  }

  .cb-v1 {
    left: 33.333%;
  }

  .cb-v2 {
    left: 66.666%;
  }

  .cb-h1,
  .cb-h2 {
    left: 0;
    right: 0;
    height: 1px;
  }

  .cb-h1 {
    top: 33.333%;
  }

  .cb-h2 {
    top: 66.666%;
  }

  .cb-handle {
    position: absolute;
    width: 18px;
    height: 18px;
    border: 3px solid #fff;
  }

  .cb-nw {
    top: -4px;
    left: -4px;
    border-right: none;
    border-bottom: none;
    cursor: nwse-resize;
  }

  .cb-ne {
    top: -4px;
    right: -4px;
    border-left: none;
    border-bottom: none;
    cursor: nesw-resize;
  }

  .cb-sw {
    bottom: -4px;
    left: -4px;
    border-right: none;
    border-top: none;
    cursor: nesw-resize;
  }

  .cb-se {
    bottom: -4px;
    right: -4px;
    border-left: none;
    border-top: none;
    cursor: nwse-resize;
  }
</style>
