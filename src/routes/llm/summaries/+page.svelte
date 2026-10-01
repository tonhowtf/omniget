<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { focusWhenReady, parseSummariesTarget } from "$lib/settings-navigation";
  import SummaryForm from "$components/llm/SummaryForm.svelte";
  import SummaryHistory from "$components/llm/SummaryHistory.svelte";
  import { summaryFocusSelector } from "$components/llm/summaries";

  let historyRevision = $state(0);
  let target = $derived(parseSummariesTarget(page.url.searchParams));

  $effect(() => {
    const selector = summaryFocusSelector(target);
    if (!selector) return;
    return focusWhenReady(selector);
  });
</script>

<svelte:head><title>{$t("settings.ai.summarize_title")}</title></svelte:head>

<div class="page page-wide summaries-page">
  <header class="page-head">
    <div>
      <h1 class="page-title" id="summaries-form-title">{$t("settings.ai.summarize_title")}</h1>
      <p class="page-lede"><bdi>{$t("settings.ai.summarize_hint")}</bdi></p>
    </div>
  </header>

  <section id="summaries-form" class="block" aria-labelledby="summaries-form-title" data-setting-key="settings.ai.summarize_title" tabindex="-1">
    <SummaryForm labelledBy="summaries-form-title" onsummarized={() => historyRevision++} />
  </section>

  <section id="summaries-history" class="block" aria-labelledby="summaries-history-title" data-setting-key="settings.ai.history" tabindex="-1">
    <h2 class="section-header-title" id="summaries-history-title">{$t("settings.ai.history")}</h2>
    <SummaryHistory labelledBy="summaries-history-title" refreshKey={historyRevision} />
  </section>
</div>

<style>
  .summaries-page { flex: 1; min-height: 0; display: grid; gap: 20px; align-content: start; overflow-y: auto; box-sizing: border-box; }
  .block { display: grid; gap: 8px; min-width: 0; outline: none; }
  .block:focus-visible { outline: var(--focus-ring); outline-offset: 4px; }
</style>
