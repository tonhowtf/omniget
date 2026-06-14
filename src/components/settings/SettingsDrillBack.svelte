<script lang="ts">
  import { t } from "$lib/i18n";

  let { title, hint = "", onBack }: {
    title: string;
    hint?: string;
    onBack: () => void;
  } = $props();
</script>

<div class="settings-drill-back">
  <button type="button" class="settings-drill-back-btn" onclick={onBack}>
    <span class="settings-drill-back-chevron" aria-hidden="true">‹</span>
    {$t('settings.drill_back')}
  </button>
  <div class="settings-drill-back-head">
    <h5 class="section-title">{title}</h5>
    {#if hint}
      <p class="settings-section-hint">{hint}</p>
    {/if}
  </div>
</div>
