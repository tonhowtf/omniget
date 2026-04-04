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

  async function handleInstallDep(name: string) {
    installingDep = name;
    try {
      await invoke("install_dependency", { name });
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

  type SettingsCategory = "general" | "downloads" | "content" | "shortcuts" | "network" | "tools" | "advanced";
  let activeCategory = $state<SettingsCategory>("general");

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
    <h2>{$t('settings.title')}</h2>

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

    <div class="category-tabs">
      {#each [
        ["general", "settings.cat_general"],
        ["downloads", "settings.cat_downloads"],
        ["network", "settings.cat_network"],
        ["tools", "settings.cat_tools"],
        ["advanced", "settings.cat_advanced"],
      ] as [cat, key] (cat)}
        <button class="cat-tab" class:active={activeCategory === cat} onclick={() => { activeCategory = cat as SettingsCategory; }}>
          {$t(key)}
        </button>
      {/each}
    </div>

    {#if activeCategory === "general"}
    {#if isWindows}
      <section class="section">
        <h5 class="section-title">{$t('settings.general.title')}</h5>
        <div class="card">
          <div class="setting-row">
            <div class="setting-col">
              <span class="setting-label">{$t('settings.general.start_with_windows')}</span>
              <span class="setting-path">{$t('settings.general.start_with_windows_desc')}</span>
            </div>
            <button
              class="toggle"
              class:on={settings.start_with_windows}
              onclick={() => updateSettings({ start_with_windows: !settings!.start_with_windows })}
              role="switch"
              aria-checked={settings.start_with_windows}
              aria-label={$t('settings.general.start_with_windows')}
            >
              <span class="toggle-knob"></span>
            </button>
          </div>
        </div>
      </section>
    {/if}

    <section class="section">
      <h5 class="section-title">{$t('settings.appearance.title')}</h5>
      <div class="card">
        <div class="setting-row">
          <span class="setting-label">{$t('settings.appearance.theme')}</span>
        </div>
        <div class="theme-grid">
          {#each [
            { id: "system", label: $t("settings.appearance.theme_system"), colors: null },
            { id: "light", label: "Light", colors: ["#fafafa", "#1a1a1a", "#E05500"] },
            { id: "dark", label: "Dark", colors: ["#0a0a0a", "#e8e8e8", "#FF7D38"] },
            { id: "catppuccin-latte", label: "Catppuccin Latte", colors: ["#eff1f5", "#4c4f69", "#1e66f5"] },
            { id: "catppuccin-frappe", label: "Catppuccin Frappé", colors: ["#303446", "#c6d0f5", "#8caaee"] },
            { id: "catppuccin-macchiato", label: "Catppuccin Macchiato", colors: ["#24273a", "#cad3f5", "#8aadf4"] },
            { id: "catppuccin-mocha", label: "Catppuccin Mocha", colors: ["#1e1e2e", "#cdd6f4", "#89b4fa"] },
            { id: "one-dark-pro", label: "One Dark Pro", colors: ["#282c34", "#abb2bf", "#61afef"] },
            { id: "dracula", label: "Dracula", colors: ["#22212C", "#F8F8F2", "#9580FF"] },
            { id: "nyxvamp-veil", label: "NyxVamp Veil", colors: ["#1E1E2E", "#D9E0EE", "#F28FAD"] },
            { id: "nyxvamp-obsidian", label: "NyxVamp Obsidian", colors: ["#000A0F", "#C0C0CE", "#F28FAD"] },
            { id: "nyxvamp-radiance", label: "NyxVamp Radiance", colors: ["#F7F7FF", "#1E1E2E", "#9655FF"] },
          ] as theme (theme.id)}
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
              <span class="theme-name">{theme.label}</span>
            </button>
          {/each}
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('settings.appearance.language')}</span>
          <select class="select" value={settings.appearance.language} onchange={changeLanguage}>
            <option value="en">{$t('settings.appearance.lang_en')}</option>
            <option value="pt">{$t('settings.appearance.lang_pt')}</option>
            <option value="zh">{$t('settings.appearance.lang_zh')}</option>
            <option value="ja">{$t('settings.appearance.lang_ja')}</option>
            <option value="it">{$t('settings.appearance.lang_it')}</option>
            <option value="fr">{$t('settings.appearance.lang_fr')}</option>
            <option value="el">{$t('settings.appearance.lang_el')}</option>
          </select>
        </div>
      </div>
    </section>
    {/if}

    {#if activeCategory === "downloads"}
    <section class="section">
      <h5 class="section-title">{$t('settings.download.hotkey_enabled')}</h5>
      <div class="card">
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.download.hotkey_enabled')} <ContextHint text={$t('hints.hotkey')} dismissKey="hotkey" /></span>
            <span class="setting-path">{$t('settings.download.hotkey_enabled_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.download.hotkey_enabled} onclick={() => toggleBool("download", "hotkey_enabled", settings!.download.hotkey_enabled)} role="switch" aria-checked={settings.download.hotkey_enabled}><span class="toggle-knob"></span></button>
        </div>
        {#if settings.download.hotkey_enabled}
          <div class="divider"></div>
          <div class="setting-row hotkey-row">
            <span class="setting-label">{$t('settings.download.hotkey_binding')}</span>
            <div class="hotkey-controls">
              <div class="hotkey-mode-switch">
                <button class="hotkey-mode-btn" class:active={hotkeyMode === 'record'} onclick={() => { hotkeyMode = 'record'; hotkeyRecording = false; }}>{$t('settings.download.hotkey_record')}</button>
                <button class="hotkey-mode-btn" class:active={hotkeyMode === 'type'} onclick={() => { hotkeyMode = 'type'; hotkeyRecording = false; }}>{$t('settings.download.hotkey_type')}</button>
              </div>
              {#if hotkeyMode === 'type'}
                <input type="text" class="input-hotkey" value={hotkeyInput} oninput={handleHotkeyInput} spellcheck="false" />
              {:else}
                <button class="input-hotkey hotkey-record-btn" class:recording={hotkeyRecording} onclick={() => { hotkeyRecording = true; }} onkeydown={hotkeyRecording ? handleHotkeyKeyDown : undefined} onblur={() => { hotkeyRecording = false; }}>
                  {hotkeyRecording ? $t('settings.download.hotkey_press') : (hotkeyInput || $t('settings.download.hotkey_press'))}
                </button>
              {/if}
            </div>
          </div>
          <div class="divider"></div>
          <div class="setting-row">
            <div class="setting-col">
              <span class="setting-label">{$t('settings.download.copy_to_clipboard_on_hotkey')}</span>
              <span class="setting-path">{$t('settings.download.copy_to_clipboard_on_hotkey_desc')}</span>
            </div>
            <button class="toggle" class:on={settings.download.copy_to_clipboard_on_hotkey} onclick={() => toggleBool("download", "copy_to_clipboard_on_hotkey", settings!.download.copy_to_clipboard_on_hotkey)} role="switch" aria-checked={settings.download.copy_to_clipboard_on_hotkey}><span class="toggle-knob"></span></button>
          </div>
        {/if}
      </div>
    </section>

    <section class="section">
      <div class="card">
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.download.default_output_dir')}</span>
            <span class="setting-path">{settings.download.default_output_dir}</span>
          </div>
          <button class="button" onclick={chooseFolder}>{$t('settings.download.choose_folder')}</button>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('settings.download.video_quality')}</span>
          <select class="select" value={settings.download.video_quality} onchange={changeQuality}>
            <option value="best">{$t('omnibox.quality_best')}</option>
            <option value="1080p">{$t('omnibox.quality_1080p')}</option>
            <option value="720p">{$t('omnibox.quality_720p')}</option>
            <option value="480p">{$t('omnibox.quality_480p')}</option>
            <option value="360p">{$t('omnibox.quality_360p')}</option>
          </select>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('settings.download.always_ask_path')}</span>
          <button class="toggle" class:on={settings.download.always_ask_path} onclick={() => toggleBool("download", "always_ask_path", settings!.download.always_ask_path)} role="switch" aria-checked={settings.download.always_ask_path}><span class="toggle-knob"></span></button>
        </div>
      </div>
    </section>

    <details class="section">
      <summary class="section-title">{$t('settings.download.organize_by_platform')}</summary>
      <div class="card">
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.download.organize_by_platform')}</span>
            <span class="setting-path">{$t('settings.download.organize_by_platform_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.download.organize_by_platform} onclick={() => toggleBool("download", "organize_by_platform", settings!.download.organize_by_platform)} role="switch" aria-checked={settings.download.organize_by_platform}><span class="toggle-knob"></span></button>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('settings.download.skip_existing')}</span>
          <button class="toggle" class:on={settings.download.skip_existing} onclick={() => toggleBool("download", "skip_existing", settings!.download.skip_existing)} role="switch" aria-checked={settings.download.skip_existing}><span class="toggle-knob"></span></button>
        </div>
      </div>
    </details>

    <details class="section">
      <summary class="section-title">{$t('settings.download.download_subtitles')}</summary>
      <div class="card">
        <div class="setting-row">
          <span class="setting-label">{$t('settings.download.download_attachments')}</span>
          <button class="toggle" class:on={settings.download.download_attachments} onclick={() => toggleBool("download", "download_attachments", settings!.download.download_attachments)} role="switch" aria-checked={settings.download.download_attachments}><span class="toggle-knob"></span></button>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('settings.download.download_descriptions')}</span>
          <button class="toggle" class:on={settings.download.download_descriptions} onclick={() => toggleBool("download", "download_descriptions", settings!.download.download_descriptions)} role="switch" aria-checked={settings.download.download_descriptions}><span class="toggle-knob"></span></button>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.download.download_subtitles')}</span>
            <span class="setting-path">{$t('settings.download.download_subtitles_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.download.download_subtitles} onclick={() => toggleBool("download", "download_subtitles", settings!.download.download_subtitles)} role="switch" aria-checked={settings.download.download_subtitles}><span class="toggle-knob"></span></button>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.download.embed_metadata')}</span>
            <span class="setting-path">{$t('settings.download.embed_metadata_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.download.embed_metadata} onclick={() => toggleBool("download", "embed_metadata", settings!.download.embed_metadata)} role="switch" aria-checked={settings.download.embed_metadata}><span class="toggle-knob"></span></button>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.download.embed_thumbnail')}</span>
            <span class="setting-path">{$t('settings.download.embed_thumbnail_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.download.embed_thumbnail} onclick={() => toggleBool("download", "embed_thumbnail", settings!.download.embed_thumbnail)} role="switch" aria-checked={settings.download.embed_thumbnail}><span class="toggle-knob"></span></button>
        </div>
      </div>
    </details>

    <details class="section">
      <summary class="section-title">{$t('settings.download.clipboard_detection')}</summary>
      <div class="card">
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.download.clipboard_detection')} <ContextHint text={$t('hints.clipboard')} dismissKey="clipboard" /></span>
            <span class="setting-path">{$t('settings.download.clipboard_detection_desc')}</span>
          </div>
          <button class="toggle" class:on={settings.download.clipboard_detection} onclick={() => toggleBool("download", "clipboard_detection", settings!.download.clipboard_detection)} role="switch" aria-checked={settings.download.clipboard_detection}><span class="toggle-knob"></span></button>
        </div>
      </div>
    </details>

    <details class="section">
      <summary class="section-title">{$t('settings.download.filename_template')}</summary>
      <div class="card">
        <div class="setting-row template-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.download.filename_template')}</span>
            <span class="setting-path">{$t('settings.download.filename_template_desc')}</span>
          </div>
          <input type="text" class="input-template" value={templateInput} oninput={handleTemplateInput} spellcheck="false" />
        </div>
        {#if templateInput}
          <div class="template-preview">
            <span class="setting-path">{$t('settings.download.filename_template_preview', { preview: previewTemplate(templateInput) })}</span>
          </div>
        {/if}
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('common.cookie_file_label')}</span>
            <span class="setting-path">{$t('common.cookie_file_hint')}</span>
          </div>
          <div class="setting-actions">
            {#if settings.download.cookie_file}
              <span class="setting-value">{settings.download.cookie_file.split(/[/\\]/).pop()}</span>
            {/if}
            <button class="button" onclick={chooseCookieFile}>{$t('common.cookie_file_choose')}</button>
          </div>
        </div>
      </div>
    </details>

    {/if}

    {#if activeCategory === "tools"}
    <section class="section">
      <h5 class="section-title">{$t('settings.telegram.title')}</h5>
      <div class="card">
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.telegram.concurrent_downloads')}</span>
            <span class="setting-path">{$t('settings.telegram.concurrent_downloads_desc')}</span>
          </div>
          <input
            type="number"
            class="input-number"
            min="1"
            max="10"
            value={settings.telegram.concurrent_downloads}
            onchange={(e) => changeNumber("telegram", "concurrent_downloads", e)}
          />
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.telegram.fix_file_extensions')}</span>
            <span class="setting-path">{$t('settings.telegram.fix_file_extensions_desc')}</span>
          </div>
          <button
            class="toggle"
            class:on={settings.telegram.fix_file_extensions}
            onclick={() => toggleBool("telegram", "fix_file_extensions", settings!.telegram.fix_file_extensions)}
            role="switch"
            aria-checked={settings.telegram.fix_file_extensions}
            aria-label={$t('settings.telegram.fix_file_extensions')}
          >
            <span class="toggle-knob"></span>
          </button>
        </div>
      </div>
    </section>

    {#if deps.length > 0}
      <section class="section">
        <h5 class="section-title">{$t('settings.dependencies.title')}</h5>
        <div class="card">
          {#each deps as dep, i}
            {#if i > 0}
              <div class="divider"></div>
            {/if}
            <div class="setting-row">
              <div class="setting-col">
                <span class="setting-label">{dep.name}</span>
                {#if dep.installed && dep.version}
                  <span class="setting-path dep-ok">v{dep.version}</span>
                {:else}
                  <span class="setting-path dep-missing">{$t('settings.dependencies.not_found')}</span>
                {/if}
              </div>
              {#if installingDep === dep.name}
                <span class="dep-spinner"></span>
              {:else}
                <button
                  class="button dep-btn"
                  onclick={() => handleInstallDep(dep.name)}
                >
                  {#if dep.installed}
                    {$t('settings.dependencies.update')}
                  {:else}
                    {$t('settings.dependencies.install')}
                  {/if}
                </button>
              {/if}
            </div>
          {/each}
        </div>
      </section>
    {/if}

    {/if}

    {#if activeCategory === "network"}
    <section class="section">
      <h5 class="section-title">{$t('settings.proxy.title')}</h5>
      <div class="card">
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.proxy.enabled')}</span>
          </div>
          <button class="toggle" class:on={settings.proxy?.enabled} onclick={() => toggleBool("proxy", "enabled", settings!.proxy?.enabled ?? false)} role="switch" aria-checked={settings.proxy?.enabled ?? false} aria-label={$t('settings.proxy.enabled')}>
            <span class="toggle-knob"></span>
          </button>
        </div>
        {#if settings.proxy?.enabled}
          <div class="divider"></div>
          <div class="setting-row">
            <span class="setting-label">{$t('settings.proxy.type')}</span>
            <select class="select" value={settings.proxy?.proxy_type ?? 'http'} onchange={changeProxyType}>
              <option value="http">HTTP</option>
              <option value="https">HTTPS</option>
              <option value="socks5">SOCKS5</option>
            </select>
          </div>
          <div class="divider"></div>
          <div class="setting-row">
            <span class="setting-label">{$t('settings.proxy.host')}</span>
            <input type="text" class="input-text" value={proxyHost} oninput={handleProxyHost} placeholder="127.0.0.1" spellcheck="false" />
          </div>
          <div class="divider"></div>
          <div class="setting-row">
            <span class="setting-label">{$t('settings.proxy.port')}</span>
            <input type="number" class="input-number" min="1" max="65535" value={settings.proxy?.port ?? 8080} onchange={(e) => changeNumber("proxy", "port", e)} />
          </div>
          <div class="divider"></div>
          <div class="setting-row">
            <span class="setting-label">{$t('settings.proxy.username')}</span>
            <input type="text" class="input-text" value={proxyUsername} oninput={handleProxyUsername} placeholder="" spellcheck="false" />
          </div>
          <div class="divider"></div>
          <div class="setting-row">
            <span class="setting-label">{$t('settings.proxy.password')}</span>
            <input type="password" class="input-text" value={proxyPassword} oninput={handleProxyPassword} placeholder="" />
          </div>
        {/if}
      </div>
    </section>

    <section class="section">
      <h5 class="section-title">{$t('settings.ytdlp_flags.title')}</h5>
      <div class="card">
        <div class="flag-grid">
          {#each YTDLP_FLAG_CATALOG as item}
            <button
              class="flag-chip"
              class:active={isFlagActive(item.flag)}
              onclick={() => toggleFlag(item.flag)}
              title={item.flag}
            >
              {item.label}
            </button>
            {#if item.hasValue && isFlagActive(item.flag)}
              <input
                class="flag-value-input"
                type="text"
                placeholder={item.placeholder}
                value={getFlagValue(item.flag)}
                oninput={(e) => {
                  const val = (e.target as HTMLInputElement).value;
                  setFlagValue(item.flag, val);
                }}
                spellcheck="false"
              />
            {/if}
          {/each}
        </div>
      </div>
    </section>

    {/if}

    {#if activeCategory === "advanced"}
    <section class="section">
      <h5 class="section-title">{$t('settings.advanced.title')}</h5>
      <div class="card">
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.max_concurrent_downloads')}</span>
            <span class="setting-path">{$t('settings.advanced.max_concurrent_downloads_desc')}</span>
          </div>
          <input
            type="number"
            class="input-number"
            min="1"
            max="10"
            value={settings.advanced.max_concurrent_downloads}
            onchange={(e) => changeNumber("advanced", "max_concurrent_downloads", e)}
          />
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('settings.advanced.max_concurrent_segments')}</span>
          <input
            type="number"
            class="input-number"
            min="1"
            max="100"
            value={settings.advanced.max_concurrent_segments}
            onchange={(e) => changeNumber("advanced", "max_concurrent_segments", e)}
          />
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.concurrent_fragments')}</span>
            <span class="setting-path">{$t('settings.advanced.concurrent_fragments_desc')}</span>
          </div>
          <input
            type="number"
            class="input-number"
            min="1"
            max="32"
            value={settings.advanced.concurrent_fragments}
            onchange={(e) => changeNumber("advanced", "concurrent_fragments", e)}
          />
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('settings.advanced.max_retries')}</span>
          <input
            type="number"
            class="input-number"
            min="1"
            max="20"
            value={settings.advanced.max_retries}
            onchange={(e) => changeNumber("advanced", "max_retries", e)}
          />
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.stagger_delay')}</span>
            <span class="setting-path">{$t('settings.advanced.stagger_delay_desc')}</span>
          </div>
          <input
            type="number"
            class="input-number"
            min="0"
            max="2000"
            step="50"
            value={settings.advanced.stagger_delay_ms}
            onchange={(e) => changeNumber("advanced", "stagger_delay_ms", e)}
          />
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.torrent_listen_port')}</span>
            <span class="setting-path">{$t('settings.advanced.torrent_listen_port_desc')}</span>
          </div>
          <input
            type="number"
            class="input-number"
            min="1024"
            max="65525"
            value={settings.advanced.torrent_listen_port}
            onchange={(e) => changeNumber("advanced", "torrent_listen_port", e)}
          />
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.advanced.cookies_from_browser')}</span>
            <span class="setting-path">{$t('settings.advanced.cookies_from_browser_desc')}</span>
          </div>
          <input
            type="text"
            class="input-text"
            placeholder="e.g., firefox, chrome, edge"
            value={settings.advanced?.cookies_from_browser ?? ""}
            onchange={(e) => updateSettings({ advanced: { cookies_from_browser: (e.target as HTMLInputElement).value.trim() } })}
          />
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('debug.enable')}</span>
            <span class="setting-path">{$t('debug.enable_desc')}</span>
          </div>
          <button
            class="toggle"
            class:on={isDebugEnabled()}
            onclick={() => setDebugEnabled(!isDebugEnabled())}
            role="switch"
            aria-checked={isDebugEnabled()}
            aria-label={$t('debug.enable')}
          >
            <span class="toggle-knob"></span>
          </button>
        </div>
        {#if isDebugEnabled()}
          <div class="divider"></div>
          <div class="setting-row">
            <span class="setting-label">{$t('debug.open_panel')}</span>
            <button class="button" onclick={() => setDebugPanelOpen(true)}>
              {$t('debug.open_panel')}
            </button>
          </div>
        {/if}
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('settings.advanced.reset')}</span>
          <button class="button reset-btn" onclick={handleReset} disabled={resetting}>
            {$t('settings.advanced.reset')}
          </button>
        </div>
      </div>
    </section>
    {/if}
  </div>
{:else}
  <div class="settings-loading">
    <span class="spinner"></span>
  </div>
{/if}

<style>
  .settings {
    display: flex;
    flex-direction: column;
    align-items: center;
    min-height: calc(100vh - var(--padding) * 4);
    padding-top: var(--padding);
    gap: var(--padding);
  }

  .settings > :global(*) {
    width: 100%;
    max-width: 560px;
  }

  .settings-loading {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: calc(100vh - var(--padding) * 4);
  }

  .spinner {
    width: 24px;
    height: 24px;
    border: 2px solid var(--input-border);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: calc(var(--padding) / 2);
  }

  .section-title {
    color: var(--gray);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .card {
    background: var(--button);
    box-shadow: var(--button-box-shadow);
    border-radius: var(--border-radius);
    padding: 0 calc(var(--padding) + 4px);
  }

  .setting-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--padding);
    padding: var(--padding) 0;
    min-height: 40px;
  }

  .setting-col {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    flex: 1;
  }

  .setting-label {
    font-size: 14.5px;
    font-weight: 500;
    color: var(--secondary);
  }

  .setting-path {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .divider {
    height: 1px;
    background: var(--button-stroke);
  }

  .select {
    padding: calc(var(--padding) / 2) 28px calc(var(--padding) / 2) var(--padding);
    font-size: 14.5px;
    font-weight: 500;
    background: var(--button-elevated);
    border-radius: calc(var(--border-radius) / 2);
    color: var(--secondary);
    border: none;
    cursor: pointer;
    flex-shrink: 0;
    appearance: none;
    background-image: url('data:image/svg+xml;utf8,<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="6 9 12 15 18 9"></polyline></svg>');
    background-repeat: no-repeat;
    background-position: right 8px center;
    background-size: 14px;
  }

  .select:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .input-number {
    width: 72px;
    padding: calc(var(--padding) / 2);
    font-size: 14.5px;
    font-weight: 500;
    text-align: center;
    background: var(--button-elevated);
    border-radius: calc(var(--border-radius) / 2);
    color: var(--secondary);
    border: 1px solid var(--input-border);
    font-variant-numeric: tabular-nums;
  }

  .input-number:focus-visible {
    border-color: var(--blue);
    outline: none;
  }

  .input-hotkey {
    width: 180px;
    padding: calc(var(--padding) / 2);
    font-size: 12.5px;
    font-weight: 500;
    background: var(--button-elevated);
    border-radius: calc(var(--border-radius) / 2);
    color: var(--secondary);
    border: 1px solid var(--input-border);
    text-align: center;
    flex-shrink: 0;
  }

  .input-hotkey:focus-visible {
    border-color: var(--blue);
    outline: none;
  }

  .hotkey-row {
    flex-wrap: wrap;
  }

  .hotkey-controls {
    display: flex;
    flex-direction: column;
    gap: 6px;
    align-items: flex-end;
  }

  .hotkey-mode-switch {
    display: flex;
    border-radius: calc(var(--border-radius) / 2);
    overflow: hidden;
  }

  .hotkey-mode-btn {
    padding: 3px 10px;
    font-size: 12px;
    font-weight: 500;
    background: transparent;
    color: var(--gray);
    border: 1px solid var(--input-border);
    cursor: pointer;
  }

  .hotkey-mode-btn:first-child {
    border-radius: calc(var(--border-radius) / 2) 0 0 calc(var(--border-radius) / 2);
    border-right: none;
  }

  .hotkey-mode-btn:last-child {
    border-radius: 0 calc(var(--border-radius) / 2) calc(var(--border-radius) / 2) 0;
  }

  .hotkey-mode-btn.active {
    background: var(--button-elevated);
    color: var(--secondary);
  }

  .hotkey-mode-btn:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .hotkey-record-btn {
    cursor: pointer;
    user-select: none;
  }

  .hotkey-record-btn.recording {
    border-color: var(--blue);
    animation: hotkey-pulse 1.5s ease-in-out infinite;
  }

  @keyframes hotkey-pulse {
    0%, 100% { border-color: var(--blue); }
    50% { border-color: var(--input-border); }
  }

  .template-row {
    flex-direction: column;
    align-items: stretch;
    gap: calc(var(--padding) / 2);
  }

  .input-template {
    width: 100%;
    padding: calc(var(--padding) / 2);
    font-size: 12.5px;
    font-weight: 500;
    background: var(--button-elevated);
    border-radius: calc(var(--border-radius) / 2);
    color: var(--secondary);
    border: 1px solid var(--input-border);
  }

  .input-template:focus-visible {
    border-color: var(--blue);
    outline: none;
  }

  .template-preview {
    padding: 0 0 calc(var(--padding) + 2px);
  }

  .template-preview .setting-path {
    word-break: break-all;
  }

  .toggle {
    position: relative;
    width: 44px;
    height: 26px;
    border-radius: 13px;
    background: var(--button-elevated);
    border: none;
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
    transition: background 0.2s;
  }

  .toggle.on {
    background: var(--blue);
  }

  .toggle-knob {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: #fff;
    transition: transform 0.2s cubic-bezier(0.53, 0.05, 0.02, 1.2);
    pointer-events: none;
  }

  .toggle.on .toggle-knob {
    transform: translateX(18px);
  }

  .toggle:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .reset-btn {
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 12.5px;
    color: var(--red);
  }

  .update-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--padding);
    padding: var(--padding);
    background: var(--blue);
    border-radius: var(--border-radius);
  }

  .update-text {
    font-size: 14.5px;
    font-weight: 500;
    color: #fff;
  }

  .update-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: calc(var(--padding) / 2);
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 12.5px;
    font-weight: 500;
    background: #fff;
    color: var(--blue);
    border: none;
    border-radius: calc(var(--border-radius) - 2px);
    cursor: pointer;
    flex-shrink: 0;
  }

  .update-btn:disabled {
    cursor: default;
    opacity: 0.7;
  }

  @media (hover: hover) {
    .update-btn:not(:disabled):hover {
      opacity: 0.9;
    }
  }

  .update-btn:focus-visible {
    outline: 2px solid #fff;
    outline-offset: 2px;
  }

  .update-spinner {
    width: 14px;
    height: 14px;
    border: 2px solid var(--content-border);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }

  .dep-ok {
    color: var(--green);
  }

  .dep-missing {
    color: var(--red);
  }

  .dep-btn {
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 12.5px;
    flex-shrink: 0;
  }

  .dep-spinner {
    width: 18px;
    height: 18px;
    border: 2px solid var(--input-border);
    border-top-color: var(--blue);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
    flex-shrink: 0;
  }

  .input-text {
    flex: 1;
    min-width: 120px;
    max-width: 200px;
    padding: calc(var(--padding) / 2);
    font-size: 13px;
    font-weight: 500;
    background: var(--button-elevated);
    border-radius: calc(var(--border-radius) / 2);
    color: var(--secondary);
    border: 1px solid var(--input-border);
  }
  .input-text:focus-visible {
    border-color: var(--blue);
    outline: none;
  }

  .flag-grid {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: calc(var(--padding) + 2px);
  }
  .flag-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 10px;
    font-size: 12px;
    font-weight: 500;
    border-radius: 6px;
    background: var(--button-elevated);
    color: var(--gray);
    cursor: pointer;
    border: 1px solid transparent;
    transition: all 0.15s;
    user-select: none;
  }
  @media (hover: hover) {
    .flag-chip:hover { opacity: 0.85; }
  }
  .flag-chip.active {
    background: var(--accent);
    color: var(--on-accent);
    border-color: var(--accent);
  }
  .flag-value-input {
    width: 70px;
    padding: 2px 6px;
    font-size: 11px;
    background: var(--button-elevated);
    color: var(--secondary);
    border: 1px solid var(--input-border);
    border-radius: 4px;
  }
  .flag-value-input:focus-visible {
    border-color: var(--blue);
    outline: none;
  }


  .category-tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 3px;
    background: var(--button);
    border-radius: var(--border-radius);
  }

  .cat-tab {
    padding: 6px 12px;
    font-size: 12px;
    font-weight: 500;
    color: var(--gray);
    background: none;
    border: none;
    border-radius: calc(var(--border-radius) - 3px);
    cursor: pointer;
    white-space: nowrap;
  }

  .cat-tab.active {
    background: var(--button-elevated);
    color: var(--secondary);
  }

  @media (hover: hover) {
    .cat-tab:not(.active):hover {
      color: var(--secondary);
    }
  }

  .cat-tab:focus-visible {
    outline: var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
  }

  .theme-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(100px, 1fr));
    gap: 8px;
    padding: 0 var(--padding) var(--padding);
  }

  .theme-card {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 8px;
    border: 2px solid transparent;
    border-radius: var(--border-radius);
    background: var(--button);
    cursor: pointer;
    transition: border-color 0.15s;
  }

  .theme-card:hover {
    background: var(--button-hover);
  }

  .theme-card.active {
    border-color: var(--accent);
  }

  .theme-preview {
    width: 100%;
    aspect-ratio: 16/10;
    border-radius: calc(var(--border-radius) / 2);
    overflow: hidden;
  }

  .preview-bg {
    width: 100%;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    position: relative;
  }

  .preview-text {
    font-size: 16px;
    font-weight: 600;
  }

  .preview-accent {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    height: 4px;
  }

  .system-preview {
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--button-elevated);
  }

  .theme-name {
    font-size: 11px;
    color: var(--tertiary);
    text-align: center;
  }
</style>
