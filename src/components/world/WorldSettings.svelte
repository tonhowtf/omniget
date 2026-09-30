<script lang="ts">
  /**
   * Settings → World: switching it on and off, the measured tier, the pinned
   * one and a recalibration.
   *
   * The renderer is imported here only when "recalibrate" is clicked, so
   * opening Settings costs nothing.
   */
  import { onMount } from "svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings } from "$lib/stores/settings-store.svelte";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type Tier = 0 | 1 | 2 | 3;
  const TIERS: Tier[] = [0, 1, 2, 3];

  let settings = $derived(getSettings());
  let enabled = $derived(settings?.world?.enabled ?? true);
  let measured = $derived(settings?.world?.tier_measured ?? null);
  let medianMs = $derived(settings?.world?.measured_median_ms ?? null);
  let measuredVersion = $derived(settings?.world?.measured_app_version ?? null);
  let pinned = $derived(settings?.world?.tier_override ?? null);

  let calibrating = $state(false);
  let lowered = $state<Tier | null>(null);

  function tierLabel(tier: number | null): string {
    if (tier === null || tier === undefined) return $t("world.settings.tier_none") as string;
    return `${tier} · ${$t(`world.settings.tier_names.${tier}`)}`;
  }

  async function recalibrate() {
    if (calibrating) return;
    calibrating = true;
    try {
      // The one place in Settings that loads the renderer, and only on click.
      const render = await import("$lib/world/render");
      const detail = await render.calibrateDetailed(null);
      let version: string | null = null;
      try {
        version = await getVersion();
      } catch {
        version = null;
      }
      await updateSettings({
        world: {
          tier_measured: detail.tier,
          measured_median_ms: Number.isFinite(detail.medianMs) ? detail.medianMs : null,
          measured_app_version: version,
        },
      });
      lowered = null;
    } catch (error) {
      showToast("error", `${$t("world.error")} ${String(error)}`);
    } finally {
      calibrating = false;
    }
  }

  async function pin(value: string) {
    const next = value === "auto" ? null : (Number(value) as Tier);
    await updateSettings({ world: { tier_override: next } });
  }

  async function toggleEnabled() {
    await updateSettings({ world: { enabled: !enabled } });
  }

  onMount(() => {
    // Emitted by the renderer's watchdog while /world is open in this webview.
    const onLowered = (event: Event) => {
      const to = (event as CustomEvent<{ to?: Tier }>).detail?.to;
      lowered = typeof to === "number" ? to : 0;
    };
    window.addEventListener("world:tier-changed", onLowered);
    return () => window.removeEventListener("world:tier-changed", onLowered);
  });
</script>

<section class="settings-section" id="settings-world">
  <h5 class="section-title">{$t("world.settings.title")}</h5>
  <p class="section-desc">{$t("world.settings.desc")}</p>

  <div class="card">
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t("world.settings.enabled")}</span>
        <span class="setting-path">{$t("world.settings.enabled_desc")}</span>
      </div>
      <button
        class="toggle"
        class:on={enabled}
        onclick={toggleEnabled}
        role="switch"
        aria-checked={enabled}
        aria-label={$t("world.settings.enabled") as string}
      ><span class="toggle-knob"></span></button>
    </div>

    <div class="divider"></div>

    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t("world.settings.tier_measured")}</span>
        <span class="setting-path">
          {#if measured === null}
            {$t("world.settings.never_measured")}
          {:else}
            {tierLabel(measured)}{#if medianMs !== null} · {medianMs.toFixed(1)} ms{/if}{#if measuredVersion} · v{measuredVersion}{/if}
          {/if}
        </span>
      </div>
      <button class="btn btn-secondary" onclick={recalibrate} disabled={calibrating}>
        {calibrating ? $t("world.settings.recalibrating") : $t("world.settings.recalibrate")}
      </button>
    </div>

    <div class="divider"></div>

    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t("world.settings.tier_pinned")}</span>
        <span class="setting-path">{$t("world.settings.tier_pinned_desc")}</span>
      </div>
      <select class="input-text select" value={pinned === null ? "auto" : String(pinned)} onchange={(e) => pin((e.target as HTMLSelectElement).value)}>
        <option value="auto">{$t("world.settings.tier_auto")}</option>
        {#each TIERS as tier (tier)}
          <option value={String(tier)}>{tierLabel(tier)}</option>
        {/each}
      </select>
    </div>

    {#if lowered !== null}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t("world.settings.tier_lowered")}</span>
          <span class="setting-path">{tierLabel(lowered)}</span>
        </div>
      </div>
    {/if}

  </div>
</section>

<style>
  .section-desc {
    margin: 0 0 12px;
    color: var(--text-secondary);
    font-size: 13px;
  }
</style>
