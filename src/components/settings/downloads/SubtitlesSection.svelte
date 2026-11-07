<script lang="ts">
  import { t } from "$lib/i18n";
  import { getSettings, saveSettings, toggleBool } from "../settings-helpers";
  import SettingGroup from "../rebrand/SettingGroup.svelte";
  import SettingRow from "../rebrand/SettingRow.svelte";
  import MoreOptions from "./MoreOptions.svelte";
  import Toggle from "./Toggle.svelte";
  import KeyAnchor from "./KeyAnchor.svelte";
  import { captionLocaleValue } from "./download-options";

  const LANGUAGES = [
    ["en", "settings.appearance.lang_en"], ["pt", "settings.appearance.lang_pt"], ["es", "settings.appearance.lang_es"],
    ["fr", "settings.appearance.lang_fr"], ["it", "settings.appearance.lang_it"], ["ja", "settings.appearance.lang_ja"],
    ["zh-Hans", "settings.appearance.lang_zh"], ["zh-Hant", "settings.appearance.lang_zh_tw"], ["el", "settings.appearance.lang_el"],
  ] as const;

  const SUBTITLE_CHILD_KEYS = ["settings.download.caption_locale", "settings.download.include_auto_subtitles", "settings.download.keep_vtt"];

  let settings = $derived(getSettings());
</script>

{#if settings}
  {@const d = settings.download}
  <SettingGroup id="subtitles" icon="captions" title={$t("settings.downloads.subtitles.title")}>
    <SettingRow id="dl-download-subtitles" label={$t("settings.download.download_subtitles")} help={$t("settings.download.download_subtitles_desc")} settingKey="settings.download.download_subtitles" status="download.download_subtitles">
      {#snippet control(c)}<KeyAnchor keys={SUBTITLE_CHILD_KEYS} active={!d.download_subtitles}><Toggle id={c.id} describedBy={c.describedBy} on={d.download_subtitles} onclick={() => toggleBool("download", "download_subtitles", d.download_subtitles)} /></KeyAnchor>{/snippet}
    </SettingRow>
    {#if d.download_subtitles}
      <SettingRow id="dl-caption-locale" label={$t("settings.download.caption_locale")} help={$t("settings.download.caption_locale_desc")} settingKey="settings.download.caption_locale" status="download.caption_locale">
        {#snippet control(c)}
          <select class="select" id={c.id} aria-describedby={c.describedBy} value={captionLocaleValue(d.caption_locale)} onchange={(e) => saveSettings({ download: { caption_locale: e.currentTarget.value } })}>
            <option value="auto">{$t("settings.downloads.subtitles.same_as_app")}</option>
            {#each LANGUAGES as [code, key] (code)}
              <option value={code}>{$t(key)}</option>
            {/each}
          </select>
        {/snippet}
      </SettingRow>
      <SettingRow id="dl-include-auto-subtitles" label={$t("settings.download.include_auto_subtitles")} help={$t("settings.download.include_auto_subtitles_desc")} settingKey="settings.download.include_auto_subtitles" status="download.include_auto_subtitles">
        {#snippet control(c)}<Toggle id={c.id} describedBy={c.describedBy} on={d.include_auto_subtitles} onclick={() => toggleBool("download", "include_auto_subtitles", d.include_auto_subtitles)} />{/snippet}
      </SettingRow>
      <MoreOptions id="subtitles">
        <SettingRow id="dl-keep-vtt" label={$t("settings.download.keep_vtt")} help={$t("settings.download.keep_vtt_desc")} settingKey="settings.download.keep_vtt" status="download.keep_vtt">
          {#snippet control(c)}<Toggle id={c.id} describedBy={c.describedBy} on={d.keep_vtt} onclick={() => toggleBool("download", "keep_vtt", d.keep_vtt)} />{/snippet}
        </SettingRow>
      </MoreOptions>
    {/if}
  </SettingGroup>
{/if}
