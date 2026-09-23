<script lang="ts">
  import SurfaceGuide from "$components/llm/SurfaceGuide.svelte";
  import { surfaceCopy } from "$components/llm/surface-copy";
  /**
   * Loops: the same prompt run round after round until a check command
   * passes or the round/minute budget runs out.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { getAgents, loadRoster } from "$lib/stores/llm-store.svelte";
  import {
    cancelLoop,
    createLoop,
    deleteLoop,
    duration,
    getJobs,
    getLoops,
    initJobsStore,
    pickFolder,
    pickState,
    shortTime,
    type LoopDef,
  } from "$lib/stores/llm-jobs-store.svelte";
  import CatalogLoops from "$components/central/run/CatalogLoops.svelte";
  import { agentLabel } from "$components/central/run/run-api";
  import { itemHref } from "$lib/central/catalog";

  /** Fields the catalog runs add to a Loop (schedule, cost budget, source). */
  type LoopExtra = LoopDef & {
    schedule?: string | null;
    max_cost_usd?: number | null;
    spent_usd?: number;
    source?: string | null;
    next_round_ms?: number | null;
  };
  function extra(l: LoopDef): LoopExtra {
    return l as LoopExtra;
  }

  let agents = $derived(getAgents());
  let loops = $derived(getLoops());
  let jobs = $derived(getJobs());

  let agentId = $state("");
  let name = $state("");
  let prompt = $state("");
  let workspace = $state<string | null>(null);
  let reviewing = $state(false);
  let maxRounds = $state<number | null>(10);
  let maxMinutes = $state<number | null>(null);
  let checkCommand = $state("");
  let creating = $state(false);
  let openId = $state<string | null>(null);
  /** Re-read on each store change, enough for a quiet elapsed column. */
  let now = $state(Date.now());

  onMount(() => {
    initJobsStore();
    void loadRoster();
    const timer = setInterval(() => (now = Date.now()), 15000);
    return () => clearInterval(timer);
  });

  $effect(() => {
    if (!agentId && agents.length > 0) agentId = agents[0].id;
  });

  function agentName(id: string): string {
    if (id.startsWith("tool:")) return agentLabel(id);
    return agents.find((a) => a.id === id)?.name ?? id;
  }

  function positive(n: number | null): number | null {
    return typeof n === "number" && n > 0 ? Math.floor(n) : null;
  }

  function reason(loop: LoopDef): string {
    return loop.stop_reason === "check_passed" ? $t("llm.loops.check_passed") : loop.stop_reason ?? "";
  }

  async function create() {
    if (!agentId || !prompt.trim()) return;
    creating = true;
    const loop = await createLoop({
      agent_id: agentId,
      prompt: prompt.trim(),
      name: name.trim() || undefined,
      workspace,
      max_rounds: positive(maxRounds) ?? 10,
      max_minutes: positive(maxMinutes),
      check_command: checkCommand.trim() || null,
    });
    creating = false;
    if (loop) {
      reviewing = false;
      prompt = "";
      name = "";
    }
  }
</script>

<svelte:head><title>{$t("llm.tab.loops")}</title></svelte:head>

<div class="page page-wide loops-page">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$surfaceCopy.routines}</h1>
      <p class="page-lede">{$t("llm.loops.lede")}</p>
    </div>
  </header>
  <SurfaceGuide text={$surfaceCopy.loopsHint} href="/help?article=routines#guide" />
  <CatalogLoops />

  <details class="creation-panel"><summary>{$surfaceCopy.createRoutine}</summary>
  <section class="surface-card form">
    <h2 class="section-header-title">{$t("llm.loops.new_title")}</h2>
    <div class="form-row">
      <label class="field">
        <span class="field-label">{$t("llm.loops.agent")}</span>
        <select class="input" bind:value={agentId}>
          {#each agents as a (a.id)}
            <option value={a.id}>{a.name}</option>
          {/each}
        </select>
      </label>
      <label class="field grow">
        <span class="field-label">{$t("llm.loops.name")}</span>
        <input class="input" type="text" bind:value={name} />
      </label>
    </div>
    <label class="field">
      <span class="field-label">{$t("llm.loops.prompt")}</span>
      <textarea class="input" rows="3" bind:value={prompt} placeholder={$t("llm.loops.prompt_placeholder")}></textarea>
    </label>
    <div class="folder">
      <button type="button" class="button" onclick={async () => (workspace = (await pickFolder()) ?? workspace)}>
        {$t("llm.loops.pick_folder")}
      </button>
      {#if workspace}
        <code class="path" title={workspace}>{workspace}</code>
        <button type="button" class="button" onclick={() => (workspace = null)}>{$t("llm.loops.clear")}</button>
      {:else}
        <span class="hint">{$t("llm.loops.workspace_none")}</span>
      {/if}
    </div>
    <div class="form-row">
      <label class="field small">
        <span class="field-label">{$t("llm.loops.max_rounds")}</span>
        <input class="input" type="number" min="1" placeholder="10" bind:value={maxRounds} />
      </label>
      <label class="field small">
        <span class="field-label">{$t("llm.loops.max_minutes")}</span>
        <input class="input" type="number" min="1" bind:value={maxMinutes} />
      </label>
      <label class="field grow">
        <span class="field-label">{$t("llm.loops.check_command")}</span>
        <input class="input mono" type="text" bind:value={checkCommand} placeholder="npm test" />
      </label>
    </div>
    <p class="hint">{$t("llm.loops.summary", { rounds: positive(maxRounds) ?? 10 })}</p>
    {#if !checkCommand.trim()}<p class="hint">{$t("llm.loops.no_check")}</p>{/if}
    {#if reviewing}
      <div class="loop-review" role="region" aria-label={$t("llm.loops.review")}>
        <strong>{agentName(agentId)} · {name || $t("llm.loops.new_title")}</strong>
        <p>{prompt}</p>
        {#if workspace}<p><code>{workspace}</code></p>{/if}
        {#if checkCommand.trim()}<p><code>{checkCommand}</code></p>{/if}
        {#if positive(maxMinutes)}<p>{$t("llm.loops.max_minutes")}: {positive(maxMinutes)}</p>{/if}
        <button type="button" class="button active" disabled={creating || !agentId || !prompt.trim()} onclick={create}>{$t("llm.loops.start")}</button>
        <button type="button" class="button" disabled={creating} onclick={() => reviewing = false}>{$t("llm.loops.edit")}</button>
      </div>
    {:else}
      <div><button type="button" class="button active" disabled={!agentId || !prompt.trim()} onclick={() => reviewing = true}>{$t("llm.loops.review")}</button></div>
    {/if}
  </section>
  </details>

  <section class="stack">
    <h2 class="section-header-title">{$t("llm.loops.list_title")}</h2>
    {#if loops.length === 0}
      <p class="hint">{$t("llm.loops.empty")}</p>
    {:else}
      <div class="rows">
        {#each loops as loop (loop.id)}
          {@const rounds = jobs.filter((j) => j.loop_id === loop.id)}
          <div class="row" class:open={openId === loop.id}>
            <button
              type="button"
              class="row-head"
              aria-expanded={openId === loop.id}
              onclick={() => (openId = openId === loop.id ? null : loop.id)}
            >
              <span class="pill {pickState(loop.state)}">{$t(`llm.loops.state.${loop.state}`)}</span>
              <span class="name">{loop.name || loop.id}</span>
              <span class="dim">{agentName(loop.agent_id)}</span>
              <span class="dim">{$t("llm.loops.rounds", { done: loop.rounds_done, max: loop.max_rounds ?? "∞" })}</span>
              {#if loop.finished_ms && reason(loop)}
                <span class="line dim">{reason(loop)}</span>
              {:else}
                <span class="line"></span>
              {/if}
              <span class="time">
                {shortTime(loop.created_ms)} · {duration(loop.created_ms, loop.finished_ms ?? now)}
              </span>
            </button>

            {#if openId === loop.id}
              <div class="detail">
                <div class="field-label">{$t("llm.loops.prompt")}</div>
                <div class="text">{loop.prompt}</div>
                {#if loop.workspace}
                  <div class="field-label">{$t("llm.loops.workspace")}</div>
                  <code class="path">{loop.workspace}</code>
                {/if}
                {#if loop.check_command}
                  <div class="field-label">{$t("llm.loops.check_command")}</div>
                  <code class="path">{loop.check_command}</code>
                {/if}
                {#if extra(loop).schedule}
                  <div class="field-label">{$t("llm.central.run.loop.schedule")}</div>
                  <code class="path">{extra(loop).schedule}{extra(loop).next_round_ms ? ` · ${$t("llm.central.run.loop.next_round")} ${shortTime(extra(loop).next_round_ms ?? null)}` : ""}</code>
                {/if}
                {#if extra(loop).max_cost_usd || extra(loop).spent_usd}
                  <div class="field-label">{$t("llm.central.run.loop.cost")}</div>
                  <span class="text">US$ {(extra(loop).spent_usd ?? 0).toFixed(4)}{extra(loop).max_cost_usd ? ` / ${extra(loop).max_cost_usd}` : ""}</span>
                {/if}
                {#if extra(loop).source}
                  <div class="field-label">{$t("llm.central.run.loop.source")}</div>
                  <a class="path" href={itemHref(extra(loop).source ?? "")}>{extra(loop).source}</a>
                {/if}
                {#if loop.last_check}
                  <div class="field-label">{$t("llm.loops.last_check")}</div>
                  <pre class="pre">{loop.last_check}</pre>
                {/if}
                <div class="field-label">{$t("llm.loops.jobs")}</div>
                {#if rounds.length === 0}
                  <p class="hint">{$t("llm.loops.jobs_empty")}</p>
                {:else}
                  <div class="round-list">
                    {#each rounds as job, i (job.id)}
                      <a class="round" href="/llm/jobs">
                        <span class="dim">#{rounds.length - i}</span>
                        <span class="pill {pickState(job.state)}">{$t(`llm.jobs.state.${job.state}`)}</span>
                      </a>
                    {/each}
                  </div>
                {/if}
                <div class="actions">
                  {#if loop.state === "running"}
                    <button type="button" class="button" onclick={() => void cancelLoop(loop.id)}>
                      {$t("llm.loops.cancel")}
                    </button>
                  {:else}
                    <button type="button" class="button" onclick={() => void deleteLoop(loop.id)}>
                      {$t("llm.loops.delete")}
                    </button>
                  {/if}
                </div>
              </div>
            {/if}
          </div>
        {/each}
      </div>
    {/if}
  </section>
</div>

<style>
  .creation-panel { margin-bottom:24px; }
  .creation-panel summary { cursor:pointer; padding:14px 16px; border:1px solid var(--separator); border-radius:var(--radius-lg); color:var(--text); font-weight:600; }
  .creation-panel[open] summary { margin-bottom:12px; }
  .creation-panel summary:focus-visible { outline:var(--focus-ring); }
  .loop-review { border: 1px solid var(--border); border-radius: var(--radius-md); background: var(--surface-mut); padding: var(--space-4); overflow-wrap: anywhere; }
  .loops-page {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .stack,
  .form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin-bottom: var(--space-5);
  }

  .form {
    padding: var(--space-4);
  }

  .form-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    align-items: flex-end;
  }

  .grow {
    flex: 1;
    min-width: 200px;
  }

  .small {
    width: 120px;
  }

  textarea.input {
    resize: vertical;
    height: auto;
    font-family: inherit;
  }

  .folder,
  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .path {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }

  .mono {
    font-family: var(--font-mono);
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

  .row-head:hover {
    background: var(--fill-1);
  }

  .name {
    font-weight: 600;
    flex-shrink: 0;
  }

  .dim {
    flex-shrink: 0;
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

  .line.dim {
    flex-shrink: 1;
  }

  .time {
    flex-shrink: 0;
    margin-left: auto;
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .pill {
    flex-shrink: 0;
    padding: 1px 8px;
    border-radius: 999px;
    font-size: var(--text-xs);
    font-weight: 600;
    white-space: nowrap;
    color: var(--text-muted);
    background: var(--fill-2);
  }

  .pill.blue {
    color: var(--blue);
    background: color-mix(in srgb, var(--blue) 14%, transparent);
  }

  .pill.orange {
    color: var(--orange);
    background: color-mix(in srgb, var(--orange) 16%, transparent);
  }

  .pill.green {
    color: var(--green);
    background: color-mix(in srgb, var(--green) 14%, transparent);
  }

  .pill.red {
    color: var(--red);
    background: color-mix(in srgb, var(--red) 14%, transparent);
  }

  .detail {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3) var(--space-3);
  }

  .text {
    font-size: var(--text-sm);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .round-list {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .round {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    text-decoration: none;
  }

  .pre {
    margin: 0;
    padding: var(--space-2);
    max-height: 320px;
    overflow: auto;
    border-radius: var(--radius-sm);
    background: var(--fill-2);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }
</style>
