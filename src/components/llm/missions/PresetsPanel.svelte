<script lang="ts">
  /**
   * Ready-made mission setups: the bots (and optionally a group) a preset
   * creates, and the criteria it proposes. Applying one creates the bots on
   * a connection you choose; "use its criteria" only prefills a new mission.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { getAgents, loadRooms, loadRoster } from "$lib/stores/llm-store.svelte";
  import { connectionAgents } from "$lib/stores/assist-bots-store.svelte";
  import { applyPreset, criterionTitleLine, errorText, missionPresets, presetTextKey, specSummary, type Criterion, type Preset } from "$lib/stores/assist-missions-store.svelte";

  let { onusecriteria }: { onusecriteria?: (criteria: Criterion[]) => void } = $props();

  let presets = $state<Preset[]>([]);
  let loaded = $state(false);
  let error = $state("");
  let busy = $state<string | null>(null);
  let connection = $state("");
  let withGroup = $state<Record<string, boolean>>({});
  let applied = $state<Record<string, string>>({});
  let connections = $derived(connectionAgents(getAgents()));

  onMount(async () => {
    void loadRoster();
    try {
      presets = await missionPresets();
    } catch (e) {
      error = errorText(e);
    } finally {
      loaded = true;
    }
  });

  $effect(() => {
    if (!connection && connections.length) connection = connections[0].id;
  });

  /** A shipped preset's text in the UI language; the backend text otherwise. */
  function presetText(p: Preset, field: "title" | "description" | "group", fallback: string): string {
    const key = presetTextKey(p.id, field);
    return key ? $t(key) : fallback;
  }

  function critTitle(c: Criterion): string {
    const line = criterionTitleLine(c);
    return line ? $t(line.key, line.params) : c.title;
  }

  async function apply(p: Preset) {
    if (!connection || busy) return;
    busy = p.id;
    error = "";
    try {
      const r = await applyPreset({ preset_id: p.id, connection_agent_id: connection, with_group: !!withGroup[p.id] && !!p.group });
      applied[p.id] = r.room ? $t("mission.presets.applied_group", { count: r.bots.length, room: r.room.title }) : $t("mission.presets.applied", { count: r.bots.length });
      await Promise.all([loadRoster(), loadRooms()]);
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = null;
    }
  }
</script>

<section class="stack" aria-labelledby="presets-title">
  <h2 class="section-header-title" id="presets-title">{$t("mission.presets.title")}</h2>
  <p class="hint">{$t("mission.presets.lede")}</p>
  {#if connections.length}
    <label class="field">
      <span class="field-label">{$t("mission.presets.connection")}</span>
      <select class="input narrow" bind:value={connection}>
        {#each connections as a (a.id)}<option value={a.id}>{a.name}</option>{/each}
      </select>
    </label>
  {:else}
    <p class="hint">{$t("mission.presets.no_connection")} <a class="link" href="/llm/accounts">{$t("mission.presets.connect")}</a></p>
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if !loaded}
    <p class="hint" role="status">{$t("mission.list.loading")}</p>
  {:else if presets.length === 0}
    <p class="hint">{$t("mission.presets.empty")}</p>
  {:else}
    <ul class="cards">
      {#each presets as p (p.id)}
        <li class="card">
          <h3>{presetText(p, "title", p.title)}</h3>
          <p class="hint">{presetText(p, "description", p.description)}</p>
          <p class="hint">{$t("mission.presets.bots", { names: p.bots.map((b) => b.name).join(", ") })}</p>
          {#if p.workspace === "project"}<p class="hint">{$t("mission.presets.needs_folder")}</p>{/if}
          <details class="adv">
            <summary>{$t("mission.presets.criteria", { count: p.criteria.length })}</summary>
            <ul class="plain">
              {#each p.criteria as c (c.id)}<li><strong>{critTitle(c)}</strong> · {$t(`mission.kind.${c.kind}`)} · <code>{specSummary(c)}</code></li>{/each}
            </ul>
            {#each p.bots as b (b.key)}
              {#if b.attribution}<p class="dim">{b.name}: {b.attribution}</p>{/if}
            {/each}
          </details>
          {#if p.group}
            <label class="check"><input type="checkbox" checked={!!withGroup[p.id]} onchange={(e) => (withGroup[p.id] = e.currentTarget.checked)} /> {$t("mission.presets.with_group", { title: presetText(p, "group", p.group.title) })}</label>
          {/if}
          <div class="actions">
            <button type="button" class="button" disabled={!connection || busy !== null} onclick={() => apply(p)}>{$t(busy === p.id ? "mission.presets.applying" : "mission.presets.apply")}</button>
            {#if onusecriteria}<button type="button" class="button" onclick={() => onusecriteria?.(p.criteria)}>{$t("mission.presets.use_criteria")}</button>{/if}
          </div>
          {#if applied[p.id]}<p class="hint" role="status">{applied[p.id]}</p>{/if}
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .stack { display: flex; flex-direction: column; gap: var(--space-3); margin-bottom: var(--space-5); min-width: 0; }
  .hint { margin: 0; font-size: var(--text-sm); color: var(--text-dim); }
  .dim { margin: 0; font-size: var(--text-xs); color: var(--text-dim); }
  .error { margin: 0; font-size: var(--text-sm); color: var(--red); overflow-wrap: anywhere; }
  .field { display: flex; flex-direction: column; gap: 4px; }
  .narrow { max-width: 320px; }
  .link { color: var(--accent-text); }
  .cards { list-style: none; margin: 0; padding: 0; display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: var(--space-3); }
  .card { display: flex; flex-direction: column; gap: var(--space-2); padding: var(--space-3); border: var(--hairline) solid var(--separator); border-radius: var(--radius-md); min-width: 0; }
  .card h3 { margin: 0; font-size: var(--text-base); font-weight: 600; }
  .plain { list-style: none; margin: var(--space-2) 0; padding: 0; display: flex; flex-direction: column; gap: 4px; font-size: var(--text-xs); overflow-wrap: anywhere; }
  .check { display: inline-flex; gap: 6px; align-items: center; font-size: var(--text-sm); }
  .actions { display: flex; flex-wrap: wrap; gap: var(--space-2); }
  .adv summary { cursor: pointer; font-size: var(--text-sm); color: var(--accent-text); }
  summary:focus-visible, .button:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
</style>
