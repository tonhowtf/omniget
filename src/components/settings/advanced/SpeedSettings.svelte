<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { getSettings, saveSettings, toggleBool, changeNumber } from "../settings-helpers";
  import SettingRow from "../rebrand/SettingRow.svelte";
  import AdvancedDisclosure from "./AdvancedDisclosure.svelte";

  type NumberRow = { key: string; field: "max_retries" | "concurrent_fragments" | "max_concurrent_segments"; min: number; max: number };
  const ADVANCED_NUMBERS: NumberRow[] = [
    { key: "max_retries", field: "max_retries", min: 1, max: 20 },
    { key: "concurrent_fragments", field: "concurrent_fragments", min: 1, max: 32 },
    { key: "max_concurrent_segments", field: "max_concurrent_segments", min: 1, max: 100 },
  ];
  const MIN_PARALLEL = 1;
  const MAX_PARALLEL = 10;

  let settings = $derived(getSettings());
  let parallel = $derived(settings?.advanced.max_concurrent_downloads ?? 1);

  async function setParallel(next: number) {
    const value = Math.min(MAX_PARALLEL, Math.max(MIN_PARALLEL, Math.round(next)));
    if (value === parallel) return;
    if (!await saveSettings({ advanced: { max_concurrent_downloads: value } })) return;
    try { await invoke("update_max_concurrent", { max: value }); }
    catch { showToast("error", $t("settings.save_failed") as string); }
  }
</script>

{#snippet toggle(id: string, describedBy: string | undefined, on: boolean, onclick: () => void)}
  <button {id} type="button" class="toggle" class:on role="switch" aria-checked={on} aria-labelledby={`${id}-label`} aria-describedby={describedBy} {onclick}><span class="toggle-knob"></span></button>
{/snippet}

{#if settings}
  <SettingRow id="setting-advanced-max_concurrent_downloads" label={$t("settings.advanced.max_concurrent_downloads")} help={$t("settings.advanced.max_concurrent_downloads_desc")}
    settingKey="settings.advanced.max_concurrent_downloads" status="advanced.max_concurrent_downloads">
    {#snippet control({ id, describedBy })}
      <span class="stepper">
        <button type="button" class="btn btn-secondary btn-sm step" aria-label={$t("settings.advanced.parallel_less") as string}
          disabled={parallel <= MIN_PARALLEL} onclick={() => setParallel(parallel - 1)}>−</button>
        <input {id} aria-describedby={describedBy} type="number" class="input-number value" dir="ltr" min={MIN_PARALLEL} max={MAX_PARALLEL} step="1"
          value={parallel} onchange={(e) => changeNumber("advanced", "max_concurrent_downloads", e)} />
        <button type="button" class="btn btn-secondary btn-sm step" aria-label={$t("settings.advanced.parallel_more") as string}
          disabled={parallel >= MAX_PARALLEL} onclick={() => setParallel(parallel + 1)}>+</button>
      </span>
    {/snippet}
  </SettingRow>
  <SettingRow id="setting-advanced-prevent_sleep" label={$t("settings.advanced.prevent_sleep")} help={$t("settings.advanced.prevent_sleep_desc")}
    settingKey="settings.advanced.prevent_sleep" status="advanced.prevent_sleep">
    {#snippet control({ id, describedBy })}
      {@render toggle(id, describedBy, settings.advanced.prevent_sleep, () => toggleBool("advanced", "prevent_sleep", settings.advanced.prevent_sleep))}
    {/snippet}
  </SettingRow>

  <AdvancedDisclosure id="settings-speed">
    {#each ADVANCED_NUMBERS as row (row.key)}
      <SettingRow id={`setting-advanced-${row.field}`} label={$t(`settings.advanced.${row.key}`)} help={$t(`settings.advanced.${row.key}_desc`)}
        settingKey={`settings.advanced.${row.key}`} status={`advanced.${row.field}`}>
        {#snippet control({ id, describedBy })}
          <input {id} aria-describedby={describedBy} type="number" class="input-number" dir="ltr" min={row.min} max={row.max} step="1"
            value={settings.advanced[row.field]} onchange={(e) => changeNumber("advanced", row.field, e)} />
        {/snippet}
      </SettingRow>
    {/each}
  </AdvancedDisclosure>
{/if}

<style>
  .stepper { display: inline-flex; align-items: center; gap: var(--space-1); }
  .stepper .step { min-inline-size: var(--control-h, 28px); justify-content: center; font-variant-numeric: tabular-nums; }
  .stepper .value { inline-size: 56px !important; text-align: center; }
</style>
