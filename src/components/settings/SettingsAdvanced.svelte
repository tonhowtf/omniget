<script lang="ts">
  import { goto } from "$app/navigation";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { isDebugEnabled, setDebugEnabled, setDebugPanelOpen } from "$lib/stores/debug-store.svelte";
  import { getSettings, saveSettings as updateSettings, toggleBool, changeNumber } from "./settings-helpers";
  import SettingsDrillBack from "./SettingsDrillBack.svelte";
  import SettingsDrillItem from "./SettingsDrillItem.svelte";

  let {
    resetting = false,
    onReset,
    section = "",
  }: {
    resetting?: boolean;
    onReset: () => void;
    section?: string;
  } = $props();

  type AdvancedDrill = "performance" | "torrent" | "expert" | "league" | "music" | "telegram" | "video" | "washmybelly" | "debug";

  let settings = $derived(getSettings());

  const DRILL_ITEMS: { id: AdvancedDrill; titleKey: string; hintKey: string }[] = [
    { id: "performance", titleKey: "settings.advanced.section_performance", hintKey: "settings.advanced.section_performance_desc" },
    { id: "torrent", titleKey: "settings.advanced.section_torrent", hintKey: "settings.advanced.section_torrent_desc" },
    { id: "expert", titleKey: "settings.advanced.section_expert", hintKey: "settings.advanced.section_expert_desc" },
    { id: "league", titleKey: "league.settings_title", hintKey: "league.settings_desc" },
    { id: "music", titleKey: "music.settings_title", hintKey: "music.settings_desc" },
    { id: "telegram", titleKey: "telegram.settings_title", hintKey: "telegram.settings_desc" },
    { id: "video", titleKey: "video.settings_title", hintKey: "video.settings_desc" },
    { id: "washmybelly", titleKey: "washmybelly.settings_title", hintKey: "washmybelly.settings_desc" },
    { id: "debug", titleKey: "settings.advanced.section_debug", hintKey: "settings.advanced.section_debug_desc" },
  ];

  let subView = $derived(DRILL_ITEMS.some(item => item.id === section) ? section as AdvancedDrill : null);

</script>

{#if settings}
  {#if subView === null}
    <nav class="settings-drill-list" aria-label={$t('settings.cat_advanced')}>
      {#each DRILL_ITEMS as item (item.id)}
        <SettingsDrillItem
          title={$t(item.titleKey)}
          hint={$t(item.hintKey)}
          onclick={() => { void goto(`/settings?tab=advanced&section=${item.id}`); }}
        />
      {/each}
    </nav>
    <div class="settings-drill-spacer"></div>
    <div class="card">
      <div class="setting-row" data-setting-key="settings.advanced.reset">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.advanced.reset')}</span>
          <span class="setting-path">{$t('settings.advanced.reset_desc')}</span>
        </div>
        <button class="button reset-btn" onclick={onReset} disabled={resetting}>{$t('settings.advanced.reset')}</button>
      </div>
    </div>
  {:else}
    {@const active = DRILL_ITEMS.find((i) => i.id === subView)}
    <div id={`settings-section-${subView}`}>
    {#if active}
      <SettingsDrillBack
        title={$t(active.titleKey)}
        hint={$t(active.hintKey)}
        onBack={() => { void goto("/settings?tab=advanced"); }}
      />
    {/if}

    {#if subView === "performance"}
      <div class="card">
        <div class="setting-row" data-setting-key="settings.advanced.max_concurrent_downloads">
          <div class="setting-col">
            <label class="setting-label" for="setting-settings-advanced-max_concurrent_downloads">{$t('settings.advanced.max_concurrent_downloads')}</label>
            <span class="setting-path" id="setting-settings-advanced-max_concurrent_downloads-help">{$t('settings.advanced.max_concurrent_downloads_desc')}</span>
          </div>
          <input id="setting-settings-advanced-max_concurrent_downloads" aria-describedby="setting-settings-advanced-max_concurrent_downloads-help" type="number" class="input-number" min="1" max="10" value={settings.advanced.max_concurrent_downloads} onchange={(e) => changeNumber("advanced", "max_concurrent_downloads", e)} />
        </div>
        <div class="divider"></div>
        <div class="setting-row" data-setting-key="settings.advanced.max_concurrent_segments">
          <div class="setting-col">
            <label class="setting-label" for="setting-settings-advanced-max_concurrent_segments">{$t('settings.advanced.max_concurrent_segments')}</label>
            <span class="setting-path" id="setting-settings-advanced-max_concurrent_segments-help">{$t('settings.advanced.max_concurrent_segments_desc')}</span>
          </div>
          <input id="setting-settings-advanced-max_concurrent_segments" aria-describedby="setting-settings-advanced-max_concurrent_segments-help" type="number" class="input-number" min="1" max="100" value={settings.advanced.max_concurrent_segments} onchange={(e) => changeNumber("advanced", "max_concurrent_segments", e)} />
        </div>
        <div class="divider"></div>
        <div class="setting-row" data-setting-key="settings.advanced.concurrent_fragments">
          <div class="setting-col">
            <label class="setting-label" for="setting-settings-advanced-concurrent_fragments">{$t('settings.advanced.concurrent_fragments')}</label>
            <span class="setting-path" id="setting-settings-advanced-concurrent_fragments-help">{$t('settings.advanced.concurrent_fragments_desc')}</span>
          </div>
          <input id="setting-settings-advanced-concurrent_fragments" aria-describedby="setting-settings-advanced-concurrent_fragments-help" type="number" class="input-number" min="1" max="32" value={settings.advanced.concurrent_fragments} onchange={(e) => changeNumber("advanced", "concurrent_fragments", e)} />
        </div>
        <div class="divider"></div>
        <div class="setting-row" data-setting-key="settings.advanced.max_retries">
          <div class="setting-col">
            <label class="setting-label" for="setting-settings-advanced-max_retries">{$t('settings.advanced.max_retries')}</label>
            <span class="setting-path" id="setting-settings-advanced-max_retries-help">{$t('settings.advanced.max_retries_desc')}</span>
          </div>
          <input id="setting-settings-advanced-max_retries" aria-describedby="setting-settings-advanced-max_retries-help" type="number" class="input-number" min="1" max="20" value={settings.advanced.max_retries} onchange={(e) => changeNumber("advanced", "max_retries", e)} />
        </div>
        <div class="divider"></div>
        <div class="setting-row" data-setting-key="settings.advanced.stagger_delay">
          <div class="setting-col">
            <label class="setting-label" for="setting-settings-advanced-stagger_delay">{$t('settings.advanced.stagger_delay')}</label>
            <span class="setting-path" id="setting-settings-advanced-stagger_delay-help">{$t('settings.advanced.stagger_delay_desc')}</span>
          </div>
          <input id="setting-settings-advanced-stagger_delay" aria-describedby="setting-settings-advanced-stagger_delay-help" type="number" class="input-number" min="0" max="2000" step="50" value={settings.advanced.stagger_delay_ms} onchange={(e) => changeNumber("advanced", "stagger_delay_ms", e)} />
        </div>
        <div class="divider"></div>
        <div class="setting-row" data-setting-key="settings.advanced.prevent_sleep">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.prevent_sleep')}</span>
            <span class="setting-path">{$t('settings.advanced.prevent_sleep_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.advanced.prevent_sleep} onclick={() => toggleBool("advanced", "prevent_sleep", settings.advanced.prevent_sleep)} role="switch" aria-checked={settings.advanced.prevent_sleep} aria-label={$t('settings.advanced.prevent_sleep') as string}><span class="toggle-knob"></span></button>
        </div>
      </div>
    {:else if subView === "torrent"}
      <div class="card">
        <div class="setting-row" data-setting-key="settings.advanced.torrent_listen_port">
          <div class="setting-col">
            <label class="setting-label" for="setting-settings-advanced-torrent_listen_port">{$t('settings.advanced.torrent_listen_port')}</label>
            <span class="setting-path" id="setting-settings-advanced-torrent_listen_port-help">{$t('settings.advanced.torrent_listen_port_desc')}</span>
          </div>
          <input id="setting-settings-advanced-torrent_listen_port" aria-describedby="setting-settings-advanced-torrent_listen_port-help" type="number" class="input-number" min="1024" max="65525" value={settings.advanced.torrent_listen_port} onchange={(e) => changeNumber("advanced", "torrent_listen_port", e)} />
        </div>
        <div class="divider"></div>
        <div class="setting-row" data-setting-key="settings.advanced.torrent_auto_trackers">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.torrent_auto_trackers')}</span>
            <span class="setting-path">{$t('settings.advanced.torrent_auto_trackers_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.advanced.torrent_auto_trackers} onclick={() => toggleBool("advanced", "torrent_auto_trackers", settings.advanced.torrent_auto_trackers)} role="switch" aria-checked={settings.advanced.torrent_auto_trackers} aria-label={$t('settings.advanced.torrent_auto_trackers') as string}><span class="toggle-knob"></span></button>
        </div>
        <div class="divider"></div>
        <div class="setting-row" data-setting-key="settings.advanced.torrent_upnp">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.torrent_upnp')}</span>
            <span class="setting-path">{$t('settings.advanced.torrent_upnp_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.advanced.torrent_upnp} onclick={() => toggleBool("advanced", "torrent_upnp", settings.advanced.torrent_upnp)} role="switch" aria-checked={settings.advanced.torrent_upnp} aria-label={$t('settings.advanced.torrent_upnp') as string}><span class="toggle-knob"></span></button>
        </div>
      </div>
    {:else if subView === "expert"}
      <div class="card">
        <div class="setting-row" data-setting-key="settings.advanced.cookies_from_browser">
          <div class="setting-col">
            <label class="setting-label" for="setting-settings-advanced-cookies_from_browser">{$t('settings.advanced.cookies_from_browser')}</label>
            <span class="setting-path" id="setting-settings-advanced-cookies_from_browser-help">{$t('settings.advanced.cookies_from_browser_desc')}</span>
          </div>
          <input id="setting-settings-advanced-cookies_from_browser" aria-describedby="setting-settings-advanced-cookies_from_browser-help" type="text" class="input-text" placeholder={$t('settings.advanced.cookies_from_browser_placeholder')} value={settings.advanced?.cookies_from_browser ?? ""} onchange={(e) => updateSettings({ advanced: { cookies_from_browser: (e.target as HTMLInputElement).value.trim() } })} />
        </div>
        <div class="divider"></div>
        <div class="setting-row" data-setting-key="settings.advanced.user_agent">
          <div class="setting-col">
            <label class="setting-label" for="setting-settings-advanced-user_agent">{$t('settings.advanced.user_agent')}</label>
            <span class="setting-path" id="setting-settings-advanced-user_agent-help">{$t('settings.advanced.user_agent_desc')}</span>
          </div>
          <input id="setting-settings-advanced-user_agent" aria-describedby="setting-settings-advanced-user_agent-help" type="text" class="input-text" placeholder={$t('settings.advanced.user_agent_placeholder')} value={settings.advanced?.user_agent ?? ""} onchange={(e) => updateSettings({ advanced: { user_agent: (e.target as HTMLInputElement).value.trim() } })} />
        </div>
      </div>
    {:else if subView === "league"}
      <div class="card">
        <div class="setting-row" data-setting-key="league.settings_enable">
          <div class="setting-col">
            <span class="setting-label">{$t('league.settings_enable')}</span>
            <span class="setting-path">{$t('league.settings_enable_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.league?.enabled ?? true} onclick={() => updateSettings({ league: { enabled: !(settings.league?.enabled ?? true) } })} role="switch" aria-checked={settings.league?.enabled ?? true} aria-label={$t('league.settings_enable') as string}><span class="toggle-knob"></span></button>
        </div>
        {#if settings.league?.enabled ?? true}
          <div class="divider"></div>
          <div class="setting-row" data-setting-key="league.settings_auto_accept">
            <div class="setting-col">
              <span class="setting-label">{$t('league.settings_auto_accept')}</span>
              <span class="setting-path">{$t('league.settings_auto_accept_desc')}</span>
            </div>
            <button class="toggle" class:on={settings.league?.auto_accept} onclick={() => { const next = !(settings.league?.auto_accept ?? false); updateSettings({ league: { auto_accept: next } }); invoke("league_auto_accept_set", { enabled: next }).catch(() => {}); }} role="switch" aria-checked={settings.league?.auto_accept ?? false} aria-label={$t('league.settings_auto_accept') as string}><span class="toggle-knob"></span></button>
          </div>
        {/if}
      </div>
    {:else if subView === "music"}
      <div class="card">
        <div class="setting-row" data-setting-key="music.settings_enable">
          <div class="setting-col">
            <span class="setting-label">{$t('music.settings_enable')}</span>
            <span class="setting-path">{$t('music.settings_enable_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.music?.enabled ?? true} onclick={() => updateSettings({ music: { enabled: !(settings.music?.enabled ?? true) } })} role="switch" aria-checked={settings.music?.enabled ?? true} aria-label={$t('music.settings_enable') as string}><span class="toggle-knob"></span></button>
        </div>
      </div>
    {:else if subView === "telegram"}
      <div class="card">
        <div class="setting-row" data-setting-key="telegram.settings_enable">
          <div class="setting-col">
            <span class="setting-label">{$t('telegram.settings_enable')}</span>
            <span class="setting-path">{$t('telegram.settings_enable_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.telegram?.enabled ?? true} onclick={() => updateSettings({ telegram: { enabled: !(settings.telegram?.enabled ?? true) } })} role="switch" aria-checked={settings.telegram?.enabled ?? true} aria-label={$t('telegram.settings_enable') as string}><span class="toggle-knob"></span></button>
        </div>
      </div>
    {:else if subView === "video"}
      <div class="card">
        <div class="setting-row" data-setting-key="video.settings_enable">
          <div class="setting-col">
            <span class="setting-label">{$t('video.settings_enable')}</span>
            <span class="setting-path">{$t('video.settings_enable_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.video?.enabled ?? true} onclick={() => updateSettings({ video: { enabled: !(settings.video?.enabled ?? true) } })} role="switch" aria-checked={settings.video?.enabled ?? true} aria-label={$t('video.settings_enable') as string}><span class="toggle-knob"></span></button>
        </div>
      </div>
    {:else if subView === "washmybelly"}
      <div class="card">
        <div class="setting-row" data-setting-key="washmybelly.settings_enable">
          <div class="setting-col">
            <span class="setting-label">{$t('washmybelly.settings_enable')}</span>
            <span class="setting-path">{$t('washmybelly.settings_enable_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.washmybelly?.enabled ?? true} onclick={() => updateSettings({ washmybelly: { enabled: !(settings.washmybelly?.enabled ?? true) } })} role="switch" aria-checked={settings.washmybelly?.enabled ?? true} aria-label={$t('washmybelly.settings_enable') as string}><span class="toggle-knob"></span></button>
        </div>
        {#if settings.washmybelly?.enabled ?? true}
          <div class="divider"></div>
          <div class="setting-row" data-setting-key="washmybelly.settings_quarantine_days">
            <div class="setting-col">
              <span class="setting-label">{$t('washmybelly.settings_quarantine_days')}</span>
              <span class="setting-path">{$t('washmybelly.settings_quarantine_days_desc')}</span>
            </div>
            <input class="input input-num" type="number" min="1" max="90" value={settings.washmybelly?.quarantine_days ?? 7} onchange={(e) => { const n = Math.max(1, Math.min(90, Number((e.currentTarget as HTMLInputElement).value) || 7)); updateSettings({ washmybelly: { quarantine_days: n } }); }} aria-label={$t('washmybelly.settings_quarantine_days') as string} />
          </div>
          <div class="divider"></div>
          <div class="setting-row" data-setting-key="washmybelly.settings_auto_hold">
            <div class="setting-col">
              <span class="setting-label">{$t('washmybelly.settings_auto_hold')}</span>
              <span class="setting-path">{$t('washmybelly.settings_auto_hold_desc')}</span>
            </div>
            <button class="toggle" class:on={settings.washmybelly?.auto_hold_power} onclick={() => updateSettings({ washmybelly: { auto_hold_power: !(settings.washmybelly?.auto_hold_power ?? false) } })} role="switch" aria-checked={settings.washmybelly?.auto_hold_power ?? false} aria-label={$t('washmybelly.settings_auto_hold') as string}><span class="toggle-knob"></span></button>
          </div>
        {/if}
      </div>
    {:else if subView === "debug"}
      <div class="card">
        <div class="setting-row" data-setting-key="debug.enable">
          <div class="setting-col">
            <span class="setting-label">{$t('debug.enable')}</span>
            <span class="setting-path">{$t('debug.enable_desc')}</span>
          </div>
          <button class="toggle" class:on={isDebugEnabled()} onclick={() => setDebugEnabled(!isDebugEnabled())} role="switch" aria-checked={isDebugEnabled()} aria-label={$t('debug.enable') as string}><span class="toggle-knob"></span></button>
        </div>
        {#if isDebugEnabled()}
          <div class="divider"></div>
          <div class="setting-row" data-setting-key="debug.open_panel">
            <div class="setting-col">
              <span class="setting-label">{$t('debug.open_panel')}</span>
              <span class="setting-path">{$t('debug.open_panel_desc')}</span>
            </div>
            <button class="button" onclick={() => setDebugPanelOpen(true)}>{$t('debug.open_panel')}</button>
          </div>
        {/if}
      </div>
    {/if}
    </div>
  {/if}
{/if}
