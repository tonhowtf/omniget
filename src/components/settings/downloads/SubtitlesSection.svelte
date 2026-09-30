<script lang="ts">
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings, toggleBool } from "../settings-helpers";
  import SettingGroup from "../rebrand/SettingGroup.svelte";
  import SettingRow from "../rebrand/SettingRow.svelte";

  type ControlArgs = { id: string; describedBy: string | undefined };

  let settings = $derived(getSettings());

  function changeCaptionLocale(e: Event) {
    const value = (e.target as HTMLSelectElement).value;
    updateSettings({ download: { caption_locale: value } });
  }
</script>

{#snippet sw(c: ControlArgs, label: string, on: boolean, onclick: () => void)}
  <button type="button" class="toggle" class:on id={c.id} aria-label={label} aria-describedby={c.describedBy} role="switch" aria-checked={on} {onclick}><span class="toggle-knob"></span></button>
{/snippet}

{#if settings}
  {@const d = settings.download}
  <SettingGroup id="subtitles" title={$t("settings.rebrand.downloads.subtitles")}>
    <SettingRow id="dl-download-subtitles" label={$t("settings.download.download_subtitles")} help={$t("settings.download.download_subtitles_desc")} settingKey="settings.download.download_subtitles" status="download.download_subtitles">
      {#snippet control(c)}{@render sw(c, $t("settings.download.download_subtitles"), d.download_subtitles, () => toggleBool("download", "download_subtitles", d.download_subtitles))}{/snippet}
    </SettingRow>
    {#if d.download_subtitles}
      <SettingRow id="dl-include-auto-subtitles" label={$t("settings.download.include_auto_subtitles")} help={$t("settings.download.include_auto_subtitles_desc")} settingKey="settings.download.include_auto_subtitles" status="download.include_auto_subtitles">
        {#snippet control(c)}{@render sw(c, $t("settings.download.include_auto_subtitles"), d.include_auto_subtitles, () => toggleBool("download", "include_auto_subtitles", d.include_auto_subtitles))}{/snippet}
      </SettingRow>
      <SettingRow id="dl-caption-locale" label={$t("settings.download.caption_locale")} help={$t("settings.download.caption_locale_desc")} settingKey="settings.download.caption_locale" status="download.caption_locale">
        {#snippet control(c)}
          <select class="select" id={c.id} aria-describedby={c.describedBy} value={d.caption_locale} onchange={changeCaptionLocale}>
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
        {/snippet}
      </SettingRow>
      <SettingRow id="dl-keep-vtt" label={$t("settings.download.keep_vtt")} help={$t("settings.download.keep_vtt_desc")} settingKey="settings.download.keep_vtt" status="download.keep_vtt">
        {#snippet control(c)}{@render sw(c, $t("settings.download.keep_vtt"), d.keep_vtt, () => toggleBool("download", "keep_vtt", d.keep_vtt))}{/snippet}
      </SettingRow>
    {/if}
    <SettingRow id="dl-translate-metadata" label={$t("settings.download.translate_metadata")} help={$t("settings.download.translate_metadata_desc")} settingKey="settings.download.translate_metadata" status="download.translate_metadata">
      {#snippet control(c)}{@render sw(c, $t("settings.download.translate_metadata"), d.translate_metadata, () => toggleBool("download", "translate_metadata", d.translate_metadata))}{/snippet}
    </SettingRow>
  </SettingGroup>
{/if}
