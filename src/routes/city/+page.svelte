<script lang="ts">
  /**
   * `/city` — the procedural 3D city (estudo 99, GameChangeLLM M0-M8).
   *
   *   (no query)  the city in `CityView`, full size, with collapsible side
   *               panels: found, inspector, avatar, create object, import, live
   *   ?bench=1    the M0 bench of the three3d backend (2k/10k instances, with
   *               and without shadow, first frame, sleep/wake), hidden
   *   ?bench=<scenario>  the CI city bench (`OMNIGET_WORLD_BENCH_ROUTE=/city`):
   *               example city, 3D tier 0 and lite mode, report to Rust
   *
   * With no city yet (`city3d_status`), founding takes the stage. Everything
   * 3D is imported dynamically by `CityView` and the bench.
   */
  import { onMount } from "svelte";
  import { page } from "$app/stores";
  import { t } from "$lib/i18n";
  import CityView from "$components/city3d/CityView.svelte";
  import FoundingPanel from "$components/city3d/FoundingPanel.svelte";
  import InspectorPanel from "$components/city3d/InspectorPanel.svelte";
  import AvatarCreator from "$components/city3d/AvatarCreator.svelte";
  import CodegeoPanel from "$components/city3d/CodegeoPanel.svelte";
  import ImportPanel from "$components/city3d/ImportPanel.svelte";
  import LivePanel from "$components/city3d/LivePanel.svelte";
  import type { CameraMode3D } from "$lib/world/render/types";
  import type { CityBackend } from "$lib/world/render/tier";
  import type { BenchReport, BenchRow, CitySelection } from "$lib/world/render/three3d";
  import type { AnyCityScene } from "$lib/world/render/citylite";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";

  type PanelId = "found" | "inspector" | "avatar" | "codegeo" | "import" | "live";
  const PANELS: PanelId[] = ["found", "inspector", "avatar", "codegeo", "import", "live"];
  const CAMERA_KEY = "omniget.city3d.camera";
  const HUD_KEY = "omniget.city3d.hud";

  let benchParam = $derived($page.url.searchParams.get("bench"));
  let bench = $derived(benchParam === "1");
  /** CI: any other value is a scenario name from the Rust bench hook. */
  let ciBench = $derived(!!benchParam && benchParam !== "1");
  let ciHost = $state<HTMLDivElement | null>(null);

  // ── city ────────────────────────────────────────────────────────────────
  let checked = $state(false);
  let hasCity = $state(false);
  let live = $state<{ city: string; server: string | null } | null>(null);
  let viewKey = $state(0);
  let selection = $state<CitySelection | null>(null);
  let scene = $state<AnyCityScene | null>(null);
  // Dev builds only: lets the test driver read the avatar's bones (live pose checks).
  if (import.meta.env.DEV) {
    $effect(() => {
      (window as unknown as Record<string, unknown>).__omnigetCity3dScene = scene;
    });
  }
  /** What the view is drawing with, and the person's choice (null = automatic). */
  let activeBackend = $state<CityBackend | null>(null);
  let backendChoice = $state<CityBackend | null>(pickBackend(getSettings()?.world?.city_backend));
  let open = $state<PanelId | null>(null);
  let collapsed = $state(false);
  let cameraMode = $state<CameraMode3D>(readPref(CAMERA_KEY) === "snap" ? "snap" : "orbit");
  let hud = $state(readPref(HUD_KEY) === "1");

  function readPref(key: string): string | null {
    try {
      return localStorage.getItem(key);
    } catch {
      return null;
    }
  }
  function writePref(key: string, value: string): void {
    try {
      localStorage.setItem(key, value);
    } catch {
      // Not remembered in a private window.
    }
  }

  async function refreshStatus(): Promise<void> {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const s = (await invoke("city3d_status")) as { mode?: string; city?: Record<string, unknown> | null };
      hasCity = !!s?.city;
      const c = s?.city ?? null;
      if (s?.mode === "server" && c) {
        const id = (c.city_id ?? (c.charter as { city_id?: string } | undefined)?.city_id ?? c.id) as string | undefined;
        live = id ? { city: id, server: (c.server as string | undefined) ?? null } : null;
      } else {
        live = null;
      }
    } catch {
      hasCity = false;
    } finally {
      checked = true;
    }
  }

  /** Founding finished: remount the view on the new city. */
  async function onFounded(): Promise<void> {
    await refreshStatus();
    viewKey++;
    open = null;
  }

  function toggle(id: PanelId): void {
    open = open === id ? null : id;
    collapsed = false;
  }

  function onselect(sel: CitySelection | null): void {
    selection = sel;
    if (sel) {
      open = "inspector";
      collapsed = false;
    }
  }

  function setCamera(m: CameraMode3D): void {
    cameraMode = m;
    writePref(CAMERA_KEY, m);
  }

  function pickBackend(v: unknown): CityBackend | null {
    return v === "lite" || v === "3d" ? v : null;
  }

  /** Remembered in the world settings; the view remounts on the new backend. */
  async function setBackend(next: CityBackend | null): Promise<void> {
    if (next === backendChoice && (next === null || next === activeBackend)) return;
    backendChoice = next;
    try {
      await updateSettings({ world: { city_backend: next } });
    } catch {
      // Not saved (no backend in a plain browser); it still applies now.
    }
    if (next === null) {
      try {
        localStorage.removeItem("omniget.city3d.backend.v1");
      } catch {
        // Nothing remembered.
      }
    }
    viewKey++;
  }

  function toggleHud(): void {
    hud = !hud;
    writePref(HUD_KEY, hud ? "1" : "0");
  }

  // ── bench ───────────────────────────────────────────────────────────────
  let benchHost = $state<HTMLDivElement | null>(null);
  let benchRunning = $state(false);
  let benchRows = $state<BenchRow[]>([]);
  let benchReport = $state<BenchReport | null>(null);
  let benchError = $state("");

  async function runBench(): Promise<void> {
    if (!benchHost || benchRunning) return;
    benchRunning = true;
    benchRows = [];
    benchReport = null;
    benchError = "";
    try {
      const { runCityBench } = await import("$lib/world/render/three3d/bench");
      benchReport = await runCityBench(benchHost, { onRow: (row) => (benchRows = [...benchRows, row]) });
      (window as unknown as Record<string, unknown>).__omnigetCity3dBench = benchReport;
      console.info("[city3d-bench]", JSON.stringify(benchReport));
    } catch (e) {
      benchError = e instanceof Error ? e.message : String(e);
    } finally {
      benchRunning = false;
    }
  }

  onMount(() => {
    if (ciBench) {
      void import("$lib/world/bench/city-runner").then(({ runCityCiBench }) => runCityCiBench(ciHost!, benchParam!));
      return;
    }
    if (bench) void runBench();
    else void refreshStatus();
    let stop: (() => void) | null = null;
    let stopWorks: (() => void) | null = null;
    if (!bench) {
      // Works under construction advance while the city is open.
      void import("$lib/city3d/api").then((api) => (stopWorks = api.startWorksTicker()));
      // A founding started elsewhere (MCP tool, another window) lands here too.
      void import("@tauri-apps/api/event")
        .then(({ listen }) =>
          listen<{ done?: boolean; error?: string | null }>("city3d:founding", (e) => {
            if (e.payload?.done && !e.payload?.error) void onFounded();
          }),
        )
        .then((un) => (stop = un))
        .catch(() => {});
    }
    return () => {
      stop?.();
      stopWorks?.();
    };
  });

  const kb = (b: number) => `${(b / 1024 / 1024).toFixed(1)} MB`;
</script>

{#if ciBench}
  <div class="ci-bench" bind:this={ciHost}></div>
{:else if bench}
  <section class="bench">
    <header class="bench-head">
      <h1>{$t("city3d.view.bench_title")}</h1>
      <button class="button" onclick={runBench} disabled={benchRunning}>
        {benchRunning ? $t("city3d.view.bench_running") : $t("city3d.view.bench_run")}
      </button>
    </header>
    {#if benchError}<p class="error">{benchError}</p>{/if}
    <div class="bench-rows">
      {#each benchRows as row (row.label)}
        <div class="bench-row" class:failed={!!row.error}>
          <strong>{row.label}</strong>
          <span>{row.instances.toLocaleString()} inst · {row.drawCalls} draws · {(row.triangles / 1000).toFixed(0)}k tri</span>
          <span>p50 {row.p50} ms · p95 {row.p95} ms · cpu {row.cpuP50} ms</span>
          <span>{$t("city3d.view.bench_first")} {row.firstMs} ms · compile {row.compileMs} ms · build {row.buildMs} ms</span>
          <span>{kb(row.gpuBytes)} · {row.gpu}</span>
          {#if row.error}<span class="error">{row.error}</span>{/if}
        </div>
      {/each}
    </div>
    {#if benchReport}
      <p class="restore" class:ok={benchReport.restore.ok}>
        {$t("city3d.view.bench_restore")}: {benchReport.restore.ok ? "ok" : "fail"}
        ({benchReport.restore.beforePixel?.join(",")} → {benchReport.restore.afterPixel?.join(",")}{benchReport.restore.canvasSwapped ? ", canvas swapped" : ""}, {benchReport.restore.ms} ms)
      </p>
      <pre class="json">{JSON.stringify(benchReport, null, 2)}</pre>
    {/if}
    <div class="bench-stage" bind:this={benchHost}></div>
  </section>
{:else}
  <section class="city-page">
    <div class="stage">
      {#if checked && hasCity}
        {#key viewKey}
          <CityView {cameraMode} diagnostics={hud} {live} backend={backendChoice} onbackend={(b) => (activeBackend = b)} {onselect} bind:scene />
        {/key}
      {:else if checked}
        <div class="founding">
          <h1>{$t("city3d.view.no_city_title")}</h1>
          <p>{$t("city3d.view.no_city_body")}</p>
          <div class="founding-panel">
            <FoundingPanel onfounded={onFounded} />
          </div>
        </div>
      {:else}
        <div class="founding"><p>{$t("city3d.view.loading")}</p></div>
      {/if}

      {#if checked && hasCity}
        <div class="toolbar" role="toolbar" aria-label={$t("city3d.view.toolbar") as string}>
          <div class="segmented" role="group" aria-label={$t("city3d.view.backend") as string}>
            <button class:active={backendChoice === null} aria-pressed={backendChoice === null} title={$t("city3d.view.backend_auto_hint") as string} onclick={() => setBackend(null)}>{$t("city3d.view.backend_auto")}</button>
            <button class:active={activeBackend === "lite"} aria-pressed={activeBackend === "lite"} onclick={() => setBackend("lite")}>{$t("city3d.view.backend_lite")}</button>
            <button class:active={activeBackend === "3d"} aria-pressed={activeBackend === "3d"} onclick={() => setBackend("3d")}>{$t("city3d.view.backend_3d")}</button>
          </div>
          {#if activeBackend !== "lite"}
            <div class="segmented" role="group" aria-label={$t("city3d.view.camera") as string}>
              <button class:active={cameraMode === "orbit"} aria-pressed={cameraMode === "orbit"} onclick={() => setCamera("orbit")}>{$t("city3d.view.camera_orbit")}</button>
              <button class:active={cameraMode === "snap"} aria-pressed={cameraMode === "snap"} onclick={() => setCamera("snap")}>{$t("city3d.view.camera_snap")}</button>
            </div>
          {/if}
          <button class="chip" class:active={hud} aria-pressed={hud} onclick={toggleHud}>{$t("city3d.view.diagnostics")}</button>
        </div>
        <p class="hint">{activeBackend === "lite" ? $t("city3d.view.hint_lite") : cameraMode === "snap" ? $t("city3d.view.hint_snap") : $t("city3d.view.hint_orbit")}</p>
      {/if}
    </div>

    {#if checked && hasCity}
      <aside class="side" class:collapsed>
        <div class="tabs" role="tablist" aria-orientation="vertical">
          <button class="collapse" onclick={() => (collapsed = !collapsed)} aria-expanded={!collapsed} aria-label={$t(collapsed ? "city3d.view.expand" : "city3d.view.collapse") as string}>
            {collapsed ? "‹" : "›"}
          </button>
          {#each PANELS as id (id)}
            <button role="tab" aria-selected={open === id} class:active={open === id} onclick={() => toggle(id)}>{$t(`city3d.view.panel_${id}`)}</button>
          {/each}
        </div>
        {#if open && !collapsed}
          <div class="panel" role="tabpanel">
            <h2>{$t(`city3d.view.panel_${open}`)}</h2>
            {#if open === "found"}
              <FoundingPanel onfounded={onFounded} />
            {:else if open === "inspector"}
              <InspectorPanel {selection} />
            {:else if open === "avatar"}
              <AvatarCreator />
            {:else if open === "codegeo"}
              <CodegeoPanel />
            {:else if open === "import"}
              <ImportPanel />
            {:else if open === "live"}
              <LivePanel onPose={(pose) => scene?.applyPose(pose)} />
            {/if}
          </div>
        {/if}
      </aside>
    {/if}
  </section>
{/if}

<style>
  .city-page {
    position: relative;
    display: flex;
    height: calc(100dvh - var(--safe-area-top, 0px) - 3rem);
    min-height: 420px;
    border-radius: var(--radius-lg, 12px);
    overflow: hidden;
    background: var(--surface, #fff);
    border: 1px solid var(--separator, rgba(127, 127, 127, 0.25));
  }
  .stage {
    position: relative;
    flex: 1;
    min-width: 0;
  }
  .toolbar {
    position: absolute;
    top: var(--space-2, 0.5rem);
    left: var(--space-2, 0.5rem);
    display: flex;
    gap: var(--space-2, 0.5rem);
    align-items: center;
  }
  .segmented {
    display: flex;
    padding: 2px;
    border-radius: var(--radius-full, 999px);
    background: var(--material, rgba(255, 255, 255, 0.8));
    backdrop-filter: blur(var(--material-blur, 12px));
  }
  .segmented button,
  .chip {
    border: 0;
    font: inherit;
    font-size: var(--text-xs, 0.75rem);
    padding: 0.25rem 0.7rem;
    border-radius: var(--radius-full, 999px);
    background: transparent;
    color: var(--text, inherit);
    cursor: pointer;
  }
  .chip {
    background: var(--material, rgba(255, 255, 255, 0.8));
    backdrop-filter: blur(var(--material-blur, 12px));
  }
  .segmented button.active,
  .chip.active {
    background: var(--accent, #0a84ff);
    color: var(--on-accent, #fff);
  }
  .segmented button:focus-visible,
  .chip:focus-visible,
  .tabs button:focus-visible {
    outline: 2px solid var(--focus-ring, var(--accent, #0a84ff));
    outline-offset: 2px;
  }
  .hint {
    position: absolute;
    right: var(--space-2, 0.5rem);
    bottom: var(--space-2, 0.5rem);
    margin: 0;
    padding: 0.2rem 0.6rem;
    border-radius: var(--radius-full, 999px);
    background: var(--material, rgba(255, 255, 255, 0.8));
    color: var(--text-muted, inherit);
    font-size: var(--text-xs, 0.72rem);
    pointer-events: none;
  }
  .founding {
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3, 0.75rem);
    padding: var(--space-6, 2rem);
    text-align: center;
    background: linear-gradient(180deg, color-mix(in srgb, #bfdcef 40%, var(--surface, #fff)), var(--surface, #fff));
  }
  .founding h1 {
    margin: 0;
    font-size: var(--text-xl, 1.4rem);
  }
  .founding p {
    margin: 0;
    color: var(--text-muted, inherit);
    max-width: 36rem;
  }
  .founding-panel {
    width: min(40rem, 100%);
    text-align: left;
  }
  .side {
    display: flex;
    border-left: 1px solid var(--separator, rgba(127, 127, 127, 0.25));
    background: var(--pane-bg, var(--surface, #fff));
    max-width: min(36rem, 58vw);
  }
  .tabs {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-2, 0.5rem) var(--space-1, 0.25rem);
    border-right: 1px solid var(--separator, rgba(127, 127, 127, 0.25));
  }
  .tabs button {
    border: 0;
    background: transparent;
    color: var(--text, inherit);
    font: inherit;
    font-size: var(--text-xs, 0.75rem);
    text-align: left;
    padding: 0.4rem 0.6rem;
    border-radius: var(--radius-sm, 6px);
    cursor: pointer;
    white-space: nowrap;
  }
  .tabs button:hover {
    background: var(--fill-2, rgba(127, 127, 127, 0.12));
  }
  .tabs button.active {
    background: var(--accent-soft, rgba(10, 132, 255, 0.15));
    color: var(--accent-text, var(--accent, #0a84ff));
  }
  .tabs .collapse {
    text-align: center;
    font-size: var(--text-md, 1rem);
  }
  .panel {
    flex: 1;
    width: 22rem;
    min-width: 0;
    overflow-y: auto;
    padding: var(--space-3, 0.75rem);
  }
  .panel h2 {
    margin: 0 0 var(--space-2, 0.5rem);
    font-size: var(--text-md, 1rem);
    overflow-wrap: anywhere;
  }
  .side.collapsed .tabs button:not(.collapse) {
    display: none;
  }
  @media (max-width: 760px) {
    .city-page {
      flex-direction: column;
      height: auto;
    }
    .stage {
      height: 58vh;
    }
    .side {
      max-width: none;
      border-left: 0;
      border-top: 1px solid var(--separator, rgba(127, 127, 127, 0.25));
      flex-direction: column;
    }
    .tabs {
      flex-direction: row;
      flex-wrap: wrap;
      border-right: 0;
    }
    .panel {
      width: auto;
    }
  }

  .bench {
    display: flex;
    flex-direction: column;
    gap: var(--space-3, 0.75rem);
  }
  .bench-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3, 0.75rem);
  }
  .bench-head h1 {
    margin: 0;
    font-size: var(--text-lg, 1.2rem);
  }
  .bench-rows {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(18rem, 1fr));
    gap: var(--space-2, 0.5rem);
  }
  .bench-row {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-2, 0.5rem) var(--space-3, 0.75rem);
    border-radius: var(--radius-md, 8px);
    background: var(--fill-1, rgba(127, 127, 127, 0.08));
    font-size: var(--text-xs, 0.75rem);
    font-family: var(--font-mono, ui-monospace, monospace);
  }
  .bench-row strong {
    font-family: var(--font-body, inherit);
    font-size: var(--text-sm, 0.85rem);
  }
  .bench-row.failed {
    outline: 1px solid var(--error, #d33);
  }
  .restore {
    margin: 0;
    color: var(--error, #d33);
  }
  .restore.ok {
    color: var(--success, #2a9d4b);
  }
  .error {
    color: var(--error, #d33);
  }
  .json {
    max-height: 16rem;
    overflow: auto;
    font-size: 11px;
    padding: var(--space-2, 0.5rem);
    border-radius: var(--radius-md, 8px);
    background: var(--fill-1, rgba(127, 127, 127, 0.08));
  }
  .ci-bench {
    width: 1280px;
    height: 720px;
  }
  .bench-stage {
    width: min(100%, 960px);
  }
</style>
