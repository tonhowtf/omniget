<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import SettingGroup from "$components/settings/rebrand/SettingGroup.svelte";
  import SettingsEmptyState from "$components/settings/rebrand/SettingsEmptyState.svelte";
  import { acquireLimitsMonitor, getLimitsMonitor, saveLimitsPrefs } from "$lib/stores/limits-monitor.svelte";
  import LimitsStrip from "./LimitsStrip.svelte";
  import AdvancedDisclosure from "$components/settings/ai/AdvancedDisclosure.svelte";

  type TrayPrefs = {
    enabled: boolean;
    accounts: string[];
    seeded: boolean;
  };
  type CatalogItem = { id: string; cli: string; name: string; shown: boolean };
  type Panel = { prefs: TrayPrefs; catalog: CatalogItem[] };

  let prefs = $state<TrayPrefs | null>(null);
  let catalog = $state<CatalogItem[]>([]);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let busy = $state(false);
  let saveError = $state<string | null>(null);

  async function load() {
    loading = true;
    loadError = null;
    try {
      const panel = await invoke<Panel>("usage_tray_view");
      prefs = panel.prefs;
      catalog = panel.catalog ?? [];
    } catch {
      loadError = "load_failed";
    } finally {
      loading = false;
    }
  }

  let stripPrefs = $derived(getLimitsMonitor().prefs);

  onMount(() => {
    void load();
    return acquireLimitsMonitor();
  });

  function toggleStrip() {
    if (stripPrefs) void saveLimitsPrefs({ ...stripPrefs, enabled: !stripPrefs.enabled });
  }

  async function set(patch: Partial<Pick<TrayPrefs, "enabled">>) {
    if (!prefs || busy) return;
    busy = true;
    saveError = null;
    try {
      prefs = await invoke<TrayPrefs>("usage_tray_set", { patch });
    } catch {
      saveError = "save_failed";
      await load();
    } finally {
      busy = false;
    }
  }

  async function toggleAccount(id: string) {
    if (busy) return;
    busy = true;
    saveError = null;
    try {
      await invoke("usage_tray_toggle_account", { id });
      const panel = await invoke<Panel>("usage_tray_view");
      prefs = panel.prefs;
      catalog = panel.catalog ?? [];
    } catch {
      saveError = "save_failed";
      await load();
    } finally {
      busy = false;
    }
  }
</script>

<SettingGroup id="limits" icon="gauge" title={$t("settings.ai.limits_title") as string} description={$t("settings.agents.limits_desc") as string}>
  <div class="cards" data-setting-key="settings.agents.limits_title">
    <div class="card" class:on={stripPrefs?.enabled}>
      <svg viewBox="0 0 80 50" aria-hidden="true" focusable="false">
        <rect class="desk" x="1" y="1" width="78" height="48" rx="5" />
        <rect class="win" x="9" y="8" width="46" height="30" rx="3" />
        <rect class="track" x="70" y="10" width="5" height="30" rx="2.500" />
        <rect class="value" x="70" y="24" width="5" height="16" rx="2.500" />
      </svg>
      <div class="card-text">
        <span class="card-label" id="limits-strip-label">{$t("settings.agents.strip_label")}</span>
        <span class="card-help"><bdi>{$t("settings.agents.strip_help")}</bdi></span>
      </div>
      <button class="toggle" class:on={stripPrefs?.enabled} type="button" role="switch" aria-checked={stripPrefs?.enabled ?? false} aria-labelledby="limits-strip-label" disabled={!stripPrefs} onclick={toggleStrip}><span class="toggle-knob"></span></button>
    </div>
    <div class="card" class:on={prefs?.enabled}>
      <svg viewBox="0 0 80 50" aria-hidden="true" focusable="false">
        <rect class="desk" x="1" y="1" width="78" height="48" rx="5" />
        <path class="bar" d="M1 11h78" />
        <circle class="track-ring" cx="66" cy="6" r="3" />
        <path class="value-ring" d="M66 3a3 3 0 0 1 3 3" />
        <path class="pop" d="M46 15h28a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H46a2 2 0 0 1-2-2V17a2 2 0 0 1 2-2z" />
        <path class="pop-line" d="M49 21h22M49 27h12" />
      </svg>
      <div class="card-text">
        <span class="card-label" id="limits-tray-label">{$t("settings.agents.tray_label")}</span>
        <span class="card-help"><bdi>{$t("settings.agents.tray_help")}</bdi></span>
      </div>
      <button class="toggle" class:on={prefs?.enabled} type="button" role="switch" aria-checked={prefs?.enabled ?? false} aria-labelledby="limits-tray-label" disabled={!prefs || busy} onclick={() => set({ enabled: !prefs!.enabled })}><span class="toggle-knob"></span></button>
    </div>
  </div>
  <AdvancedDisclosure id="limits">
    <h3>{$t("settings.agents.strip_label")}</h3>
    <LimitsStrip />
    <h3>{$t("settings.agents.tray_label")}</h3>
  {#if loading && !prefs}
    <p class="indicator-state" role="status">{$t("settings.rebrand.state.loading")}</p>
  {:else if loadError && !prefs}
    <SettingsEmptyState error title={$t("settings.rebrand.state.unavailable") as string} onRetry={load} />
  {:else if prefs}
    <p class="indicator-state"><bdi>{$t("settings.ai.tray_menu_hint")}</bdi></p>
    <fieldset class="indicator-accounts" disabled={busy} data-setting-key="usage_tray.menu.accounts">
      <legend>{$t("usage_tray.menu.accounts")}</legend>
      {#if catalog.length === 0}
        <p class="indicator-state"><bdi>{$t("usage_tray.menu.no_accounts")}</bdi></p>
      {:else}
        {#each catalog as item (item.id)}
          <label class="indicator-account">
            <input type="checkbox" checked={item.shown} onchange={() => toggleAccount(item.id)} />
            <span>{item.name}</span>
          </label>
        {/each}
      {/if}
    </fieldset>

    {#if saveError}
      <p class="indicator-error" role="alert">{$t("settings.rebrand.state.save_failed")}</p>
    {/if}
  {/if}
  </AdvancedDisclosure>
</SettingGroup>

<style>
  .cards { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 280px), 1fr)); gap: var(--space-2); }
  .card { display: flex; align-items: center; gap: var(--space-2); padding: var(--space-2); border: 1px solid var(--content-border); border-radius: var(--radius-md); background: var(--primary); }
  .card.on { border-color: var(--accent); }
  .card svg { flex: none; display: block; inline-size: 80px; block-size: 50px; }
  .card-text { flex: 1; display: flex; flex-direction: column; gap: 2px; min-inline-size: 0; }
  .card-label { font-size: 13px; line-height: 18px; font-weight: 600; color: var(--text); }
  .card-help { font-size: 12px; line-height: 17px; color: var(--text-muted); }
  .desk { fill: var(--surface-src); stroke: var(--content-border); }
  .win { fill: var(--surface-hi-src, var(--surface-src)); stroke: var(--tertiary); }
  .track { fill: var(--surface-hi-src, var(--surface-src)); stroke: var(--content-border); }
  .value { fill: var(--accent); }
  .bar { fill: none; stroke: var(--content-border); }
  .track-ring { fill: none; stroke: var(--tertiary); }
  .value-ring { fill: none; stroke: var(--accent); stroke-width: 1.500; stroke-linecap: round; }
  .pop { fill: var(--surface-hi-src, var(--surface-src)); stroke: var(--tertiary); }
  .pop-line { fill: none; stroke: var(--accent); stroke-width: 2; stroke-linecap: round; }
  h3 { margin-block: var(--space-3) var(--space-1); font-size: 13px; line-height: 18px; font-weight: 600; color: var(--text); }
  .indicator-state {
    margin: 0;
    font-size: 12px;
    line-height: 18px;
    color: var(--text-muted);
  }
  .indicator-accounts {
    margin: var(--space-3) 0 0;
    padding: 0;
    border: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-inline-size: 0;
  }
  .indicator-accounts legend {
    padding: 0 0 var(--space-1);
    font-size: 13px;
    line-height: 20px;
    color: var(--text);
  }
  .indicator-account {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-block-size: 32px;
    font-size: 13px;
    overflow-wrap: anywhere;
    cursor: pointer;
  }
  .indicator-account input {
    accent-color: var(--accent);
  }
  .indicator-error {
    margin: var(--space-2) 0 0;
    font-size: 12px;
    line-height: 18px;
    color: var(--danger);
    overflow-wrap: anywhere;
  }
</style>
