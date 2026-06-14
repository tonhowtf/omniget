<script lang="ts">
  import { tick } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t, locale, loadTranslations } from "$lib/i18n";
  import { getSettings, updateSettings, resetSettings } from "$lib/stores/settings-store.svelte";
  import { getUpdateInfo } from "$lib/stores/update-store.svelte";
  import { installUpdate } from "$lib/updater";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { refreshYtdlpStatus } from "$lib/stores/dependency-store.svelte";
  import { isDebugEnabled, setDebugEnabled, setDebugPanelOpen } from "$lib/stores/debug-store.svelte";
  import ContextHint from "$components/hints/ContextHint.svelte";
  import SettingsPlugins from "$components/settings/SettingsPlugins.svelte";
  import SettingsAdvanced from "$components/settings/SettingsAdvanced.svelte";
  import SettingsAppearance from "$components/settings/SettingsAppearance.svelte";
  import SettingsNetwork from "$components/settings/SettingsNetwork.svelte";
  import SettingsDownloads from "$components/settings/SettingsDownloads.svelte";
  import SettingsTypography from "$components/settings/SettingsTypography.svelte";
  import SettingsBrowserExtension from "$components/settings/SettingsBrowserExtension.svelte";
  import SettingsCookies from "$components/settings/SettingsCookies.svelte";
  import SettingsChannels from "$components/settings/SettingsChannels.svelte";
  import SettingsAI from "$components/settings/SettingsAI.svelte";

  type DependencyStatus = {
    name: string;
    installed: boolean;
    version: string | null;
  };

  let settings = $derived(getSettings());
  let updateInfo = $derived(getUpdateInfo());
  let isWindows = typeof navigator !== "undefined" && navigator.userAgent.includes("Windows");
  let resetting = $state(false);
  let updating = $state(false);
  let deps = $state<DependencyStatus[]>([]);
  let installingDep = $state<string | null>(null);

  async function loadDeps() {
    try {
      deps = await invoke<DependencyStatus[]>("check_dependencies");
    } catch {}
  }

  async function chooseCookieFile() {
    const selected = await open({
      title: "Select cookies.txt file",
      filters: [{ name: "Cookies", extensions: ["txt"] }],
      multiple: false,
    });
    if (selected && typeof selected === "string") {
      await updateSettings({ download: { cookie_file: selected } });
    }
  }

  async function handleInstallDep(name: string, variant: string | null = null) {
    const wasInstalled = deps.find((d) => d.name === name)?.installed ?? false;
    installingDep = name;
    try {
      const version = await invoke<string>("install_dependency", { name, variant, force: wasInstalled });
      await loadDeps();
      await refreshYtdlpStatus();
      showToast(
        "success",
        $t(
          wasInstalled ? "settings.dependencies.update_success" : "settings.dependencies.install_success",
          { name, version },
        ) as string,
      );
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : e.message ?? $t("common.error"));
    } finally {
      installingDep = null;
    }
  }

  $effect(() => {
    if (settings) {
      loadDeps();
    }
  });

  async function handleUpdate() {
    updating = true;
    try {
      await installUpdate();
    } catch {
      updating = false;
    }
  }

  async function changeTheme(value: string) {
    await updateSettings({ appearance: { theme: value } });
  }

  const CORE_THEMES = [
    { id: "system", labelKey: "settings.appearance.theme_system", colors: null as string[] | null },
    { id: "light", labelKey: null, label: "Light", colors: ["#fafafa", "#1a1a1a", "#E05500"] },
    { id: "dark", labelKey: null, label: "Dark", colors: ["#0a0a0a", "#e8e8e8", "#FF7D38"] },
  ];
  const MORE_THEMES = [
    { id: "catppuccin-latte", label: "Catppuccin Latte", colors: ["#eff1f5", "#4c4f69", "#1e66f5"] },
    { id: "catppuccin-frappe", label: "Catppuccin Frappé", colors: ["#303446", "#c6d0f5", "#8caaee"] },
    { id: "catppuccin-macchiato", label: "Catppuccin Macchiato", colors: ["#24273a", "#cad3f5", "#8aadf4"] },
    { id: "catppuccin-mocha", label: "Catppuccin Mocha", colors: ["#1e1e2e", "#cdd6f4", "#89b4fa"] },
    { id: "one-dark-pro", label: "One Dark Pro", colors: ["#282c34", "#abb2bf", "#61afef"] },
    { id: "dracula", label: "Dracula", colors: ["#22212C", "#F8F8F2", "#9580FF"] },
    { id: "nyxvamp-veil", label: "NyxVamp Veil", colors: ["#1E1E2E", "#D9E0EE", "#F28FAD"] },
    { id: "nyxvamp-obsidian", label: "NyxVamp Obsidian", colors: ["#000A0F", "#C0C0CE", "#F28FAD"] },
    { id: "nyxvamp-radiance", label: "NyxVamp Radiance", colors: ["#F7F7FF", "#1E1E2E", "#9655FF"] },
    { id: "eink-day", label: "E-ink Day", colors: ["#f5f2ea", "#1d1d1b", "#2b2b28"] },
    { id: "eink-sepia", label: "E-ink Sepia", colors: ["#f0e6d2", "#3f2e20", "#7a4a22"] },
    { id: "eink-night", label: "E-ink Night", colors: ["#0a0a0a", "#d4d4d0", "#b0b0ab"] },
  ];
  const MORE_THEME_IDS = new Set(MORE_THEMES.map((t) => t.id));
  let selectedInMore = $derived(MORE_THEME_IDS.has(settings?.appearance?.theme ?? ""));

  async function changeLanguage(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    await updateSettings({ appearance: { language: value } });
    await loadTranslations(value, "/settings");
    locale.set(value);
  }

  async function chooseFolder() {
    const selected = await open({ directory: true, title: $t("settings.download.default_output_dir") });
    if (selected) {
      await updateSettings({ download: { default_output_dir: selected } });
    }
  }

  async function toggleBool(section: string, key: string, current: boolean) {
    await updateSettings({ [section]: { [key]: !current } });
  }

  async function changeNumber(section: string, key: string, e: Event) {
    const value = parseInt((e.target as HTMLInputElement).value, 10);
    if (!isNaN(value) && value > 0) {
      await updateSettings({ [section]: { [key]: value } });
      if (key === "max_concurrent_downloads") {
        try {
          await invoke("update_max_concurrent", { max: value });
        } catch {}
      }
    }
  }

  async function changeQuality(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    await updateSettings({ download: { video_quality: value } });
  }

  type SettingsCategory = "downloads" | "appearance" | "typography" | "network" | "cookies" | "channels" | "ai" | "plugins" | "advanced";

  const SETTINGS_NAV_GROUPS: {
    labelKey: string;
    items: [SettingsCategory, string][];
  }[] = [
    {
      labelKey: "settings.group_general",
      items: [
        ["appearance", "settings.cat_appearance"],
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
        ["plugins", "settings.cat_plugins"],
      ],
    },
    {
      labelKey: "settings.group_advanced",
      items: [["advanced", "settings.cat_advanced"]],
    },
  ];

  let activeCategory = $state<SettingsCategory>("downloads");

  let tabApplied = false;
  $effect(() => {
    if (tabApplied || typeof window === "undefined") return;
    tabApplied = true;
    const tab = new URLSearchParams(window.location.search).get("tab");
    const valid = ["downloads", "appearance", "typography", "network", "cookies", "channels", "ai", "plugins", "advanced"];
    if (tab && valid.includes(tab)) {
      activeCategory = tab as SettingsCategory;
    }
  });

  let searchQuery = $state("");
  let normalizedQuery = $derived(searchQuery.trim().toLowerCase());
  let isSearching = $derived(normalizedQuery.length > 0);
  let categoryMatchCounts = $state<Partial<Record<SettingsCategory, number>>>({});

  const PANEL_MATCH_SELECTORS =
    ".setting-row, .preset-card, .settings-section-head, .section-title, .deps-table-row";

  function clearSearchHighlights(root: ParentNode) {
    root.querySelectorAll("mark.settings-search-hit").forEach((mark) => {
      const parent = mark.parentNode;
      if (!parent) return;
      parent.replaceChild(document.createTextNode(mark.textContent ?? ""), mark);
      parent.normalize();
    });
  }

  function highlightPlainText(el: HTMLElement, query: string) {
    if (el.children.length > 0) return;
    const text = el.textContent ?? "";
    const lower = text.toLowerCase();
    const idx = lower.indexOf(query);
    if (idx === -1) return;
    const before = text.slice(0, idx);
    const match = text.slice(idx, idx + query.length);
    const after = text.slice(idx + query.length);
    el.textContent = "";
    if (before) el.appendChild(document.createTextNode(before));
    const mark = document.createElement("mark");
    mark.className = "settings-search-hit";
    mark.textContent = match;
    el.appendChild(mark);
    if (after) el.appendChild(document.createTextNode(after));
  }

  function applySearchHighlights(container: HTMLElement, query: string) {
    clearSearchHighlights(container);
    if (!query) return;
    const highlightSelectors =
      ".setting-path, .settings-section-hint, .preset-desc, .preset-label";
    container.querySelectorAll<HTMLElement>(highlightSelectors).forEach((el) => {
      if (el.style.display === "none") return;
      highlightPlainText(el, query);
    });
    container.querySelectorAll<HTMLElement>(".setting-label").forEach((el) => {
      if (el.style.display === "none" || el.children.length > 0) return;
      highlightPlainText(el, query);
    });
  }

  function countPanelMatches(panel: HTMLElement, query: string): number {
    let count = 0;
    panel.querySelectorAll<HTMLElement>(PANEL_MATCH_SELECTORS).forEach((el) => {
      if ((el.textContent ?? "").toLowerCase().includes(query)) count += 1;
    });
    return count;
  }

  function resetSettingsSearchDisplay(content: HTMLElement) {
    content
      .querySelectorAll<HTMLElement>(
        ".setting-row, .section-title, .settings-section-head, .card, .preset-card, .section, .deps-table-row",
      )
      .forEach((el) => {
        el.style.display = "";
      });
    clearSearchHighlights(content);
    categoryMatchCounts = {};
  }

  function applySettingsPanelFilter(content: HTMLElement, query: string) {
    const rows = content.querySelectorAll<HTMLElement>(
      ".setting-row, .section-title, .settings-section-head, .deps-table-row",
    );
    rows.forEach((row) => {
      const text = (row.textContent ?? "").toLowerCase();
      row.style.display = text.includes(query) ? "" : "none";
    });
    content.querySelectorAll<HTMLElement>(".preset-card").forEach((card) => {
      const text = (card.textContent ?? "").toLowerCase();
      card.style.display = text.includes(query) ? "" : "none";
    });
    content.querySelectorAll<HTMLElement>(".card").forEach((card) => {
      const hasVisibleRow = Array.from(
        card.querySelectorAll<HTMLElement>(".setting-row"),
      ).some((r) => r.style.display !== "none");
      card.style.display = hasVisibleRow ? "" : "none";
    });
    content.querySelectorAll<HTMLElement>(".section").forEach((section) => {
      const titleEl = section.querySelector<HTMLElement>(
        ".section-title, .settings-section-head",
      );
      const hasVisibleRow = Array.from(
        section.querySelectorAll<HTMLElement>(".setting-row, .deps-table-row"),
      ).some((r) => r.style.display !== "none");
      const hasVisibleCard = Array.from(
        section.querySelectorAll<HTMLElement>(".card"),
      ).some((c) => c.style.display !== "none");
      const hasVisiblePreset = Array.from(
        section.querySelectorAll<HTMLElement>(".preset-card"),
      ).some((p) => p.style.display !== "none");
      const hasMatchingTitle = (titleEl?.textContent ?? "")
        .toLowerCase()
        .includes(query);
      section.style.display =
        hasVisibleRow || hasVisibleCard || hasVisiblePreset || hasMatchingTitle
          ? ""
          : "none";
    });
  }

  $effect(() => {
    const q = normalizedQuery;
    if (typeof document === "undefined") return;
    let cancelled = false;

    (async () => {
      await tick();
      if (cancelled) return;
      const content = document.querySelector<HTMLElement>(".settings-content");
      if (!content) return;

      if (!q) {
        resetSettingsSearchDisplay(content);
        return;
      }

      if (isSearching) await tick();
      if (cancelled) return;

      applySettingsPanelFilter(content, q);

      const counts: Partial<Record<SettingsCategory, number>> = {};
      content.querySelectorAll<HTMLElement>("[data-settings-cat]").forEach((panel) => {
        const cat = panel.dataset.settingsCat as SettingsCategory | undefined;
        if (!cat) return;
        counts[cat] = countPanelMatches(panel, q);
      });
      categoryMatchCounts = counts;

      applySearchHighlights(content, q);
    })();

    return () => {
      cancelled = true;
    };
  });

  function groupHasSearchMatch(group: (typeof SETTINGS_NAV_GROUPS)[number]): boolean {
    if (!isSearching) return true;
    return group.items.some(([cat]) => (categoryMatchCounts[cat] ?? 0) > 0);
  }

  let templateInput = $state("");
  let templateTimer = $state<ReturnType<typeof setTimeout> | null>(null);
  let hotkeyInput = $state("");
  let hotkeyTimer = $state<ReturnType<typeof setTimeout> | null>(null);
  let hotkeyMode = $state<"record" | "type">("record");
  let hotkeyRecording = $state(false);
  let proxyHost = $state("");
  let proxyUsername = $state("");
  let proxyPassword = $state("");
  let proxyTimer = $state<ReturnType<typeof setTimeout> | null>(null);

  $effect(() => {
    if (settings) {
      templateInput = settings.download.filename_template;
      hotkeyInput = settings.download.hotkey_binding;
      proxyHost = settings.proxy?.host ?? "";
      proxyUsername = settings.proxy?.username ?? "";
      proxyPassword = settings.proxy?.password ?? "";
    }
  });

  function previewTemplate(template: string): string {
    return template
      .replace("%(title).200s", "My Video Title")
      .replace("%(title)s", "My Video Title")
      .replace("%(id)s", "dQw4w9WgXcQ")
      .replace("%(ext)s", "mp4")
      .replace("%(uploader)s", "Channel Name")
      .replace("%(upload_date)s", "20260217")
      .replace("%(resolution)s", "1920x1080")
      .replace("%(fps)s", "30")
      .replace("%(duration)s", "212");
  }

  function handleTemplateInput(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    templateInput = value;
    if (templateTimer) clearTimeout(templateTimer);
    templateTimer = setTimeout(async () => {
      if (value.trim() && value.includes("%(ext)s")) {
        await updateSettings({ download: { filename_template: value } });
      }
    }, 800);
  }

  function handleHotkeyInput(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    hotkeyInput = value;
    if (hotkeyTimer) clearTimeout(hotkeyTimer);
    hotkeyTimer = setTimeout(async () => {
      if (value.trim()) {
        await updateSettings({ download: { hotkey_binding: value } });
      }
    }, 800);
  }

  function mapKeyName(key: string): string | null {
    if (key.length === 1 && /[a-zA-Z]/.test(key)) return key.toUpperCase();
    if (key.length === 1 && /[0-9]/.test(key)) return key;
    if (/^F([1-9]|1[0-2])$/.test(key)) return key;
    const map: Record<string, string> = {
      " ": "Space", ArrowUp: "Up", ArrowDown: "Down", ArrowLeft: "Left", ArrowRight: "Right",
      Enter: "Enter", Tab: "Tab", Escape: "Escape", Backspace: "Backspace", Delete: "Delete",
      Home: "Home", End: "End", PageUp: "PageUp", PageDown: "PageDown", Insert: "Insert",
    };
    return map[key] ?? null;
  }

  function handleHotkeyKeyDown(e: KeyboardEvent) {
    e.preventDefault();
    e.stopPropagation();
    if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) return;
    const keyName = mapKeyName(e.key);
    if (!keyName) return;
    const parts: string[] = [];
    if (e.ctrlKey || e.metaKey) parts.push("CmdOrCtrl");
    if (e.shiftKey) parts.push("Shift");
    if (e.altKey) parts.push("Alt");
    parts.push(keyName);
    const value = parts.join("+");
    hotkeyInput = value;
    hotkeyRecording = false;
    updateSettings({ download: { hotkey_binding: value } });
  }

  async function changeProxyType(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    await updateSettings({ proxy: { proxy_type: value } });
  }

  function handleProxyHost(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    proxyHost = value;
    if (proxyTimer) clearTimeout(proxyTimer);
    proxyTimer = setTimeout(async () => {
      await updateSettings({ proxy: { host: value } });
    }, 800);
  }

  function handleProxyUsername(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    proxyUsername = value;
    if (proxyTimer) clearTimeout(proxyTimer);
    proxyTimer = setTimeout(async () => {
      await updateSettings({ proxy: { username: value } });
    }, 800);
  }

  function handleProxyPassword(e: Event) {
    const value = (e.target as HTMLInputElement).value;
    proxyPassword = value;
    if (proxyTimer) clearTimeout(proxyTimer);
    proxyTimer = setTimeout(async () => {
      await updateSettings({ proxy: { password: value } });
    }, 800);
  }

  async function handleReset() {
    if (!confirm($t("settings.advanced.reset_confirm"))) return;
    resetting = true;
    try {
      await resetSettings();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : e.message ?? $t("common.error"));
    } finally {
      resetting = false;
    }
  }
</script>

{#if settings}
  <div class="settings settings-mac">
    {#if updateInfo.available}
      <div class="update-banner">
        <span class="update-text">
          {#if updating}
            {$t('settings.update_downloading')}
          {:else}
            {$t('settings.update_available', { version: updateInfo.version })}
          {/if}
        </span>
        <button class="update-btn" onclick={handleUpdate} disabled={updating}>
          {#if updating}
            <span class="update-spinner"></span>
          {:else}
            {$t('settings.update_button')}
          {/if}
        </button>
      </div>
    {/if}

    <div class="settings-mac-body">
      <aside class="settings-mac-sidebar">
        <div class="settings-search-row">
          <input
            type="search"
            class="settings-search"
            placeholder={$t('settings.search_placeholder')}
            value={searchQuery}
            oninput={(e) => searchQuery = (e.target as HTMLInputElement).value}
            spellcheck="false"
            aria-label={$t('settings.search_placeholder')}
          />
        </div>
        {#each SETTINGS_NAV_GROUPS as group (group.labelKey)}
          {#if groupHasSearchMatch(group)}
            <div class="settings-mac-nav-group">
              <div class="mac-nav-section-header">{$t(group.labelKey)}</div>
              {#each group.items as [cat, key] (cat)}
                {#if !isSearching || (categoryMatchCounts[cat] ?? 0) > 0}
                  <button
                    class="settings-mac-cat"
                    class:active={!isSearching && activeCategory === cat}
                    class:search-match={isSearching && (categoryMatchCounts[cat] ?? 0) > 0}
                    onclick={() => { activeCategory = cat; searchQuery = ""; }}
                  >
                    <span class="settings-mac-cat-label">{$t(key)}</span>
                    {#if isSearching && (categoryMatchCounts[cat] ?? 0) > 0}
                      <span class="settings-cat-match-count">{categoryMatchCounts[cat]}</span>
                    {/if}
                  </button>
                {/if}
              {/each}
            </div>
          {/if}
        {/each}
      </aside>

      <div class="settings-mac-content settings-content">

    {#if isSearching || activeCategory === "appearance"}
      <div class="settings-panel" data-settings-cat="appearance">
        <SettingsAppearance searchActive={isSearching} />
      </div>
    {/if}

    {#if isSearching || activeCategory === "typography"}
      <div class="settings-panel" data-settings-cat="typography">
        <SettingsTypography />
      </div>
    {/if}

    {#if isSearching || activeCategory === "downloads"}
      <div class="settings-panel" data-settings-cat="downloads">
        <SettingsDownloads searchActive={isSearching} />
      </div>
    {/if}

    {#if isSearching || activeCategory === "plugins"}
      <div class="settings-panel" data-settings-cat="plugins">
        <SettingsPlugins
          {deps}
          {installingDep}
          onInstallDep={handleInstallDep}
          onRefresh={loadDeps}
        />
      </div>
    {/if}

    {#if isSearching || activeCategory === "network"}
      <div class="settings-panel" data-settings-cat="network">
        <SettingsBrowserExtension />
        <SettingsNetwork />
      </div>
    {/if}

    {#if isSearching || activeCategory === "cookies"}
      <div class="settings-panel" data-settings-cat="cookies">
        <SettingsCookies />
      </div>
    {/if}

    {#if isSearching || activeCategory === "channels"}
      <div class="settings-panel" data-settings-cat="channels">
        <SettingsChannels />
      </div>
    {/if}

    {#if isSearching || activeCategory === "ai"}
      <div class="settings-panel" data-settings-cat="ai">
        <SettingsAI />
      </div>
    {/if}

    {#if isSearching || activeCategory === "advanced"}
      <div class="settings-panel" data-settings-cat="advanced">
        <SettingsAdvanced {resetting} onReset={handleReset} searchActive={isSearching} />
      </div>
    {/if}
      </div>
    </div>
  </div>
{:else}
  <div class="settings-loading">
    <span class="spinner"></span>
  </div>
{/if}

