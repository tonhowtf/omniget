<script lang="ts">
  import { t } from "$lib/i18n";
  import {
    getSettings,
    updateSettings,
    toggleBool,
    changeQuality,
  } from "../settings-helpers";
  import { YTDLP_PRESETS, matchActivePreset, type YtdlpPresetId } from "$lib/ytdlp-presets";

  let { embedded = false }: { embedded?: boolean } = $props();

  let settings = $derived(getSettings());
  let activePreset = $derived<YtdlpPresetId | null>(matchActivePreset(settings));

  async function applyPreset(id: YtdlpPresetId) {
    const preset = YTDLP_PRESETS.find((p) => p.id === id);
    if (!preset) return;
    await updateSettings({ download: preset.download });
  }

  let speedNum = $state<number | null>(null);
  let speedUnit = $state<"K" | "M">("M");

  $effect(() => {
    const raw = settings?.download.speed_limit?.trim() ?? "";
    const m = raw.match(/^(\d+(?:\.\d+)?)([KM])?$/i);
    if (m) {
      speedNum = Number(m[1]);
      speedUnit = (m[2]?.toUpperCase() as "K" | "M") ?? "M";
    } else {
      speedNum = null;
    }
  });

  function applySpeedLimit() {
    const value = speedNum && speedNum > 0 ? `${speedNum}${speedUnit}` : "";
    updateSettings({ download: { speed_limit: value } });
  }
</script>

{#if settings}
  {#if !embedded}
    <div class="settings-section-head section-title">
      <h5 class="section-title">{$t('settings.download.section_quality')}</h5>
      <p class="settings-section-hint">{$t('settings.download.section_quality_desc')}</p>
    </div>
  {/if}

  <div class="card">
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t('settings.download.video_quality')}</span>
        <span class="setting-path">{$t('settings.download.video_quality_desc')}</span>
      </div>
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
      <div class="setting-col">
        <span class="setting-label">{$t('settings.download.speed_limit')}</span>
        <span class="setting-path">{$t('settings.download.speed_limit_desc')}</span>
      </div>
      <div class="speed-limit">
        <input
          type="number"
          class="downloads-speed-input"
          min="0"
          step="1"
          inputmode="numeric"
          placeholder={$t('settings.download.speed_limit_unlimited') as string}
          bind:value={speedNum}
          onchange={applySpeedLimit}
          aria-label={$t('settings.download.speed_limit') as string}
        />
        <select class="select" bind:value={speedUnit} onchange={applySpeedLimit} aria-label="unit">
          <option value="K">KB/s</option>
          <option value="M">MB/s</option>
        </select>
      </div>
    </div>
    <div class="divider"></div>
    <div class="setting-row">
      <div class="setting-col">
        <span class="setting-label">{$t('settings.download.always_ask_path')}</span>
        <span class="setting-path">{$t('settings.download.always_ask_path_desc')}</span>
      </div>
      <button class="toggle" class:on={settings.download.always_ask_path} onclick={() => toggleBool("download", "always_ask_path", settings.download.always_ask_path)} role="switch" aria-checked={settings.download.always_ask_path} aria-label={$t('settings.download.always_ask_path') as string}><span class="toggle-knob"></span></button>
    </div>
  </div>

  <div class="preset-grid">
    {#each YTDLP_PRESETS as preset (preset.id)}
      <button
        class="preset-card"
        class:active={activePreset === preset.id}
        onclick={() => applyPreset(preset.id)}
        type="button"
      >
        <span class="preset-label">{$t(preset.labelKey)}</span>
        <span class="preset-desc">{$t(preset.descKey)}</span>
      </button>
    {/each}
  </div>
{/if}
