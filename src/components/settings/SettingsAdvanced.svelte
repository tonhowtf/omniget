<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { isDebugEnabled, setDebugEnabled, setDebugPanelOpen } from "$lib/stores/debug-store.svelte";
  import { getSettings, updateSettings, toggleBool, changeNumber } from "./settings-helpers";
  import SettingsDrillBack from "./SettingsDrillBack.svelte";
  import SettingsDrillItem from "./SettingsDrillItem.svelte";

  let {
    resetting = false,
    onReset,
    searchActive = false,
  }: {
    resetting?: boolean;
    onReset: () => void;
    searchActive?: boolean;
  } = $props();

  type AdvancedDrill = "performance" | "torrent" | "expert" | "league" | "music" | "video" | "washmybelly" | "debug";
  let subView = $state<AdvancedDrill | null>(null);

  let settings = $derived(getSettings());

  const DRILL_ITEMS: { id: AdvancedDrill; titleKey: string; hintKey: string }[] = [
    { id: "performance", titleKey: "settings.advanced.section_performance", hintKey: "settings.advanced.section_performance_desc" },
    { id: "torrent", titleKey: "settings.advanced.section_torrent", hintKey: "settings.advanced.section_torrent_desc" },
    { id: "expert", titleKey: "settings.advanced.section_expert", hintKey: "settings.advanced.section_expert_desc" },
    { id: "league", titleKey: "league.settings_title", hintKey: "league.settings_desc" },
    { id: "music", titleKey: "music.settings_title", hintKey: "music.settings_desc" },
    { id: "video", titleKey: "video.settings_title", hintKey: "video.settings_desc" },
    { id: "washmybelly", titleKey: "washmybelly.settings_title", hintKey: "washmybelly.settings_desc" },
    { id: "debug", titleKey: "settings.advanced.section_debug", hintKey: "settings.advanced.section_debug_desc" },
  ];

  $effect(() => {
    if (searchActive) subView = null;
  });
</script>

{#if settings}
  {#if searchActive}
    <div class="settings-section-head section-title">
      <h5 class="section-title">{$t('settings.advanced.section_performance')}</h5>
      <p class="settings-section-hint">{$t('settings.advanced.section_performance_desc')}</p>
    </div>
    <div class="card">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.advanced.max_concurrent_downloads')}</span>
          <span class="setting-path">{$t('settings.advanced.max_concurrent_downloads_desc')}</span>
        </div>
        <input type="number" class="input-number" min="1" max="10" value={settings.advanced.max_concurrent_downloads} onchange={(e) => changeNumber("advanced", "max_concurrent_downloads", e)} />
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.advanced.max_concurrent_segments')}</span>
          <span class="setting-path">{$t('settings.advanced.max_concurrent_segments_desc')}</span>
        </div>
        <input type="number" class="input-number" min="1" max="100" value={settings.advanced.max_concurrent_segments} onchange={(e) => changeNumber("advanced", "max_concurrent_segments", e)} />
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.advanced.concurrent_fragments')}</span>
          <span class="setting-path">{$t('settings.advanced.concurrent_fragments_desc')}</span>
        </div>
        <input type="number" class="input-number" min="1" max="32" value={settings.advanced.concurrent_fragments} onchange={(e) => changeNumber("advanced", "concurrent_fragments", e)} />
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.advanced.max_retries')}</span>
          <span class="setting-path">{$t('settings.advanced.max_retries_desc')}</span>
        </div>
        <input type="number" class="input-number" min="1" max="20" value={settings.advanced.max_retries} onchange={(e) => changeNumber("advanced", "max_retries", e)} />
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.advanced.stagger_delay')}</span>
          <span class="setting-path">{$t('settings.advanced.stagger_delay_desc')}</span>
        </div>
        <input type="number" class="input-number" min="0" max="2000" step="50" value={settings.advanced.stagger_delay_ms} onchange={(e) => changeNumber("advanced", "stagger_delay_ms", e)} />
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.advanced.prevent_sleep')}</span>
          <span class="setting-path">{$t('settings.advanced.prevent_sleep_desc')}</span>
        </div>
        <button class="toggle" class:on={settings.advanced.prevent_sleep} onclick={() => toggleBool("advanced", "prevent_sleep", settings.advanced.prevent_sleep)} role="switch" aria-checked={settings.advanced.prevent_sleep} aria-label={$t('settings.advanced.prevent_sleep') as string}><span class="toggle-knob"></span></button>
      </div>
    </div>

    <div class="settings-section-head section-title">
      <h5 class="section-title">{$t('settings.advanced.section_torrent')}</h5>
      <p class="settings-section-hint">{$t('settings.advanced.section_torrent_desc')}</p>
    </div>
    <div class="card">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.advanced.torrent_listen_port')}</span>
          <span class="setting-path">{$t('settings.advanced.torrent_listen_port_desc')}</span>
        </div>
        <input type="number" class="input-number" min="1024" max="65525" value={settings.advanced.torrent_listen_port} onchange={(e) => changeNumber("advanced", "torrent_listen_port", e)} />
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.advanced.torrent_auto_trackers')}</span>
          <span class="setting-path">{$t('settings.advanced.torrent_auto_trackers_desc')}</span>
        </div>
        <button class="toggle" class:on={settings.advanced.torrent_auto_trackers} onclick={() => toggleBool("advanced", "torrent_auto_trackers", settings.advanced.torrent_auto_trackers)} role="switch" aria-checked={settings.advanced.torrent_auto_trackers} aria-label={$t('settings.advanced.torrent_auto_trackers') as string}><span class="toggle-knob"></span></button>
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.advanced.torrent_upnp')}</span>
          <span class="setting-path">{$t('settings.advanced.torrent_upnp_desc')}</span>
        </div>
        <button class="toggle" class:on={settings.advanced.torrent_upnp} onclick={() => toggleBool("advanced", "torrent_upnp", settings.advanced.torrent_upnp)} role="switch" aria-checked={settings.advanced.torrent_upnp} aria-label={$t('settings.advanced.torrent_upnp') as string}><span class="toggle-knob"></span></button>
      </div>
    </div>

    <div class="settings-section-head section-title">
      <h5 class="section-title">{$t('settings.advanced.section_expert')}</h5>
      <p class="settings-section-hint">{$t('settings.advanced.section_expert_desc')}</p>
    </div>
    <div class="card">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.advanced.cookies_from_browser')}</span>
          <span class="setting-path">{$t('settings.advanced.cookies_from_browser_desc')}</span>
        </div>
        <input type="text" class="input-text" placeholder={$t('settings.advanced.cookies_from_browser_placeholder')} value={settings.advanced?.cookies_from_browser ?? ""} onchange={(e) => updateSettings({ advanced: { cookies_from_browser: (e.target as HTMLInputElement).value.trim() } })} />
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.advanced.user_agent')}</span>
          <span class="setting-path">{$t('settings.advanced.user_agent_desc')}</span>
        </div>
        <input type="text" class="input-text" placeholder={$t('settings.advanced.user_agent_placeholder')} value={settings.advanced?.user_agent ?? ""} onchange={(e) => updateSettings({ advanced: { user_agent: (e.target as HTMLInputElement).value.trim() } })} />
      </div>
    </div>

    <div class="settings-section-head section-title">
      <h5 class="section-title">{$t('settings.advanced.section_debug')}</h5>
      <p class="settings-section-hint">{$t('settings.advanced.section_debug_desc')}</p>
    </div>
    <div class="card">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('debug.enable')}</span>
          <span class="setting-path">{$t('debug.enable_desc')}</span>
        </div>
        <button class="toggle" class:on={isDebugEnabled()} onclick={() => setDebugEnabled(!isDebugEnabled())} role="switch" aria-checked={isDebugEnabled()} aria-label={$t('debug.enable') as string}><span class="toggle-knob"></span></button>
      </div>
      {#if isDebugEnabled()}
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('debug.open_panel')}</span>
            <span class="setting-path">{$t('debug.open_panel_desc')}</span>
          </div>
          <button class="button" onclick={() => setDebugPanelOpen(true)}>{$t('debug.open_panel')}</button>
        </div>
      {/if}
    </div>

    <section class="section">
      <div class="card">
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.reset')}</span>
            <span class="setting-path">{$t('settings.advanced.reset_desc')}</span>
          </div>
          <button class="button reset-btn" onclick={onReset} disabled={resetting}>{$t('settings.advanced.reset')}</button>
        </div>
      </div>
    </section>
  {:else if subView === null}
    <nav class="settings-drill-list" aria-label={$t('settings.cat_advanced')}>
      {#each DRILL_ITEMS as item (item.id)}
        <SettingsDrillItem
          title={$t(item.titleKey)}
          hint={$t(item.hintKey)}
          onclick={() => { subView = item.id; }}
        />
      {/each}
    </nav>
    <div class="settings-drill-spacer"></div>
    <div class="card">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.advanced.reset')}</span>
          <span class="setting-path">{$t('settings.advanced.reset_desc')}</span>
        </div>
        <button class="button reset-btn" onclick={onReset} disabled={resetting}>{$t('settings.advanced.reset')}</button>
      </div>
    </div>
  {:else}
    {@const active = DRILL_ITEMS.find((i) => i.id === subView)}
    {#if active}
      <SettingsDrillBack
        title={$t(active.titleKey)}
        hint={$t(active.hintKey)}
        onBack={() => { subView = null; }}
      />
    {/if}

    {#if subView === "performance"}
      <div class="card">
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.max_concurrent_downloads')}</span>
            <span class="setting-path">{$t('settings.advanced.max_concurrent_downloads_desc')}</span>
          </div>
          <input type="number" class="input-number" min="1" max="10" value={settings.advanced.max_concurrent_downloads} onchange={(e) => changeNumber("advanced", "max_concurrent_downloads", e)} />
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.max_concurrent_segments')}</span>
            <span class="setting-path">{$t('settings.advanced.max_concurrent_segments_desc')}</span>
          </div>
          <input type="number" class="input-number" min="1" max="100" value={settings.advanced.max_concurrent_segments} onchange={(e) => changeNumber("advanced", "max_concurrent_segments", e)} />
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.concurrent_fragments')}</span>
            <span class="setting-path">{$t('settings.advanced.concurrent_fragments_desc')}</span>
          </div>
          <input type="number" class="input-number" min="1" max="32" value={settings.advanced.concurrent_fragments} onchange={(e) => changeNumber("advanced", "concurrent_fragments", e)} />
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.max_retries')}</span>
            <span class="setting-path">{$t('settings.advanced.max_retries_desc')}</span>
          </div>
          <input type="number" class="input-number" min="1" max="20" value={settings.advanced.max_retries} onchange={(e) => changeNumber("advanced", "max_retries", e)} />
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.stagger_delay')}</span>
            <span class="setting-path">{$t('settings.advanced.stagger_delay_desc')}</span>
          </div>
          <input type="number" class="input-number" min="0" max="2000" step="50" value={settings.advanced.stagger_delay_ms} onchange={(e) => changeNumber("advanced", "stagger_delay_ms", e)} />
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.prevent_sleep')}</span>
            <span class="setting-path">{$t('settings.advanced.prevent_sleep_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.advanced.prevent_sleep} onclick={() => toggleBool("advanced", "prevent_sleep", settings.advanced.prevent_sleep)} role="switch" aria-checked={settings.advanced.prevent_sleep} aria-label={$t('settings.advanced.prevent_sleep') as string}><span class="toggle-knob"></span></button>
        </div>
      </div>
    {:else if subView === "torrent"}
      <div class="card">
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.torrent_listen_port')}</span>
            <span class="setting-path">{$t('settings.advanced.torrent_listen_port_desc')}</span>
          </div>
          <input type="number" class="input-number" min="1024" max="65525" value={settings.advanced.torrent_listen_port} onchange={(e) => changeNumber("advanced", "torrent_listen_port", e)} />
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.torrent_auto_trackers')}</span>
            <span class="setting-path">{$t('settings.advanced.torrent_auto_trackers_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.advanced.torrent_auto_trackers} onclick={() => toggleBool("advanced", "torrent_auto_trackers", settings.advanced.torrent_auto_trackers)} role="switch" aria-checked={settings.advanced.torrent_auto_trackers} aria-label={$t('settings.advanced.torrent_auto_trackers') as string}><span class="toggle-knob"></span></button>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.torrent_upnp')}</span>
            <span class="setting-path">{$t('settings.advanced.torrent_upnp_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.advanced.torrent_upnp} onclick={() => toggleBool("advanced", "torrent_upnp", settings.advanced.torrent_upnp)} role="switch" aria-checked={settings.advanced.torrent_upnp} aria-label={$t('settings.advanced.torrent_upnp') as string}><span class="toggle-knob"></span></button>
        </div>
      </div>
    {:else if subView === "expert"}
      <div class="card">
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.cookies_from_browser')}</span>
            <span class="setting-path">{$t('settings.advanced.cookies_from_browser_desc')}</span>
          </div>
          <input type="text" class="input-text" placeholder={$t('settings.advanced.cookies_from_browser_placeholder')} value={settings.advanced?.cookies_from_browser ?? ""} onchange={(e) => updateSettings({ advanced: { cookies_from_browser: (e.target as HTMLInputElement).value.trim() } })} />
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.user_agent')}</span>
            <span class="setting-path">{$t('settings.advanced.user_agent_desc')}</span>
          </div>
          <input type="text" class="input-text" placeholder={$t('settings.advanced.user_agent_placeholder')} value={settings.advanced?.user_agent ?? ""} onchange={(e) => updateSettings({ advanced: { user_agent: (e.target as HTMLInputElement).value.trim() } })} />
        </div>
      </div>
    {:else if subView === "league"}
      <div class="card">
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('league.settings_enable')}</span>
            <span class="setting-path">{$t('league.settings_enable_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.league?.enabled ?? true} onclick={() => updateSettings({ league: { enabled: !(settings.league?.enabled ?? true) } })} role="switch" aria-checked={settings.league?.enabled ?? true} aria-label={$t('league.settings_enable') as string}><span class="toggle-knob"></span></button>
        </div>
        {#if settings.league?.enabled ?? true}
          <div class="divider"></div>
          <div class="setting-row">
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
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('music.settings_enable')}</span>
            <span class="setting-path">{$t('music.settings_enable_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.music?.enabled ?? true} onclick={() => updateSettings({ music: { enabled: !(settings.music?.enabled ?? true) } })} role="switch" aria-checked={settings.music?.enabled ?? true} aria-label={$t('music.settings_enable') as string}><span class="toggle-knob"></span></button>
        </div>
      </div>
    {:else if subView === "video"}
      <div class="card">
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('video.settings_enable')}</span>
            <span class="setting-path">{$t('video.settings_enable_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.video?.enabled ?? true} onclick={() => updateSettings({ video: { enabled: !(settings.video?.enabled ?? true) } })} role="switch" aria-checked={settings.video?.enabled ?? true} aria-label={$t('video.settings_enable') as string}><span class="toggle-knob"></span></button>
        </div>
      </div>
    {:else if subView === "washmybelly"}
      <div class="card">
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('washmybelly.settings_enable')}</span>
            <span class="setting-path">{$t('washmybelly.settings_enable_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.washmybelly?.enabled ?? true} onclick={() => updateSettings({ washmybelly: { enabled: !(settings.washmybelly?.enabled ?? true) } })} role="switch" aria-checked={settings.washmybelly?.enabled ?? true} aria-label={$t('washmybelly.settings_enable') as string}><span class="toggle-knob"></span></button>
        </div>
        {#if settings.washmybelly?.enabled ?? true}
          <div class="divider"></div>
          <div class="setting-row">
            <div class="setting-col">
              <span class="setting-label">{$t('washmybelly.settings_quarantine_days')}</span>
              <span class="setting-path">{$t('washmybelly.settings_quarantine_days_desc')}</span>
            </div>
            <input class="input input-num" type="number" min="1" max="90" value={settings.washmybelly?.quarantine_days ?? 7} onchange={(e) => { const n = Math.max(1, Math.min(90, Number((e.currentTarget as HTMLInputElement).value) || 7)); updateSettings({ washmybelly: { quarantine_days: n } }); }} aria-label={$t('washmybelly.settings_quarantine_days') as string} />
          </div>
          <div class="divider"></div>
          <div class="setting-row">
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
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('debug.enable')}</span>
            <span class="setting-path">{$t('debug.enable_desc')}</span>
          </div>
          <button class="toggle" class:on={isDebugEnabled()} onclick={() => setDebugEnabled(!isDebugEnabled())} role="switch" aria-checked={isDebugEnabled()} aria-label={$t('debug.enable') as string}><span class="toggle-knob"></span></button>
        </div>
        {#if isDebugEnabled()}
          <div class="divider"></div>
          <div class="setting-row">
            <div class="setting-col">
              <span class="setting-label">{$t('debug.open_panel')}</span>
              <span class="setting-path">{$t('debug.open_panel_desc')}</span>
            </div>
            <button class="button" onclick={() => setDebugPanelOpen(true)}>{$t('debug.open_panel')}</button>
          </div>
        {/if}
      </div>
    {/if}
  {/if}
{/if}
