<script lang="ts">
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

  type AdvancedDrill = "performance" | "torrent" | "expert" | "debug";
  let subView = $state<AdvancedDrill | null>(null);

  let settings = $derived(getSettings());

  const DRILL_ITEMS: { id: AdvancedDrill; titleKey: string; hintKey: string }[] = [
    { id: "performance", titleKey: "settings.advanced.section_performance", hintKey: "settings.advanced.section_performance_desc" },
    { id: "torrent", titleKey: "settings.advanced.section_torrent", hintKey: "settings.advanced.section_torrent_desc" },
    { id: "expert", titleKey: "settings.advanced.section_expert", hintKey: "settings.advanced.section_expert_desc" },
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
