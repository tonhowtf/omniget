<script lang="ts">
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
  import { tick } from "svelte";
  import { t } from "$lib/i18n";
  import {
    getSettings,
    getSettingsLoadState,
    getPendingEffects,
    loadSettings,
    retrySettingsEffect,
    type SettingsSection,
  } from "$lib/stores/settings-store.svelte";
  import type { ResetScope } from "$lib/stores/settings-v3-types";
  import { getUpdateInfo } from "$lib/stores/update-store.svelte";
  import { installUpdate } from "$lib/updater";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { searchSettings, settingsEntryUrl, type SettingSearchEntry } from "$lib/settings-search";
  import { CATEGORY_LABEL_KEYS, focusWhenReady, resolveSettingsLocation, type SettingsCategory } from "$lib/settings-navigation";
  import SettingsShell from "$components/settings/rebrand/SettingsShell.svelte";
  import SettingsNav from "$components/settings/rebrand/SettingsNav.svelte";
  import SettingsSearchResults from "$components/settings/rebrand/SettingsSearchResults.svelte";
  import SettingGroup from "$components/settings/rebrand/SettingGroup.svelte";
  import SettingRow from "$components/settings/rebrand/SettingRow.svelte";
  import SettingsEmptyState from "$components/settings/rebrand/SettingsEmptyState.svelte";
  import SettingsIcon from "$components/settings/rebrand/SettingsIcon.svelte";
  import SettingsResetDialog from "$components/settings/rebrand/SettingsResetDialog.svelte";
  import SettingsGeneral from "$components/settings/rebrand/SettingsGeneral.svelte";
  import SettingsAppearance from "$components/settings/SettingsAppearance.svelte";
  import SettingsTypography from "$components/settings/SettingsTypography.svelte";
  import SettingsDownloads from "$components/settings/SettingsDownloads.svelte";
  import SettingsAI from "$components/settings/SettingsAI.svelte";
  import SettingsBrowserExtension from "$components/settings/SettingsBrowserExtension.svelte";
  import SettingsWorld from "$components/settings/SettingsWorld.svelte";
  import RichPresencePanel from "$components/settings/RichPresencePanel.svelte";
  import SettingsNetwork from "$components/settings/SettingsNetwork.svelte";
  import SettingsDependencies from "$components/settings/SettingsDependencies.svelte";
  import SettingsStorage from "$components/settings/SettingsStorage.svelte";
  import SettingsAdvanced from "$components/settings/SettingsAdvanced.svelte";
  import SettingsCanvas from "$components/settings/SettingsCanvas.svelte";

  let settings = $derived(getSettings());
  let loadState = $derived(getSettingsLoadState());
  let pendingEffects = $derived(getPendingEffects().filter((effect) => effect.retryable));
  let updateInfo = $derived(getUpdateInfo());
  let updating = $state(false);

  // ---- Location: aliases and legacy URLs normalize in place (no new history entry).
  let location = $derived(resolveSettingsLocation(page.url));
  let activeCategory = $derived<SettingsCategory>(location.tab);

  $effect(() => {
    const { changed, canonicalHref } = location;
    if (!changed) return;
    void goto(canonicalHref, { replaceState: true, noScroll: true, keepFocus: true });
  });

  // ---- Local search: labels and help only, never values, paths or secrets.
  let searchQuery = $state("");
  let lastSearch = $state("");
  let searchInput = $state<HTMLInputElement | null>(null);
  let isSearching = $derived(searchQuery.trim().length > 0);
  let results = $derived(searchSettings(searchQuery, (key) => $t(key)));
  let cameFromSearch = $state(false);

  async function openResult(entry: SettingSearchEntry) {
    lastSearch = searchQuery;
    cameFromSearch = true;
    focusedHref = "";
    searchQuery = "";
    await goto(settingsEntryUrl(entry), { noScroll: true, keepFocus: true });
  }

  function backToSearch() {
    searchQuery = lastSearch;
    cameFromSearch = false;
    void tick().then(() => searchInput?.focus());
  }

  function isEditable(node: Element | null): boolean {
    if (!node || node === searchInput) return false;
    const el = node as HTMLElement;
    return el.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName);
  }

  // Cmd/Ctrl+F focuses the local search, but never steals a field being edited.
  function onKeydown(e: KeyboardEvent) {
    if (e.key.toLowerCase() !== "f" || !(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey) return;
    if (isEditable(document.activeElement)) return;
    if (document.querySelector("dialog[open]")) return;
    e.preventDefault();
    searchInput?.focus();
    searchInput?.select();
  }

  // ---- Focus the destination after it mounts: exact enabled control, else its
  // section, else the page title. Plain category clicks keep focus on the nav.
  let ready = $derived(settings !== null);
  let focusedHref = "";
  $effect(() => {
    const { setting, section, canonicalHref } = location;
    if (isSearching || location.changed || !ready) return;
    if (!setting && !section) return;
    // Once per destination: later saves re-render the panel but must not move focus.
    if (focusedHref === canonicalHref) return;
    focusedHref = canonicalHref;
    let cleanup = () => {};
    let cancelled = false;
    const focusSection = () => {
      if (cancelled) return;
      cleanup = section
        ? focusWhenReady(`#settings-group-${CSS.escape(section)}`, { timeoutMs: 3000 })
        : focusWhenReady("#settings-panel-title", { timeoutMs: 1000 });
    };
    void tick().then(() => {
      if (cancelled) return;
      if (setting) {
        cleanup = focusWhenReady(`#settings-active-panel [data-setting-key="${CSS.escape(setting)}"]`, {
          timeoutMs: 1500,
          preferControl: true,
          onUnavailable: focusSection,
        });
      } else {
        focusSection();
      }
    });
    return () => { cancelled = true; cleanup(); };
  });

  // ---- Reset.
  let resetOpen = $state(false);
  let resetScope = $state<ResetScope>({ kind: "app" });
  function askReset(scope: ResetScope) {
    resetScope = scope;
    resetOpen = true;
  }
  let resetSectionName = $derived(resetScope.kind === "section" ? $t(CATEGORY_LABEL_KEYS[resetScope.section as SettingsCategory]) : "");

  async function handleUpdate() {
    updating = true;
    try { await installUpdate(); }
    catch { showToast("error", $t("common.error")); updating = false; }
  }

  let retryingEffects = $state(false);
  async function retryEffects() {
    if (retryingEffects) return;
    retryingEffects = true;
    try { for (const effect of pendingEffects) await retrySettingsEffect(effect.id); }
    catch { /* The store keeps the effect pending; the banner stays. */ }
    finally { retryingEffects = false; }
  }

  // Recovered-profile banner: rendered only once its catalog key exists.
  const RECOVERED_KEY = "settings.rebrand.state.recovered";
  let recoveredFields = $derived(loadState.kind === "ready" && loadState.loadStatus.kind === "recovered" ? loadState.loadStatus.fields : []);
  let recoveredText = $derived($t(RECOVERED_KEY));

  const SECTION_OF: Record<SettingsCategory, SettingsSection> = {
    general: "general", appearance: "appearance", downloads: "downloads",
    ai: "ai", integrations: "integrations", advanced: "advanced",
  };

  const MANAGERS: { href: string; titleKey: string; hintKey?: string }[] = [
    { href: "/llm/accounts", titleKey: "llm.accounts.title" },
    { href: "/llm/models", titleKey: "llm.models.title", hintKey: "llm.surface.models_hint" },
    { href: "/llm/roster", titleKey: "llm.roster.title", hintKey: "llm.surface.roster_hint" },
    { href: "/llm/mcp", titleKey: "llm.mcp.title", hintKey: "llm.surface.mcp_hint" },
    { href: "/llm/skills", titleKey: "llm.skills.title", hintKey: "llm.surface.skills_hint" },
  ];
</script>

<svelte:window onkeydown={onKeydown} />

{#snippet managerLink(href: string, titleKey: string, hintKey?: string)}
  <a class="manager-link" {href}>
    <span class="manager-text">
      <span class="manager-title">{$t(titleKey)}</span>
      {#if hintKey}<span class="manager-hint">{$t(hintKey)}</span>{/if}
    </span>
    <SettingsIcon name="chevron" size={18} />
  </a>
{/snippet}

{#snippet sectionReset(category: SettingsCategory)}
  <div class="section-reset">
    <button type="button" class="btn btn-secondary btn-sm" onclick={() => askReset({ kind: "section", section: SECTION_OF[category] })}>
      {$t("settings.rebrand.reset.section_action")}
    </button>
  </div>
{/snippet}

{#snippet nav()}
  <!-- The hint replaces the native placeholder, which cannot wrap and was clipped
       in the 196 px column; it sits under the input and grows the field instead. -->
  <div class="settings-search-field">
    <span class="search-icon"><SettingsIcon name="search" size={18} /></span>
    <input
      bind:this={searchInput}
      bind:value={searchQuery}
      type="search"
      class="settings-search-input"
      aria-label={$t("settings.rebrand.search.label")}
      aria-describedby="settings-search-hint"
      spellcheck="false"
      autocomplete="off"
    />
    <span id="settings-search-hint" class="search-hint" aria-hidden="true" hidden={searchQuery !== ""}>{$t("settings.rebrand.search.placeholder")}</span>
    {#if isSearching}
      <button type="button" class="clear-search" aria-label={$t("settings.rebrand.search.clear")} onclick={() => { searchQuery = ""; searchInput?.focus(); }}>×</button>
    {/if}
  </div>
  <SettingsNav active={activeCategory} searching={isSearching} onNavigate={() => { searchQuery = ""; lastSearch = ""; cameFromSearch = false; }} />
{/snippet}

{#snippet banner()}
  {#if updateInfo.available}
    <div class="settings-banner">
      <span>{updating ? $t("settings.update_downloading") : $t("settings.update_available", { version: updateInfo.version })}</span>
      <button type="button" class="btn btn-secondary btn-sm" onclick={handleUpdate} disabled={updating}>{$t("settings.update_button")}</button>
    </div>
  {/if}
  {#if recoveredFields.length > 0 && recoveredText !== RECOVERED_KEY}
    <div class="settings-banner" role="status">
      <SettingsIcon name="alert" size={18} />
      <span>{recoveredText} <bdi dir="ltr">{recoveredFields.join(", ")}</bdi></span>
    </div>
  {/if}
  {#if pendingEffects.length > 0}
    <div class="settings-banner" role="status">
      <span>{$t("settings.rebrand.state.pending_apply")}</span>
      <button type="button" class="btn btn-secondary btn-sm" onclick={retryEffects} disabled={retryingEffects}>{$t("settings.rebrand.state.retry_apply")}</button>
    </div>
  {/if}
{/snippet}

<SettingsShell {nav} {banner} busy={loadState.kind === "loading"}>
  {#if isSearching}
    <SettingsSearchResults {results} onOpen={openResult} onClear={() => { searchQuery = ""; searchInput?.focus(); }} />
  {:else}
    <section id="settings-active-panel" class="settings-panel-rebrand" aria-labelledby="settings-panel-title">
      <h1 id="settings-panel-title" class="settings-title" tabindex="-1">{$t(CATEGORY_LABEL_KEYS[activeCategory])}</h1>

      {#if cameFromSearch && lastSearch && location.setting}
        <div class="search-destination">
          <p>{$t("settings.search.destination_hint")}</p>
          <button type="button" class="btn btn-secondary btn-sm" onclick={backToSearch}>{$t("settings.search.back")}</button>
        </div>
      {/if}

      {#if loadState.kind === "failed"}
        <!-- Never show a form over defaults when the profile could not be read. -->
        <SettingsEmptyState error title={$t("settings.rebrand.state.unavailable")} onRetry={() => { void loadSettings().catch(() => {}); }} />
      {:else if !settings}
        <div class="settings-skeleton" role="status" aria-busy="true">
          {#each [0, 1, 2, 3] as row (row)}<span class="skeleton-row" aria-hidden="true"></span>{/each}
          <span class="visually-hidden">{$t("settings.rebrand.state.loading")}</span>
        </div>
      {:else if activeCategory === "general"}
        <SettingsGeneral />
        {@render sectionReset("general")}
      {:else if activeCategory === "appearance"}
        <SettingsAppearance />
        <SettingGroup id="typography" title={$t("settings.typography.section_title")} advanced>
          <SettingsTypography />
        </SettingGroup>
        {@render sectionReset("appearance")}
      {:else if activeCategory === "downloads"}
        <!-- Cookies, channels and rules are groups inside SettingsDownloads. -->
        <SettingsDownloads />
        <SettingGroup id="templates" title={$t("omnibox.adv.templates_title")} description={$t("omnibox.adv.templates_empty")} advanced>
          <div data-setting-key="omnibox.adv.templates_title">
            {@render managerLink("/?panel=download-advanced", "omnibox.adv.templates_title")}
          </div>
        </SettingGroup>
        {@render sectionReset("downloads")}
      {:else if activeCategory === "ai"}
        <SettingsAI />
        <SettingGroup id="managers" title={$t("llm.accounts.title")}>
          <ul class="manager-list">
            {#each MANAGERS as item (item.href)}
              <li data-setting-key={item.titleKey}>{@render managerLink(item.href, item.titleKey, item.hintKey)}</li>
            {/each}
          </ul>
        </SettingGroup>
        <SettingGroup id="execution" title={$t("llm.watchdog.title")} description={$t("llm.watchdog.subtitle")} advanced>
          <ul class="manager-list">
            <li data-setting-key="llm.prune.title">{@render managerLink("/llm/models", "llm.prune.title")}</li>
            <li data-setting-key="llm.watchdog.title">{@render managerLink("/llm/models", "llm.watchdog.title")}</li>
          </ul>
        </SettingGroup>
        {@render sectionReset("ai")}
      {:else if activeCategory === "integrations"}
        <SettingsBrowserExtension />
        <SettingGroup id="extension-install" title={$t("settings.browser_ext.update_install")}>
          <SettingsDependencies part="extension" />
        </SettingGroup>
        <SettingGroup id="music" title={$t("music.settings_title")} description={$t("music.settings_desc")}>
          <SettingsAdvanced part="music" />
          <div data-setting-key="music.settings_title">{@render managerLink("/music?view=settings", "music.settings_title")}</div>
        </SettingGroup>
        <SettingGroup id="video" title={$t("video.settings_title")} description={$t("video.settings_desc")}>
          <SettingsAdvanced part="video" />
        </SettingGroup>
        <SettingGroup id="tube" title={$t("tube.settings_title")} description={$t("tube.settings_desc")}>
          <SettingsAdvanced part="tube" />
        </SettingGroup>
        <SettingGroup id="read" title={$t("read.settings_title")} description={$t("read.settings_desc")}>
          <SettingsAdvanced part="read" />
        </SettingGroup>
        <SettingGroup id="telegram" title={$t("telegram.settings_title")} description={$t("telegram.settings_desc")}>
          <SettingsAdvanced part="telegram" />
        </SettingGroup>
        <SettingGroup id="washmybelly" title={$t("washmybelly.settings_title")} description={$t("washmybelly.settings_desc")}>
          <SettingsAdvanced part="washmybelly" />
        </SettingGroup>
        <SettingsWorld />
        <SettingGroup id="canvas" title={$t("canvas.settings.title")}>
          <SettingsCanvas />
        </SettingGroup>
        <SettingGroup id="league" title={$t("league.settings_title")} description={$t("league.settings_desc")}>
          <SettingsAdvanced part="league" />
        </SettingGroup>
        <RichPresencePanel />
        <SettingGroup id="mcp-server" title={$t("llm.mcp.server_title")} advanced>
          <div data-setting-key="llm.mcp.server_title">{@render managerLink("/llm/mcp?mode=server", "llm.mcp.server_title", "llm.surface.mcp_server_hint")}</div>
        </SettingGroup>
        {@render sectionReset("integrations")}
      {:else if activeCategory === "advanced"}
        <SettingGroup id="network" title={$t("settings.cat_network")}>
          <SettingsNetwork />
        </SettingGroup>
        <SettingGroup id="torrent" title={$t("settings.advanced.section_torrent")} description={$t("settings.advanced.section_torrent_desc")} advanced>
          <SettingsAdvanced part="torrent" />
        </SettingGroup>
        <SettingGroup id="performance" title={$t("settings.advanced.section_performance")} description={$t("settings.advanced.section_performance_desc")}>
          <SettingsAdvanced part="performance" />
        </SettingGroup>
        <SettingGroup id="dependencies" title={$t("settings.dependencies.title")}>
          <SettingsDependencies part="binaries" />
        </SettingGroup>
        <SettingGroup id="storage" title={$t("settings.storage.title")} description={$t("settings.storage.description")}>
          <SettingsStorage />
        </SettingGroup>
        <SettingGroup id="expert" title={$t("settings.advanced.section_expert")} description={$t("settings.advanced.section_expert_desc")} advanced>
          <SettingsAdvanced part="expert" />
        </SettingGroup>
        <SettingGroup id="debug" title={$t("settings.advanced.section_debug")} description={$t("settings.advanced.section_debug_desc")} advanced>
          <SettingsAdvanced part="debug" />
        </SettingGroup>
        <SettingGroup id="reset" title={$t("settings.advanced.reset")}>
          <SettingRow id="settings-reset-section" label={$t("settings.rebrand.reset.section_action")} help={$t("settings.rebrand.reset.section_body")}>
            {#snippet control({ id, describedBy })}
              <button {id} type="button" class="btn btn-secondary btn-sm" aria-describedby={describedBy} onclick={() => askReset({ kind: "section", section: "advanced" })}>
                {$t("settings.rebrand.reset.section_action")}
              </button>
            {/snippet}
          </SettingRow>
          <SettingRow id="settings-reset-app" label={$t("settings.rebrand.reset.app_action")} help={$t("settings.rebrand.reset.app_body")} settingKey="settings.advanced.reset">
            {#snippet control({ id, describedBy })}
              <button {id} type="button" class="btn btn-destructive btn-sm" aria-describedby={describedBy} onclick={() => askReset({ kind: "app" })}>
                {$t("settings.rebrand.reset.app_action")}
              </button>
            {/snippet}
          </SettingRow>
        </SettingGroup>
      {/if}
    </section>
  {/if}
</SettingsShell>

<SettingsResetDialog bind:open={resetOpen} scope={resetScope} sectionName={resetSectionName} />

<style>
  .settings-search-field { position: relative; display: grid; grid-template-columns: minmax(0, 1fr); }
  .search-icon { position: absolute; z-index: 1; inset-block-start: 7px; inset-inline-start: 8px; color: var(--text-muted); pointer-events: none; display: inline-flex; }
  .settings-search-input, .search-hint { grid-area: 1 / 1; }
  .search-hint {
    box-sizing: border-box;
    padding-block: 7px;
    padding-inline: 33px 9px;
    font-size: 13px;
    line-height: 18px;
    color: var(--text-muted);
    overflow-wrap: anywhere;
    pointer-events: none;
  }
  .search-hint[hidden] { display: none; }
  .settings-search-input {
    inline-size: 100%;
    block-size: 100%;
    min-block-size: 32px;
    box-sizing: border-box;
    padding-block: 0;
    padding-inline: 32px 28px;
    border: 1px solid var(--separator);
    border-radius: 6px;
    background: var(--control-bg, var(--surface));
    color: var(--text);
    font: inherit;
    font-size: 13px;
  }
  .settings-search-input:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .settings-search-input::-webkit-search-cancel-button { display: none; }
  .clear-search {
    position: absolute;
    inset-block-start: 4px;
    inset-inline-end: 4px;
    inline-size: 24px;
    block-size: 24px;
    border: 0;
    border-radius: 6px;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
  }
  .clear-search:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .settings-panel-rebrand { min-inline-size: 0; }
  .settings-banner {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2) var(--space-3);
    margin-block-end: var(--space-4);
    padding: var(--space-3);
    border: 1px solid var(--separator);
    border-radius: 10px;
    background: var(--accent-soft);
    font-size: 13px;
    line-height: 20px;
    color: var(--text);
  }
  .search-destination {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
    margin-block-end: var(--space-4);
    padding: var(--space-3);
    border-inline-start: 2px solid var(--accent);
    background: var(--accent-soft);
  }
  .search-destination p { margin: 0; flex: 1 1 16rem; font-size: 12px; line-height: 18px; color: var(--text-muted); }
  .settings-skeleton { display: flex; flex-direction: column; gap: var(--space-3); }
  .skeleton-row { display: block; block-size: 40px; border-radius: 6px; background: var(--surface-hi); }
  .visually-hidden { position: absolute; inline-size: 1px; block-size: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  .manager-list { list-style: none; margin: 0; padding: 0; }
  .manager-list li + li { border-block-start: 1px solid var(--separator); }
  .manager-link {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-block-size: 40px;
    padding-block: var(--space-2);
    color: var(--text);
    text-decoration: none;
    border-radius: 6px;
  }
  .manager-link:hover .manager-title { color: var(--accent-text, var(--accent)); }
  .manager-link:focus-visible { outline: var(--focus-ring); outline-offset: var(--focus-ring-offset, 2px); }
  .manager-text { display: flex; flex-direction: column; min-inline-size: 0; }
  .manager-title { font-size: 13px; line-height: 20px; }
  .manager-hint { font-size: 12px; line-height: 18px; color: var(--text-muted); overflow-wrap: anywhere; }
  .section-reset { display: flex; justify-content: flex-end; padding-block: var(--space-4); border-block-start: 1px solid var(--separator); }
</style>
