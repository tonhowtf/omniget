<script lang="ts">
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { tick, untrack } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { getSettings, resetSettings } from "$lib/stores/settings-store.svelte";
  import { getUpdateInfo } from "$lib/stores/update-store.svelte";
  import { installUpdate } from "$lib/updater";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { refreshYtdlpStatus } from "$lib/stores/dependency-store.svelte";
  import { searchSettings, settingsEntryUrl, SETTINGS_SEARCH_INDEX, type SettingsCategory, type SettingSearchEntry } from "$lib/settings-search";
  import type { DependencyStatus } from "$components/settings/settings-helpers";
  import WorkspaceAppearance from "$components/settings/WorkspaceAppearance.svelte";
  import SettingsDependencies from "$components/settings/SettingsDependencies.svelte";
  import SettingsAdvanced from "$components/settings/SettingsAdvanced.svelte";
  import SettingsRules from "$components/settings/SettingsRules.svelte";
  import SettingsStorage from "$components/settings/SettingsStorage.svelte";
  import SettingsAppearance from "$components/settings/SettingsAppearance.svelte";
  import SettingsProfile from "$components/settings/SettingsProfile.svelte";
  import SettingsNetwork from "$components/settings/SettingsNetwork.svelte";
  import SettingsDownloads from "$components/settings/SettingsDownloads.svelte";
  import SettingsTypography from "$components/settings/SettingsTypography.svelte";
  import SettingsBrowserExtension from "$components/settings/SettingsBrowserExtension.svelte";
  import SettingsCookies from "$components/settings/SettingsCookies.svelte";
  import SettingsChannels from "$components/settings/SettingsChannels.svelte";
  import SettingsAI from "$components/settings/SettingsAI.svelte";
  import SettingsWorld from "$components/settings/SettingsWorld.svelte";
  import SettingsCanvas from "$components/settings/SettingsCanvas.svelte";

  let settings = $derived(getSettings());
  let updateInfo = $derived(getUpdateInfo());
  let resetting = $state(false);
  let updating = $state(false);
  let deps = $state<DependencyStatus[]>([]);
  let depsError = $state(false);
  let depsLoading = $state(false);
  let installingDep = $state<string | null>(null);

  async function loadDeps() {
    if (depsLoading) return;
    depsLoading = true;
    depsError = false;
    try { deps = await invoke<DependencyStatus[]>("check_dependencies"); }
    catch { depsError = true; }
    finally { depsLoading = false; }
  }

  async function handleInstallDep(name: string) {
    if (installingDep) return;
    const wasInstalled = deps.find(d => d.name === name)?.installed ?? false;
    installingDep = name;
    try {
      const version = await invoke<string>("install_dependency", { name, force: wasInstalled });
      await loadDeps();
      await refreshYtdlpStatus();
      showToast("success", $t(wasInstalled ? "settings.dependencies.update_success" : "settings.dependencies.install_success", { name, version }));
    } catch { showToast("error", $t("common.error")); }
    finally { installingDep = null; }
  }

  async function handleUpdate() {
    updating = true;
    try { await installUpdate(); }
    catch { showToast("error", $t("common.error")); updating = false; }
  }

  async function handleReset() {
    if (!confirm($t("settings.advanced.reset_confirm"))) return;
    resetting = true;
    try { await resetSettings(); }
    catch { showToast("error", $t("settings.save_failed")); }
    finally { resetting = false; }
  }

  const SETTINGS_NAV_GROUPS: {
    labelKey: string;
    items: [SettingsCategory, string][];
  }[] = [
    {
      labelKey: "settings.group_general",
      items: [
        ["appearance", "settings.cat_appearance"],
        ["profile", "settings.cat_profile"],
        ["typography", "settings.cat_typography"],
      ],
    },
    {
      labelKey: "settings.group_media",
      items: [["downloads", "settings.cat_downloads"]],
    },
    {
      labelKey: "settings.group_integrations",
      items: [
        ["network", "settings.cat_network"],
        ["cookies", "settings.cat_cookies"],
        ["channels", "settings.cat_channels"],
        ["ai", "settings.cat_ai"],
        ["world", "settings.cat_world"],
        ["dependencies", "settings.dependencies.title"],
      ],
    },
    {
      labelKey: "settings.group_advanced",
      items: [["advanced", "settings.cat_advanced"]],
    },
  ];

  const CATEGORY_ICONS: Record<string, string> = {
    appearance: "M12 3a9 9 0 1 0 0 18c1.2 0 1.6-.9 1.2-1.7-.5-1 .1-2.3 1.3-2.3H16a5 5 0 0 0 5-5c0-5-4-9-9-9zM7.5 12a1 1 0 1 1 0-2 1 1 0 0 1 0 2zm3-4a1 1 0 1 1 0-2 1 1 0 0 1 0 2zm5 0a1 1 0 1 1 0-2 1 1 0 0 1 0 2z",
    profile: "M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM4 20a8 8 0 0 1 16 0",
    typography: "M4 7V5h16v2M12 5v14M9 19h6",
    world: "M12 3l9 5-9 5-9-5 9-5zM3 12l9 5 9-5M3 16l9 5 9-5",
    downloads: "M12 4v11m0 0 4-4m-4 4-4-4M5 20h14",
    network: "M12 20a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM3 12h18M12 4c3 3 3 13 0 16M12 4c-3 3-3 13 0 16",
    cookies: "M12 3a9 9 0 1 0 9 9 3 3 0 0 1-3-3 3 3 0 0 1-3-3 3 3 0 0 1-3-3zM8 12h.01M12 16h.01M9 8h.01M15 13h.01",
    channels: "M4 6h16v12H4zM4 10h16M9 14h6",
    ai: "M12 3l1.9 5.8a2 2 0 0 0 1.3 1.3L21 12l-5.8 1.9a2 2 0 0 0-1.3 1.3L12 21l-1.9-5.8a2 2 0 0 0-1.3-1.3L3 12l5.8-1.9a2 2 0 0 0 1.3-1.3z",
    dependencies: "M9 3h6v2.5a2 2 0 0 1-2 2h-2a2 2 0 0 1-2-2zM6 7h12a2 2 0 0 1 2 2v4h-2a2 2 0 1 0 0 4h2v2a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2v-2h2a2 2 0 1 0 0-4H4V9a2 2 0 0 1 2-2z",
    advanced: "M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6",
  };

  let activeCategory = $derived.by(() => {
    const tab = page.url.searchParams.get("tab");
    return SETTINGS_NAV_GROUPS.flatMap(group => group.items).some(([id]) => id === tab)
      ? tab as SettingsCategory : "downloads";
  });
  let activeSection = $derived(page.url.searchParams.get("section") ?? "");
  let selectedEntry = $derived(SETTINGS_SEARCH_INDEX.find(entry => entry.key === page.url.searchParams.get("setting") && entry.category === activeCategory));
  let activeTitleKey = $derived(SETTINGS_NAV_GROUPS.flatMap(group => group.items).find(([id]) => id === activeCategory)?.[1] ?? "settings.cat_downloads");
  let searchQuery = $state("");
  let lastSearch = $state("");
  let isSearching = $derived(searchQuery.trim().length > 0);
  let results = $derived(searchSettings(searchQuery, key => $t(key)));

  async function openResult(entry: SettingSearchEntry) {
    lastSearch = searchQuery;
    searchQuery = "";
    await goto(settingsEntryUrl(entry), { noScroll: true, keepFocus: true });
  }

  $effect(() => {
    if (activeCategory === "dependencies") untrack(() => { void loadDeps(); });
  });

  $effect(() => {
    const key = selectedEntry?.key;
    const section = activeSection;
    const searching = isSearching;
    if (!key || searching) return;
    let cancelled = false;
    void tick().then(async () => {
      await tick(); // Let section props open their destination before moving focus.
      if (cancelled) return;
      const panel = document.getElementById("settings-active-panel");
      const match = Array.from(panel?.querySelectorAll<HTMLElement>("[data-setting-key]") ?? []).find(node => node.dataset.settingKey === key);
      const exactTarget = match && !match.matches(":disabled") ? match : undefined;
      const target = exactTarget
        ?? document.getElementById(`settings-section-${section}`)
        ?? document.getElementById("settings-panel-title");
      if (!target) return;
      // If a conditional field is unavailable, focus its section, not an unrelated toggle.
      const control = exactTarget
        ? (target.matches("button, input, select, textarea, a[href]") ? target : target.querySelector<HTMLElement>("input:not([disabled]), select:not([disabled]), textarea:not([disabled]), button:not([disabled]), a[href]"))
        : null;
      const focusTarget = control ?? target;
      if (!control) focusTarget.tabIndex = -1;
      focusTarget.focus({ preventScroll: true });
      target.scrollIntoView({ block: "nearest" });
    });
    return () => { cancelled = true; };
  });
</script>

{#if settings}
  <div class="settings settings-mac">
    {#if updateInfo.available}
      <div class="update-banner">
        <span class="update-text">{updating ? $t('settings.update_downloading') : $t('settings.update_available', { version: updateInfo.version })}</span>
        <button class="update-btn" onclick={handleUpdate} disabled={updating}>{$t('settings.update_button')}</button>
      </div>
    {/if}
    <div class="settings-mac-body">
      <aside class="settings-mac-sidebar">
        <div class="settings-search-row search-field">
          <input type="search" class="settings-search" placeholder={$t('settings.search_placeholder')} bind:value={searchQuery} spellcheck="false" aria-label={$t('settings.search_placeholder')} />
          {#if isSearching}<button class="clear-search" onclick={() => { searchQuery = ""; }} aria-label={$t('settings.search.clear')}>×</button>{/if}
        </div>
        {#each SETTINGS_NAV_GROUPS as group (group.labelKey)}
          <nav class="settings-mac-nav-group" aria-label={$t(group.labelKey)}>
            <div class="mac-nav-section-header">{$t(group.labelKey)}</div>
            {#each group.items as [cat, key] (cat)}
              <a class="settings-mac-cat" class:active={!isSearching && activeCategory === cat} aria-current={!isSearching && activeCategory === cat ? "page" : undefined} href={`/settings?tab=${cat}`} onclick={() => { searchQuery = ""; lastSearch = ""; }}>
                <span class="settings-mac-cat-icon" aria-hidden="true"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d={CATEGORY_ICONS[cat]} /></svg></span>
                <span class="settings-mac-cat-label">{$t(key)}</span>
              </a>
            {/each}
          </nav>
        {/each}
      </aside>
      <div class="settings-mac-content settings-content">
        {#if isSearching}
          <section class="settings-results" aria-labelledby="settings-results-title">
            <h1 id="settings-results-title">{$t('settings.search.title')}</h1>
            <p role="status">{$t('settings.search.count', { count: results.length })}</p>
            {#if results.length === 0}
              <p>{$t('settings.search.empty')}</p>
              <button class="button" onclick={() => { searchQuery = ""; }}>{$t('settings.search.clear')}</button>
            {:else}
              <ul>
                {#each results as entry (entry.key)}
                  <li><button class="settings-result" onclick={() => openResult(entry)}>
                    <span><strong>{$t(entry.key)}</strong><small>{$t(SETTINGS_NAV_GROUPS.flatMap(group => group.items).find(([id]) => id === entry.category)?.[1] ?? entry.sectionKey)}{entry.sectionKey !== entry.key ? ` · ${$t(entry.sectionKey)}` : ""}</small></span><span aria-hidden="true">›</span>
                  </button></li>
                {/each}
              </ul>
            {/if}
          </section>
        {/if}
          <section hidden={isSearching} id="settings-active-panel" class="settings-panel" aria-labelledby="settings-panel-title">
            <h1 id="settings-panel-title" tabindex="-1">{$t(activeTitleKey)}</h1>
            {#if selectedEntry}
              <div class="settings-search-destination" role="status">
                <strong>{$t(selectedEntry.key)}</strong>
                <p>{$t('settings.search.destination_hint')}</p>
                {#if lastSearch}<button class="button" onclick={() => { searchQuery = lastSearch; }}>{$t('settings.search.back')}</button>{/if}
              </div>
            {/if}
            {#if activeCategory === "appearance"}
              <div id="settings-section-workspace"><WorkspaceAppearance /></div>
              <SettingsAppearance section={activeSection} />
            {:else if activeCategory === "profile"}
              <SettingsProfile />
            {:else if activeCategory === "typography"}
              <SettingsTypography />
            {:else if activeCategory === "downloads"}
              <SettingsDownloads section={activeSection} />
            {:else if activeCategory === "dependencies"}
              {#if depsLoading}<p role="status">{$t('common.loading')}</p>{/if}
              {#if depsError}<div role="alert"><p>{$t('settings.dependencies.load_failed')}</p><button class="button" onclick={loadDeps}>{$t('settings.retry')}</button></div>{/if}
              <SettingsDependencies {deps} {installingDep} onInstallDep={handleInstallDep} onRefresh={loadDeps} />
            {:else if activeCategory === "network"}
              <div id="settings-section-extension"><SettingsBrowserExtension /></div>
              <div id="settings-section-proxy"><SettingsNetwork /></div>
            {:else if activeCategory === "cookies"}
              <SettingsCookies />
            {:else if activeCategory === "channels"}
              <SettingsChannels />
            {:else if activeCategory === "ai"}
              <SettingsAI />
            {:else if activeCategory === "world"}
              <SettingsWorld />
              <SettingsCanvas />
            {:else if activeCategory === "advanced"}
              <div id="settings-section-rules"><SettingsRules /></div>
              <div id="settings-section-storage"><SettingsStorage /></div>
              <SettingsAdvanced {resetting} onReset={handleReset} section={activeSection} />
            {/if}
          </section>
      </div>
    </div>
  </div>
{:else}
  <div class="settings-loading" role="status"><span class="spinner" aria-hidden="true"></span><span>{$t('common.loading')}</span></div>
{/if}

<style>
  [hidden] { display: none; }
  .search-field { position: relative; }
  .settings-search { padding-right: 28px; }
  .clear-search { position: absolute; right: 4px; top: 2px; width: 24px; height: 24px; border: 0; border-radius: var(--radius-sm); background: transparent; color: var(--text-muted); cursor: pointer; }
  .settings-mac-cat { text-decoration: none; box-sizing: border-box; }
  h1 { font-size: var(--text-xl); line-height: var(--leading-xl); margin: 0 0 var(--space-3); }
  .settings-results { width: 100%; }
  .settings-results p, .settings-search-destination p { color: var(--text-muted); line-height: var(--leading-base); }
  .settings-results ul { list-style: none; padding: 0; margin: var(--space-3) 0; }
  .settings-results li + li { border-top: 1px solid var(--separator); }
  .settings-result { display: flex; width: 100%; justify-content: space-between; align-items: center; gap: var(--space-3); padding: var(--space-3); color: var(--text); background: var(--surface); border: 0; text-align: left; cursor: pointer; }
  .settings-result small { display: block; color: var(--text-muted); margin-top: var(--space-1); }
  .settings-result:hover { background: var(--surface-hi); }
  .settings-result:focus-visible, .clear-search:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset); }
  .settings-search-destination { padding: var(--space-3); border-left: 2px solid var(--accent); background: var(--accent-soft); }
</style>
