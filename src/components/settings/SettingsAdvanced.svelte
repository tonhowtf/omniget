<script lang="ts" module>
  export type AdvancedPart = "performance" | "torrent" | "expert" | "league" | "music" | "telegram" | "video" | "tube" | "read" | "washmybelly" | "debug";
</script>

<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { isDebugEnabled, setDebugEnabled, setDebugPanelOpen } from "$lib/stores/debug-store.svelte";
  import { getSettings, saveSettings as updateSettings, toggleBool, changeNumber } from "./settings-helpers";
  import SettingRow from "./rebrand/SettingRow.svelte";

  let { part }: { part: AdvancedPart } = $props();

  let settings = $derived(getSettings());

  type NumberRow = { key: string; field: string; min: number; max: number; step?: number };
  const PERFORMANCE_NUMBERS: NumberRow[] = [
    { key: "max_concurrent_downloads", field: "max_concurrent_downloads", min: 1, max: 10 },
    { key: "max_concurrent_segments", field: "max_concurrent_segments", min: 1, max: 100 },
    { key: "concurrent_fragments", field: "concurrent_fragments", min: 1, max: 32 },
    { key: "max_retries", field: "max_retries", min: 1, max: 20 },
    { key: "stagger_delay", field: "stagger_delay_ms", min: 0, max: 2000, step: 50 },
  ];

  function advancedValue(field: string): number {
    return (settings?.advanced as unknown as Record<string, number> | undefined)?.[field] ?? 0;
  }
</script>

{#snippet toggle(id: string, describedBy: string | undefined, on: boolean, onclick: () => void)}
  <button {id} type="button" class="toggle" class:on role="switch" aria-checked={on} aria-labelledby={`${id}-label`} aria-describedby={describedBy} {onclick}><span class="toggle-knob"></span></button>
{/snippet}

{#if settings}
  {#if part === "performance"}
    {#each PERFORMANCE_NUMBERS as row (row.key)}
      <SettingRow id={`setting-advanced-${row.field}`} label={$t(`settings.advanced.${row.key}`)} help={$t(`settings.advanced.${row.key}_desc`)}
        settingKey={`settings.advanced.${row.key}`} status={`advanced.${row.field}`}>
        {#snippet control({ id, describedBy })}
          <input {id} aria-describedby={describedBy} type="number" class="input-number" dir="ltr" min={row.min} max={row.max} step={row.step ?? 1}
            value={advancedValue(row.field)} onchange={(e) => changeNumber("advanced", row.field, e)} />
        {/snippet}
      </SettingRow>
    {/each}
    <SettingRow id="setting-advanced-prevent_sleep" label={$t("settings.advanced.prevent_sleep")} help={$t("settings.advanced.prevent_sleep_desc")}
      settingKey="settings.advanced.prevent_sleep" status="advanced.prevent_sleep">
      {#snippet control({ id, describedBy })}
        {@render toggle(id, describedBy, settings.advanced.prevent_sleep, () => toggleBool("advanced", "prevent_sleep", settings.advanced.prevent_sleep))}
      {/snippet}
    </SettingRow>
  {:else if part === "torrent"}
    <SettingRow id="setting-advanced-torrent_listen_port" label={$t("settings.advanced.torrent_listen_port")} help={$t("settings.advanced.torrent_listen_port_desc")}
      settingKey="settings.advanced.torrent_listen_port" status="advanced.torrent_listen_port">
      {#snippet control({ id, describedBy })}
        <input {id} aria-describedby={describedBy} type="number" class="input-number" dir="ltr" min="1024" max="65525"
          value={settings.advanced.torrent_listen_port} onchange={(e) => changeNumber("advanced", "torrent_listen_port", e)} />
      {/snippet}
    </SettingRow>
    <SettingRow id="setting-advanced-torrent_auto_trackers" label={$t("settings.advanced.torrent_auto_trackers")} help={$t("settings.advanced.torrent_auto_trackers_desc")}
      settingKey="settings.advanced.torrent_auto_trackers" status="advanced.torrent_auto_trackers">
      {#snippet control({ id, describedBy })}
        {@render toggle(id, describedBy, settings.advanced.torrent_auto_trackers, () => toggleBool("advanced", "torrent_auto_trackers", settings.advanced.torrent_auto_trackers))}
      {/snippet}
    </SettingRow>
    <SettingRow id="setting-advanced-torrent_upnp" label={$t("settings.advanced.torrent_upnp")} help={$t("settings.advanced.torrent_upnp_desc")}
      settingKey="settings.advanced.torrent_upnp" status="advanced.torrent_upnp">
      {#snippet control({ id, describedBy })}
        {@render toggle(id, describedBy, settings.advanced.torrent_upnp, () => toggleBool("advanced", "torrent_upnp", settings.advanced.torrent_upnp))}
      {/snippet}
    </SettingRow>
  {:else if part === "expert"}
    <SettingRow id="setting-advanced-cookies_from_browser" label={$t("settings.advanced.cookies_from_browser")} help={$t("settings.advanced.cookies_from_browser_desc")}
      settingKey="settings.advanced.cookies_from_browser" status="advanced.cookies_from_browser">
      {#snippet control({ id, describedBy })}
        <input {id} aria-describedby={describedBy} type="text" class="input-text" dir="ltr" spellcheck="false" placeholder={$t("settings.advanced.cookies_from_browser_placeholder")}
          value={settings.advanced?.cookies_from_browser ?? ""} onchange={(e) => updateSettings({ advanced: { cookies_from_browser: (e.target as HTMLInputElement).value.trim() } })} />
      {/snippet}
    </SettingRow>
    <SettingRow id="setting-advanced-user_agent" label={$t("settings.advanced.user_agent")} help={$t("settings.advanced.user_agent_desc")}
      settingKey="settings.advanced.user_agent" status="advanced.user_agent">
      {#snippet control({ id, describedBy })}
        <input {id} aria-describedby={describedBy} type="text" class="input-text" dir="ltr" spellcheck="false" placeholder={$t("settings.advanced.user_agent_placeholder")}
          value={settings.advanced?.user_agent ?? ""} onchange={(e) => updateSettings({ advanced: { user_agent: (e.target as HTMLInputElement).value.trim() } })} />
      {/snippet}
    </SettingRow>
  {:else if part === "league"}
    {@const leagueOn = settings.league?.enabled ?? true}
    <SettingRow id="setting-league-enabled" label={$t("league.settings_enable")} help={$t("league.settings_enable_desc")} settingKey="league.settings_enable" status="league.enabled">
      {#snippet control({ id, describedBy })}
        {@render toggle(id, describedBy, leagueOn, () => updateSettings({ league: { enabled: !leagueOn } }))}
      {/snippet}
    </SettingRow>
    {#if leagueOn}
      <SettingRow id="setting-league-auto_accept" label={$t("league.settings_auto_accept")} help={$t("league.settings_auto_accept_desc")} settingKey="league.settings_auto_accept" status="league.auto_accept">
        {#snippet control({ id, describedBy })}
          {@render toggle(id, describedBy, settings.league?.auto_accept ?? false, () => {
            const next = !(settings.league?.auto_accept ?? false);
            void updateSettings({ league: { auto_accept: next } }).then((saved) => {
              // Only tell the running client after the value is confirmed on disk.
              if (saved) invoke("league_auto_accept_set", { enabled: next }).catch(() => {});
            });
          })}
        {/snippet}
      </SettingRow>
    {/if}
  {:else if part === "music"}
    {@const musicOn = settings.music?.enabled ?? true}
    <SettingRow id="setting-music-enabled" label={$t("music.settings_enable")} help={$t("music.settings_enable_desc")} settingKey="music.settings_enable" status="music.enabled">
      {#snippet control({ id, describedBy })}
        {@render toggle(id, describedBy, musicOn, () => updateSettings({ music: { enabled: !musicOn } }))}
      {/snippet}
    </SettingRow>
  {:else if part === "telegram"}
    {@const telegramOn = settings.telegram?.enabled ?? true}
    <SettingRow id="setting-telegram-enabled" label={$t("telegram.settings_enable")} help={$t("telegram.settings_enable_desc")} settingKey="telegram.settings_enable" status="telegram.enabled">
      {#snippet control({ id, describedBy })}
        {@render toggle(id, describedBy, telegramOn, () => updateSettings({ telegram: { enabled: !telegramOn } }))}
      {/snippet}
    </SettingRow>
  {:else if part === "video"}
    {@const videoOn = settings.video?.enabled ?? true}
    <SettingRow id="setting-video-enabled" label={$t("video.settings_enable")} help={$t("video.settings_enable_desc")} settingKey="video.settings_enable" status="video.enabled">
      {#snippet control({ id, describedBy })}
        {@render toggle(id, describedBy, videoOn, () => updateSettings({ video: { enabled: !videoOn } }))}
      {/snippet}
    </SettingRow>
  {:else if part === "tube"}
    {@const tubeOn = settings.tube?.enabled ?? true}
    <SettingRow id="setting-tube-enabled" label={$t("tube.settings_enable")} help={$t("tube.settings_enable_desc")} settingKey="tube.settings_enable" status="tube.enabled">
      {#snippet control({ id, describedBy })}
        {@render toggle(id, describedBy, tubeOn, () => updateSettings({ tube: { ...(settings.tube ?? {}), enabled: !tubeOn } }))}
      {/snippet}
    </SettingRow>
  {:else if part === "read"}
    {@const readOn = settings.read?.enabled ?? true}
    <SettingRow id="setting-read-enabled" label={$t("read.settings_enable")} help={$t("read.settings_enable_desc")} settingKey="read.settings_enable" status="read.enabled">
      {#snippet control({ id, describedBy })}
        {@render toggle(id, describedBy, readOn, () => updateSettings({ read: { enabled: !readOn } }))}
      {/snippet}
    </SettingRow>
  {:else if part === "washmybelly"}
    {@const wmbOn = settings.washmybelly?.enabled ?? true}
    <SettingRow id="setting-washmybelly-enabled" label={$t("washmybelly.settings_enable")} help={$t("washmybelly.settings_enable_desc")} settingKey="washmybelly.settings_enable" status="washmybelly.enabled">
      {#snippet control({ id, describedBy })}
        {@render toggle(id, describedBy, wmbOn, () => updateSettings({ washmybelly: { enabled: !wmbOn } }))}
      {/snippet}
    </SettingRow>
    {#if wmbOn}
      <SettingRow id="setting-washmybelly-quarantine_days" label={$t("washmybelly.settings_quarantine_days")} help={$t("washmybelly.settings_quarantine_days_desc")} settingKey="washmybelly.settings_quarantine_days" status="washmybelly.quarantine_days">
        {#snippet control({ id, describedBy })}
          <input {id} class="input input-num" type="number" min="1" max="90" aria-describedby={describedBy} value={settings.washmybelly?.quarantine_days ?? 7} onchange={(e) => { const n = Math.max(1, Math.min(90, Number((e.currentTarget as HTMLInputElement).value) || 7)); updateSettings({ washmybelly: { quarantine_days: n } }); }} />
        {/snippet}
      </SettingRow>
      <SettingRow id="setting-washmybelly-auto_hold" label={$t("washmybelly.settings_auto_hold")} help={$t("washmybelly.settings_auto_hold_desc")} settingKey="washmybelly.settings_auto_hold" status="washmybelly.auto_hold_power">
        {#snippet control({ id, describedBy })}
          {@render toggle(id, describedBy, settings.washmybelly?.auto_hold_power ?? false, () => updateSettings({ washmybelly: { auto_hold_power: !(settings.washmybelly?.auto_hold_power ?? false) } }))}
        {/snippet}
      </SettingRow>
    {/if}
  {:else if part === "debug"}
    <SettingRow id="setting-debug-enable" label={$t("debug.enable")} help={$t("debug.enable_desc")} settingKey="debug.enable">
      {#snippet control({ id, describedBy })}
        {@render toggle(id, describedBy, isDebugEnabled(), () => setDebugEnabled(!isDebugEnabled()))}
      {/snippet}
    </SettingRow>
    {#if isDebugEnabled()}
      <SettingRow id="setting-debug-open_panel" label={$t("debug.open_panel")} help={$t("debug.open_panel_desc")} settingKey="debug.open_panel">
        {#snippet control({ id, describedBy })}
          <button {id} type="button" class="btn btn-secondary btn-sm" aria-describedby={describedBy} onclick={() => setDebugPanelOpen(true)}>{$t("debug.open_panel")}</button>
        {/snippet}
      </SettingRow>
    {/if}
  {/if}
{/if}
