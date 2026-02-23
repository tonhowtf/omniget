<script lang="ts">
  import { t } from "$lib/i18n";

  let { selectedQuality = $bindable("best"), selectedFormatId } = $props();
</script>

{#if !selectedFormatId}
  <div class="quality-select-wrapper">
    <span class="quality-label">{$t('omnibox.quality')}</span>
    <select class="quality-select" bind:value={selectedQuality}>
      <option value="best">{$t('omnibox.quality_best')}</option>
      <option value="1080p">{$t('omnibox.quality_1080p')}</option>
      <option value="720p">{$t('omnibox.quality_720p')}</option>
      <option value="480p">{$t('omnibox.quality_480p')}</option>
      <option value="360p">{$t('omnibox.quality_360p')}</option>
    </select>
  </div>
{/if}

<style>
  .quality-select-wrapper {
    display: flex;
    align-items: center;
    gap: var(--padding);
  }

  .quality-label {
    font-size: 12.5px;
    font-weight: 500;
    color: var(--gray);
  }

  .quality-select {
    padding: 6px 10px;
    font-size: 14.5px;
    background: var(--button);
    border: 1px solid var(--input-border);
    border-radius: calc(var(--border-radius) - 2px);
    color: var(--secondary);
    cursor: pointer;
  }

  .quality-select:focus-visible {
    border-color: var(--secondary);
    outline: none;
  }
</style>
