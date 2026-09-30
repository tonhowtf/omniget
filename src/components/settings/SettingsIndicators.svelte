<script lang="ts">
  /**
   * Settings > General > indicators: the limits strip and the menu bar usage
   * icon. Mounted by the settings route with no props.
   *
   * Opening this reads the saved prefs and nothing else: it never opens
   * `/limits-strip` or `/usage-panel`, never switches an indicator on and
   * never refreshes a reading. Only the controls below write, through the
   * same commands the native menu uses (`usage_tray_set`,
   * `usage_tray_toggle_account`). `seeded` stays internal.
   */
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import LimitsStrip from "$components/llm/accounts/LimitsStrip.svelte";
  import SettingGroup from "./rebrand/SettingGroup.svelte";
  import SettingRow from "./rebrand/SettingRow.svelte";
  import SettingsEmptyState from "./rebrand/SettingsEmptyState.svelte";

  type Shows = "session" | "weekly" | "busiest";
  type TrayPrefs = {
    enabled: boolean;
    accounts: string[];
    seeded: boolean;
    shows: Shows;
    percent_text: boolean;
    colored: boolean;
    warn: number;
    critical: number;
  };
  type CatalogItem = { id: string; cli: string; name: string; shown: boolean };
  type Panel = { prefs: TrayPrefs; catalog: CatalogItem[] };

  const SHOWS: Shows[] = ["session", "weekly", "busiest"];
  /** The same presets the native menu offers (`THRESHOLDS` in usage_tray/mod.rs). */
  const THRESHOLDS: [number, number][] = [[70, 90], [80, 95], [90, 98]];

  let prefs = $state<TrayPrefs | null>(null);
  let catalog = $state<CatalogItem[]>([]);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let busy = $state(false);
  let saveError = $state<string | null>(null);

  let thresholdValue = $derived(prefs ? `${prefs.warn}-${prefs.critical}` : "");
  // A pair set elsewhere (hand-edited file) stays selectable instead of being rewritten.
  let customThreshold = $derived(!!prefs && !THRESHOLDS.some(([w, c]) => w === prefs!.warn && c === prefs!.critical));

  async function load() {
    loading = true;
    loadError = null;
    try {
      const panel = await invoke<Panel>("usage_tray_view");
      prefs = panel.prefs;
      catalog = panel.catalog ?? [];
    } catch {
      // Raw backend errors may carry file paths; the UI shows a stable state only.
      loadError = "load_failed";
    } finally {
      loading = false;
    }
  }

  onMount(() => { void load(); });

  async function set(patch: Partial<Pick<TrayPrefs, "enabled" | "shows" | "percent_text" | "colored" | "warn" | "critical">>) {
    if (!prefs || busy) return;
    busy = true;
    saveError = null;
    try {
      prefs = await invoke<TrayPrefs>("usage_tray_set", { patch });
    } catch {
      // The backend kept the saved prefs; show those instead of the attempt.
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

  function setThreshold(value: string) {
    const [warn, critical] = value.split("-").map(Number);
    if (Number.isInteger(warn) && Number.isInteger(critical)) void set({ warn, critical });
  }

  function thresholdLabel(warn: number, critical: number): string {
    return $t("usage_tray.menu.threshold_item", { warn, critical }) as string;
  }
</script>

<SettingGroup id="limits-strip" title={$t("llm.limits.title") as string} description={$t("llm.limits.hint") as string}>
  <LimitsStrip embedded />
</SettingGroup>

<SettingGroup id="usage-tray" title={$t("usage_tray.menu.toggle") as string} description={$t("usage_tray.panel.empty_body") as string}>
  {#if loading && !prefs}
    <p class="indicator-state" role="status">{$t("settings.rebrand.state.loading")}</p>
  {:else if loadError && !prefs}
    <SettingsEmptyState error title={$t("settings.rebrand.state.unavailable") as string} onRetry={load} />
  {:else if prefs}
    <SettingRow id="usage-tray-enabled" label={$t("usage_tray.menu.toggle") as string}>
      {#snippet control({ id, describedBy })}
        <button
          {id}
          type="button"
          class="toggle"
          class:on={prefs!.enabled}
          role="switch"
          aria-checked={prefs!.enabled}
          aria-label={$t("usage_tray.menu.toggle") as string}
          aria-describedby={describedBy}
          disabled={busy}
          onclick={() => set({ enabled: !prefs!.enabled })}
        >
          <span class="toggle-knob"></span>
        </button>
      {/snippet}
    </SettingRow>

    <SettingRow id="usage-tray-shows" label={$t("usage_tray.menu.icon_shows") as string}>
      {#snippet control({ id, describedBy })}
        <select {id} class="input" aria-describedby={describedBy} value={prefs!.shows} disabled={busy}
          onchange={(e) => set({ shows: e.currentTarget.value as Shows })}>
          {#each SHOWS as value (value)}
            <option {value}>{$t(`usage_tray.menu.shows_${value}`)}</option>
          {/each}
        </select>
      {/snippet}
    </SettingRow>

    <SettingRow id="usage-tray-percent" label={$t("usage_tray.menu.percent_text") as string}>
      {#snippet control({ id, describedBy })}
        <button {id} type="button" class="toggle" class:on={prefs!.percent_text} role="switch"
          aria-checked={prefs!.percent_text} aria-label={$t("usage_tray.menu.percent_text") as string} aria-describedby={describedBy} disabled={busy}
          onclick={() => set({ percent_text: !prefs!.percent_text })}>
          <span class="toggle-knob"></span>
        </button>
      {/snippet}
    </SettingRow>

    <SettingRow id="usage-tray-colored" label={$t("usage_tray.menu.colored") as string}>
      {#snippet control({ id, describedBy })}
        <button {id} type="button" class="toggle" class:on={prefs!.colored} role="switch"
          aria-checked={prefs!.colored} aria-label={$t("usage_tray.menu.colored") as string} aria-describedby={describedBy} disabled={busy}
          onclick={() => set({ colored: !prefs!.colored })}>
          <span class="toggle-knob"></span>
        </button>
      {/snippet}
    </SettingRow>

    <SettingRow id="usage-tray-thresholds" label={$t("usage_tray.menu.thresholds") as string}>
      {#snippet control({ id, describedBy })}
        <select {id} class="input" aria-describedby={describedBy} value={thresholdValue} disabled={busy}
          onchange={(e) => setThreshold(e.currentTarget.value)}>
          {#if customThreshold}
            <option value={thresholdValue}>{thresholdLabel(prefs!.warn, prefs!.critical)}</option>
          {/if}
          {#each THRESHOLDS as [warn, critical] (`${warn}-${critical}`)}
            <option value={`${warn}-${critical}`}>{thresholdLabel(warn, critical)}</option>
          {/each}
        </select>
      {/snippet}
    </SettingRow>

    <fieldset class="indicator-accounts" disabled={busy}>
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
</SettingGroup>

<style>
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
