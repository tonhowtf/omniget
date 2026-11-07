<script lang="ts">
  import { t } from "$lib/i18n";
  import { pointIn } from "./plan";
  import type { Click, Preview } from "./types";

  let {
    preview,
    loading = false,
    clicks = [],
    clickable = false,
    onclick,
  }: {
    preview: Preview | null;
    loading?: boolean;
    clicks?: Click[];
    clickable?: boolean;
    onclick?: (click: Click) => void;
  } = $props();

  let split = $state(50);
  let frame = $state<HTMLDivElement | null>(null);

  function place(event: MouseEvent, keep: boolean) {
    if (!clickable || !frame || !onclick) return;
    const point = pointIn(event.clientX, event.clientY, frame.getBoundingClientRect());
    if (point) onclick({ ...point, keep });
  }

  function onPointer(event: MouseEvent) {
    if (event.button !== 0) return;
    place(event, !event.altKey);
  }

  function onContext(event: MouseEvent) {
    if (!clickable) return;
    event.preventDefault();
    place(event, false);
  }
</script>

<div class="co-compare" class:busy={loading}>
  {#if preview}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div
      class="co-frame"
      class:clickable
      bind:this={frame}
      style:aspect-ratio="{preview.width} / {preview.height}"
      style:width="min(100%, calc(56vh * {preview.width} / {preview.height}))"
      onclick={onPointer}
      oncontextmenu={onContext}
    >
      {#if clickable}
        <img class="co-layer co-after" src={preview.after} alt={$t("cutout.after") as string} draggable="false" />
        {#each clicks as click, i (i)}
          <span class="co-dot" class:remove={!click.keep} style:left="{click.x * 100}%" style:top="{click.y * 100}%" aria-hidden="true"></span>
        {/each}
      {:else}
        <img class="co-layer" src={preview.before} alt={$t("cutout.before") as string} draggable="false" />
        <img class="co-layer co-after" src={preview.after} alt={$t("cutout.after") as string} draggable="false" style:clip-path="inset(0 0 0 {split}%)" />
        <span class="co-divider" style:left="{split}%" aria-hidden="true"></span>
        <span class="co-tag co-tag-before" aria-hidden="true">{$t("cutout.before")}</span>
        <span class="co-tag co-tag-after" aria-hidden="true">{$t("cutout.after")}</span>
      {/if}
    </div>
    {#if !clickable}
      <input class="co-split" type="range" min="0" max="100" step="1" bind:value={split} aria-label={$t("cutout.compare_slider") as string} />
    {/if}
  {:else}
    <div class="co-frame co-placeholder">
      {#if loading}
        <span class="spinner" aria-hidden="true"></span>
        <p>{$t("cutout.preview_working")}</p>
      {/if}
    </div>
  {/if}
  {#if loading && preview}
    <span class="co-busy spinner" aria-hidden="true"></span>
  {/if}
</div>

<style>
  .co-compare {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: center;
  }

  .co-frame {
    position: relative;
    width: 100%;
    overflow: hidden;
    border-radius: var(--radius-lg);
    background-color: #fff;
    background-image:
      linear-gradient(45deg, #d9d9de 25%, transparent 25%),
      linear-gradient(-45deg, #d9d9de 25%, transparent 25%),
      linear-gradient(45deg, transparent 75%, #d9d9de 75%),
      linear-gradient(-45deg, transparent 75%, #d9d9de 75%);
    background-size: 16px 16px;
    background-position: 0 0, 0 8px, 8px -8px, -8px 0;
    box-shadow: inset 0 0 0 var(--hairline) var(--content-border);
  }

  .co-frame.clickable {
    cursor: crosshair;
  }

  .co-placeholder {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    min-height: 280px;
    background: var(--surface);
    color: var(--text-dim);
  }

  .co-placeholder p {
    margin: 0;
    font-size: var(--text-sm);
  }

  .co-layer {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
    user-select: none;
    -webkit-user-drag: none;
  }

  .co-divider {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 2px;
    margin-left: -1px;
    background: var(--accent);
    box-shadow: 0 0 0 1px rgba(0, 0, 0, 0.15);
    pointer-events: none;
  }

  .co-tag {
    position: absolute;
    top: var(--space-2);
    padding: 2px var(--space-2);
    border-radius: var(--radius-sm);
    background: rgba(0, 0, 0, 0.55);
    color: #fff;
    font-size: var(--text-xs);
    font-weight: 600;
    pointer-events: none;
  }

  .co-tag-before {
    left: var(--space-2);
  }

  .co-tag-after {
    right: var(--space-2);
  }

  .co-dot {
    position: absolute;
    width: 14px;
    height: 14px;
    margin: -7px 0 0 -7px;
    border-radius: 50%;
    background: #34c759;
    box-shadow: 0 0 0 2px #fff, 0 1px 3px rgba(0, 0, 0, 0.4);
    pointer-events: none;
  }

  .co-dot.remove {
    background: #ff3b30;
  }

  .co-split {
    width: min(320px, 100%);
  }

  .co-busy {
    position: absolute;
    top: var(--space-3);
    right: var(--space-3);
    width: 16px;
    height: 16px;
  }

  .co-compare.busy .co-frame:not(.co-placeholder) {
    opacity: 0.85;
  }
</style>
