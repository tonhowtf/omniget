<script lang="ts">
  /**
   * The chat's view of missions. `chip` mode: a compact chip in the
   * conversation header when the conversation has missions, linking to the
   * mission. `action` mode: "Make it a mission" beside the composer, shown
   * while there is a draft; it opens the create form prefilled with the text,
   * the conversation and its bot.
   */
  import { t } from "$lib/i18n";
  import {
    getMissionTick, initMissionsStore, isFinal, listMissions, missionStateKey, missionTone,
    type MissionRow,
  } from "$lib/stores/assist-missions-store.svelte";
  import CreateMission from "./CreateMission.svelte";

  let {
    mode,
    conversationId = null,
    botId = null,
    roomId = null,
    draft = "",
  }: { mode: "chip" | "action"; conversationId?: string | null; botId?: string | null; roomId?: string | null; draft?: string } = $props();

  let missions = $state<MissionRow[]>([]);
  let open = $state(false);
  let objective = $state("");

  // The newest unfinished mission leads; otherwise the newest one.
  let lead = $derived(missions.find((m) => !isFinal(m.state)) ?? missions[0] ?? null);

  $effect(() => {
    if (mode !== "chip") return;
    initMissionsStore();
    void getMissionTick();
    const id = conversationId;
    if (!id) {
      missions = [];
      return;
    }
    listMissions({ conversation_id: id, limit: 20 })
      .then((list) => { if (id === conversationId) missions = list; })
      .catch(() => { missions = []; });
  });

  function start() {
    objective = draft;
    open = true;
  }
</script>

{#if mode === "chip"}
  {#if lead}
    <a class="chip" href={`/llm/missions?id=${encodeURIComponent(lead.id)}`} title={lead.objective}>
      <span class="dot {missionTone(lead.state)}" aria-hidden="true"></span>
      {$t("mission.chip.label")} · {$t(missionStateKey(lead.state))}
      {#if missions.length > 1}<span class="more">+{missions.length - 1}</span>{/if}
    </a>
  {/if}
{:else if conversationId && (botId || roomId) && draft.trim()}
  <div class="action-row">
    <button type="button" class="make" onclick={start} title={$t("mission.chip.make_hint")}>{$t("mission.chip.make")}</button>
  </div>
{/if}

{#if open}
  <CreateMission {objective} {conversationId} {botId} {roomId} onclose={() => (open = false)} />
{/if}

<style>
  .chip { display: inline-flex; align-items: center; gap: 6px; padding: 1px 8px; border-radius: var(--radius-full, 999px); font-size: var(--text-caption, 12px); color: var(--text-muted); background: var(--fill-1); text-decoration: none; white-space: nowrap; }
  .chip:hover { background: var(--accent-soft); color: var(--accent-hi); }
  .chip:focus-visible, .make:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--text-dim); }
  .dot.blue { background: var(--blue); }
  .dot.orange { background: var(--orange); }
  .dot.green { background: var(--green); }
  .dot.red { background: var(--red); }
  .more { color: var(--text-dim); }
  .action-row { display: flex; justify-content: flex-end; padding: 0 var(--space-5); }
  .make { background: none; border: 0; padding: 2px 0; font: inherit; font-size: var(--text-sm); color: var(--accent-text); cursor: pointer; }
  .make:hover { text-decoration: underline; }
</style>
