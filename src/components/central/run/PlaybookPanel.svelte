<script lang="ts">
  /**
   * Playbooks in /llm/jobs: a workflow as a chain of jobs (step N gets what
   * steps 1..N-1 reported). Build one by hand or from a catalog workflow id,
   * each step with its own agent (role) and runner; follow the runs live.
   */
  import { onMount } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import ProjectPicker from "$components/central/catalog/ProjectPicker.svelte";
  import { errText } from "$lib/central/catalog";
  import RunnerPicker from "./RunnerPicker.svelte";
  import {
    agentLabel,
    runPlaybookCancel,
    runPlaybookDelete,
    runPlaybookPrepare,
    runPlaybooksList,
    runPlaybookStart,
    type PlaybookRun,
    type PlaybookStep,
    type RunnerSpec,
  } from "./run-api";

  let { agentNames = {} }: { agentNames?: Record<string, string> } = $props();

  let runs = $state<PlaybookRun[]>([]);
  let openId = $state<string | null>(null);

  let name = $state("");
  let description = $state("");
  let runner = $state<RunnerSpec>({ kind: "agent", id: "omni" });
  let workspace = $state<string | null>(null);
  let task = $state("");
  let workflowId = $state("");
  let steps = $state<PlaybookStep[]>([
    { name: "plan", prompt: "", agent: null },
    { name: "build", prompt: "", agent: null },
  ]);
  let setup = $state<string[]>([]);
  let source = $state<string | null>(null);
  let busy = $state(false);
  let error = $state("");

  onMount(() => {
    void reload();
    const un = listen<PlaybookRun>("llm://playbook", (e) => {
      const p = e.payload;
      const i = runs.findIndex((r) => r.id === p.id);
      runs = i >= 0 ? runs.map((r) => (r.id === p.id ? p : r)) : [p, ...runs];
    });
    return () => void un.then((f) => f());
  });

  async function reload() {
    try {
      runs = await runPlaybooksList();
    } catch {
      runs = [];
    }
  }

  async function fromWorkflow() {
    if (!workflowId.trim()) return;
    busy = true;
    error = "";
    try {
      const p = await runPlaybookPrepare({ itemId: workflowId.trim(), runnerSpec: runner, workspace });
      name = p.name;
      description = p.description;
      steps = p.steps;
      setup = p.setup;
      source = p.source ?? null;
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  function addStep() {
    steps = [...steps, { name: `step ${steps.length + 1}`, prompt: "", agent: null }];
  }

  function removeStep(i: number) {
    steps = steps.filter((_, j) => j !== i);
  }

  async function start() {
    busy = true;
    error = "";
    try {
      const run = await runPlaybookStart({
        playbook: {
          name: name.trim() || "playbook",
          description: description.trim(),
          source,
          runner,
          workspace,
          steps: steps.map((s) => ({ ...s, agent: s.agent?.trim() || null, command: s.command?.trim() || null })),
          setup,
        },
        task: task.trim() || null,
        targets: null,
      });
      showToast("success", $t("llm.central.run.playbook.started", { name: run.name }));
      openId = run.id;
      await reload();
    } catch (e) {
      error = errText(e);
    } finally {
      busy = false;
    }
  }

  function pill(state: string): string {
    if (state === "running") return "blue";
    if (state === "done") return "green";
    if (state === "failed") return "red";
    if (state === "cancelled" || state === "skipped") return "";
    return "";
  }
</script>

<section class="stack">
  <h2 class="section-header-title">{$t("llm.central.run.playbook.title")}</h2>
  <p class="hint">{$t("llm.central.run.playbook.lede")}</p>

  {#if runs.length}
    <div class="rows">
      {#each runs as r (r.id)}
        <div class="row" class:open={openId === r.id}>
          <button type="button" class="row-head" aria-expanded={openId === r.id} onclick={() => (openId = openId === r.id ? null : r.id)}>
            <span class="pill {pill(r.state)}">{$t(`llm.central.run.state.${r.state}`)}</span>
            <span class="name">{r.name}</span>
            <span class="dim">{$t("llm.central.run.playbook.progress", { done: String(r.steps.filter((s) => s.state === "done").length), total: String(r.steps.length) })}</span>
            <span class="line dim">{r.error ?? ""}</span>
          </button>
          {#if openId === r.id}
            <ol class="steps">
              {#each r.steps as s, i (i)}
                <li>
                  <div class="step-head">
                    <span class="pill {pill(s.state)}">{$t(`llm.central.run.state.${s.state}`)}</span>
                    <strong>{s.name}</strong>
                    <span class="dim">{s.runner_label ?? agentLabel(s.agent_id, agentNames)}</span>
                    {#if s.job_id}<code class="dim">{s.job_id}</code>{/if}
                  </div>
                  {#if s.output}<pre class="pre">{s.output}</pre>{/if}
                </li>
              {/each}
            </ol>
            <div class="actions">
              {#if r.state === "running"}
                <button type="button" class="button" onclick={() => void runPlaybookCancel(r.id)}>{$t("llm.central.run.cancel")}</button>
              {:else}
                <button type="button" class="button" onclick={async () => { await runPlaybookDelete(r.id); await reload(); }}>{$t("llm.central.run.delete")}</button>
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  <details class="creation-panel">
    <summary>{$t("llm.central.run.playbook.new")}</summary>
    <div class="surface-card form">
      <div class="form-row">
        <label class="field grow">
          <span class="field-label">{$t("llm.central.run.playbook.from_workflow")}</span>
          <input class="input mono" type="text" placeholder="cct:workflows/…" bind:value={workflowId} />
        </label>
        <button type="button" class="button" disabled={busy || !workflowId.trim()} onclick={fromWorkflow}>{$t("llm.central.run.playbook.load")}</button>
      </div>
      <div class="form-row">
        <label class="field grow">
          <span class="field-label">{$t("llm.central.run.playbook.name")}</span>
          <input class="input" type="text" bind:value={name} />
        </label>
      </div>
      <label class="field">
        <span class="field-label">{$t("llm.central.run.playbook.goal")}</span>
        <input class="input" type="text" bind:value={description} />
      </label>
      <RunnerPicker bind:value={runner} />
      <div class="field">
        <span class="field-label">{$t("llm.central.run.project")}</span>
        <ProjectPicker bind:value={workspace} allowNone />
      </div>
      <div class="field-label">{$t("llm.central.run.playbook.steps")}</div>
      {#each steps as s, i (i)}
        <div class="step-edit">
          <div class="form-row">
            <span class="dim">#{i + 1}</span>
            <input class="input" type="text" bind:value={s.name} aria-label={$t("llm.central.run.playbook.step_name")} />
            <input class="input mono grow" type="text" bind:value={s.agent} placeholder={$t("llm.central.run.playbook.step_agent")} />
            <button type="button" class="button" onclick={() => removeStep(i)} disabled={steps.length <= 1}>✕</button>
          </div>
          <textarea class="input" rows="2" bind:value={s.prompt} placeholder={$t("llm.central.run.playbook.step_prompt")}></textarea>
        </div>
      {/each}
      <div><button type="button" class="button" onclick={addStep}>{$t("llm.central.run.playbook.add_step")}</button></div>
      <label class="field">
        <span class="field-label">{$t("llm.central.run.playbook.task")}</span>
        <textarea class="input" rows="2" bind:value={task}></textarea>
      </label>
      {#if setup.length}<p class="hint">{$t("llm.central.run.playbook.setup", { list: setup.join(", ") })}</p>{/if}
      {#if error}<p class="err">{error}</p>{/if}
      <div>
        <button type="button" class="button active" disabled={busy || steps.length === 0 || steps.some((s) => !s.name.trim())} onclick={start}>
          {$t("llm.central.run.playbook.start")}
        </button>
      </div>
    </div>
  </details>
</section>

<style>
  .stack,
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin-bottom: var(--space-5);
  }
  .form {
    padding: var(--space-4);
    margin-bottom: 0;
  }
  .creation-panel summary {
    cursor: pointer;
    padding: 10px 14px;
    border: 1px solid var(--separator);
    border-radius: var(--radius-lg);
    font-weight: 600;
  }
  .creation-panel[open] summary {
    margin-bottom: 12px;
  }
  .form-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: flex-end;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .grow {
    flex: 1;
    min-width: 200px;
  }
  .mono {
    font-family: var(--font-mono);
  }
  textarea.input {
    resize: vertical;
    height: auto;
    font-family: inherit;
  }
  .step-edit {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--fill-1);
  }
  .hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .err {
    margin: 0;
    color: var(--red);
    font-size: var(--text-sm);
  }
  .rows {
    display: flex;
    flex-direction: column;
    border: var(--hairline) solid var(--separator);
    border-radius: var(--radius-md);
    overflow: hidden;
  }
  .row + .row {
    border-top: var(--hairline) solid var(--separator);
  }
  .row.open {
    background: var(--fill-1);
  }
  .row-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    padding: var(--space-2) var(--space-3);
    font-size: var(--text-sm);
    text-align: left;
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
  }
  .name {
    font-weight: 600;
  }
  .dim {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .line {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pill {
    flex-shrink: 0;
    padding: 1px 8px;
    border-radius: 999px;
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-muted);
    background: var(--fill-2);
  }
  .pill.blue {
    color: var(--blue);
    background: color-mix(in srgb, var(--blue) 14%, transparent);
  }
  .pill.green {
    color: var(--green);
    background: color-mix(in srgb, var(--green) 14%, transparent);
  }
  .pill.red {
    color: var(--red);
    background: color-mix(in srgb, var(--red) 14%, transparent);
  }
  .steps {
    margin: 0;
    padding: var(--space-2) var(--space-3) var(--space-2) var(--space-6);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .step-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .pre {
    margin: 4px 0 0;
    padding: var(--space-2);
    max-height: 220px;
    overflow: auto;
    border-radius: var(--radius-sm);
    background: var(--fill-2);
    font-size: var(--text-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }
  .actions {
    display: flex;
    gap: var(--space-2);
    padding: 0 var(--space-3) var(--space-3);
  }
</style>
