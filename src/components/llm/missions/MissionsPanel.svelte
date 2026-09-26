<script lang="ts">
  /**
   * Activity → missions: every objective given to a bot or a group, with a
   * plain state, the task count, where the verdict stands and whether the
   * conclusion is automatic or depends on your judgement. Click a row to
   * follow it. Live through `assist://mission`.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { getAgents, getRooms, loadRooms, loadRoster } from "$lib/stores/llm-store.svelte";
  import { getAsks, getJobs, initJobsStore, pollAsks } from "$lib/stores/llm-jobs-store.svelte";
  import {
    MISSION_FILTERS, completionKey, errorText, getMissionTick, initMissionsStore, listMissions, matchesMissionFilter,
    missionAsks, missionStateKey, missionTone, rowVerdictLines, shortTime, waitReason, isExternalMission,
    externalClientName, mcpClientNames,
    type Criterion, type MissionFilter, type MissionRow,
  } from "$lib/stores/assist-missions-store.svelte";
  import MissionDetail from "./MissionDetail.svelte";
  import CreateMission from "./CreateMission.svelte";

  let {
    openId: requestedId = null,
    conversationId = null,
    creating = $bindable(false),
    seedCriteria = [],
  }: { openId?: string | null; conversationId?: string | null; creating?: boolean; seedCriteria?: Criterion[] } = $props();

  let all = $state<MissionRow[]>([]);
  let loaded = $state(false);
  let error = $state("");
  let filter = $state<MissionFilter>("all");
  let openId = $state<string | null>(null);
  let agents = $derived(getAgents());
  let rooms = $derived(getRooms());
  let visible = $derived(all.filter((m) => matchesMissionFilter(m, filter)));
  let clientNames = $state<Record<string, string>>({});
  let anyExternal = $derived(all.some((m) => isExternalMission(m)));
  $effect(() => {
    if (anyExternal) void mcpClientNames().then((n) => (clientNames = n));
  });

  async function refresh() {
    try {
      all = await listMissions(conversationId ? { conversation_id: conversationId, limit: 200 } : { limit: 200 });
      error = "";
    } catch (e) {
      error = errorText(e);
    } finally {
      loaded = true;
    }
  }

  onMount(() => {
    initMissionsStore();
    initJobsStore();
    void loadRoster();
    void loadRooms();
    // Tool approvals of mission turns: the chat does not drive them, so this
    // page asks for them (every few seconds and on each new question).
    void pollAsks();
    const timer = setInterval(() => void pollAsks(), 2500);
    let unlisten: (() => void) | null = null;
    let gone = false;
    import("@tauri-apps/api/event")
      .then(({ listen }) => Promise.all([
        listen("llm://tool-ask", () => void pollAsks()),
        listen("omni://tool-resolved", () => void pollAsks()),
      ]))
      .then((stops) => {
        const stop = () => stops.forEach((f) => f());
        if (gone) stop();
        else unlisten = stop;
      })
      .catch(() => {});
    return () => {
      gone = true;
      clearInterval(timer);
      unlisten?.();
    };
  });

  function waiting(m: MissionRow) {
    return waitReason(m, missionAsks(getAsks(), getJobs() ?? [], m));
  }

  $effect(() => {
    void getMissionTick();
    void conversationId;
    void refresh();
  });

  $effect(() => {
    if (requestedId) openId = requestedId;
  });

  function who(m: MissionRow): string {
    if (m.room_id) return rooms.find((r) => r.id === m.room_id)?.title ?? m.room_id;
    return agents.find((a) => a.id === m.bot_id)?.name ?? m.bot_id ?? "—";
  }

  function firstLine(text: string): string {
    return text.split("\n").find((l) => l.trim())?.trim() ?? "";
  }

  function created(id: string) {
    openId = id;
    void refresh();
  }
</script>

<section class="stack" aria-labelledby="missions-title">
  <div class="title-row">
    <h2 class="section-header-title" id="missions-title">{$t("mission.list.title")}</h2>
    <button type="button" class="button active" onclick={() => (creating = true)}>{$t("mission.list.new")}</button>
  </div>
  <p class="hint">{$t("mission.list.lede")}</p>

  <div class="filters" role="group" aria-label={$t("mission.list.title")}>
    {#each MISSION_FILTERS as key (key)}
      <button class="button" type="button" aria-pressed={filter === key} onclick={() => (filter = key)}>
        {$t(`mission.filter.${key}`)} · {all.filter((m) => matchesMissionFilter(m, key)).length}
      </button>
    {/each}
  </div>

  {#if error}
    <p class="error" role="alert">{error}</p>
    <div><button type="button" class="button" onclick={refresh}>{$t("mission.retry")}</button></div>
  {:else if !loaded}
    <p class="hint" role="status">{$t("mission.list.loading")}</p>
  {:else if visible.length === 0}
    <p class="hint">{$t(all.length ? "mission.list.no_matches" : "mission.list.empty")}</p>
  {:else}
    <div class="rows">
      {#each visible as m (m.id)}
        {@const wait = waiting(m)}
        <div class="row" class:open={openId === m.id}>
          <button type="button" class="row-head" aria-expanded={openId === m.id} onclick={() => (openId = openId === m.id ? null : m.id)}>
            <span class="pill {missionTone(m.state)}">
              {#if m.driving}<span class="pulse" aria-hidden="true"></span>{/if}
              {$t(missionStateKey(m.state))}
            </span>
            {#if isExternalMission(m)}
              {@const client = externalClientName(m, clientNames)}
              <span class="tag ext" title={$t("mission.external.hint")}>{client ? $t("mission.external.badge", { name: client }) : $t("mission.external.badge_unnamed")}</span>
            {/if}
            <span class="agent">{who(m)}</span>
            <span class="line">{firstLine(m.objective)}</span>
            {#if m.tasks_total > 0}<span class="tag">{$t("mission.list.tasks", { done: m.tasks_done, total: m.tasks_total })}</span>{/if}
            {#each rowVerdictLines(m.verdict, m.verdict_counts) as v (v.key)}<span class="tag" class:warn={v.key !== "mission.verdict.passed"}>{$t(v.key, v.params)}</span>{/each}
            {#if m.completion === "human"}<span class="tag">{$t(completionKey("human"))}</span>{/if}
            <span class="time">{shortTime(m.updated_ms)}</span>
          </button>
          {#if wait && openId !== m.id}
            <button type="button" class="waits" title={$t(wait.key, wait.params)} onclick={() => (openId = m.id)}>
              <span class="waits-label">{$t("mission.list.waits_for_you")}</span>
              <span class="waits-reason">{$t(wait.key, wait.params)}</span>
            </button>
          {/if}
          {#if openId === m.id}
            <MissionDetail missionId={m.id} ondeleted={() => { openId = null; void refresh(); }} />
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</section>

{#if creating}
  <CreateMission initialCriteria={seedCriteria} onclose={() => (creating = false)} oncreated={created} />
{/if}

<style>
  .stack { display: flex; flex-direction: column; gap: var(--space-3); margin-bottom: var(--space-5); min-width: 0; }
  .title-row { display: flex; align-items: center; justify-content: space-between; gap: var(--space-3); flex-wrap: wrap; }
  .title-row h2 { margin: 0; }
  .hint { margin: 0; font-size: var(--text-sm); color: var(--text-dim); }
  .error { margin: 0; font-size: var(--text-sm); color: var(--red); overflow-wrap: anywhere; }
  .filters { display: flex; flex-wrap: wrap; gap: var(--space-2); }
  .filters [aria-pressed="true"] { color: var(--accent-hi); background: var(--accent-soft); }
  .rows { display: flex; flex-direction: column; border: var(--hairline) solid var(--separator); border-radius: var(--radius-md); overflow: hidden; }
  .row + .row { border-top: var(--hairline) solid var(--separator); }
  .row.open { background: var(--fill-1); }
  .row-head { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-2) var(--space-3); width: 100%; padding: var(--space-2) var(--space-3); font-size: var(--text-sm); text-align: left; background: none; border: 0; color: inherit; cursor: pointer; }
  .row-head:hover { background: var(--fill-1); }
  .row-head:focus-visible { outline: var(--focus-ring); outline-offset: -2px; }
  .agent { flex-shrink: 0; font-size: var(--text-xs); font-weight: 500; color: var(--text-muted); }
  .line { flex: 1; min-width: 120px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .time { margin-left: auto; font-size: var(--text-xs); color: var(--text-dim); font-variant-numeric: tabular-nums; }
  .tag { flex-shrink: 0; padding: 0 6px; border-radius: 999px; font-size: var(--text-xs); color: var(--text-muted); background: var(--fill-2); }
  .waits { display: flex; gap: var(--space-2); align-items: baseline; width: 100%; padding: 4px var(--space-3) var(--space-2); border: 0; background: color-mix(in srgb, var(--orange) 10%, transparent); color: inherit; font-size: var(--text-xs); text-align: left; cursor: pointer; min-width: 0; }
  .waits:focus-visible { outline: var(--focus-ring); outline-offset: -2px; }
  .waits-label { flex-shrink: 0; font-weight: 600; color: var(--orange); }
  .waits-reason { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; color: var(--text-muted); }
  .tag.ext { color: var(--accent-text); background: var(--accent-soft); }
  .tag.warn { color: var(--orange); background: color-mix(in srgb, var(--orange) 14%, transparent); }
  .pill { display: inline-flex; align-items: center; gap: 6px; flex-shrink: 0; padding: 1px 8px; border-radius: 999px; font-size: var(--text-xs); font-weight: 600; white-space: nowrap; color: var(--text-muted); background: var(--fill-2); }
  .pill.blue { color: var(--blue); background: color-mix(in srgb, var(--blue) 14%, transparent); }
  .pill.orange { color: var(--orange); background: color-mix(in srgb, var(--orange) 16%, transparent); }
  .pill.green { color: var(--green); background: color-mix(in srgb, var(--green) 14%, transparent); }
  .pill.red { color: var(--red); background: color-mix(in srgb, var(--red) 14%, transparent); }
  .pulse { width: 6px; height: 6px; border-radius: 50%; background: currentColor; animation: pulse 1.2s ease-in-out infinite; }
  @keyframes pulse { 50% { opacity: 0.3; } }
  @media (prefers-reduced-motion: reduce) { .pulse { animation: none; } }
</style>
