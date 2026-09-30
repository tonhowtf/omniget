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
  import SettingGroup from "$components/settings/rebrand/SettingGroup.svelte";
  import SettingRow from "$components/settings/rebrand/SettingRow.svelte";

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
      // The renderer's error text is diagnostic, not user copy.
      console.warn("[world] calibration failed", error);
      showToast("error", $t("world.error") as string);
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

<SettingGroup id="world" title={$t("world.settings.title")} description={$t("world.settings.desc")}>
  <SettingRow id="world-enabled" label={$t("world.settings.enabled")} help={$t("world.settings.enabled_desc")} settingKey="world.settings.enabled" status="world.enabled">
    {#snippet control(c)}
      <button
        type="button"
        class="toggle"
        class:on={enabled}
        id={c.id}
        aria-describedby={c.describedBy}
        onclick={toggleEnabled}
        role="switch"
        aria-checked={enabled}
        aria-label={$t("world.settings.enabled") as string}
      ><span class="toggle-knob"></span></button>
    {/snippet}
  </SettingRow>
</SettingGroup>

<SettingGroup id="world-advanced" title={$t("settings.rebrand.category.advanced")} advanced>
  <SettingRow id="world-recalibrate" label={$t("world.settings.tier_measured")} help={measured === null ? ($t("world.settings.never_measured") as string) : `${tierLabel(measured)}${medianMs !== null ? ` · ${medianMs.toFixed(1)} ms` : ""}${measuredVersion ? ` · v${measuredVersion}` : ""}`} settingKey="world.settings.tier_measured">
    {#snippet control(c)}
      <button type="button" class="btn btn-secondary" id={c.id} aria-describedby={c.describedBy} onclick={recalibrate} disabled={calibrating} aria-busy={calibrating}>
        {calibrating ? $t("world.settings.recalibrating") : $t("world.settings.recalibrate")}
      </button>
    {/snippet}
  </SettingRow>

  <SettingRow id="world-tier-pinned" label={$t("world.settings.tier_pinned")} help={$t("world.settings.tier_pinned_desc")} settingKey="world.settings.tier_pinned" status="world.tier_override">
    {#snippet control(c)}
      <select id={c.id} aria-describedby={c.describedBy} class="input-text select" value={pinned === null ? "auto" : String(pinned)} onchange={(e) => pin((e.target as HTMLSelectElement).value)}>
        <option value="auto">{$t("world.settings.tier_auto")}</option>
        {#each TIERS as tier (tier)}
          <option value={String(tier)}>{tierLabel(tier)}</option>
        {/each}
      </select>
    {/snippet}
  </SettingRow>

  {#if lowered !== null}
    <p class="world-lowered" role="status">
      <strong>{$t("world.settings.tier_lowered")}</strong> {tierLabel(lowered)}
    </p>
  {/if}

</SettingGroup>

<style>
  .world-lowered { margin: 0; padding-block: var(--space-2); font-size: 12px; line-height: 18px; color: var(--text-muted); }
</style>
