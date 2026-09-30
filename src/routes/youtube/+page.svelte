<script lang="ts">
  import { t } from "$lib/i18n";
  import { tubePrefs } from "$lib/tube/prefs.svelte";
  import TubeApp from "$components/tube/TubeApp.svelte";

  let enabled = $derived(tubePrefs().enabled);
</script>

{#if !enabled}
  <div class="off">
    <img src="/icons/tube/tube-64.webp" srcset="/icons/tube/tube-64.webp 1x, /icons/tube/tube-128.webp 2x" alt="" width="32" height="32" />
    <h2>{$t("tube.app.disabled_title")}</h2>
    <p>{$t("superpowers.disabled_hint")}</p>
    <a class="btn btn-primary" href="/settings?tab=advanced">{$t("superpowers.open_settings")}</a>
  </div>
{:else}
  <div class="page">
    <TubeApp />
  </div>
{/if}

<style>
  .page {
    height: 100%;
    min-height: 0;
  }
  .off {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-8) auto;
    text-align: center;
    color: var(--text-dim);
  }
  .off h2 {
    margin: 0;
    color: var(--text);
  }
</style>
