<script lang="ts">
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import RichPresencePanel from "./RichPresencePanel.svelte";
  import SettingsDrillBack from "./SettingsDrillBack.svelte";
  import SettingsDrillItem from "./SettingsDrillItem.svelte";
  import {
    getSettings,
    updateSettings,
    changeTheme,
    changeLanguage,
    CORE_THEMES,
    MORE_THEMES,
    MORE_THEME_IDS,
  } from "./settings-helpers";

  let { searchActive = false }: { searchActive?: boolean } = $props();

  let settings = $derived(getSettings());
  let selectedInMore = $derived(MORE_THEME_IDS.has(settings?.appearance?.theme ?? ""));
  let moreThemesOpen = $state(false);
  let rpcDrillOpen = $state(false);

  $effect(() => {
    if (selectedInMore) moreThemesOpen = true;
  });

  $effect(() => {
    if (searchActive) rpcDrillOpen = false;
  });
</script>

{#if settings}
  <section class="section">
    <h5 class="section-title">{$t('settings.general.title')}</h5>
    <div class="card">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.general.start_with_system')}</span>
          <span class="setting-path">{$t('settings.general.start_with_system_desc')}</span>
        </div>
        <button
          class="toggle"
          class:on={settings.start_with_system}
          onclick={() => updateSettings({ start_with_system: !settings.start_with_system })}
          role="switch"
          aria-checked={settings.start_with_system}
          aria-label={$t('settings.general.start_with_system') as string}
        >
          <span class="toggle-knob"></span>
        </button>
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.general.start_minimized')}</span>
          <span class="setting-path">{$t('settings.general.start_minimized_desc')}</span>
        </div>
        <button
          class="toggle"
          class:on={settings.start_minimized}
          onclick={() => updateSettings({ start_minimized: !settings.start_minimized })}
          role="switch"
          aria-checked={settings.start_minimized}
          aria-label={$t('settings.general.start_minimized') as string}
        >
          <span class="toggle-knob"></span>
        </button>
      </div>
    </div>
  </section>

  <section class="section">
    <h5 class="section-title">{$t('settings.appearance.title')}</h5>
    <div class="card">
      <div class="setting-row">
        <span class="setting-label">{$t('settings.appearance.theme')}</span>
      </div>
      <div class="theme-grid">
        {#each CORE_THEMES as theme (theme.id)}
          <button
            class="theme-card"
            class:active={settings.appearance.theme === theme.id}
            onclick={() => changeTheme(theme.id)}
          >
            {#if theme.colors}
              <div class="theme-preview">
                <div class="preview-bg" style="background: {theme.colors[0]}">
                  <div class="preview-text" style="color: {theme.colors[1]}">Aa</div>
                  <div class="preview-accent" style="background: {theme.colors[2]}"></div>
                </div>
              </div>
            {:else}
              <div class="theme-preview system-preview">
                <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="1.5"><circle cx="12" cy="12" r="5"/><path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42"/></svg>
              </div>
            {/if}
            <span class="theme-name">{theme.labelKey ? $t(theme.labelKey) : theme.label}</span>
          </button>
        {/each}
      </div>
      {#if !moreThemesOpen}
        <button type="button" class="more-themes-toggle" onclick={() => { moreThemesOpen = true; }}>
          {$t("settings.appearance.more_themes")}
        </button>
      {:else}
        <div class="more-themes">
          <div class="theme-grid">
            {#each MORE_THEMES as theme (theme.id)}
              <button
                class="theme-card"
                class:active={settings.appearance.theme === theme.id}
                onclick={() => changeTheme(theme.id)}
              >
                <div class="theme-preview">
                  <div class="preview-bg" style="background: {theme.colors[0]}">
                    <div class="preview-text" style="color: {theme.colors[1]}">Aa</div>
                    <div class="preview-accent" style="background: {theme.colors[2]}"></div>
                  </div>
                </div>
                <span class="theme-name">{theme.label}</span>
              </button>
            {/each}
          </div>
        </div>
      {/if}
      <div class="divider"></div>
      <div class="setting-row">
        <span class="setting-label">{$t('settings.appearance.language')}</span>
        <select class="select" value={settings.appearance.language} onchange={changeLanguage}>
          <option value="en">{$t('settings.appearance.lang_en')}</option>
		  <option value="ru">{$t('settings.appearance.lang_ru')}</option>
          <option value="pt">{$t('settings.appearance.lang_pt')}</option>
          <option value="zh">{$t('settings.appearance.lang_zh')}</option>
          <option value="ja">{$t('settings.appearance.lang_ja')}</option>
          <option value="it">{$t('settings.appearance.lang_it')}</option>
          <option value="fr">{$t('settings.appearance.lang_fr')}</option>
          <option value="es">{$t('settings.appearance.lang_es')}</option>
          <option value="el">{$t('settings.appearance.lang_el')}</option>
        </select>
      </div>
    </div>
  </section>

  <section class="section">
    <div class="card">
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.general.reset_hints')}</span>
          <span class="setting-path">{$t('settings.general.reset_hints_desc')}</span>
        </div>
        <button
          class="button"
          onclick={() => {
            const keys = Object.keys(localStorage).filter((k) => k.startsWith('hint_dismissed_'));
            for (const k of keys) localStorage.removeItem(k);
            showToast('info', $t('settings.general.hints_restored') as string);
          }}
        >{$t('settings.general.restore')}</button>
      </div>
    </div>
  </section>

  {#if searchActive || rpcDrillOpen}
    {#if rpcDrillOpen && !searchActive}
      <SettingsDrillBack
        title={$t('settings.rpc.title')}
        hint={$t('settings.rpc.desc')}
        onBack={() => { rpcDrillOpen = false; }}
      />
    {/if}
    <RichPresencePanel />
  {:else}
    <nav class="settings-drill-list">
      <SettingsDrillItem
        title={$t('settings.rpc.title')}
        hint={$t('settings.rpc.desc')}
        onclick={() => { rpcDrillOpen = true; }}
      />
    </nav>
  {/if}
{/if}
