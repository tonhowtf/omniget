<script lang="ts">
  /**
   * /llm/reading: the reading companions. Pick a bot (those with a journey
   * come first) and see its books, progress and film rounds.
   */
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { getAgents, loadRoster } from "$lib/stores/llm-store.svelte";
  import ReadingPanel from "$components/llm/reading/ReadingPanel.svelte";
  import type { Journey } from "$lib/stores/assist-reading-store.svelte";

  let journeys = $state<Journey[]>([]);
  let selected = $state<string | null>(null);
  let failed = $state(false);

  let bots = $derived.by(() => {
    const withJourney = new Set(journeys.map((j) => j.bot_id));
    const agents = getAgents();
    const known = new Set(agents.map((a) => a.id));
    const list = agents.map((a) => ({ id: a.id, name: a.name, reading: withJourney.has(a.id) }));
    for (const id of withJourney) if (!known.has(id)) list.push({ id, name: id, reading: true });
    return list.sort((x, y) => Number(y.reading) - Number(x.reading) || x.name.localeCompare(y.name));
  });

  $effect(() => {
    if (!selected && bots.length) selected = bots[0].id;
  });

  onMount(async () => {
    void loadRoster();
    try {
      journeys = await invoke<Journey[]>("assist_reading_list_all");
    } catch {
      failed = true;
    }
  });
</script>

<svelte:head><title>{$t("assist.reading.page_title")}</title></svelte:head>

<div class="page page-wide reading-page">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$t("assist.reading.page_title")}</h1>
      <p class="page-lede">{$t("assist.reading.page_lede")}</p>
    </div>
    {#if bots.length}
      <label class="pick">
        <span>{$t("assist.reading.pick_bot")}</span>
        <select bind:value={selected}>
          {#each bots as b (b.id)}
            <option value={b.id}>{b.name}{b.reading ? ` · ${$t("assist.reading.has_journey")}` : ""}</option>
          {/each}
        </select>
      </label>
    {/if}
  </header>

  {#if failed}
    <p class="notice error" role="alert">{$t("assist.reading.err_generic")}</p>
  {/if}
  {#if bots.length === 0}
    <p class="notice" role="status">{$t("assist.reading.no_bots")}</p>
  {:else if selected}
    {#key selected}
      <ReadingPanel botId={selected} />
    {/key}
  {/if}
</div>

<style>
  .reading-page { display: grid; gap: 20px; }
  .page-head { display: flex; flex-wrap: wrap; gap: 12px; justify-content: space-between; align-items: end; }
  .pick { display: grid; gap: 4px; font-size: 13px; }
  select:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
</style>
