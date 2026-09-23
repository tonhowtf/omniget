<script lang="ts">
  import SurfaceGuide from "$components/llm/SurfaceGuide.svelte";
  import { surfaceCopy } from "$components/llm/surface-copy";
  /**
   * Jobs: background agent runs. A form to start one, the list with live
   * state, the pending permission ask inline, and the triggers (cron and
   * webhook) that start jobs on their own.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { getAgents, loadRoster } from "$lib/stores/llm-store.svelte";
  import {
    answerAsk,
    cancelJob,
    deleteJob,
    deleteTrigger,
    duration,
    usageLabel,
    fetchJob,
    fireTrigger,
    getAsks,
    getBridge,
    getJobs,
    getLastJobEvent,
    getTriggers,
    initJobsStore,
    pickFolder,
    pickState,
    pollAsks,
    saveTrigger,
    shortTime,
    submitJob,
    type Job,
    type Trigger,
  } from "$lib/stores/llm-jobs-store.svelte";
  import PlaybookPanel from "$components/central/run/PlaybookPanel.svelte";
  import SandboxReview from "$components/central/run/SandboxReview.svelte";
  import RunNow from "$components/central/run/RunNow.svelte";
  import { agentLabel } from "$components/central/run/run-api";

  let agents = $derived(getAgents());
  let jobs = $derived(getJobs());
  let filter = $state("all");
  const filters = ["all", "active", "attention", "done"];
  function matches(job: Job, key: string) {
    if (key === "active") return isActive(job);
    if (key === "attention") return job.state === "waiting_approval" || job.state === "failed";
    if (key === "done") return !isActive(job);
    return true;
  }
  let visibleJobs = $derived(jobs.filter(job => matches(job, filter)));
  let triggers = $derived(getTriggers());
  let bridge = $derived(getBridge());
  let asks = $derived(getAsks());
  let waiting = $derived(jobs.some((j) => j.state === "waiting_approval"));

  let agentId = $state("");
  let prompt = $state("");
  let workspace = $state<string | null>(null);
  let submitting = $state(false);

  let openId = $state<string | null>(null);
  let detail = $state<Job | null>(null);

  let trKind = $state<"cron" | "webhook">("cron");
  let trName = $state("");
  let trCron = $state("*/15 * * * *");
  let trAgent = $state("");
  let trPrompt = $state("");
  let trWorkspace = $state<string | null>(null);
  let trSaving = $state(false);

  onMount(() => {
    initJobsStore();
    void loadRoster();
  });

  $effect(() => {
    if (agents.length === 0) return;
    if (!agentId) agentId = agents[0].id;
    if (!trAgent) trAgent = agents[0].id;
  });

  // Asks are polled only while some job waits for an answer.
  $effect(() => {
    if (!waiting) return;
    void pollAsks();
    const timer = setInterval(() => void pollAsks(), 2000);
    return () => clearInterval(timer);
  });

  // A job event for the open row refreshes its full log.
  $effect(() => {
    const ev = getLastJobEvent();
    if (ev && ev.id === openId) void refreshDetail(ev.id);
  });

  function agentName(id: string): string {
    if (id.startsWith("tool:")) return agentLabel(id);
    return agents.find((a) => a.id === id)?.name ?? id;
  }

  let agentNames = $derived(Object.fromEntries(agents.map((a) => [a.id, a.name])));
  let catalogAgent = $state("");

  function kindLabel(kind: string): string {
    return kind === "playbook" ? $t("llm.central.run.kind.playbook") : $t(`llm.jobs.kind.${kind}`);
  }

  /** Sandbox options of a tool job (catalog runs), if any. */
  function sandboxOf(job: Job): unknown {
    return (job as Job & { spec?: { sandbox?: unknown } | null }).spec?.sandbox ?? null;
  }

  function firstLine(text: string): string {
    return text.split("\n").find((l) => l.trim())?.trim() ?? "";
  }

  function isActive(job: Job): boolean {
    return job.state === "queued" || job.state === "running" || job.state === "waiting_approval";
  }

  async function refreshDetail(id: string) {
    const job = await fetchJob(id);
    if (job && openId === id) detail = job;
  }

  function toggle(job: Job) {
    if (openId === job.id) {
      openId = null;
      detail = null;
      return;
    }
    openId = job.id;
    detail = null;
    void refreshDetail(job.id);
  }

  async function run() {
    if (!agentId || !prompt.trim()) return;
    submitting = true;
    const job = await submitJob(agentId, prompt.trim(), workspace);
    submitting = false;
    if (job) prompt = "";
  }

  async function createTrigger() {
    if (!trAgent || !trPrompt.trim()) return;
    trSaving = true;
    const saved = await saveTrigger({
      name: trName.trim() || undefined,
      kind: trKind,
      cron: trKind === "cron" ? trCron.trim() : null,
      agent_id: trAgent,
      prompt: trPrompt.trim(),
      workspace: trWorkspace,
      enabled: true,
    });
    trSaving = false;
    if (saved) {
      trName = "";
      trPrompt = "";
    }
  }

  function toggleTrigger(tr: Trigger) {
    void saveTrigger({
      id: tr.id,
      name: tr.name,
      kind: tr.kind,
      cron: tr.cron,
      agent_id: tr.agent_id,
      prompt: tr.prompt,
      workspace: tr.workspace,
      enabled: !tr.enabled,
    });
  }

  function hookUrl(tr: Trigger): string {
    return `${bridge?.base_url ?? ""}/v1/hooks/${tr.id}`;
  }

  function hookCurl(tr: Trigger): string {
    return `curl -X POST -H "Authorization: Bearer ${bridge?.token ?? ""}" -d 'payload' ${hookUrl(tr)}`;
  }

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      showToast("success", $t("llm.jobs.copied"));
    } catch (e) {
      showToast("error", String(e));
    }
  }

  async function fire(tr: Trigger) {
    const job = await fireTrigger(tr.id);
    if (job) showToast("success", $t("llm.jobs.trigger.fired"));
  }
</script>

<svelte:head><title>{$t("llm.tab.jobs")}</title></svelte:head>

<div class="page page-wide jobs-page">
  <header class="page-head">
    <div>
      <h1 class="page-title">{$surfaceCopy.tasks}</h1>
      <p class="page-lede">{$t("llm.jobs.lede")}</p>
    </div>
  </header>
  <SurfaceGuide text={$surfaceCopy.jobsHint} href="/help?article=tasks#guide" />

  <details class="creation-panel"><summary>{$surfaceCopy.create}</summary>
  <section class="surface-card form">
    <h2 class="section-header-title">{$t("llm.jobs.new_title")}</h2>
    <div class="form-row">
      <label class="field">
        <span class="field-label">{$t("llm.jobs.agent")}</span>
        <select class="input" bind:value={agentId}>
          {#each agents as a (a.id)}
            <option value={a.id}>{a.name}</option>
          {/each}
        </select>
      </label>
      <div class="field grow">
        <span class="field-label">{$t("llm.jobs.workspace")}</span>
        <div class="folder">
          <button type="button" class="button" onclick={async () => (workspace = (await pickFolder()) ?? workspace)}>
            {$t("llm.jobs.pick_folder")}
          </button>
          {#if workspace}
            <code class="path" title={workspace}>{workspace}</code>
            <button type="button" class="button" onclick={() => (workspace = null)}>{$t("llm.jobs.clear")}</button>
          {:else}
            <span class="hint">{$t("llm.jobs.workspace_none")}</span>
          {/if}
        </div>
      </div>
    </div>
    <label class="field">
      <span class="field-label">{$t("llm.jobs.prompt")}</span>
      <textarea class="input" rows="3" bind:value={prompt} placeholder={$t("llm.jobs.prompt_placeholder")}></textarea>
    </label>
    <div>
      <button type="button" class="button active" disabled={submitting || !agentId || !prompt.trim()} onclick={run}>
        {$t("llm.jobs.run")}
      </button>
    </div>
  </section>
  </details>

  <section class="stack">
    <h2 class="section-header-title">{$t("llm.jobs.list_title")}</h2>
    <div class="job-filters" role="group" aria-label={$t("llm.jobs.list_title")}>
      {#each filters as key}<button class="button" type="button" aria-pressed={filter === key} onclick={() => filter = key}>{$t(`llm.jobs.filter_${key}`)} · {jobs.filter(job => matches(job, key)).length}</button>{/each}
    </div>
    {#if visibleJobs.length === 0}
      <p class="hint">{$t(jobs.length ? "llm.jobs.no_matches" : "llm.jobs.empty")}</p>
    {:else}
      <div class="rows">
        {#each visibleJobs as job (job.id)}
          {@const ask = job.state === "waiting_approval" ? asks.find((a) => a.request_id === job.request_id) : undefined}
          <div class="row" class:open={openId === job.id}>
            <button type="button" class="row-head" aria-expanded={openId === job.id} onclick={() => toggle(job)}>
              <span class="pill {pickState(job.state)}">{$t(`llm.jobs.state.${job.state}`)}</span>
              <span class="kind">{kindLabel(job.kind)}</span>
              <span class="agent">{agentName(job.agent_id)}</span>
              <span class="line">{firstLine(job.prompt)}</span>
              <span class="time">
                {shortTime(job.created_ms)}
                {#if job.finished_ms}· {duration(job.started_ms ?? job.created_ms, job.finished_ms)}{/if}
                {#if job.usage}· {usageLabel(job)}{/if}
              </span>
            </button>

            {#if ask}
              <div class="ask">
                <div class="ask-title">{$t("llm.jobs.ask.title", { tool: ask.tool })}</div>
                <pre class="pre">{ask.preview}</pre>
                <div class="actions">
                  <button type="button" class="button active" onclick={() => void answerAsk(ask, true, false)}>
                    {$t("llm.jobs.ask.allow")}
                  </button>
                  <button type="button" class="button" onclick={() => void answerAsk(ask, true, true)}>
                    {$t("llm.jobs.ask.always")}
                  </button>
                  <button type="button" class="button" onclick={() => void answerAsk(ask, false, false)}>
                    {$t("llm.jobs.ask.deny")}
                  </button>
                </div>
              </div>
            {/if}

            {#if openId === job.id}
              {@const full = detail ?? job}
              <div class="detail">
                <div class="field-label">{$t("llm.jobs.prompt")}</div>
                <div class="text">{full.prompt}</div>
                {#if full.workspace}
                  <div class="field-label">{$t("llm.jobs.workspace")}</div>
                  <code class="path">{full.workspace}</code>
                {/if}
                {#if full.result}
                  <div class="field-label">{$t("llm.jobs.result")}</div>
                  <div class="text">{full.result}</div>
                {/if}
                {#if full.error}
                  <div class="field-label">{$t("llm.jobs.error")}</div>
                  <div class="text err">{full.error}</div>
                {/if}
                <div class="field-label">{$t("llm.jobs.log")}</div>
                <pre class="pre log">{full.log || $t("llm.jobs.log_empty")}</pre>
                {#if sandboxOf(full)}
                  <SandboxReview jobId={job.id} finished={!isActive(job)} />
                {/if}
                <div class="actions">
                  {#if isActive(job)}
                    <button type="button" class="button" onclick={() => void cancelJob(job.id)}>
                      {$t("llm.jobs.cancel")}
                    </button>
                  {:else}
                    <button type="button" class="button" onclick={() => void deleteJob(job.id)}>
                      {$t("llm.jobs.delete")}
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

  <PlaybookPanel {agentNames} />

  <section class="stack">
    <h2 class="section-header-title">{$t("llm.central.run.now.catalog_title")}</h2>
    <p class="hint">{$t("llm.central.run.now.catalog_lede")}</p>
    <div class="form-row">
      <label class="field grow">
        <span class="field-label">{$t("llm.central.run.now.catalog_id")}</span>
        <input class="input" type="text" placeholder="cct:agents/development-team/frontend-developer" bind:value={catalogAgent} />
      </label>
      <RunNow agentId={catalogAgent.trim() || null} itemName={catalogAgent.trim()} workspace={workspace} />
    </div>
  </section>

  <section class="stack">
    <h2 class="section-header-title">{$t("llm.jobs.trigger.title")}</h2>
    <p class="hint">{$t("llm.jobs.trigger.lede")}</p>

    {#if triggers.length === 0}
      <p class="hint">{$t("llm.jobs.trigger.empty")}</p>
    {:else}
      <div class="rows">
        {#each triggers as tr (tr.id)}
          <div class="row trigger">
            <div class="tr-head">
              <span class="tr-name">{tr.name || tr.id}</span>
              <span class="tag">{$t(`llm.jobs.trigger.kind_${tr.kind}`)}</span>
              <span class="agent">{agentName(tr.agent_id)}</span>
              <span class="time">
                {$t("llm.jobs.trigger.fired_count", { count: tr.fire_count })}
                {#if tr.last_fired_ms}· {shortTime(tr.last_fired_ms)}{/if}
              </span>
              <label class="enabled">
                <input class="checkbox" type="checkbox" checked={tr.enabled} onchange={() => toggleTrigger(tr)} />
                <span>{$t("llm.jobs.trigger.enabled")}</span>
              </label>
              <button type="button" class="button" onclick={() => void fire(tr)}>{$t("llm.jobs.trigger.fire")}</button>
              <button type="button" class="button" onclick={() => void deleteTrigger(tr.id)}>
                {$t("llm.jobs.delete")}
              </button>
            </div>
            {#if tr.kind === "cron"}
              <code class="path">{tr.cron}</code>
            {:else}
              <div class="copyline">
                <code class="path">{hookUrl(tr)}</code>
                <button type="button" class="button" onclick={() => void copy(hookUrl(tr))}>{$t("llm.jobs.copy")}</button>
              </div>
              <div class="copyline">
                <code class="path">{hookCurl(tr)}</code>
                <button type="button" class="button" onclick={() => void copy(hookCurl(tr))}>
                  {$t("llm.jobs.trigger.copy_curl")}
                </button>
              </div>
            {/if}
            <div class="line muted">{firstLine(tr.prompt)}</div>
          </div>
        {/each}
      </div>
    {/if}

    <div class="surface-card form">
      <h3 class="section-header-title">{$t("llm.jobs.trigger.new_title")}</h3>
      <div class="form-row">
        <label class="field">
          <span class="field-label">{$t("llm.jobs.trigger.kind")}</span>
          <select class="input" bind:value={trKind}>
            <option value="cron">{$t("llm.jobs.trigger.kind_cron")}</option>
            <option value="webhook">{$t("llm.jobs.trigger.kind_webhook")}</option>
          </select>
        </label>
        <label class="field grow">
          <span class="field-label">{$t("llm.jobs.trigger.name")}</span>
          <input class="input" type="text" bind:value={trName} />
        </label>
        {#if trKind === "cron"}
          <label class="field">
            <span class="field-label">{$t("llm.jobs.trigger.cron")}</span>
            <input class="input mono" type="text" bind:value={trCron} placeholder="*/15 * * * *" />
          </label>
        {/if}
        <label class="field">
          <span class="field-label">{$t("llm.jobs.agent")}</span>
          <select class="input" bind:value={trAgent}>
            {#each agents as a (a.id)}
              <option value={a.id}>{a.name}</option>
            {/each}
          </select>
        </label>
      </div>
      {#if trKind === "cron"}
        <p class="hint">{$t("llm.jobs.trigger.cron_hint")}</p>
      {/if}
      <label class="field">
        <span class="field-label">{$t("llm.jobs.prompt")}</span>
        <textarea class="input" rows="3" bind:value={trPrompt}></textarea>
        <span class="field-hint">{$t("llm.jobs.trigger.body_hint", { token: "{{body}}" })}</span>
      </label>
      <div class="folder">
        <button type="button" class="button" onclick={async () => (trWorkspace = (await pickFolder()) ?? trWorkspace)}>
          {$t("llm.jobs.pick_folder")}
        </button>
        {#if trWorkspace}
          <code class="path" title={trWorkspace}>{trWorkspace}</code>
          <button type="button" class="button" onclick={() => (trWorkspace = null)}>{$t("llm.jobs.clear")}</button>
        {:else}
          <span class="hint">{$t("llm.jobs.workspace_none")}</span>
        {/if}
      </div>
      <div>
        <button
          type="button"
          class="button active"
          disabled={trSaving || !trAgent || !trPrompt.trim() || (trKind === "cron" && !trCron.trim())}
          onclick={createTrigger}
        >
          {$t("llm.jobs.trigger.create")}
        </button>
      </div>
    </div>
  </section>
</div>

<style>
  .creation-panel { margin-bottom:24px; }
  .creation-panel summary { cursor:pointer; padding:14px 16px; border:1px solid var(--separator); border-radius:var(--radius-lg); color:var(--text); font-weight:600; }
  .creation-panel[open] summary { margin-bottom:12px; }
  .creation-panel summary:focus-visible { outline:var(--focus-ring); }
  .job-filters { display: flex; flex-wrap: wrap; gap: var(--space-2); }
  .job-filters [aria-pressed="true"] { color: var(--accent-hi); background: var(--accent-soft); }
  .jobs-page {
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

  textarea.input {
    resize: vertical;
    height: auto;
    font-family: inherit;
  }

  .folder,
  .actions,
  .copyline {
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

  .row-head,
  .tr-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    padding: var(--space-2) var(--space-3);
    font-size: var(--text-sm);
    text-align: left;
  }

  .row-head {
    background: none;
    border: 0;
    color: inherit;
    cursor: pointer;
  }

  .row-head:hover {
    background: var(--fill-1);
  }

  .tr-head {
    flex-wrap: wrap;
    padding: 0;
  }

  .trigger {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
  }

  .tr-name {
    font-weight: 600;
  }

  .enabled {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .kind,
  .agent,
  .time {
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
  }

  .agent {
    color: var(--text-muted);
    font-weight: 500;
  }

  .time {
    margin-left: auto;
    font-variant-numeric: tabular-nums;
  }

  .line {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .line.muted {
    font-size: var(--text-sm);
    color: var(--text-dim);
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

  .ask,
  .detail {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3) var(--space-3);
  }

  .ask {
    margin: 0 var(--space-3) var(--space-3);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--orange) 8%, transparent);
  }

  .ask-title {
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .text {
    font-size: var(--text-sm);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }

  .text.err {
    color: var(--red);
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
