<script lang="ts">
  /**
   * The city canvas: a thin wrapper around `createCityScene` (three3d) or,
   * in lite mode, `createLiteCityScene` (the 2D renderer with the kit baked
   * into sprites). The backend comes from the setting `world.city_backend`
   * (lite / 3d), else from the 3D calibration (over the tier-0 budget → lite),
   * run once and remembered; a 3D scene that fails to start falls back to
   * lite. It also picks the 3D tier (pinned → measured → the same run), sizes
   * the canvas, sleeps with the tab hidden and wakes on return, and shows a
   * diagnostics HUD on demand. Both backends are imported dynamically.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { getSettings } from "$lib/stores/settings-store.svelte";
  import type { CameraMode3D, Tier } from "$lib/world/render/types";
  import type { CityBackend } from "$lib/world/render/tier";
  import type { CitySelection, CityStats } from "$lib/world/render/three3d";
  import type { AnyCityScene, BackendReason } from "$lib/world/render/citylite";

  interface Props {
    cameraMode?: CameraMode3D;
    /** fps, tier and counts over the canvas. */
    diagnostics?: boolean;
    /** Residents from the server session instead of local walkers. */
    live?: { city: string; server: string | null } | null;
    /** Pins the tier (no calibration, no watchdog). */
    tier?: Tier | null;
    /** Forces the backend; null reads the setting, then calibrates. */
    backend?: CityBackend | null;
    onbackend?: (backend: CityBackend, reason: BackendReason) => void;
    onselect?: (sel: CitySelection | null) => void;
    onstats?: (stats: CityStats) => void;
    onready?: (scene: AnyCityScene) => void;
    onerror?: (code: string) => void;
    scene?: AnyCityScene | null;
  }
  let {
    cameraMode = "orbit",
    diagnostics = false,
    live = null,
    tier: pinned = null,
    backend: forcedBackend = null,
    onbackend,
    onselect,
    onstats,
    onready,
    onerror,
    scene = $bindable(null),
  }: Props = $props();

  const TIER_KEY = "omniget.city3d.tier.v1";
  const BACKEND_KEY = "omniget.city3d.backend.v1";

  let host = $state<HTMLDivElement | null>(null);
  let canvas = $state<HTMLCanvasElement | null>(null);
  let status = $state<"calibrating" | "loading" | "ready" | "error">("loading");
  let errorCode = $state("");
  let stats = $state<CityStats | null>(null);
  let city: AnyCityScene | null = null;
  let active = $state<CityBackend | null>(null);
  let fellBack = $state(false);

  function pickTier(v: unknown): Tier | null {
    return v === 0 || v === 1 || v === 2 || v === 3 ? v : null;
  }

  function remembered(): Tier | null {
    try {
      return pickTier(Number(localStorage.getItem(TIER_KEY) ?? "x"));
    } catch {
      return null;
    }
  }

  function pickBackend(v: unknown): CityBackend | null {
    return v === "lite" || v === "3d" ? v : null;
  }

  function rememberedBackend(): CityBackend | null {
    try {
      return pickBackend(localStorage.getItem(BACKEND_KEY));
    } catch {
      return null;
    }
  }

  function remember(key: string, value: string): void {
    try {
      localStorage.setItem(key, value);
    } catch {
      // Private window: calibrate again next time.
    }
  }

  /** One 3D calibration run gives both the tier and the backend. */
  async function calibrateOnce(): Promise<{ tier: Tier; backend: CityBackend }> {
    status = "calibrating";
    const { calibrate3d } = await import("$lib/world/render/calibrate");
    const { cityBackendFromCalibration } = await import("$lib/world/render/tier");
    const probe = document.createElement("canvas");
    const r = await calibrate3d(probe);
    const backend = cityBackendFromCalibration(r);
    remember(TIER_KEY, String(r.tier));
    remember(BACKEND_KEY, backend);
    return { tier: r.tier, backend };
  }

  async function chooseBackend(): Promise<{ backend: CityBackend; reason: BackendReason; tier: Tier | null }> {
    const set = pickBackend(forcedBackend) ?? pickBackend(getSettings()?.world?.city_backend);
    if (set) return { backend: set, reason: "setting", tier: null };
    const known = rememberedBackend();
    if (known) return { backend: known, reason: "remembered", tier: null };
    const r = await calibrateOnce();
    return { backend: r.backend, reason: "calibration", tier: r.tier };
  }

  async function chooseTier(measured: Tier | null): Promise<{ tier: Tier; pinned: boolean }> {
    const forced = pickTier(pinned) ?? pickTier(getSettings()?.world?.tier_override);
    if (forced !== null) return { tier: forced, pinned: true };
    if (measured !== null) return { tier: measured, pinned: false };
    const known = remembered();
    if (known !== null) return { tier: known, pinned: false };
    return { tier: (await calibrateOnce()).tier, pinned: false };
  }

  function resize(): void {
    if (!city || !host) return;
    const rect = host.getBoundingClientRect();
    city.resize(rect.width, rect.height, window.devicePixelRatio || 1);
  }

  async function bootLite(reason: BackendReason): Promise<void> {
    status = "loading";
    const mod = await import("$lib/world/render/citylite");
    active = "lite";
    onbackend?.("lite", reason);
    city = await mod.createLiteCityScene({
      canvas: canvas!,
      onSelect: (sel) => onselect?.(sel),
      onStats: (s) => {
        stats = s;
        onstats?.(s);
      },
      onCanvasReplaced: (c) => (canvas = c),
      onError: (code) => {
        errorCode = code;
        onerror?.(code);
      },
    });
  }

  /** A canvas that already gave out a WebGL context cannot give another kind. */
  function freshCanvas(): void {
    if (!canvas) return;
    const next = canvas.cloneNode(false) as HTMLCanvasElement;
    canvas.replaceWith(next);
    canvas = next;
  }

  async function boot(): Promise<void> {
    try {
      const pick = await chooseBackend();
      if (pick.backend === "lite") {
        await bootLite(pick.reason);
      } else {
        try {
          await boot3d(pick);
        } catch (e) {
          // 3D could not start (no context, shader, kit): the lite mode always runs.
          city?.destroy();
          city = null;
          fellBack = true;
          errorCode = e instanceof Error ? e.message : String(e);
          if (pick.reason !== "setting") remember(BACKEND_KEY, "lite");
          freshCanvas();
          await bootLite("fallback");
        }
      }
      resize();
      await city!.ready;
      scene = city;
      status = "ready";
      onready?.(city!);
    } catch (e) {
      errorCode = e instanceof Error ? e.message : String(e);
      status = "error";
      onerror?.(errorCode);
    }
  }

  async function boot3d(pick: { reason: BackendReason; tier: Tier | null }): Promise<void> {
    const choice = await chooseTier(pick.tier);
    status = "loading";
    const mod = await import("$lib/world/render/three3d");
    active = "3d";
    onbackend?.("3d", pick.reason);
    const made = await mod.createCityScene({
      canvas: canvas!,
      tier: choice.tier,
      camera: cameraMode,
      watchdog: !choice.pinned,
      live,
      onSelect: (sel) => onselect?.(sel),
      onStats: (s) => {
        stats = s;
        onstats?.(s);
      },
      onTierChanged: (next) => {
        // The watchdog only goes down; the next session starts from here.
        try {
          localStorage.setItem(TIER_KEY, String(next));
        } catch {
          // Not remembered; harmless.
        }
      },
      onCanvasReplaced: (c) => (canvas = c),
      onError: (code) => {
        errorCode = code;
        onerror?.(code);
      },
    });
    city = made;
    resize();
    await made.ready;
  }

  $effect(() => {
    const m = cameraMode;
    city?.setCameraMode(m);
  });

  onMount(() => {
    void boot();
    const ro = new ResizeObserver(() => resize());
    if (host) ro.observe(host);
    const onVisibility = () => {
      if (!city) return;
      if (document.hidden) city.sleep();
      else void city.wake().catch((e) => (errorCode = e instanceof Error ? e.message : String(e)));
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      ro.disconnect();
      document.removeEventListener("visibilitychange", onVisibility);
      city?.destroy();
      city = null;
      scene = null;
    };
  });

  const mb = (b: number) => (b / (1024 * 1024)).toFixed(1);
</script>

<div class="city-view" bind:this={host}>
  <canvas bind:this={canvas} class="canvas" tabindex="0" aria-label={$t("city3d.view.canvas_label") as string}></canvas>

  {#if status !== "ready"}
    <div class="state" role="status">
      {#if status === "calibrating"}
        <span>{$t("city3d.view.calibrating")}</span>
      {:else if status === "loading"}
        <span>{$t("city3d.view.loading")}</span>
      {:else}
        <span class="error">{$t("city3d.view.error")} <code>{errorCode}</code></span>
      {/if}
    </div>
  {/if}

  {#if diagnostics && stats}
    <div class="hud" aria-hidden="true">
      <span>{stats.fps.toFixed(0)} fps · {active === "lite" ? $t("city3d.view.backend_lite") : $t("city3d.view.backend_3d")}</span>
      <span>{$t("city3d.view.hud_tier")} {stats.tier}</span>
      <span>{stats.instances.toLocaleString()} / {stats.instancesLoaded.toLocaleString()} {$t("city3d.view.hud_instances")}</span>
      <span>{stats.drawCalls} {$t("city3d.view.hud_draws")} · {(stats.triangles / 1000).toFixed(0)}k tri</span>
      <span>{stats.regions} {$t("city3d.view.hud_regions")} · {stats.people} {$t("city3d.view.hud_people")}{stats.rigged ? "" : " (capsule)"}</span>
      <span>{stats.cpuMs.toFixed(2)} ms cpu · {mb(stats.memoryBytes)} MB</span>
      <span>{$t("city3d.view.hud_kit")}: {stats.kitReal ? $t("city3d.view.hud_kit_real") : $t("city3d.view.hud_kit_fallback")} · {stats.compileMs.toFixed(0)} ms compile</span>
      <span class="gpu">{stats.gpu}</span>
    </div>
  {/if}
  {#if fellBack && status === "ready"}
    <div class="warn" title={errorCode}>{$t("city3d.view.lite_fallback")}</div>
  {:else if errorCode && status === "ready"}
    <div class="warn">{errorCode}</div>
  {/if}
</div>

<style>
  .city-view {
    position: relative;
    width: 100%;
    height: 100%;
    min-height: 0;
    overflow: hidden;
    background: #bfdcef;
  }
  .canvas {
    display: block;
    width: 100%;
    height: 100%;
    touch-action: none;
    outline: none;
  }
  .canvas:focus-visible {
    box-shadow: inset 0 0 0 2px var(--accent, #0a84ff);
  }
  .state {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    pointer-events: none;
    font-size: var(--text-sm, 0.85rem);
    color: var(--text, #1c1c1e);
  }
  .state span {
    padding: var(--space-2, 0.5rem) var(--space-3, 0.75rem);
    border-radius: var(--radius-full, 999px);
    background: var(--material, rgba(255, 255, 255, 0.8));
    backdrop-filter: blur(var(--material-blur, 12px));
  }
  .state .error {
    color: var(--error, #d33);
  }
  .hud {
    position: absolute;
    left: var(--space-2, 0.5rem);
    bottom: var(--space-2, 0.5rem);
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-2, 0.5rem);
    border-radius: var(--radius-md, 8px);
    background: rgba(0, 0, 0, 0.6);
    color: #fff;
    font: 11px/1.35 var(--font-mono, ui-monospace, monospace);
    pointer-events: none;
    max-width: 22rem;
  }
  .hud .gpu {
    opacity: 0.7;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .warn {
    position: absolute;
    top: var(--space-2, 0.5rem);
    left: 50%;
    transform: translateX(-50%);
    padding: 0.2rem 0.6rem;
    border-radius: var(--radius-full, 999px);
    background: var(--warning, #e6a23c);
    color: var(--on-warning, #000);
    font-size: var(--text-xs, 0.75rem);
  }
</style>
