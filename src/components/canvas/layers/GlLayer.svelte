<script lang="ts">
  import { onMount, untrack } from "svelte";
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
  import { canvasPrefs, imageBudgetBytes } from "$lib/canvas/prefs.svelte";
  import { watchImages } from "$lib/canvas/images.svelte";
  import { registerSemantic } from "$lib/native-agent/semantic";

  const engine = useEngine();
  let glCanvas = $state<HTMLCanvasElement | null>(null);
  let fallbackCanvas = $state<HTMLCanvasElement | null>(null);
  let theme: CanvasTheme | null = null;
  let forced = false;
  const th = () => (theme ??= readTheme());

  onMount(() =>
    registerSemantic("canvas-layer", () => ({
      ready: true,
      kind: "infinite canvas",
      renderer: engine.glFallback ? "canvas2d" : "webgl2",
      workspaceId: engine.store.workspaceId,
      floorId: engine.store.floorId,
      zoom: Math.round(engine.camera.zoom * 100) / 100,
    })),
  );

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
      registerPass(createSnapshotPass(engine, imageBudgetBytes)),
      registerPass(createSpritePass(engine, () => th())),
      registerPass(createOverlayPass(engine, () => th())),
      registerTermPass(() => engine.invalidate()),
    ];
    const stopImages = watchImages(engine);
    if (canvasPrefs().performance.force_canvas2d) {
      forced = true;
      setTermGlAvailable(false);
      engine.glFallback = true;
    } else if (!engine.layer.canvas) {
      startGl(glCanvas);
    }
    return () => {
      stopTheme();
      stopImages();
      for (const u of unregister) u();
    };
  });

  function startGl(c: HTMLCanvasElement | null) {
    const ok = c
      ? engine.layer.init(c, {
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
  }

  $effect(() => {
    const force = canvasPrefs().performance.force_canvas2d;
    untrack(() => {
      if (force && !engine.glFallback) {
        forced = true;
        engine.layer.destroy();
        setTermGlAvailable(false);
        engine.glFallback = true;
      } else if (!force && forced) {
        forced = false;
        engine.glFallback = false;
      }
    });
  });

  $effect(() => {
    const c = glCanvas;
    if (!c || engine.glFallback) return;
    untrack(() => {
      if (engine.layer.canvas && engine.layer.canvas !== c) engine.layer.destroy();
      if (!engine.layer.canvas) startGl(c);
    });
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
  <canvas bind:this={fallbackCanvas} class="l0" aria-hidden="true" data-agent-renderer="canvas-layer"></canvas>
{:else}
  <canvas bind:this={glCanvas} class="l0" aria-hidden="true" data-agent-renderer="canvas-layer"></canvas>
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
