<script lang="ts">
  /**
   * `/world` — the agent canvas.
   *
   *   ?bench=selftest  the renderer's own reproducible measurement
   *   ?bench=<name>    the world bench of `scripts/world-bench.mjs`
   *   (no query)       the agent canvas
   */
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import Canvas from "$components/canvas/Canvas.svelte";
  import { lastWorkspaceId } from "$lib/canvas/palette/navigate";

  let enabled = $derived(getSettings()?.world?.enabled ?? true);

  let benchCanvas = $state<HTMLCanvasElement | null>(null);
  let mode = $state<"world" | "bench">("world");
  let benchState = $state<"running" | "done" | "error">("running");
  let errorCode = $state("");

  onMount(() => {
    const scenario = $page.url.searchParams.get("bench");
    if (!scenario) return;
    mode = "bench";
    void (async () => {
      try {
        if (scenario === "selftest") {
          const mod = await import("$lib/world/render");
          const report = await mod.runSelftest(benchCanvas!);
          await mod.reportSelftest(report);
          if (report.error) {
            errorCode = report.error;
            benchState = "error";
            return;
          }
        } else {
          const { runWorldBench } = await import("$lib/world/bench/runner");
          await runWorldBench(benchCanvas!, scenario);
        }
        benchState = "done";
      } catch (e) {
        errorCode = e instanceof Error ? e.message : String(e);
        benchState = "error";
      }
    })();
  });
</script>

<div class="world-page" class:fill={mode === "world" && enabled}>
  <header class="world-head">
    <h1>{$t("world.title")}</h1>
    <p class="world-sub">{$t("world.subtitle")}</p>
  </header>

  {#if mode === "bench"}
    <div class="bench-stage">
      <canvas bind:this={benchCanvas} class="bench-canvas" width="960" height="540"></canvas>
      {#if benchState === "running"}
        <p class="bench-note">{$t("world.bench_running")}</p>
      {:else if benchState === "error"}
        <p class="bench-note bench-error">{$t("world.error")} {errorCode}</p>
      {/if}
    </div>
  {:else if enabled}
    <div class="place-canvas">
      <Canvas workspaceId={lastWorkspaceId()} />
    </div>
  {:else}
    <p class="world-off">{$t("world.disabled")}</p>
  {/if}
</div>

<style>
  .world-page {
    padding: 1.5rem;
    display: flex;
    flex-direction: column;
    gap: 1rem;
  }
  /* The canvas takes every pixel the shell gives the route. */
  .world-page.fill {
    flex: 1;
    min-height: 0;
    box-sizing: border-box;
  }
  .place-canvas {
    position: relative;
    flex: 1;
    min-height: 24rem;
    border-radius: 12px;
    overflow: hidden;
  }
  .world-head h1 {
    margin: 0;
    font-size: 1.4rem;
  }
  .world-sub {
    margin: 0.25rem 0 0;
    opacity: 0.7;
    font-size: 0.85rem;
  }
  .bench-stage {
    position: relative;
    width: 100%;
    max-width: 960px;
    aspect-ratio: 16 / 9;
    border-radius: 12px;
    overflow: hidden;
    background: #10131a;
  }
  .bench-canvas {
    width: 100%;
    height: 100%;
    display: block;
  }
  .bench-note {
    position: absolute;
    bottom: 8px;
    left: 8px;
    margin: 0;
    color: #e8eaf0;
    font-size: 12px;
  }
  .bench-error {
    color: #ff8f7a;
  }
  .world-off {
    margin: 0;
    opacity: 0.7;
    font-size: 0.9rem;
  }
</style>
