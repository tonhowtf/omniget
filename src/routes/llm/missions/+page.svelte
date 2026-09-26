<script lang="ts">
  /**
   * Activity → Missions: durable objectives with completion criteria. The
   * list follows `assist://mission`; `?id=` opens one directly (the chat's
   * mission chip links here). Presets stay below, collapsed.
   */
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import MissionsPanel from "$components/llm/missions/MissionsPanel.svelte";
  import PresetsPanel from "$components/llm/missions/PresetsPanel.svelte";
  import type { Criterion } from "$lib/stores/assist-missions-store.svelte";

  let openId = $derived(page.url.searchParams.get("id"));
  let creating = $state(false);
  let seedCriteria = $state<Criterion[]>([]);

  // "New mission" after a preset starts blank again.
  $effect(() => {
    if (!creating) seedCriteria = [];
  });

  function useCriteria(criteria: Criterion[]) {
    seedCriteria = criteria;
    creating = true;
  }
</script>

<svelte:head><title>{$t("mission.page_title")}</title></svelte:head>

<div class="page page-wide missions-page">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$t("mission.page_title")}</h1>
      <p class="page-lede">{$t("mission.page_lede")}</p>
    </div>
  </header>

  <MissionsPanel {openId} bind:creating {seedCriteria} />

  <details class="presets">
    <summary>{$t("mission.presets.title")}</summary>
    <PresetsPanel onusecriteria={useCriteria} />
  </details>
</div>

<style>
  .missions-page { flex: 1; min-height: 0; overflow-y: auto; }
  .presets summary { cursor: pointer; padding: 14px 16px; border: 1px solid var(--separator); border-radius: var(--radius-lg); color: var(--text); font-weight: 600; }
  .presets[open] summary { margin-bottom: 12px; }
  .presets summary:focus-visible { outline: var(--focus-ring); }
</style>
