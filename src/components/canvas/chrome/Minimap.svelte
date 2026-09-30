<script lang="ts">
  // Minimap (L2, own Canvas 2D): click or drag to move the camera. Redraws on
  // document changes and at most 10 Hz while the camera moves; ⌘⇧M toggles it.
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { drawMinimap, minimapToWorld, MINIMAP_H, MINIMAP_W, type MinimapTransform } from "$lib/canvas/minimap";
  import { agentStates } from "$lib/canvas/state/agents.svelte";

  const engine = useEngine();
  let canvas = $state<HTMLCanvasElement | null>(null);
  let transform: MinimapTransform | null = null;
  let last = 0;
  let pending = false;
  let dragging = false;

  function redraw() {
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    if (canvas.width !== MINIMAP_W * dpr) {
      canvas.width = MINIMAP_W * dpr;
      canvas.height = MINIMAP_H * dpr;
    }
    const cs = getComputedStyle(canvas);
    transform = drawMinimap(ctx, engine, dpr, {
      bg: cs.getPropertyValue("--mm-bg").trim() || "rgba(20,20,24,0.85)",
      view: cs.getPropertyValue("--mm-view").trim() || "#ff9f0a",
      drawing: "rgba(160,160,170,0.5)",
    });
    last = performance.now();
  }

  function schedule() {
    if (pending) return;
    const wait = Math.max(0, 100 - (performance.now() - last));
    pending = true;
    setTimeout(() => {
      pending = false;
      redraw();
    }, wait);
  }

  onMount(() => engine.onFrame(() => schedule()));

  $effect(() => {
    void engine.store.version;
    void engine.selection.ids;
    void agentStates.byTerminal;
    if (engine.minimapVisible) schedule();
  });

  function moveTo(e: PointerEvent) {
    if (!canvas || !transform) return;
    const r = canvas.getBoundingClientRect();
    const w = minimapToWorld(transform, e.clientX - r.left, e.clientY - r.top);
    const c = engine.camera;
    c.set({ x: w.x - c.vw / (2 * c.zoom), y: w.y - c.vh / (2 * c.zoom) });
    engine.invalidate();
  }
</script>

{#if engine.minimapVisible}
  <!-- data-cover-ignore: a portal under the minimap still goes live (its
       webview paints above the chrome anyway). -->
  <div class="minimap" data-canvas-chrome data-cover-ignore>
    <button
      type="button"
      class="hide"
      title={$t("canvas.engine.minimapHide", { default: "Hide minimap" })}
      aria-label={$t("canvas.engine.minimapHide", { default: "Hide minimap" })}
      onclick={() => (engine.minimapVisible = false)}
    >
      <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true"><path d="M2 2l6 6M8 2l-6 6" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" /></svg>
    </button>
    <canvas
      bind:this={canvas}
      style:width="{MINIMAP_W}px"
      style:height="{MINIMAP_H}px"
      aria-label={$t("canvas.engine.minimap", { default: "Minimap" })}
      onpointerdown={(e) => {
        dragging = true;
        (e.currentTarget as HTMLCanvasElement).setPointerCapture(e.pointerId);
        moveTo(e);
      }}
      onpointermove={(e) => dragging && moveTo(e)}
      onpointerup={() => (dragging = false)}
    ></canvas>
  </div>
{/if}

<style>
  .minimap {
    position: relative;
    border-radius: 10px;
    overflow: hidden;
    border: 1px solid var(--border);
    box-shadow: 0 6px 20px rgba(0, 0, 0, 0.25);
    --mm-bg: color-mix(in srgb, var(--popup-bg, #1c1c1e) 92%, transparent);
    --mm-view: var(--accent);
  }
  .hide {
    all: unset;
    position: absolute;
    top: 4px;
    right: 4px;
    width: 18px;
    height: 18px;
    display: grid;
    place-items: center;
    border-radius: 5px;
    color: var(--text-muted);
    background: color-mix(in srgb, var(--popup-bg, #1c1c1e) 80%, transparent);
    cursor: pointer;
    opacity: 0;
    transition: opacity 120ms ease;
  }
  .minimap:hover .hide,
  .hide:focus-visible {
    opacity: 1;
  }
  .hide:hover {
    color: var(--text);
  }
  canvas {
    display: block;
    cursor: pointer;
  }
</style>
