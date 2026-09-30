<script lang="ts">
  /**
   * `/world` — the route of the agents' world.
   *
   * Three modes, and the first two are the CI's:
   *   ?bench=selftest  the renderer's own reproducible measurement
   *   ?bench=<name>    the world bench of `.github/workflows/world-bench.yml`
   *   (no query)       the world itself: the agent canvas, or the city
   *
   * The bench modes keep a bare canvas of their own: they measure the renderer
   * and the IPC, and a HUD, a session and a map would measure something else.
   * Everything under `$lib/world` is imported dynamically from here, so an app
   * that never opens this route never downloads, parses or runs a byte of the
   * world.
   */
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import Canvas from "$components/canvas/Canvas.svelte";
  import { lastWorkspaceId } from "$lib/canvas/palette/navigate";
  import CityCanvas from "$components/world/CityCanvas.svelte";
  import CityPanel, { type EditorHandoff } from "$components/world/CityPanel.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";

  // The world is opt-in (`settings.world.enabled`, off by default). The nav
  // item is already hidden in that case, but the route can still be reached by
  // its URL. The bench modes are exempt: they measure the renderer and never
  // touch the world.
  let enabled = $derived(getSettings()?.world?.enabled ?? true);

  // The city: the persistent world on an OmniDisc instance. Set, it replaces
  // the canvas on screen.
  let cityMode = $state<{ city: string; server: string | null } | null>(null);
  let cityState = $state<{ region: string; ent: number; tick: number; interior: boolean } | null>(null);
  let cityRef = $state<{ say: (ent: number, text: string) => void; goTo: (tile: [number, number]) => Promise<void>; refreshFinishes: () => void } | null>(null);
  let cityDiag = $state<{ fps: number; tier: number; backend: string; connection: string; error: string }>({ fps: 0, tier: 0, backend: "", connection: "online", error: "" });

  // Two places, one route: the agent canvas on this machine, and the city on
  // the server. The last one chosen comes back (any other stored value, like
  // the old "home", means the canvas); nothing connects until the user presses
  // a button in the city tab.
  const PLACE_KEY = "omniget.world.place";
  function readPlace(): "canvas" | "city" {
    try {
      return localStorage.getItem(PLACE_KEY) === "city" ? "city" : "canvas";
    } catch {
      return "canvas";
    }
  }
  let place = $state<"canvas" | "city">(readPlace());
  function choosePlace(next: "canvas" | "city"): void {
    place = next;
    try {
      localStorage.setItem(PLACE_KEY, next);
    } catch {
      // storage unavailable
    }
  }
  let cityPanelRef = $state<{ refreshHome: () => void; clearTool: () => void; selectionChanged: (id: string | null) => void; editorChanged: () => void; inspect: (ent: number | null) => void } | null>(null);
  let cityCrop = $state("carrot");
  let cityEdit = $state<EditorHandoff | null>(null);

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

<div class="world-page" class:fill={mode === "world" && enabled && !cityMode && place === "canvas"}>
  <header class="world-head">
    <h1>{$t("world.title")}</h1>
    <p class="world-sub">{$t("world.subtitle")}</p>
    {#if mode === "world" && enabled && !cityMode}
      <div class="places" role="tablist" aria-label={$t("world.places") as string}>
        <button type="button" role="tab" id="tab-canvas" aria-controls="panel-place" aria-selected={place === "canvas"} class:active={place === "canvas"} onclick={() => choosePlace("canvas")}>
          {$t("world.tab.canvas")}
        </button>
        <button type="button" role="tab" id="tab-city" aria-controls="panel-place" aria-selected={place === "city"} class:active={place === "city"} title={$t("world.place_city_hint") as string} onclick={() => choosePlace("city")}>
          {$t("world.tab.city")}
        </button>
      </div>
    {/if}
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
  {:else if enabled && cityMode}
    <div class="world-main city-main">
      {#key cityMode.city + (cityMode.server ?? "")}
        <CityCanvas
          bind:this={cityRef}
          city={cityMode.city}
          server={cityMode.server}
          crop={cityCrop}
          editor={cityEdit?.editor ?? null}
          tool={cityEdit?.tool ?? null}
          published={cityEdit?.published ?? null}
          plotRect={cityEdit?.plotRect ?? null}
          picture={cityEdit?.picture ?? null}
          finishPreview={cityEdit?.finishPreview ?? null}
          onconnection={(c) => (cityDiag = { ...cityDiag, connection: c })}
          onpick={(e) => cityPanelRef?.inspect(e)}
          onstats={(_s, fps, tier, backend) => {
            // Coarse: the panel shows it in its diagnostics area only.
            if (Math.abs(fps - cityDiag.fps) >= 1 || tier !== cityDiag.tier || backend !== cityDiag.backend) cityDiag = { ...cityDiag, fps, tier, backend };
          }}
          onedit={() => cityPanelRef?.editorChanged()}
          onselect={(id) => cityPanelRef?.selectionChanged(id)}
          oncleartool={() => cityPanelRef?.clearTool()}
          onfarm={(r) => {
            if (r.ok) cityPanelRef?.refreshHome();
            else showToast("error", $t(`world.city.farm_${r.code.toLowerCase().replace(/^err_world_(farm_)?/, "")}`) as string);
          }}
          onstate={(s) => (cityState = s)}
          onfailed={(error) => {
            showToast("error", error);
            cityMode = null;
            cityState = null;
          }}
        />
      {/key}
    </div>
    <CityPanel
      bind:this={cityPanelRef}
      city={cityMode}
      where={cityState}
      crop={cityCrop}
      oncrop={(c) => (cityCrop = c)}
      oneditor={(s) => (cityEdit = s)}
      onfinish={() => cityRef?.refreshFinishes()}
      diag={cityDiag}
      onenter={(c) => (cityMode = c)}
      onleave={() => {
        cityMode = null;
        cityState = null;
        cityEdit = null;
      }}
      onsay={(ent, text) => cityRef?.say(ent, text)}
      ongoto={(tile) => void cityRef?.goTo(tile)}
    />
  {:else if enabled && place === "city"}
    <div id="panel-place" role="tabpanel" aria-labelledby="tab-city">
      <CityPanel
        city={null}
        where={null}
        onenter={(c) => {
          cityMode = c;
        }}
        onleave={() => (cityMode = null)}
      />
    </div>
  {:else if enabled}
    <div id="panel-place" role="tabpanel" aria-labelledby="tab-canvas" class="place-canvas">
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
  .world-main {
    display: flex;
    align-items: flex-start;
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
  .places {
    display: inline-flex;
    gap: 0.25rem;
    margin-top: 0.8rem;
    padding: 0.2rem;
    border-radius: 10px;
    background: rgba(127, 127, 127, 0.1);
  }
  .places button {
    padding: 0.35rem 0.9rem;
    border-radius: 8px;
    border: 1px solid transparent;
    background: transparent;
    color: inherit;
    font: inherit;
    font-size: 0.85rem;
    cursor: pointer;
  }
  .places button.active {
    border-color: var(--accent, #0a84ff);
    background: color-mix(in srgb, var(--accent, #0a84ff) 14%, transparent);
  }
  .places button:focus-visible {
    outline: 2px solid var(--accent, #0a84ff);
    outline-offset: 2px;
  }
  .world-main > :global(.stage) {
    flex: 1;
    min-width: 0;
  }
  @media (max-width: 900px) {
    .world-main {
      flex-direction: column;
      align-items: stretch;
    }
    .world-main.city-main > :global(.stage) {
      max-height: 42vh;
    }
  }
  /* In the city the panel sits under the canvas at every width: the canvas
     stays on screen while the editor scrolls, so the selected object and the
     tile being clicked never leave the view. */
  .world-main.city-main {
    position: sticky;
    top: 0;
    z-index: 2;
    background: var(--bg, #1c1c1e);
    padding-bottom: 0.5rem;
  }
  .world-main.city-main > :global(.stage) {
    max-height: 58vh;
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
