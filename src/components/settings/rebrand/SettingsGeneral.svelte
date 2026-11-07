<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { getSettings, saveSettings as updateSettings, changeLanguage } from "../settings-helpers";
  import { reopenOnboarding } from "$lib/stores/onboarding-store.svelte";
  import SettingsProfile from "../SettingsProfile.svelte";
  import SettingGroup from "./SettingGroup.svelte";
  import SettingRow from "./SettingRow.svelte";
  import SettingStatus from "./SettingStatus.svelte";
  import GeneralChoice from "./GeneralChoice.svelte";

  const LANGUAGES = [
    ["en", "settings.appearance.lang_en"], ["pt", "settings.appearance.lang_pt"], ["es", "settings.appearance.lang_es"],
    ["fr", "settings.appearance.lang_fr"], ["it", "settings.appearance.lang_it"], ["ru", "settings.appearance.lang_ru"],
    ["el", "settings.appearance.lang_el"], ["fa", "settings.appearance.lang_fa"], ["lo", "settings.appearance.lang_lo"],
    ["ja", "settings.appearance.lang_ja"], ["zh", "settings.appearance.lang_zh"], ["zh-TW", "settings.appearance.lang_zh_tw"],
  ] as const;

  const onLinux = typeof navigator !== "undefined" && /linux/i.test(navigator.userAgent);

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

  type AutostartStatus = { supported: boolean; registered: boolean; blockedBySystem: boolean };
  let autostart = $state<AutostartStatus | null>(null);

  async function refreshAutostart() {
    try {
      autostart = await invoke<AutostartStatus>("autostart_status");
    } catch {
      autostart = null;
    }
  }

  onMount(refreshAutostart);

  async function toggleStart() {
    await updateSettings({ start_with_system: !settings?.start_with_system });
    await refreshAutostart();
  }
</script>

{#snippet toggle(id: string, describedBy: string | undefined, on: boolean, onclick: () => void, disabled = false)}
  <button {id} type="button" class="toggle" class:on role="switch" aria-checked={on} aria-labelledby={`${id}-label`} aria-describedby={describedBy} {disabled} {onclick}><span class="toggle-knob"></span></button>
{/snippet}

{#if settings}
  <SettingGroup id="startup" icon="power" title={$t("settings.general.on_start_title")}>
    <SettingRow
      id="settings-start_with_system"
      label={$t("settings.general.start_with_system")}
      help={$t("settings.general.start_with_system_desc")}
      settingKey="settings.general.start_with_system"
      status="start_with_system"
      disabledReason={autostart && !autostart.supported ? ($t("settings.general.autostart_portable") as string) : undefined}
    >
      {#snippet control({ id, describedBy })}
        {@render toggle(id, describedBy, settings.start_with_system ?? false, toggleStart, autostart?.supported === false)}
      {/snippet}
    </SettingRow>
    {#if settings.start_with_system && autostart?.blockedBySystem}
      <p class="setting-row-reason" role="status"><bdi>{$t("settings.general.autostart_blocked")}</bdi></p>
    {/if}
    {#if onLinux}<p class="setting-path wrap">{$t("settings.general.run_in_background_flatpak")}</p>{/if}
    <div data-setting-key="settings.general.start_minimized">
      <GeneralChoice
        name="settings-start-mode"
        label={$t("settings.general.start_minimized") as string}
        disabled={!settings.start_with_system}
        value={settings.start_minimized ? "tray" : "window"}
        options={[
          { value: "window", art: "window", label: $t("settings.general.on_start_open") as string, help: $t("settings.general.on_start_open_desc") as string },
          { value: "tray", art: "tray", label: $t("settings.general.on_start_tray") as string, help: $t("settings.general.on_start_tray_desc") as string },
        ]}
        onchange={(v) => updateSettings({ start_minimized: v === "tray" })}
      />
      <SettingStatus path="start_minimized" />
    </div>
  </SettingGroup>

  <SettingGroup id="closing" icon="window-close" title={$t("settings.general.on_close_title")}>
    <div data-setting-key="settings.general.exit_on_close">
      <GeneralChoice
        name="settings-close-mode"
        label={$t("settings.general.on_close_title") as string}
        value={settings.exit_on_close ? "quit" : "keep"}
        options={[
          { value: "keep", art: "keep", label: $t("settings.general.on_close_keep") as string, help: $t("settings.general.on_close_keep_desc") as string },
          { value: "quit", art: "quit", label: $t("settings.general.on_close_quit") as string, help: $t("settings.general.on_close_quit_desc") as string },
        ]}
        onchange={(v) => updateSettings({ exit_on_close: v === "quit" })}
      />
      <SettingStatus path="exit_on_close" />
    </div>
    <SettingRow
      id="settings-run_in_background"
      label={$t("settings.general.run_in_background")}
      help={$t("settings.general.run_in_background_desc")}
      settingKey="settings.general.run_in_background"
      status="run_in_background"
    >
      {#snippet control({ id, describedBy })}
        {@render toggle(id, describedBy, settings.run_in_background ?? false, () => updateSettings({ run_in_background: !settings.run_in_background }), settings.exit_on_close ?? false)}
      {/snippet}
    </SettingRow>
  </SettingGroup>

  <SettingGroup id="language" icon="globe" title={$t("settings.appearance.language")}>
    {#snippet actions()}
      <span data-setting-key="settings.appearance.language">
        <select id="settings-language" class="select" aria-label={$t("settings.appearance.language") as string} value={settings.appearance.language} onchange={changeLanguage}>
          {#each LANGUAGES as [code, key] (code)}
            <option value={code} lang={code}>{$t(key)}</option>
          {/each}
        </select>
      </span>
    {/snippet}
    <SettingStatus path="appearance.language" />
  </SettingGroup>

  <SettingGroup id="profile" icon="person" title={$t("profile.title")} description={$t("profile.desc")}>
    <div data-setting-key="profile.title"><SettingsProfile /></div>
  </SettingGroup>

  <SettingGroup id="more" icon="more" title={$t("settings.general.more_title")} description={$t("settings.general.more_desc")} advanced>
    <SettingRow id="settings-reopen-guide" label={$t("settings.general.reopen_guide")} help={$t("settings.general.reopen_guide_desc")} settingKey="settings.general.reopen_guide">
      {#snippet control({ id, describedBy })}
        <button {id} type="button" class="btn btn-secondary btn-sm" aria-describedby={describedBy} onclick={reopenOnboarding}>{$t("settings.general.reopen_guide_action")}</button>
      {/snippet}
    </SettingRow>
    <SettingRow id="settings-reset-hints" label={$t("settings.general.reset_hints")} help={$t("settings.general.reset_hints_desc")} settingKey="settings.general.reset_hints">
      {#snippet control({ id, describedBy })}
        <button {id} type="button" class="btn btn-secondary btn-sm" aria-describedby={describedBy} onclick={restoreHints}>{$t("settings.general.reset_hints_action")}</button>
      {/snippet}
    </SettingRow>
  </SettingGroup>
{/if}
