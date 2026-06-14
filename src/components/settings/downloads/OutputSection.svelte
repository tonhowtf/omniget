<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import {
    getSettings,
    updateSettings,
    chooseFolder,
    toggleBool,
  } from "../settings-helpers";

  let { embedded = false }: { embedded?: boolean } = $props();

  let settings = $derived(getSettings());

  type PathLimitInfo = { limit: number; current: number; reserve: number; ok: boolean };
  let pathInfo = $state<PathLimitInfo | null>(null);

  $effect(() => {
    const dir = settings?.download.default_output_dir;
    if (!dir) {
      pathInfo = null;
      return;
    }
    let cancelled = false;
    invoke<PathLimitInfo>("validate_output_path", { outputDir: dir })
      .then((info) => {
        if (!cancelled) pathInfo = info;
      })
      .catch(() => {
        if (!cancelled) pathInfo = null;
      });
    return () => {
      cancelled = true;
    };
  });

  let templateInput = $state("");
  let templateTimer: ReturnType<typeof setTimeout> | null = null;

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
</script>

{#if settings}
  {#if !embedded}
    <div class="settings-section-head section-title">
      <h5 class="section-title">{$t('settings.download.section_output')}</h5>
      <p class="settings-section-hint">{$t('settings.download.section_output_desc')}</p>
    </div>
  {/if}

  <div class="card">
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t('settings.download.default_output_dir')}</span>
        <span class="setting-path">{settings.download.default_output_dir}</span>
        {#if pathInfo && !pathInfo.ok}
          <span class="path-warning" role="status">
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
              <path d="M10.29 3.86 1.82 18a2 2 0 0 0 1.71 3h16.94a2 2 0 0 0 1.71-3L13.71 3.86a2 2 0 0 0-3.42 0Z" />
              <line x1="12" y1="9" x2="12" y2="13" />
              <line x1="12" y1="17" x2="12.01" y2="17" />
            </svg>
            {$t('settings.download.path_too_long', { current: pathInfo.current, limit: pathInfo.limit })}
          </span>
        {/if}
      </div>
      <button class="button" onclick={chooseFolder}>{$t('settings.download.choose_folder')}</button>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t('settings.download.organize_by_platform')}</span>
        <span class="setting-path">{$t('settings.download.organize_by_platform_desc')}</span>
      </div>
      <button class="toggle" class:on={settings.download.organize_by_platform} onclick={() => toggleBool("download", "organize_by_platform", settings.download.organize_by_platform)} role="switch" aria-checked={settings.download.organize_by_platform} aria-label={$t('settings.download.organize_by_platform') as string}><span class="toggle-knob"></span></button>
    </div>

    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t('settings.download.skip_existing')}</span>
        <span class="setting-path">{$t('settings.download.skip_existing_desc')}</span>
      </div>
      <button class="toggle" class:on={settings.download.skip_existing} onclick={() => toggleBool("download", "skip_existing", settings.download.skip_existing)} role="switch" aria-checked={settings.download.skip_existing} aria-label={$t('settings.download.skip_existing') as string}><span class="toggle-knob"></span></button>
    </div>
    <div class="divider"></div>
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
  </div>
{/if}
