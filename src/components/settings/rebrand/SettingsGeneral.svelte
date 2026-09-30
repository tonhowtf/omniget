<script lang="ts">
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { getSettings, saveSettings as updateSettings, changeLanguage } from "../settings-helpers";
  import SettingsProfile from "../SettingsProfile.svelte";
  import SettingsIndicators from "../SettingsIndicators.svelte";
  import SettingGroup from "./SettingGroup.svelte";
  import SettingRow from "./SettingRow.svelte";

  // Language names are written in their own script so anyone can find theirs.
  const LANGUAGES = [
    ["en", "settings.appearance.lang_en"], ["pt", "settings.appearance.lang_pt"], ["es", "settings.appearance.lang_es"],
    ["fr", "settings.appearance.lang_fr"], ["it", "settings.appearance.lang_it"], ["ru", "settings.appearance.lang_ru"],
    ["el", "settings.appearance.lang_el"], ["fa", "settings.appearance.lang_fa"], ["lo", "settings.appearance.lang_lo"],
    ["ja", "settings.appearance.lang_ja"], ["zh", "settings.appearance.lang_zh"], ["zh-TW", "settings.appearance.lang_zh_tw"],
  ] as const;

  const STARTUP = [
    ["start_with_system", "settings.general.start_with_system"],
    ["start_minimized", "settings.general.start_minimized"],
    ["exit_on_close", "settings.general.exit_on_close"],
  ] as const;

  let settings = $derived(getSettings());

  function restoreHints() {
    try {
      const keys = Object.keys(localStorage).filter((k) => k.startsWith("hint_dismissed_"));
      for (const k of keys) localStorage.removeItem(k);
      showToast("info", $t("settings.general.hints_restored") as string);
    } catch {
      showToast("error", $t("common.error") as string);
    }
  }
</script>

{#snippet toggle(id: string, describedBy: string | undefined, on: boolean, onclick: () => void)}
  <button {id} type="button" class="toggle" class:on role="switch" aria-checked={on} aria-labelledby={`${id}-label`} aria-describedby={describedBy} {onclick}><span class="toggle-knob"></span></button>
{/snippet}

{#if settings}
  <SettingGroup id="language" title={$t("settings.appearance.language")}>
    <SettingRow id="settings-language" label={$t("settings.appearance.language")} settingKey="settings.appearance.language" status="appearance.language">
      {#snippet control({ id, describedBy })}
        <select {id} class="select" aria-describedby={describedBy} value={settings.appearance.language} onchange={changeLanguage}>
          {#each LANGUAGES as [code, key] (code)}
            <option value={code} lang={code}>{$t(key)}</option>
          {/each}
        </select>
      {/snippet}
    </SettingRow>
  </SettingGroup>

  <SettingGroup id="startup" title={$t("settings.general.title")}>
    {#each STARTUP as [field, key] (field)}
      <SettingRow id={`settings-${field}`} label={$t(key)} help={$t(`${key}_desc`)} settingKey={key} status={field}>
        {#snippet control({ id, describedBy })}
          {@render toggle(id, describedBy, settings[field] ?? false, () => updateSettings({ [field]: !settings[field] }))}
        {/snippet}
      </SettingRow>
    {/each}
  </SettingGroup>

  <SettingGroup id="profile" title={$t("profile.title")} description={$t("profile.desc")}>
    <div data-setting-key="profile.title"><SettingsProfile /></div>
  </SettingGroup>

  <SettingGroup id="guidance" title={$t("settings.general.reset_hints")}>
    <SettingRow id="settings-reset-hints" label={$t("settings.general.reset_hints")} help={$t("settings.general.reset_hints_desc")} settingKey="settings.general.reset_hints">
      {#snippet control({ id, describedBy })}
        <button {id} type="button" class="btn btn-secondary btn-sm" aria-describedby={describedBy} onclick={restoreHints}>{$t("settings.general.restore")}</button>
      {/snippet}
    </SettingRow>
  </SettingGroup>

  <!-- Limits strip and menu-bar usage indicator (groups limits-strip and usage-tray). -->
  <SettingsIndicators />
{/if}
