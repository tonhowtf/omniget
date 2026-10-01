<script lang="ts">
  import { t } from "$lib/i18n";
  import { getSettings, updateSettings, changeQuality } from "../settings-helpers";
  import { YTDLP_PRESETS, matchActivePreset, type YtdlpPresetId } from "$lib/ytdlp-presets";
  import SettingGroup from "../rebrand/SettingGroup.svelte";
  import SettingRow from "../rebrand/SettingRow.svelte";

  let { part = "quality" }: { part?: "quality" | "presets" } = $props();

  let settings = $derived(getSettings());
  let activePreset = $derived<YtdlpPresetId | null>(matchActivePreset(settings));

  async function applyPreset(id: YtdlpPresetId) {
    const preset = YTDLP_PRESETS.find((p) => p.id === id);
    if (!preset) return;
    await updateSettings({ download: preset.download });
  }
</script>

{#if settings}
  {#if part === "quality"}
    <SettingGroup id="quality" title={$t("settings.rebrand.downloads.quality")}>
      <SettingRow id="dl-video-quality" label={$t("settings.download.video_quality")} help={$t("settings.download.video_quality_desc")} settingKey="settings.download.video_quality" status="download.video_quality">
        {#snippet control(c)}
          <select class="select" id={c.id} aria-describedby={c.describedBy} value={settings.download.video_quality} onchange={changeQuality}>
            <option value="best">{$t("omnibox.quality_best")}</option>
            <option value="1080p">{$t("omnibox.quality_1080p")}</option>
            <option value="720p">{$t("omnibox.quality_720p")}</option>
            <option value="480p">{$t("omnibox.quality_480p")}</option>
            <option value="360p">{$t("omnibox.quality_360p")}</option>
          </select>
        {/snippet}
      </SettingRow>
    </SettingGroup>
  {:else}
    <SettingGroup id="presets" title={$t("settings.download.section_quality")} description={$t("settings.download.section_quality_desc")} advanced>
      <div class="preset-grid" role="group" aria-labelledby="settings-group-presets-title">
        {#each YTDLP_PRESETS as preset (preset.id)}
          <button
            class="preset-card"
            class:active={activePreset === preset.id}
            aria-pressed={activePreset === preset.id}
            onclick={() => applyPreset(preset.id)}
            type="button"
          >
            <span class="preset-label">{$t(preset.labelKey)}</span>
            <span class="preset-desc">{$t(preset.descKey)}</span>
          </button>
        {/each}
      </div>
    </SettingGroup>
  {/if}
{/if}
