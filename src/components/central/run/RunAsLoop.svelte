<script lang="ts">
  /**
   * "Run as an OmniGet Loop" for a catalog loop: pick the runner (roster
   * agent, Claude/Codex account, ACP agent or a coding CLI), the project, where
   * the loop's referenced components get installed, then the interval, stop
   * check and budget (rounds, minutes, cost) the Loops engine enforces.
   */
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import ProjectPicker from "$components/central/catalog/ProjectPicker.svelte";
  import PlanDialog from "$components/central/catalog/PlanDialog.svelte";
  import { agentkitDetect, errText, type DetectedTarget, type InstallPlan } from "$lib/central/catalog";
  import RunDialog from "./RunDialog.svelte";
  import RunnerPicker from "./RunnerPicker.svelte";
  import {
    runLoopPrepare,
    runLoopStart,
    scheduleValid,
    type LoopPlan,
    type RunnerSpec,
  } from "./run-api";

  let {
    itemId,
    itemName = "",
    workspace: initialWorkspace = null,
    label = "",
    onstarted,
  }: {
    itemId: string;
    itemName?: string;
    workspace?: string | null;
    label?: string;
    onstarted?: (loopId: string) => void;
  } = $props();

  let open = $state(false);
  let runner = $state<RunnerSpec>({ kind: "agent", id: "omni" });
  let workspace = $state<string | null>(null);
  let tools = $state<DetectedTarget[]>([]);
  let targets = $state<string[]>([]);
  let prepared = $state<{ loop: LoopPlan; plan: InstallPlan | null } | null>(null);
  let installed = $state(false);
  let showPlan = $state(false);
  let busy = $state(false);
  let error = $state("");

  // editable loop fields
  let name = $state("");
  let prompt = $state("");
  let schedule = $state("");
  let check = $state("");
  let maxRounds = $state<number | null>(10);
  let maxMinutes = $state<number | null>(60);
  let maxCost = $state<number | null>(null);

  async function show() {
    open = true;
    workspace = initialWorkspace;
    prepared = null;
    error = "";
    await loadTools();
  }

  async function loadTools() {
    try {
      tools = (await agentkitDetect(workspace)).filter((d) => d.installed);
      targets = tools.filter((d) => d.enabled_by_default).map((d) => d.id);
    } catch (e) {
      tools = [];
    }
  }

  function toggle(id: string) {
    targets = targets.includes(id) ? targets.filter((x) => x !== id) : [...targets, id];
  }

  async function prepare() {
    busy = true;
    error = "";
    try {
      prepared = await runLoopPrepare({ itemId, runnerSpec: runner, workspace, targets });
      installed = !prepared.plan;
      const l = prepared.loop;
      name = l.name;
      prompt = l.prompt;
      schedule = l.interval && l.interval !== "on-demand" ? l.interval : "";
      check = l.check_command ?? "";
      maxRounds = l.max_rounds;
      maxMinutes = l.max_minutes;
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  let units = $derived(prepared?.plan?.units ?? []);
  let newUnits = $derived(units.filter((u) => u.status === "new" || u.status === "update").length);

  async function start() {
    if (!prepared) return;
    busy = true;
    error = "";
    try {
      const l = await runLoopStart({
        runner,
        name: name.trim() || prepared.loop.name,
        prompt: prompt.trim(),
        workspace,
        schedule: schedule.trim() || null,
        checkCommand: check.trim() || null,
        maxRounds: maxRounds && maxRounds > 0 ? Math.floor(maxRounds) : null,
        maxMinutes: maxMinutes && maxMinutes > 0 ? Math.floor(maxMinutes) : null,
        maxCostUsd: maxCost && maxCost > 0 ? maxCost : null,
        source: itemId,
        planId: installed ? null : (prepared.plan?.id ?? null),
      });
      showToast("success", $t("llm.central.run.loop.started", { name: l.name }));
      onstarted?.(l.id);
      open = false;
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }
</script>

<button type="button" class="button" onclick={show}>{label || $t("llm.central.agentkit.loop.run")}</button>

{#if open}
  <RunDialog title={$t("llm.central.run.loop.title")} subtitle={itemName || itemId} {busy} onclose={() => (open = false)}>
    <RunnerPicker bind:value={runner} />
    <div class="field">
      <span class="field-label">{$t("llm.central.run.project")}</span>
      <ProjectPicker bind:value={workspace} allowNone onchange={() => void loadTools()} />
      {#if !workspace}<p class="hint">{$t("llm.central.run.loop.no_project")}</p>{/if}
    </div>
    {#if tools.length}
      <div class="field">
        <span class="field-label">{$t("llm.central.run.loop.install_into")}</span>
        <div class="chips">
          {#each tools as d (d.id)}
            <label class="chip"><input type="checkbox" checked={targets.includes(d.id)} onchange={() => toggle(d.id)} /> {d.name}</label>
          {/each}
        </div>
        <p class="hint">{$t("llm.central.run.loop.install_hint")}</p>
      </div>
    {/if}

    {#if !prepared}
      <div class="row-end">
        <button type="button" class="button active" disabled={busy} onclick={prepare}>{$t("llm.central.run.loop.prepare")}</button>
      </div>
    {:else}
      <hr />
      <label class="field">
        <span class="field-label">{$t("llm.central.run.loop.name")}</span>
        <input class="input" type="text" bind:value={name} />
      </label>
      <label class="field">
        <span class="field-label">{$t("llm.central.run.loop.prompt")}</span>
        <textarea class="input" rows="5" bind:value={prompt}></textarea>
      </label>
      <div class="grid">
        <label class="field">
          <span class="field-label">{$t("llm.central.run.loop.schedule")}</span>
          <input class="input mono" class:bad={!scheduleValid(schedule)} type="text" placeholder="10m · daily · */30 * * * *" bind:value={schedule} />
        </label>
        <label class="field">
          <span class="field-label">{$t("llm.central.run.loop.check")}</span>
          <input class="input mono" type="text" placeholder="npm test" bind:value={check} />
        </label>
        <label class="field">
          <span class="field-label">{$t("llm.central.run.loop.max_rounds")}</span>
          <input class="input" type="number" min="1" bind:value={maxRounds} />
        </label>
        <label class="field">
          <span class="field-label">{$t("llm.central.run.loop.max_minutes")}</span>
          <input class="input" type="number" min="1" bind:value={maxMinutes} />
        </label>
        <label class="field">
          <span class="field-label">{$t("llm.central.run.loop.max_cost")}</span>
          <input class="input" type="number" min="0" step="0.5" placeholder="US$" bind:value={maxCost} />
        </label>
      </div>
      <p class="hint">{schedule.trim() ? $t("llm.central.run.loop.schedule_hint") : $t("llm.central.run.loop.back_to_back")}</p>
      {#if !check.trim()}<p class="hint">{$t("llm.central.run.loop.no_check")}</p>{/if}
      {#if prepared.loop.stop_condition}
        <p class="hint"><strong>{$t("llm.central.run.loop.stop_condition")}:</strong> {prepared.loop.stop_condition}</p>
      {/if}
      {#if prepared.loop.budget}
        <details><summary class="hint">{$t("llm.central.run.loop.budget_text")}</summary><pre class="pre">{prepared.loop.budget}</pre></details>
      {/if}
      {#if prepared.loop.components.length}
        <div class="field">
          <span class="field-label">{$t("llm.central.run.loop.components")}</span>
          <div class="chips">{#each prepared.loop.components as c (c)}<code class="chip mono">{c}</code>{/each}</div>
        </div>
      {/if}
      {#if prepared.plan}
        <p class="hint">
          {installed
            ? $t("llm.central.run.loop.installed")
            : $t("llm.central.run.loop.plan_summary", { units: String(newUnits), files: String(prepared.plan.files.filter((f) => f.action !== "unchanged").length) })}
          <button type="button" class="link" onclick={() => (showPlan = true)}>{$t("llm.central.run.loop.see_plan")}</button>
        </p>
      {/if}
    {/if}
    {#if error}<p class="err">{error}</p>{/if}
    {#snippet footer()}
      {#if prepared}
        <button type="button" class="button" disabled={busy} onclick={() => (prepared = null)}>{$t("llm.central.run.back")}</button>
        <button type="button" class="button active" disabled={busy || !prompt.trim() || !scheduleValid(schedule)} onclick={start}>
          {installed || !prepared.plan ? $t("llm.central.run.loop.start") : $t("llm.central.run.loop.install_start")}
        </button>
      {/if}
    {/snippet}
  </RunDialog>
{/if}

{#if showPlan && prepared?.plan}
  <PlanDialog plan={prepared.plan} onclose={() => (showPlan = false)} onapplied={() => (installed = true)} />
{/if}

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: var(--space-3);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border-radius: var(--radius-full);
    background: var(--fill-1);
    font-size: var(--text-xs);
  }
  .mono {
    font-family: var(--font-mono);
  }
  .bad {
    outline: 1px solid var(--warning);
  }
  .hint {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
  }
  .err {
    margin: 0;
    color: var(--red);
    font-size: var(--text-sm);
  }
  .row-end {
    display: flex;
    justify-content: flex-end;
  }
  textarea.input {
    resize: vertical;
    height: auto;
    font-family: inherit;
  }
  .pre {
    white-space: pre-wrap;
    font-size: var(--text-xs);
    background: var(--fill-1);
    padding: var(--space-2);
    border-radius: var(--radius-sm);
  }
  .link {
    border: 0;
    background: none;
    color: var(--accent);
    cursor: pointer;
    padding: 0 0 0 4px;
    font-size: inherit;
  }
  hr {
    border: 0;
    border-top: var(--hairline) solid var(--separator);
    width: 100%;
    margin: 0;
  }
</style>
