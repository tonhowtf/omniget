<script lang="ts">
  /** Legacy URL opens the same local connection flow as Connections. */
  import SurfaceGuide from "$components/llm/SurfaceGuide.svelte";
  import { t } from "$lib/i18n";
</script>

<svelte:head><title>{$t("llm.tab.local")}</title></svelte:head>

<div class="page soon">
  <h1 class="page-title">{$t("llm.tab.local")}</h1>
  <SurfaceGuide text={$t("llm.surface.local_hint")} href="/help?article=local#guide" />
  <a class="button primary" href="/llm/accounts?guide=local">{$t("llm.accounts.wizard.local_model")} →</a>
</div>

<style>
  .soon {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
</style>
