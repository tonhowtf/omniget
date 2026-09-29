<script lang="ts">
  import SurfaceGuide from "$components/llm/SurfaceGuide.svelte";
  import RunModeSelect from "$components/llm/RunModeSelect.svelte";
  import type { RunMode } from "$lib/stores/llm-run-mode.svelte";
  /**
   * Loops: the same prompt run round after round until a check command
   * passes or the round/minute budget runs out. Without a check command an
   * independent auditor (the agent's small model) judges each round (3.7).
   */
  import { onMount } from "svelte";
  import { page } from "$app/state";
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
    resumeLoop,
    settleLoop,
    shortTime,
    auditOverhead,
    type LoopAudit,
    type LoopDef,
  } from "$lib/stores/llm-jobs-store.svelte";
  import CatalogLoops from "$components/central/run/CatalogLoops.svelte";
  import { agentLabel } from "$components/central/run/run-api";
  import { itemHref } from "$lib/central/catalog";

  /** Fields the catalog runs add to a Loop (schedule, source). */
  type LoopExtra = LoopDef & {
    schedule?: string | null;
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
  /** A cost ceiling (US$) pauses the loop with a budget incident (3.3). */
  let maxCost = $state<number | null>(null);
  let checkCommand = $state("");
  /** Resume by itself after a restart (3.1); unchecked = the setting. */
  let autoResume = $state(false);
  let tokenBudget = $state<number | null>(null);
  // A loop made here is the person's own: the chat's default (2.7, D2).
  let runMode = $state<RunMode>("auto_edits");
  let creating = $state(false);
  let openId = $state<string | null>(null);
  // `?loop=<id>` (a row of /llm/work) opens that loop.
  let linkedLoop = $derived(page.url.searchParams.get("loop"));
  let followedLoop: string | null = null;
  $effect(() => {
    if (linkedLoop && linkedLoop !== followedLoop && loops.some((l) => l.id === linkedLoop)) {
      followedLoop = linkedLoop;
      openId = linkedLoop;
    }
  });
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

  /** Stop reasons the app names; anything else (an error) shows as it is. */
  const KNOWN_STOPS = ["audit_complete", "audit_blocked", "audit_unavailable", "audit_disagrees", "token_budget", "max_rounds"];

  function reason(loop: LoopDef): string {
    if (budgetIncident(loop)) return $t("llm.loops.paused_budget") as string;
    const r = loop.stop_reason ?? "";
    if (r === "check_passed") return $t("llm.loops.check_passed");
    if (KNOWN_STOPS.includes(r)) return $t(`llm.loops.stop.${r}`);
    if (r.startsWith("round_failed: ")) return $t("llm.loops.stop.round_failed", { error: r.slice(14) });
    return r;
  }

  function verdictClass(a: LoopAudit): string {
    switch (a.verdict) {
      case "complete":
        return "green";
      case "blocked":
        return "orange";
      case "continue":
        return "blue";
      default:
        return "red";
    }
  }

  function usd(n: number | null | undefined): string {
    return `US$ ${(n ?? 0).toFixed(4)}`;
  }

  /** The budget incident that paused the loop, if one did. */
  function budgetIncident(loop: LoopDef): string | null {
    return loop.state === "paused" && loop.stop_reason?.startsWith("budget:") ? loop.stop_reason.slice(7) : null;
  }

  /** What the rounds cost: dollars (marked when estimated), else weighted tokens (C5). */
  function cost(loop: LoopDef): string {
    const usd = loop.spent_usd ?? 0;
    const tokens = loop.spent_tokens ?? 0;
    const parts: string[] = [];
    if (usd > 0) {
      parts.push(`US$ ${usd.toFixed(4)}${(loop.spent_usd_estimated ?? 0) > 0 ? ` ${$t("llm.loops.cost_estimated")}` : ""}`);
    }
    if (tokens > 0) parts.push($t("llm.loops.cost_tokens", { tokens: tokens.toLocaleString() }) as string);
    if ((loop.unpriced_rounds ?? 0) > 0) parts.push($t("llm.loops.cost_unknown", { count: loop.unpriced_rounds ?? 0 }) as string);
    return parts.join(" · ");
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
      auto_resume: autoResume ? true : null,
      run_mode: runMode,
      max_cost_usd: typeof maxCost === "number" && maxCost > 0 ? maxCost : null,
      token_budget: positive(tokenBudget),
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
      <h1 class="page-title">{$t("llm.tab.loops")}</h1>
      <p class="page-lede">{$t("llm.loops.lede")}</p>
    </div>
  </header>
  <SurfaceGuide text={$t("llm.surface.loops_hint")} href="/help?article=routines#guide" />
  <CatalogLoops />

  <details class="creation-panel"><summary>{$t("llm.surface.create_loop")}</summary>
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
      <label class="field small">
        <span class="field-label">{$t("llm.loops.max_cost")}</span>
        <input class="input" type="number" min="0" step="any" placeholder="0.50" bind:value={maxCost} />
      </label>
      <label class="field grow">
        <span class="field-label">{$t("llm.loops.check_command")}</span>
        <input class="input mono" type="text" bind:value={checkCommand} placeholder="npm test" />
      </label>
      <label class="field small">
        <span class="field-label">{$t("llm.loops.token_budget")}</span>
        <input class="input" type="number" min="1" step="1000" bind:value={tokenBudget} />
      </label>
    </div>
    <label class="auto-resume">
      <input type="checkbox" bind:checked={autoResume} />
      <span>
        <span class="field-label">{$t("llm.loops.auto_resume")}</span>
        <span class="hint">{$t("llm.loops.auto_resume_hint")}</span>
      </span>
    </label>
    <div class="form-row">
      <RunModeSelect bind:value={runMode} />
    </div>
    <p class="hint">{$t("llm.loops.run_mode_hint")}</p>
    <p class="hint">{$t("llm.loops.summary", { rounds: positive(maxRounds) ?? 10 })}</p>
    {#if !checkCommand.trim()}<p class="hint">{$t("llm.loops.no_check")}</p>{/if}
    {#if reviewing}
      <div class="loop-review" role="region" aria-label={$t("llm.loops.review")}>
        <strong>{agentName(agentId)} · {name || $t("llm.loops.new_title")}</strong>
        <p>{prompt}</p>
        {#if workspace}<p><code>{workspace}</code></p>{/if}
        {#if checkCommand.trim()}<p><code>{checkCommand}</code></p>{/if}
        {#if positive(maxMinutes)}<p>{$t("llm.loops.max_minutes")}: {positive(maxMinutes)}</p>{/if}
        {#if typeof maxCost === "number" && maxCost > 0}<p>{$t("llm.loops.max_cost")}: US$ {maxCost}</p>{/if}
        {#if positive(tokenBudget)}<p>{$t("llm.loops.token_budget")}: {positive(tokenBudget)}</p>{/if}
        <p>{$t("llm.run_mode.label")}: {$t(`llm.run_mode.${runMode}`)}</p>
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
              <span class="pill {pickState(loop.state)}">{loop.state === "interrupted" ? $t("assist.runs.job_interrupted") : $t(`llm.loops.state.${loop.state}`)}</span>
              <span class="name">{loop.name || loop.id}</span>
              <span class="dim">{agentName(loop.agent_id)}</span>
              <span class="dim">{$t("llm.loops.rounds", { done: loop.rounds_done, max: loop.max_rounds ?? "∞" })}</span>
              {#if cost(loop)}<span class="dim">{cost(loop)}</span>{/if}
              {#if (loop.finished_ms || loop.state === "paused") && reason(loop)}
                <span class="line dim">{reason(loop)}</span>
              {:else if loop.note}
                <span class="line dim" title={$t("llm.loops.note")}>{loop.note}</span>
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
                <div class="field-label">{$t("llm.run_mode.label")}</div>
                <div class="text" title={$t(`llm.run_mode.${loop.run_mode ?? "auto_edits"}_hint`) as string}>{$t(`llm.run_mode.${loop.run_mode ?? "auto_edits"}`)}</div>
                {#if extra(loop).schedule}
                  <div class="field-label">{$t("llm.central.run.loop.schedule")}</div>
                  <code class="path">{extra(loop).schedule}{extra(loop).next_round_ms ? ` · ${$t("llm.central.run.loop.next_round")} ${shortTime(extra(loop).next_round_ms ?? null)}` : ""}</code>
                {/if}
                {#if loop.max_cost_usd || cost(loop)}
                  <div class="field-label">{$t("llm.central.run.loop.cost")}</div>
                  <span class="text">{cost(loop) || "US$ 0.0000"}{loop.max_cost_usd ? ` / US$ ${loop.max_cost_usd}` : ""}</span>
                {/if}
                {#if extra(loop).source}
                  <div class="field-label">{$t("llm.central.run.loop.source")}</div>
                  <a class="path" href={itemHref(extra(loop).source ?? "")}>{extra(loop).source}</a>
                {/if}
                {#if loop.last_check}
                  <div class="field-label">{$t("llm.loops.last_check")}</div>
                  <pre class="pre">{loop.last_check}</pre>
                {/if}
                {#if loop.token_budget || loop.tokens_used}
                  <div class="field-label">{$t("llm.loops.token_budget")}</div>
                  <span class="text">{$t("llm.loops.tokens", { used: loop.tokens_used ?? 0, budget: loop.token_budget ?? "∞" })}</span>
                {/if}
                {#if loop.rounds_guided}
                  <p class="hint">{$t("llm.loops.guided", { count: loop.rounds_guided })}</p>
                {/if}
                {#if loop.audits && loop.audits.length > 0}
                  {@const cost = auditOverhead(loop.audits)}
                  <div class="field-label">{$t("llm.loops.audits")}</div>
                  <ol class="audit-list">
                    {#each loop.audits as a (a.round)}
                      <li class="audit">
                        <span class="dim">#{a.round}</span>
                        <span class="pill {verdictClass(a)}">{a.verdict ? $t(`llm.loops.verdict.${a.verdict}`) : $t("llm.loops.verdict.none")}</span>
                        <span class="audit-note">{a.note || a.error || ""}</span>
                        {#if a.claimed_done}
                          <span class="tag" title={$t("llm.loops.claimed_done_hint")}>{$t("llm.loops.claimed_done")}</span>
                        {/if}
                        <span class="dim" title={$t(`llm.loops.side_reason.${a.reason.replace("+fallback", "")}`)}>{a.model}</span>
                        {#if a.cost_usd != null}<span class="dim">{usd(a.cost_usd)}</span>{/if}
                      </li>
                    {/each}
                  </ol>
                  {#if cost.share != null}
                    <p class="hint">{$t("llm.loops.overhead", { audit: usd(cost.audit), rounds: usd(cost.rounds), percent: Math.round(cost.share * 100) })}</p>
                  {/if}
                {/if}
                <div class="field-label">{$t("llm.loops.jobs")}</div>
                {#if rounds.length === 0}
                  <p class="hint">{$t("llm.loops.jobs_empty")}</p>
                {:else}
                  <div class="round-list">
                    {#each rounds as job, i (job.id)}
                      <a class="round" href="/llm/jobs">
                        <span class="dim">#{rounds.length - i}</span>
                        <span class="pill {pickState(job.state)}">{job.state === "interrupted" ? $t("assist.runs.job_interrupted") : $t(`llm.jobs.state.${job.state}`)}</span>
                      </a>
                    {/each}
                  </div>
                {/if}
                {#if loop.state === "interrupted"}
                  <p class="hint">{$t("assist.runs.loop_interrupted_hint")}</p>
                {:else if loop.state === "blocked"}
                  <p class="hint">{$t("llm.loops.blocked_hint")}</p>
                {/if}
                {#if budgetIncident(loop)}
                  <p class="hint">
                    {$t("llm.loops.paused_budget_hint")}
                    <a href={`/llm/usage?incident=${encodeURIComponent(budgetIncident(loop) ?? "")}#budgets`}>{$t("llm.loops.open_incident")}</a>
                  </p>
                {/if}
                <div class="actions">
                  {#if loop.state === "interrupted"}
                    <button type="button" class="button active" onclick={() => void resumeLoop(loop.id)}>
                      {$t("assist.runs.action_resume")}
                    </button>
                    <button type="button" class="button" onclick={() => void settleLoop(loop.id, true)}>
                      {$t("assist.runs.action_mark_done")}
                    </button>
                    <button type="button" class="button" onclick={() => void settleLoop(loop.id, false)}>
                      {$t("assist.runs.action_discard")}
                    </button>
                  {:else if loop.state === "blocked"}
                    <button type="button" class="button active" onclick={() => void resumeLoop(loop.id)}>
                      {$t("assist.runs.action_resume")}
                    </button>
                    <button type="button" class="button" onclick={() => void deleteLoop(loop.id)}>
                      {$t("llm.loops.delete")}
                    </button>
                  {:else if loop.state === "running" || loop.state === "paused"}
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
  .auto-resume { display: flex; align-items: flex-start; gap: var(--space-2); cursor: pointer; }
  .auto-resume > span { display: flex; flex-direction: column; gap: 2px; }
  .auto-resume input { margin-top: 3px; }
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

  .audit-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .audit {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    font-size: var(--text-sm);
  }

  .audit-note {
    flex: 1;
    min-width: 0;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .tag {
    flex-shrink: 0;
    padding: 0 6px;
    border: var(--hairline) solid var(--separator);
    border-radius: var(--radius-sm);
    font-size: var(--text-xs);
    color: var(--text-dim);
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
