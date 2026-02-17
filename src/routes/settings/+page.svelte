<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open } from "@tauri-apps/plugin-dialog";
  import { t, locale, loadTranslations } from "$lib/i18n";
  import { getSettings, updateSettings, resetSettings, loadSettings } from "$lib/stores/settings-store.svelte";
  import { getUpdateInfo } from "$lib/stores/update-store.svelte";
  import { installUpdate } from "$lib/updater";
  import { showToast } from "$lib/stores/toast-store.svelte";

  type DependencyStatus = {
    name: string;
    installed: boolean;
    version: string | null;
  };

  let settings = $derived(getSettings());
  let updateInfo = $derived(getUpdateInfo());
  let resetting = $state(false);
  let updating = $state(false);
  let deps = $state<DependencyStatus[]>([]);
  let installingDep = $state<string | null>(null);

  async function loadDeps() {
    try {
      deps = await invoke<DependencyStatus[]>("check_dependencies");
    } catch {}
  }

  async function handleInstallDep(name: string) {
    installingDep = name;
    try {
      await invoke("install_dependency", { name });
      await loadDeps();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : e.message ?? "Error");
    } finally {
      installingDep = null;
    }
  }

  $effect(() => {
    if (settings) loadDeps();
  });

  async function handleUpdate() {
    updating = true;
    try {
      await installUpdate();
    } catch {
      updating = false;
    }
  }

  async function changeTheme(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
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

  let templateInput = $state("");
  let templateTimer = $state<ReturnType<typeof setTimeout> | null>(null);

  $effect(() => {
    if (settings) {
      templateInput = settings.download.filename_template;
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

  async function handleReset() {
    if (!confirm($t("settings.advanced.reset_confirm"))) return;
    resetting = true;
    try {
      await resetSettings();
    } catch (e: any) {
      showToast("error", typeof e === "string" ? e : e.message ?? "Error");
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

    <section class="section">
      <h5 class="section-title">{$t('settings.appearance.title')}</h5>
      <div class="card">
        <div class="setting-row">
          <span class="setting-label">{$t('settings.appearance.theme')}</span>
          <select class="select" value={settings.appearance.theme} onchange={changeTheme}>
            <option value="system">{$t('settings.appearance.theme_system')}</option>
            <option value="light">{$t('settings.appearance.theme_light')}</option>
            <option value="dark">{$t('settings.appearance.theme_dark')}</option>
          </select>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('settings.appearance.language')}</span>
          <select class="select" value={settings.appearance.language} onchange={changeLanguage}>
            <option value="pt">{$t('settings.appearance.lang_pt')}</option>
            <option value="en">{$t('settings.appearance.lang_en')}</option>
          </select>
        </div>
      </div>
    </section>

    <section class="section">
      <h5 class="section-title">{$t('settings.download.title')}</h5>
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
          <span class="setting-label">{$t('settings.download.always_ask_path')}</span>
          <button
            class="toggle"
            class:on={settings.download.always_ask_path}
            onclick={() => toggleBool("download", "always_ask_path", settings!.download.always_ask_path)}
            role="switch"
            aria-checked={settings.download.always_ask_path}
            aria-label={$t('settings.download.always_ask_path')}
          >
            <span class="toggle-knob"></span>
          </button>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.download.organize_by_platform')}</span>
            <span class="setting-path">{$t('settings.download.organize_by_platform_desc')}</span>
          </div>
          <button
            class="toggle"
            class:on={settings.download.organize_by_platform}
            onclick={() => toggleBool("download", "organize_by_platform", settings!.download.organize_by_platform)}
            role="switch"
            aria-checked={settings.download.organize_by_platform}
            aria-label={$t('settings.download.organize_by_platform')}
          >
            <span class="toggle-knob"></span>
          </button>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('settings.download.video_quality')}</span>
          <select class="select" value={settings.download.video_quality} onchange={changeQuality}>
            <option value="360p">360p</option>
            <option value="480p">480p</option>
            <option value="720p">720p</option>
            <option value="1080p">1080p</option>
            <option value="best">Best</option>
          </select>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('settings.download.skip_existing')}</span>
          <button
            class="toggle"
            class:on={settings.download.skip_existing}
            onclick={() => toggleBool("download", "skip_existing", settings!.download.skip_existing)}
            role="switch"
            aria-checked={settings.download.skip_existing}
            aria-label={$t('settings.download.skip_existing')}
          >
            <span class="toggle-knob"></span>
          </button>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('settings.download.download_attachments')}</span>
          <button
            class="toggle"
            class:on={settings.download.download_attachments}
            onclick={() => toggleBool("download", "download_attachments", settings!.download.download_attachments)}
            role="switch"
            aria-checked={settings.download.download_attachments}
            aria-label={$t('settings.download.download_attachments')}
          >
            <span class="toggle-knob"></span>
          </button>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <span class="setting-label">{$t('settings.download.download_descriptions')}</span>
          <button
            class="toggle"
            class:on={settings.download.download_descriptions}
            onclick={() => toggleBool("download", "download_descriptions", settings!.download.download_descriptions)}
            role="switch"
            aria-checked={settings.download.download_descriptions}
            aria-label={$t('settings.download.download_descriptions')}
          >
            <span class="toggle-knob"></span>
          </button>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.download.embed_metadata')}</span>
            <span class="setting-path">{$t('settings.download.embed_metadata_desc')}</span>
          </div>
          <button
            class="toggle"
            class:on={settings.download.embed_metadata}
            onclick={() => toggleBool("download", "embed_metadata", settings!.download.embed_metadata)}
            role="switch"
            aria-checked={settings.download.embed_metadata}
            aria-label={$t('settings.download.embed_metadata')}
          >
            <span class="toggle-knob"></span>
          </button>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.download.embed_thumbnail')}</span>
            <span class="setting-path">{$t('settings.download.embed_thumbnail_desc')}</span>
          </div>
          <button
            class="toggle"
            class:on={settings.download.embed_thumbnail}
            onclick={() => toggleBool("download", "embed_thumbnail", settings!.download.embed_thumbnail)}
            role="switch"
            aria-checked={settings.download.embed_thumbnail}
            aria-label={$t('settings.download.embed_thumbnail')}
          >
            <span class="toggle-knob"></span>
          </button>
        </div>
        <div class="divider"></div>
        <div class="setting-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.download.clipboard_detection')}</span>
            <span class="setting-path">{$t('settings.download.clipboard_detection_desc')}</span>
          </div>
          <button
            class="toggle"
            class:on={settings.download.clipboard_detection}
            onclick={() => toggleBool("download", "clipboard_detection", settings!.download.clipboard_detection)}
            role="switch"
            aria-checked={settings.download.clipboard_detection}
            aria-label={$t('settings.download.clipboard_detection')}
          >
            <span class="toggle-knob"></span>
          </button>
        </div>
        <div class="divider"></div>
        <div class="setting-row template-row">
          <div class="setting-col">
            <span class="setting-label">{$t('settings.download.filename_template')}</span>
            <span class="setting-path">{$t('settings.download.filename_template_desc')}</span>
          </div>
          <input
            type="text"
            class="input-template"
            value={templateInput}
            oninput={handleTemplateInput}
            spellcheck="false"
          />
        </div>
        {#if templateInput}
          <div class="template-preview">
            <span class="setting-path">{$t('settings.download.filename_template_preview', { preview: previewTemplate(templateInput) })}</span>
          </div>
        {/if}
      </div>
    </section>

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
          <span class="setting-label">{$t('settings.advanced.reset')}</span>
          <button class="button reset-btn" onclick={handleReset} disabled={resetting}>
            {$t('settings.advanced.reset')}
          </button>
        </div>
      </div>
    </section>
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
    padding-top: calc(var(--padding) * 2);
    gap: calc(var(--padding) * 1.5);
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
    padding: calc(var(--padding) + 2px) 0;
    min-height: 48px;
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
    padding: calc(var(--padding) / 2) var(--padding);
    font-size: 14.5px;
    font-weight: 500;
    background: var(--button-elevated);
    border-radius: calc(var(--border-radius) / 2);
    color: var(--secondary);
    border: none;
    cursor: pointer;
    flex-shrink: 0;
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
    border: 2px solid rgba(47, 138, 249, 0.3);
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
</style>
