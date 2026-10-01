<script lang="ts">
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { TOOL_ICONS } from "$lib/canvas/icons";
  import Icon from "./Icon.svelte";

  const engine = useEngine();
  let drag: { x: number; z: number; moved: boolean } | null = null;

  function down(e: PointerEvent) {
    drag = { x: e.clientX, z: engine.camera.zoom, moved: false };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }
  function move(e: PointerEvent) {
    if (!drag) return;
    const dx = e.clientX - drag.x;
    if (Math.abs(dx) > 2) drag.moved = true;
    if (drag.moved) engine.setZoom(drag.z * Math.exp(dx * 0.01));
  }
  function up() {
    if (drag && !drag.moved) engine.setZoom(1);
    drag = null;
  }
</script>

<div class="zoom" data-canvas-chrome>
  <button type="button" title={$t("canvas.engine.zoomOut", { default: "Zoom out" })} onclick={() => engine.setZoom(engine.camera.zoom / 1.25)}>
    <Icon d={TOOL_ICONS.minus} />
  </button>
  <button
    type="button"
    class="pct"
    title={$t("canvas.engine.zoomDrag", { default: "Drag to zoom, click for 100%" })}
    onpointerdown={down}
    onpointermove={move}
    onpointerup={up}
  >
    {Math.round(engine.zoomUi * 100)}%
  </button>
  <button type="button" title={$t("canvas.engine.zoomIn", { default: "Zoom in" })} onclick={() => engine.setZoom(engine.camera.zoom * 1.25)}>
    <Icon d={TOOL_ICONS.plus} />
  </button>
  <button type="button" title={$t("canvas.engine.zoomFit", { default: "Fit everything" })} onclick={() => engine.fitAll()}>
    <Icon d={TOOL_ICONS.fit} />
  </button>
  <button
    type="button"
    class:on={engine.minimapVisible}
    title={$t("canvas.engine.toggleMinimap", { default: "Minimap" })}
    onclick={() => (engine.minimapVisible = !engine.minimapVisible)}
  >
    <Icon d={TOOL_ICONS.map} />
  </button>
</div>

<style>
  .zoom {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    padding: 3px;
    border-radius: 10px;
    background: var(--popup-bg, var(--surface));
    border: 1px solid var(--border);
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.2);
  }
  button {
    all: unset;
    min-width: 28px;
    height: 28px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 6px;
    color: var(--text);
    cursor: pointer;
    font-size: 12px;
  }
  button:hover,
  button.on {
    background: var(--fill-1, rgba(127, 127, 127, 0.15));
  }
  button:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent));
  }
  .pct {
    min-width: 48px;
    font-variant-numeric: tabular-nums;
    cursor: ew-resize;
  }
</style>
