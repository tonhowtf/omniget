<script lang="ts">
  /**
   * Reading companion of one bot: the curation skill, streaming
   * preferences, the books being read (progress per edition), and each
   * round of films with where to watch in Brazil and the subtitle status.
   */
  import { t } from "$lib/i18n";
  import { loadSkills } from "$lib/stores/llm-skills-store.svelte";
  import JourneyForm from "./JourneyForm.svelte";
  import JourneyCard from "./JourneyCard.svelte";
  import AccessPrefsEditor from "./AccessPrefsEditor.svelte";
  import {
    getOverview,
    getReadingErrorKey,
    installSkill,
    isReadingLoading,
    loadOverview,
  } from "$lib/stores/assist-reading-store.svelte";

  let { botId }: { botId: string } = $props();

  let overview = $derived(getOverview(botId));
  let errorKey = $derived(getReadingErrorKey());
  let showForm = $state(false);
  let skillBusy = $state(false);
  let skillError = $state<string | null>(null);
  let skillNote = $state<string | null>(null);
  let confirmReplace = $state(false);

  $effect(() => {
    if (botId) void loadOverview(botId);
  });

  async function install(replace: boolean) {
    skillBusy = true;
    skillError = null;
    const out = await installSkill(botId, replace);
    skillBusy = false;
    confirmReplace = false;
    if (!out.ok) {
      skillError = out.errorKey;
      return;
    }
    skillNote = out.value.outcome?.needs_confirm ? "assist.reading.skill_needs_confirm" : "assist.reading.skill_installed";
    // The bot screen lists installed skills to link; refresh that list now.
    void loadSkills(true);
  }
</script>

<div class="reading-panel" data-bot={botId}>
  <header>
    <h2 class="block-title">{$t("assist.reading.panel_title")}</h2>
    <p class="dim">{$t("assist.reading.panel_lede")}</p>
  </header>

  {#if errorKey}
    <p class="notice error" role="alert">{$t(errorKey)}</p>
    <button type="button" class="button" disabled={isReadingLoading()} onclick={() => loadOverview(botId)}>{$t("assist.reading.retry")}</button>
  {:else if !overview && isReadingLoading()}
    <p class="dim" role="status">{$t("assist.reading.loading")}</p>
  {/if}

  {#if overview}
    <section class="skill" aria-labelledby="reading-skill-{botId}">
      <h3 id="reading-skill-{botId}" class="section-title">{$t("assist.reading.skill_title")}</h3>
      {#if overview.skill?.installed}
        <p class="dim">{overview.skill_bound ? $t("assist.reading.skill_bound") : $t("assist.reading.skill_ready")}</p>
        {#if overview.skill.differs}
          <p class="dim">{$t("assist.reading.skill_differs")}</p>
          {#if confirmReplace}
            <span class="confirm">
              {$t("assist.reading.skill_replace_confirm")}
              <button type="button" class="button small" disabled={skillBusy} onclick={() => install(true)}>{$t("assist.reading.yes_replace")}</button>
              <button type="button" class="button small" onclick={() => (confirmReplace = false)}>{$t("assist.reading.cancel")}</button>
            </span>
          {:else}
            <button type="button" class="link" onclick={() => (confirmReplace = true)}>{$t("assist.reading.skill_replace")}</button>
          {/if}
        {/if}
      {:else}
        <p class="dim">{$t("assist.reading.skill_missing")}</p>
        <button type="button" class="button" disabled={skillBusy} onclick={() => install(false)}>
          {skillBusy ? $t("assist.reading.working") : $t("assist.reading.skill_install")}
        </button>
      {/if}
      {#if skillNote}<p class="notice" role="status">{$t(skillNote)}</p>{/if}
      {#if skillError}<p class="notice error" role="alert">{$t(skillError)}</p>{/if}
    </section>

    <AccessPrefsEditor {botId} prefs={overview.prefs} />

    <section class="journeys" aria-labelledby="reading-journeys-{botId}">
      <div class="row">
        <h3 id="reading-journeys-{botId}" class="section-title">{$t("assist.reading.journeys_title")}</h3>
        {#if !showForm}
          <button type="button" class="button" onclick={() => (showForm = true)}>{$t("assist.reading.new_journey")}</button>
        {/if}
      </div>
      {#if showForm}
        <JourneyForm {botId} onDone={() => (showForm = false)} />
      {/if}
      {#if overview.journeys.length === 0 && !showForm}
        <p class="notice" role="status">{$t("assist.reading.empty")}</p>
      {/if}
      {#each overview.journeys as detail (detail.journey.id)}
        <JourneyCard {botId} {detail} />
      {/each}
    </section>
  {/if}
</div>

<style>
  .reading-panel { display: grid; gap: 20px; }
  header { display: grid; gap: 4px; }
  section { display: grid; gap: 8px; }
  .row { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; justify-content: space-between; }
  .dim { margin: 0; color: var(--text-muted); font-size: 13px; line-height: 1.5; max-width: 72ch; }
  .section-title { margin: 0; font-size: 15px; font-weight: 600; color: var(--text); }
  .link { background: none; border: 0; padding: 4px 0; color: var(--accent-text); font-size: 13px; cursor: pointer; border-radius: var(--radius-sm); justify-self: start; }
  .link:hover { text-decoration: underline; }
  .link:focus-visible, .button:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
  .button { justify-self: start; }
  .small { min-height: 32px; padding: 4px 10px; font-size: 13px; }
  .confirm { display: inline-flex; flex-wrap: wrap; gap: 6px; align-items: center; font-size: 13px; }
</style>
