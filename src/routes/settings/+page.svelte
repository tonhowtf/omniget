<script lang="ts">
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
    installingDep = name;
    try {
      await invoke("install_dependency", { name, variant });
      await loadDeps();
      await refreshYtdlpStatus();
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

  type SettingsCategory = "downloads" | "appearance" | "typography" | "network" | "plugins" | "advanced";
  let activeCategory = $state<SettingsCategory>("downloads");

  let searchQuery = $state("");
  let normalizedQuery = $derived(searchQuery.trim().toLowerCase());
  let isSearching = $derived(normalizedQuery.length > 0);

  $effect(() => {
    const q = normalizedQuery;
    if (typeof document === "undefined") return;
    const rows = document.querySelectorAll<HTMLElement>(
      ".settings-content .setting-row, .settings-content .section-title",
    );
    if (!q) {
      rows.forEach((row) => {
        row.style.display = "";
      });
      document
        .querySelectorAll<HTMLElement>(".settings-content .card")
        .forEach((card) => (card.style.display = ""));
      document
        .querySelectorAll<HTMLElement>(".settings-content .section")
        .forEach((section) => (section.style.display = ""));
      return;
    }
    rows.forEach((row) => {
      const text = (row.textContent || "").toLowerCase();
      row.style.display = text.includes(q) ? "" : "none";
    });
    document
      .querySelectorAll<HTMLElement>(".settings-content .card")
      .forEach((card) => {
        const hasVisibleRow = Array.from(
          card.querySelectorAll<HTMLElement>(".setting-row"),
        ).some((r) => r.style.display !== "none");
        card.style.display = hasVisibleRow ? "" : "none";
      });
    document
      .querySelectorAll<HTMLElement>(".settings-content .section")
      .forEach((section) => {
        const titleEl = section.querySelector<HTMLElement>(".section-title");
        const hasVisibleCard = Array.from(
          section.querySelectorAll<HTMLElement>(".card"),
        ).some((c) => c.style.display !== "none");
        const hasMatchingTitle =
          (titleEl?.textContent || "").toLowerCase().includes(q);
        section.style.display = hasVisibleCard || hasMatchingTitle ? "" : "none";
      });
  });

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

  const YTDLP_FLAG_CATALOG = [
    { flag: "--embed-subs", label: "Embed subtitles" },
    { flag: "--write-thumbnail", label: "Save thumbnail" },
    { flag: "--write-description", label: "Save description" },
    { flag: "--write-comments", label: "Save comments" },
    { flag: "--restrict-filenames", label: "ASCII filenames" },
    { flag: "--no-overwrites", label: "No overwrites" },
    { flag: "--prefer-free-formats", label: "Free formats" },
    { flag: "--force-ipv4", label: "Force IPv4" },
    { flag: "--geo-bypass", label: "Geo bypass" },
    { flag: "--limit-rate", label: "Limit rate", hasValue: true, placeholder: "e.g. 1M" },
    { flag: "--sleep-interval", label: "Sleep interval", hasValue: true, placeholder: "e.g. 5" },
  ];

  async function toggleFlag(flag: string) {
    let current = [...(settings?.download?.extra_ytdlp_flags ?? [])];
    const idx = current.findIndex(f => f === flag || f.startsWith(flag + " "));
    if (idx >= 0) {
      current.splice(idx, 1);
    } else {
      current.push(flag);
    }
    await updateSettings({ download: { extra_ytdlp_flags: current } });
  }

  async function setFlagValue(flag: string, value: string) {
    let current = [...(settings?.download?.extra_ytdlp_flags ?? [])];
    const idx = current.findIndex(f => f === flag || f.startsWith(flag + " "));
    if (idx >= 0) {
      current[idx] = value ? `${flag} ${value}` : flag;
    }
    await updateSettings({ download: { extra_ytdlp_flags: current } });
  }

  function isFlagActive(flag: string): boolean {
    return (settings?.download?.extra_ytdlp_flags ?? []).some(f => f === flag || f.startsWith(flag + " "));
  }

  function getFlagValue(flag: string): string {
    const f = (settings?.download?.extra_ytdlp_flags ?? []).find(f => f.startsWith(flag + " "));
    return f ? f.slice(flag.length + 1) : "";
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
  <div class="settings">
    <h2 class="page-title">{$t('settings.title')}</h2>

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

    <div class="category-tabs" class:searching={isSearching}>
      {#each [
        ["downloads", "settings.cat_downloads"],
        ["appearance", "settings.cat_appearance"],
        ["typography", "settings.cat_typography"],
        ["network", "settings.cat_network"],
        ["plugins", "settings.cat_plugins"],
        ["advanced", "settings.cat_advanced"],
      ] as [cat, key] (cat)}
        <button class="cat-tab" class:active={!isSearching && activeCategory === cat} onclick={() => { activeCategory = cat as SettingsCategory; searchQuery = ""; }}>
          {$t(key)}
        </button>
      {/each}
    </div>

    <div class="settings-content">

    {#if isSearching || activeCategory === "appearance"}
      <SettingsAppearance />
    {/if}

    {#if isSearching || activeCategory === "typography"}
      <SettingsTypography />
    {/if}

    {#if isSearching || activeCategory === "downloads"}
      <SettingsDownloads />
    {/if}

    {#if isSearching || activeCategory === "plugins"}
      <SettingsPlugins
        {deps}
        {installingDep}
        onInstallDep={handleInstallDep}
        onRefresh={loadDeps}
      />
    {/if}

    {#if isSearching || activeCategory === "network"}
      <SettingsBrowserExtension />
      <SettingsNetwork />
    {/if}

    {#if isSearching || activeCategory === "advanced"}
      <SettingsAdvanced {resetting} onReset={handleReset} />
    {/if}
    </div>
  </div>
{:else}
  <div class="settings-loading">
    <span class="spinner"></span>
  </div>
{/if}

