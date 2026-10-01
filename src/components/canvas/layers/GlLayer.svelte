<script lang="ts">
  import { onMount } from "svelte";
  import { useEngine } from "$lib/canvas/engine.svelte";
  import { registerPass } from "$lib/canvas/render/gl-layer";
  import { createGridPass } from "$lib/canvas/render/grid";
  import { createPlatesPass, createOverlayPass } from "$lib/canvas/render/plates-pass";
  import { createMeshPass } from "$lib/canvas/render/mesh-pass";
  import { createCordsPass } from "$lib/canvas/render/cords-pass";
  import { createSnapshotPass } from "$lib/canvas/render/snapshot-textures";
  import { createSpritePass } from "$lib/canvas/render/sprite-pass";
  import { createCanvas2dDrawer } from "$lib/canvas/render/canvas2d";
  import { readTheme, watchTheme, type CanvasTheme } from "$lib/canvas/render/theme";
  import { registerTermPass, setTermGlAvailable } from "$lib/canvas/term/pass";
  import { canvasPrefs } from "$lib/canvas/prefs.svelte";

  const engine = useEngine();
  let glCanvas = $state<HTMLCanvasElement | null>(null);
  let fallbackCanvas = $state<HTMLCanvasElement | null>(null);
  let theme: CanvasTheme | null = null;
  const th = () => (theme ??= readTheme());

  onMount(() => {
    const stopTheme = watchTheme(() => {
      theme = readTheme();
      engine.invalidate();
    });
    const unregister = [
      registerPass(createGridPass(() => th().grid, () => canvasPrefs().background === "grid")),
      registerPass(createPlatesPass(engine, () => th())),
      registerPass(createMeshPass(engine)),
      registerPass(createCordsPass(engine, () => th())),
      registerPass(createSnapshotPass(engine)),
      registerPass(createSpritePass(engine, () => th())),
      registerPass(createOverlayPass(engine, () => th())),
      registerTermPass(() => engine.invalidate()),
    ];
    const ok = glCanvas
      ? engine.layer.init(glCanvas, {
          onFallback: () => {
            setTermGlAvailable(false);
            engine.glFallback = true;
          },
          onRestored: () => engine.invalidate(),
        })
      : false;
    setTermGlAvailable(ok);
    if (!ok) engine.glFallback = true;
    engine.invalidate();
    return () => {
      stopTheme();
      for (const u of unregister) u();
    };
  });

  $effect(() => {
    if (!engine.glFallback || !fallbackCanvas) return;
    const c = fallbackCanvas;
    const resize = () => {
      const dpr = Math.min(window.devicePixelRatio || 1, 2);
      c.width = Math.max(1, Math.round(engine.camera.vw * dpr));
      c.height = Math.max(1, Math.round(engine.camera.vh * dpr));
    };
    resize();
    const draw = createCanvas2dDrawer(engine, c, () => th());
    engine.fallbackDraw = draw
      ? (frame) => {
          if (c.width !== Math.round(frame.viewport.w * frame.viewport.dpr)) resize();
          draw(frame);
        }
      : null;
    engine.invalidate();
    return () => {
      engine.fallbackDraw = null;
    };
  });
</script>

{#if engine.glFallback}
  <canvas bind:this={fallbackCanvas} class="l0" aria-hidden="true"></canvas>
{:else}
  <canvas bind:this={glCanvas} class="l0" aria-hidden="true"></canvas>
{/if}

<style>
  .l0 {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
    display: block;
  }
</style>
