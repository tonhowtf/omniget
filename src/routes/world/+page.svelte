<script lang="ts">
  /**
   * `/world` — the route of the agents' world.
   *
   * Three modes, and the first two are the CI's:
   *   ?bench=selftest  the renderer's own reproducible measurement
   *   ?bench=<name>    the world bench of `.github/workflows/world-bench.yml`
   *   ?demo=1          the world, with the residents put to work by scripted
   *                    jobs (`&agents=8` for a crowded house); what the README
   *                    films and what the owner looks at to see agents working
   *   (no query)       the world itself, in `WorldCanvas`
   *
   * The bench modes keep a bare canvas of their own: they measure the renderer
   * and the IPC, and a HUD, a session and a house map would measure something
   * else. Everything under `$lib/world` is imported dynamically from here, so
   * an app that never opens this route never downloads, parses or runs a byte
   * of the world.
   */
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import WorldCanvas from "$components/world/WorldCanvas.svelte";
  import HousePanel from "$components/world/HousePanel.svelte";
  import CityCanvas from "$components/world/CityCanvas.svelte";
  import CityPanel, { type EditorHandoff } from "$components/world/CityPanel.svelte";
  import ActivityPanel from "$components/world/ActivityPanel.svelte";
  import type { AgentRow } from "$lib/world/activity";
  import { showToast } from "$lib/stores/toast-store.svelte";

  // The world is opt-in (`settings.world.enabled`, off by default). The nav
  // item is already hidden in that case, but the route can still be reached by
  // its URL, and mounting the canvas would be the one thing that asks the
  // backend to build a world. The bench modes are exempt: they measure the
  // renderer and never touch the simulation.
  let enabled = $derived(getSettings()?.world?.enabled ?? true);

  // Home, or a visit to someone's open house. Changing it remounts the canvas,
  // which is what closes one session and opens the other.
  let visit = $state<{ code: string; server: string | null } | null>(null);
  // The city: the persistent world on an OmniDisc instance. Set, it replaces
  // the house on screen; the house keeps ticking in the backend meanwhile.
  let cityMode = $state<{ city: string; server: string | null } | null>(null);
  let cityState = $state<{ region: string; ent: number; tick: number; interior: boolean } | null>(null);
  let cityRef = $state<{ say: (ent: number, text: string) => void; goTo: (tile: [number, number]) => Promise<void> } | null>(null);
  let cityPanelRef = $state<{ refreshHome: () => void; clearTool: () => void; selectionChanged: (id: string | null) => void; editorChanged: () => void } | null>(null);
  let cityCrop = $state("carrot");
  let cityEdit = $state<EditorHandoff | null>(null);
  let canvasRef = $state<{ say: (ent: number, text: string) => void; focus: (ent: number) => void; frameHouse: () => void } | null>(null);
  let residents = $state<AgentRow[]>([]);
  let demoBusy = $state(false);

  /** Scripted jobs on a scripted provider: the house at work, no token spent. */
  async function runDemo(agents?: number): Promise<void> {
    if (demoBusy) return;
    demoBusy = true;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("world_demo", { agents: agents ?? null });
      // The posts are in different rooms: the whole house is the shot.
      canvasRef?.frameHouse();
      // The button comes back when the scripted turns are about over.
      setTimeout(() => (demoBusy = false), 22_000);
    } catch (e) {
      demoBusy = false;
      showToast("error", e instanceof Error ? e.message : String(e));
    }
  }

  function onCanvasReady(): void {
    const params = $page.url.searchParams;
    if (params.get("demo") !== "1" || visit) return;
    const n = Number(params.get("agents"));
    void runDemo(Number.isInteger(n) && n > 0 ? n : undefined);
  }

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

<div class="world-page">
  <header class="world-head">
    <h1>{$t("world.title")}</h1>
    <p class="world-sub">{$t("world.subtitle")}</p>
    <a class="yard-link" href="/world/yard">{$t("world.yard.title")} →</a>
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
    <div class="world-main">
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
      onenter={(c) => (cityMode = c)}
      onleave={() => {
        cityMode = null;
        cityState = null;
        cityEdit = null;
      }}
      onsay={(ent, text) => cityRef?.say(ent, text)}
      ongoto={(tile) => void cityRef?.goTo(tile)}
    />
  {:else if enabled}
    <div class="world-main">
      {#key visit?.code ?? "home"}
        <WorldCanvas
          bind:this={canvasRef}
          {visit}
          followAll={demoBusy}
          onagents={(list) => (residents = list)}
          onready={onCanvasReady}
          onvisitfailed={(error) => {
            showToast("error", error);
            visit = null;
          }}
        />
      {/key}
      {#if !visit}
        <ActivityPanel
          agents={residents}
          {demoBusy}
          onfocus={(ent) => canvasRef?.focus(ent)}
          ondemo={() => void runDemo()}
        />
      {/if}
    </div>
    <HousePanel
      visiting={visit !== null}
      onvisit={(v) => (visit = v)}
      onleave={() => (visit = null)}
      onsay={(ent, text) => canvasRef?.say(ent, text)}
    />
    <CityPanel
      city={null}
      where={null}
      onenter={(c) => {
        visit = null;
        cityMode = c;
      }}
      onleave={() => (cityMode = null)}
    />
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
  .world-main > :global(.stage) {
    flex: 1;
    min-width: 0;
  }
  @media (max-width: 900px) {
    .world-main {
      flex-direction: column;
    }
  }
  .world-head h1 {
    margin: 0;
    font-size: 1.4rem;
  }
  .yard-link {
    display: inline-block;
    margin-top: .6rem;
    color: #bc965d;
    font-size: .85rem;
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
