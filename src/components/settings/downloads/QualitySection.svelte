<script lang="ts">
  import { t } from "$lib/i18n";
  import { getSettings, saveSettings, toggleBool } from "../settings-helpers";
  import SettingGroup from "../rebrand/SettingGroup.svelte";
  import SettingRow from "../rebrand/SettingRow.svelte";
  import SettingStatus from "../rebrand/SettingStatus.svelte";
  import GeneralChoice from "../rebrand/GeneralChoice.svelte";
  import SettingsIcon from "../rebrand/SettingsIcon.svelte";
  import MoreOptions from "./MoreOptions.svelte";
  import Toggle from "./Toggle.svelte";
  import { LOWER_QUALITIES, customQuality, lowerQuality, qualityCard } from "./download-options";

  let settings = $derived(getSettings());
  let moreOpen = $state(false);
  let lower = $derived(lowerQuality(settings?.download.video_quality));
  let custom = $derived(customQuality(settings?.download.video_quality));

  $effect(() => {
    if (lower) moreOpen = true;
  });

  function setQuality(value: string) {
    if (!value) return;
    void saveSettings({ download: { video_quality: value } });
  }
</script>

{#snippet screen(bars: number)}
  <rect class="art-screen" x="14" y="7" width="52" height="32" rx="3" />
  <path class="art-stand" d="M34 39v4M28 44h24" />
  {#each Array(bars) as _, i (i)}
    <rect class="art-bar" x={20 + i * 11} y={30 - i * 6} width="7" height={4 + i * 6} rx="1" />
  {/each}
{/snippet}

{#snippet bestArt()}{@render screen(4)}{/snippet}
{#snippet fullHdArt()}{@render screen(3)}{/snippet}
{#snippet hdArt()}{@render screen(2)}{/snippet}

{#if settings}
  {@const d = settings.download}
  <SettingGroup id="quality" icon="gauge" title={$t("settings.rebrand.downloads.quality")}>
    <div class="choice-block" data-setting-key="settings.download.video_quality">
      <p class="choice-title">{$t("settings.download.video_quality")}</p>
      <p class="choice-help">{$t("settings.download.video_quality_desc")}</p>
      <GeneralChoice
        name="dl-video-quality"
        label={$t("settings.download.video_quality") as string}
        value={qualityCard(d.video_quality)}
        options={[
          { value: "best", draw: bestArt, label: $t("omnibox.quality_best") as string, help: $t("settings.downloads.quality.best_desc") as string },
          { value: "1080p", draw: fullHdArt, label: $t("omnibox.quality_1080p") as string, help: $t("settings.downloads.quality.full_hd_desc") as string },
          { value: "720p", draw: hdArt, label: $t("omnibox.quality_720p") as string, help: $t("settings.downloads.quality.hd_desc") as string },
        ]}
        onchange={setQuality}
      />
      {#if custom}
        <p class="choice-current" role="status"><span class="current-mark" aria-hidden="true"><SettingsIcon name="alert" size={18} /></span>{$t("settings.downloads.quality.custom_current", { value: custom })}</p>
      {/if}
      <SettingStatus path="download.video_quality" />
    </div>
    <SettingRow id="dl-audio-format" label={$t("settings.downloads.quality.audio_format")} help={$t("settings.downloads.quality.audio_format_desc")} settingKey="settings.downloads.quality.audio_format" status="download.music_audio_format">
      {#snippet control(c)}
        <select class="select" id={c.id} aria-describedby={c.describedBy} value={d.music_audio_format} onchange={(e) => saveSettings({ download: { music_audio_format: e.currentTarget.value } })}>
          <option value="m4a">M4A (AAC)</option>
          <option value="mp3">MP3</option>
          <option value="flac">FLAC ({$t("settings.downloads.quality.lossless")})</option>
          <option value="opus">Opus</option>
          <option value="wav">WAV</option>
        </select>
      {/snippet}
    </SettingRow>

    <MoreOptions id="quality" bind:open={moreOpen}>
      <SettingRow id="dl-lower-quality" label={$t("settings.downloads.quality.lower")} help={$t("settings.downloads.quality.lower_desc")} settingKey="settings.downloads.quality.lower">
        {#snippet control(c)}
          <select class="select" id={c.id} aria-describedby={c.describedBy} value={lower} onchange={(e) => setQuality(e.currentTarget.value)}>
            <option value="" disabled>{$t("settings.downloads.quality.lower_pick")}</option>
            {#each LOWER_QUALITIES as q (q)}
              <option value={q}>{$t(`omnibox.quality_${q}`)}</option>
            {/each}
          </select>
        {/snippet}
      </SettingRow>
      <SettingRow id="dl-video-compatibility" label={$t("settings.download.video_compatibility")} help={$t("settings.download.video_compatibility_desc")} settingKey="settings.download.video_compatibility" status="download.video_compatibility">
        {#snippet control(c)}
          <select class="select" id={c.id} aria-describedby={c.describedBy} value={d.video_compatibility} onchange={(e) => saveSettings({ download: { video_compatibility: e.currentTarget.value as "best" | "compatible" } })}>
            <option value="best">{$t("settings.download.video_compatibility_best")}</option>
            <option value="compatible">{$t("settings.download.video_compatibility_compatible")}</option>
          </select>
        {/snippet}
      </SettingRow>
      <SettingRow id="dl-embed-metadata" label={$t("settings.download.embed_metadata")} help={$t("settings.download.embed_metadata_desc")} settingKey="settings.download.embed_metadata" status="download.embed_metadata">
        {#snippet control(c)}<Toggle id={c.id} describedBy={c.describedBy} on={d.embed_metadata} onclick={() => toggleBool("download", "embed_metadata", d.embed_metadata)} />{/snippet}
      </SettingRow>
      <SettingRow id="dl-embed-thumbnail" label={$t("settings.download.embed_thumbnail")} help={$t("settings.download.embed_thumbnail_desc")} settingKey="settings.download.embed_thumbnail" status="download.embed_thumbnail">
        {#snippet control(c)}<Toggle id={c.id} describedBy={c.describedBy} on={d.embed_thumbnail} onclick={() => toggleBool("download", "embed_thumbnail", d.embed_thumbnail)} />{/snippet}
      </SettingRow>
      <SettingRow id="dl-write-nfo-sidecar" label={$t("settings.download.write_nfo_sidecar")} help={$t("settings.download.write_nfo_sidecar_desc")} settingKey="settings.download.write_nfo_sidecar" status="download.write_nfo_sidecar">
        {#snippet control(c)}<Toggle id={c.id} describedBy={c.describedBy} on={d.write_nfo_sidecar} onclick={() => toggleBool("download", "write_nfo_sidecar", d.write_nfo_sidecar)} />{/snippet}
      </SettingRow>
    </MoreOptions>
  </SettingGroup>
{/if}

<style>
  .choice-block { padding-block: var(--space-1) var(--space-2); min-inline-size: 0; }
  .choice-title { margin: 0; font-size: var(--text-base); line-height: 20px; color: var(--text); }
  .choice-help { margin: 2px 0 0; font-size: var(--text-sm); line-height: 18px; color: var(--text-muted); }
  .choice-current { display: flex; align-items: center; gap: var(--space-2); margin: var(--space-2) 0 0; font-size: var(--text-sm); line-height: 18px; color: var(--text); overflow-wrap: anywhere; }
  .current-mark { display: inline-flex; flex: none; color: var(--text-muted); }
  .art-screen { fill: var(--surface-hi-src, var(--surface-src)); stroke: var(--tertiary); }
  .art-stand { fill: none; stroke: var(--tertiary); stroke-linecap: round; }
  .art-bar { fill: var(--accent); }
</style>
