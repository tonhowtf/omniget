<script lang="ts">
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings, toggleBool } from "../settings-helpers";

  let { embedded = false }: { embedded?: boolean } = $props();

  let settings = $derived(getSettings());

  function changeCaptionLocale(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    updateSettings({ download: { caption_locale: value } });
  }
</script>

{#if settings}
  {#if !embedded}
    <div class="settings-section-head section-title">
      <h5 class="section-title">{$t('settings.download.section_subtitles')}</h5>
      <p class="settings-section-hint">{$t('settings.download.section_subtitles_desc')}</p>
    </div>
  {/if}

  <div class="card">
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t('settings.download.download_subtitles')}</span>
        <span class="setting-path">{$t('settings.download.download_subtitles_desc')}</span>
      </div>
      <button class="toggle" class:on={settings.download.download_subtitles} onclick={() => toggleBool("download", "download_subtitles", settings.download.download_subtitles)} role="switch" aria-checked={settings.download.download_subtitles} aria-label={$t('settings.download.download_subtitles') as string}><span class="toggle-knob"></span></button>
    </div>
    {#if settings.download.download_subtitles}
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.download.include_auto_subtitles')}</span>
          <span class="setting-path">{$t('settings.download.include_auto_subtitles_desc')}</span>
        </div>
        <button class="toggle" class:on={settings.download.include_auto_subtitles} onclick={() => toggleBool("download", "include_auto_subtitles", settings.download.include_auto_subtitles)} role="switch" aria-checked={settings.download.include_auto_subtitles} aria-label={$t('settings.download.include_auto_subtitles') as string}><span class="toggle-knob"></span></button>
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.download.caption_locale')}</span>
          <span class="setting-path">{$t('settings.download.caption_locale_desc')}</span>
        </div>
        <select class="select" value={settings.download.caption_locale} onchange={changeCaptionLocale}>
          <option value="en">English</option>
          <option value="pt">Português</option>
          <option value="es">Español</option>
          <option value="fr">Français</option>
          <option value="it">Italiano</option>
          <option value="ja">日本語</option>
          <option value="zh-Hans">简体中文</option>
          <option value="zh-Hant">繁體中文</option>
          <option value="el">Ελληνικά</option>
        </select>
      </div>
      <div class="divider"></div>
      <div class="setting-row">
        <div class="setting-col">
          <span class="setting-label">{$t('settings.download.keep_vtt')}</span>
          <span class="setting-path">{$t('settings.download.keep_vtt_desc')}</span>
        </div>
        <button class="toggle" class:on={settings.download.keep_vtt} onclick={() => toggleBool("download", "keep_vtt", settings.download.keep_vtt)} role="switch" aria-checked={settings.download.keep_vtt} aria-label={$t('settings.download.keep_vtt') as string}><span class="toggle-knob"></span></button>
      </div>
    {/if}
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t('settings.download.translate_metadata')}</span>
        <span class="setting-path">{$t('settings.download.translate_metadata_desc')}</span>
      </div>
      <button class="toggle" class:on={settings.download.translate_metadata} onclick={() => toggleBool("download", "translate_metadata", settings.download.translate_metadata)} role="switch" aria-checked={settings.download.translate_metadata} aria-label={$t('settings.download.translate_metadata') as string}><span class="toggle-knob"></span></button>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t('settings.download.embed_metadata')}</span>
        <span class="setting-path">{$t('settings.download.embed_metadata_desc')}</span>
      </div>
      <button class="toggle" class:on={settings.download.embed_metadata} onclick={() => toggleBool("download", "embed_metadata", settings.download.embed_metadata)} role="switch" aria-checked={settings.download.embed_metadata} aria-label={$t('settings.download.embed_metadata') as string}><span class="toggle-knob"></span></button>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t('settings.download.embed_thumbnail')}</span>
        <span class="setting-path">{$t('settings.download.embed_thumbnail_desc')}</span>
      </div>
      <button class="toggle" class:on={settings.download.embed_thumbnail} onclick={() => toggleBool("download", "embed_thumbnail", settings.download.embed_thumbnail)} role="switch" aria-checked={settings.download.embed_thumbnail} aria-label={$t('settings.download.embed_thumbnail') as string}><span class="toggle-knob"></span></button>
    </div>
  </div>
{/if}
