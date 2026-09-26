<script lang="ts">
  /**
   * New mission: an objective, who works on it (a bot or a group), what
   * "done" means (the criteria), an optional budget and folder. From the
   * Chat it becomes `assist_mission_from_chat` (the conversation's folder is
   * the workspace and the turns show in the chat); elsewhere it is
   * `assist_mission_create`. A preset only prefills the criteria.
   */
  import { onMount, untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { getAgents, getRooms, loadRooms, loadRoster } from "$lib/stores/llm-store.svelte";
  import {
    createMission, errorText, groupPlan, isAbsolutePath, missionFromChat, missionPresets, parseBudgetField, presetTextKey,
    validateDraft, type Criterion, type NewBudget, type NewTask, type Preset,
  } from "$lib/stores/assist-missions-store.svelte";
  import CriteriaEditor from "./CriteriaEditor.svelte";

  let {
    objective: initialObjective = "",
    conversationId = null,
    botId: initialBot = null,
    roomId: initialRoom = null,
    initialCriteria = [],
    onclose,
    oncreated,
  }: {
    objective?: string;
    conversationId?: string | null;
    botId?: string | null;
    roomId?: string | null;
    initialCriteria?: Criterion[];
    onclose?: () => void;
    oncreated?: (missionId: string) => void;
  } = $props();

  // The props only seed the form; editing never writes back to the caller.
  const seed = untrack(() => ({ objective: initialObjective, bot: initialBot, room: initialRoom, criteria: initialCriteria }));
  let objective = $state(seed.objective);
  let target = $state<"bot" | "group">(seed.room ? "group" : "bot");
  let botId = $state(seed.bot ?? "");
  let roomId = $state(seed.room ?? "");
  let criteria = $state<Criterion[]>(structuredClone($state.snapshot(seed.criteria)));
  let tasks = $state<NewTask[]>([]);
  // Picked with the dialog or typed by hand; checked on submit.
  let workspaceText = $state("");
  let workspace = $derived(workspaceText.trim() ? workspaceText.trim() : null);
  let usd = $state("");
  let tokens = $state("");
  let turns = $state("");
  let minutes = $state("");
  let start = $state(true);
  let presets = $state<Preset[]>([]);
  let presetId = $state("");
  let busy = $state(false);
  let planning = $state(false);
  let error = $state("");
  let errorKey = $state<string | null>(null);

  let fromChat = $derived(!!conversationId);
  let agents = $derived(getAgents());
  let rooms = $derived(getRooms());
  let preset = $derived(presets.find((p) => p.id === presetId) ?? null);
  let chatBot = $derived.by(() => {
    if (!fromChat) return null;
    if (seed.room) {
      const r = rooms.find((x) => x.id === seed.room);
      return r?.coordinator ?? r?.default_bot ?? r?.members[0]?.bot ?? null;
    }
    return seed.bot;
  });

  onMount(() => {
    void loadRoster();
    void loadRooms();
    missionPresets().then((p) => (presets = p)).catch(() => {});
  });

  $effect(() => {
    if (!botId && agents.length && !fromChat) botId = agents[0].id;
  });

  function applyPresetCriteria() {
    if (!preset) return;
    criteria = structuredClone($state.snapshot(preset.criteria)).map((c: Criterion) => ({ ...c, origin: c.origin ?? "preset" }));
  }

  async function pickWorkspace() {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const r = await open({ directory: true, multiple: false });
      if (typeof r === "string") workspaceText = r;
    } catch {
      errorKey = "mission.create.err_picker";
    }
  }

  async function plan() {
    if (!roomId || !objective.trim() || planning) return;
    planning = true;
    error = "";
    try {
      tasks = await groupPlan(roomId, objective.trim());
    } catch (e) {
      error = errorText(e);
    } finally {
      planning = false;
    }
  }

  function budget(): NewBudget | null {
    const b = {
      usd: parseBudgetField(usd),
      tokens: parseBudgetField(tokens, true),
      turns: parseBudgetField(turns, true),
      max_minutes: parseBudgetField(minutes, true),
    };
    if (Object.values(b).some((v) => v === undefined)) return null;
    return b as NewBudget;
  }

  async function submit() {
    if (busy) return;
    error = "";
    errorKey = validateDraft({ objective, criteria });
    if (errorKey) return;
    if (!fromChat && workspace && !isAbsolutePath(workspace)) {
      errorKey = "mission.path.err_absolute";
      return;
    }
    const b = budget();
    if (!b) {
      errorKey = "mission.create.err_budget";
      return;
    }
    const snapshot = $state.snapshot(criteria) as Criterion[];
    busy = true;
    try {
      let detail;
      if (fromChat) {
        if (!chatBot) {
          errorKey = "mission.create.err_target";
          return;
        }
        detail = await missionFromChat({ conversation_id: conversationId!, bot_id: chatBot, objective: objective.trim(), criteria: snapshot, budget: b, start });
      } else {
        if ((target === "bot" && !botId) || (target === "group" && !roomId)) {
          errorKey = "mission.create.err_target";
          return;
        }
        detail = await createMission({
          objective: objective.trim(),
          bot_id: target === "bot" ? botId : null,
          room_id: target === "group" ? roomId : null,
          criteria: snapshot,
          tasks: target === "group" && tasks.length ? ($state.snapshot(tasks) as NewTask[]) : undefined,
          budget: b,
          workspace,
          start,
        });
      }
      oncreated?.(detail.mission.id);
      onclose?.();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="overlay" role="presentation" onclick={(e) => { if (e.target === e.currentTarget && !busy) onclose?.(); }}>
  <div class="dialog" role="dialog" aria-modal="true" aria-labelledby="mission-create-title">
    <header class="head">
      <h2 id="mission-create-title">{$t(fromChat ? "mission.create.title_chat" : "mission.create.title")}</h2>
      <button type="button" class="close" aria-label={$t("mission.close")} onclick={() => onclose?.()}>×</button>
    </header>
    <div class="body">
      <p class="hint">{$t("mission.create.lede")}</p>

      <label class="field">
        <span class="field-label">{$t("mission.create.objective")}</span>
        <textarea class="input" rows="3" bind:value={objective} placeholder={$t("mission.create.objective_placeholder")}></textarea>
      </label>

      {#if fromChat}
        <p class="hint">
          {$t("mission.create.chat_bot", { name: agents.find((a) => a.id === chatBot)?.name ?? chatBot ?? "—" })}
          · {$t("mission.create.chat_workspace")}
        </p>
      {:else}
        <div class="seg" role="radiogroup" aria-label={$t("mission.create.who")}>
          <label><input type="radio" name="mission-target" value="bot" bind:group={target} /> {$t("mission.create.target_bot")}</label>
          <label><input type="radio" name="mission-target" value="group" bind:group={target} /> {$t("mission.create.target_group")}</label>
        </div>
        {#if target === "bot"}
          <label class="field">
            <span class="field-label">{$t("mission.create.bot")}</span>
            <select class="input" bind:value={botId}>
              {#each agents as a (a.id)}<option value={a.id}>{a.name}</option>{/each}
            </select>
          </label>
        {:else}
          <label class="field">
            <span class="field-label">{$t("mission.create.group")}</span>
            <select class="input" bind:value={roomId}>
              <option value="">—</option>
              {#each rooms as r (r.id)}<option value={r.id}>{r.title}</option>{/each}
            </select>
          </label>
          {#if rooms.length === 0}<p class="hint">{$t("mission.create.no_groups")}</p>{/if}
          <div class="actions">
            <button type="button" class="button" disabled={!roomId || !objective.trim() || planning} onclick={plan}>
              {$t(planning ? "mission.create.planning" : "mission.create.plan")}
            </button>
            <span class="hint">{$t("mission.create.plan_hint")}</span>
          </div>
          {#if tasks.length}
            <ol class="tasks">
              {#each tasks as task, i (i)}
                <li>
                  <span>{task.title || task.input}</span>
                  {#if task.bot_id}<span class="tag">{agents.find((a) => a.id === task.bot_id)?.name ?? task.bot_id}</span>{/if}
                  <button type="button" class="button small" onclick={() => (tasks = tasks.filter((_, j) => j !== i))}>{$t("mission.create.remove_task")}</button>
                </li>
              {/each}
            </ol>
          {/if}
        {/if}
      {/if}

      <h3>{$t("mission.criteria.heading")}</h3>
      {#if presets.length}
        <div class="actions">
          <select class="input" bind:value={presetId} aria-label={$t("mission.create.preset")}>
            <option value="">{$t("mission.create.preset")}</option>
            {#each presets as p (p.id)}{@const tk = presetTextKey(p.id, "title")}<option value={p.id}>{tk ? $t(tk) : p.title}</option>{/each}
          </select>
          <button type="button" class="button" disabled={!preset} onclick={applyPresetCriteria}>{$t("mission.create.preset_apply")}</button>
        </div>
        {#if preset}
          {@const dk = presetTextKey(preset.id, "description")}
          <p class="hint">{dk ? $t(dk) : preset.description}</p>
          {#if preset.workspace === "project" && !workspace && !fromChat}<p class="hint">{$t("mission.create.preset_needs_folder")}</p>{/if}
        {/if}
      {/if}
      <CriteriaEditor bind:criteria disabled={busy} />

      {#if !fromChat}
        <div class="field">
          <span class="field-label">{$t("mission.create.workspace")}</span>
          <div class="actions">
            <input class="input grow" type="text" bind:value={workspaceText} title={workspaceText} placeholder={$t("mission.path.folder_placeholder")} aria-label={$t("mission.create.workspace")} autocomplete="off" spellcheck={false} />
            <button type="button" class="button" onclick={pickWorkspace}>{$t("mission.create.pick_folder")}</button>
            {#if workspace}
              <button type="button" class="button" onclick={() => (workspaceText = "")}>{$t("mission.create.clear")}</button>
            {/if}
          </div>
          {#if !workspace}<span class="hint">{$t("mission.create.no_workspace")}</span>{/if}
        </div>
      {/if}

      <details class="more">
        <summary>{$t("mission.create.budget")}</summary>
        <p class="hint">{$t("mission.create.budget_hint")}</p>
        <div class="grid">
          <label class="field"><span class="field-label">{$t("mission.create.budget_usd")}</span><input class="input" inputmode="decimal" bind:value={usd} placeholder="—" /></label>
          <label class="field"><span class="field-label">{$t("mission.create.budget_tokens")}</span><input class="input" inputmode="numeric" bind:value={tokens} placeholder="—" /></label>
          <label class="field"><span class="field-label">{$t("mission.create.budget_turns")}</span><input class="input" inputmode="numeric" bind:value={turns} placeholder="—" /></label>
          <label class="field"><span class="field-label">{$t("mission.create.budget_minutes")}</span><input class="input" inputmode="numeric" bind:value={minutes} placeholder="—" /></label>
        </div>
      </details>

      <label class="check"><input type="checkbox" bind:checked={start} /> <span>{$t("mission.create.start_now")}</span></label>

      {#if errorKey}<p class="error" role="alert">{$t(errorKey)}</p>{/if}
      {#if error}<p class="error" role="alert">{error}</p>{/if}
    </div>
    <footer class="foot">
      <button type="button" class="button" onclick={() => onclose?.()} disabled={busy}>{$t("mission.cancel")}</button>
      <button type="button" class="button active" onclick={submit} disabled={busy}>
        {$t(busy ? "mission.create.creating" : start ? "mission.create.submit_start" : "mission.create.submit_draft")}
      </button>
    </footer>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; background: rgba(0, 0, 0, 0.5); display: grid; place-items: center; z-index: 900; }
  .dialog { width: min(720px, 94vw); max-height: 90vh; display: flex; flex-direction: column; background: var(--surface); border-radius: var(--radius-lg); box-shadow: inset 0 0 0 var(--hairline) var(--content-border); }
  .head { display: flex; align-items: center; justify-content: space-between; padding: var(--space-3) var(--space-4); }
  .head h2 { margin: 0; font-size: var(--text-md); font-weight: 600; }
  h3 { margin: var(--space-2) 0 0; font-size: var(--text-base); font-weight: 600; }
  .close { background: transparent; border: 0; color: var(--text-dim); font-size: 20px; line-height: 1; cursor: pointer; }
  .body { padding: 0 var(--space-4) var(--space-3); display: flex; flex-direction: column; gap: var(--space-3); overflow: auto; min-height: 0; }
  .foot { padding: var(--space-3) var(--space-4); display: flex; justify-content: flex-end; gap: var(--space-2); border-top: var(--hairline) solid var(--separator); }
  .field { display: flex; flex-direction: column; gap: 4px; min-width: 0; }
  textarea.input { resize: vertical; height: auto; font-family: inherit; }
  .hint { margin: 0; font-size: var(--text-sm); color: var(--text-dim); }
  .error { margin: 0; font-size: var(--text-sm); color: var(--red); overflow-wrap: anywhere; }
  .seg { display: flex; gap: var(--space-4); font-size: var(--text-sm); }
  .seg label, .check { display: inline-flex; gap: 6px; align-items: center; font-size: var(--text-sm); }
  .actions { display: flex; flex-wrap: wrap; gap: var(--space-2); align-items: center; min-width: 0; }
  .tasks { margin: 0; padding-left: 20px; display: flex; flex-direction: column; gap: 4px; font-size: var(--text-sm); }
  .tasks li span { margin-right: 6px; }
  .tag { padding: 0 6px; border-radius: 999px; font-size: var(--text-xs); color: var(--text-muted); background: var(--fill-2); }
  .grow { flex: 1 1 240px; min-width: 0; font-family: var(--font-mono); font-size: var(--text-xs); text-overflow: ellipsis; }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(140px, 1fr)); gap: var(--space-2); margin-top: var(--space-2); }
  .more summary { cursor: pointer; font-size: var(--text-sm); color: var(--accent-text); }
  summary:focus-visible, .button:focus-visible, .close:focus-visible { outline: var(--focus-ring); outline-offset: 2px; }
</style>
