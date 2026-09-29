<script lang="ts">
  import SurfaceGuide from "$components/llm/SurfaceGuide.svelte";
  import RunsPanel from "$components/llm/activity/RunsPanel.svelte";
  import DiffView from "$components/llm/activity/DiffView.svelte";
  import JobReview from "$components/llm/activity/JobReview.svelte";
  import { promptTitle } from "$components/llm/activity/run-status";
  /**
   * Jobs: background agent runs. A form to start one, the list with live
   * state, what each job changed (files and diff), the pending permission
   * ask inline, and the triggers (cron and webhook) that start jobs on
   * their own, with what they do about missed times.
   */
  import { onMount, untrack } from "svelte";
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { getAgents, loadRoster } from "$lib/stores/llm-store.svelte";
  import { setLlmFocus } from "$lib/llm/ui-focus.svelte";
  import {
    answerAsk,
    cancelJob,
    CATCH_UP_CAP,
    deleteJob,
    diffReasonKey,
    deleteTrigger,
    discardJob,
    markJobDone,
    muteTrigger,
    resumeJob,
    duration,
    usageLabel,
    fetchJob,
    fetchJobDiff,
    fireTrigger,
    getAsks,
    getBridge,
    getJobs,
    getLastJobEvent,
    getQueue,
    getTriggers,
    initJobsStore,
    pickFolder,
    markJobSeen,
    pickState,
    queuedEntry,
    removeJobWorktree,
    queueLines,
    reasonLine,
    setJobLimits,
    saveTrigger,
    shortTime,
    submitJob,
    triggerInput,
    type CatchUp,
    type Isolation,
    type Job,
    type JobDiffView,
    type Trigger,
    type TriggerConcurrency,
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
    if (key === "attention")
      return (
        job.state === "waiting_approval" ||
        job.state === "review" ||
        job.state === "failed" ||
        job.state === "interrupted"
      );
    if (key === "done") return !isActive(job);
    return true;
  }
  let visibleJobs = $derived(jobs.filter(job => matches(job, filter)));
  let triggers = $derived(getTriggers());
  let bridge = $derived(getBridge());
  let asks = $derived(getAsks());
  // What runs and what waits, pushed by the gate (`llm://queue`).
  let queue = $derived(getQueue());

  /** "Position 2 of 5 · folder busy: job j1 is writing there" for a waiting job. */
  function waitText(job: Job): string {
    const entry = queuedEntry(job.id);
    const lines = entry && queue ? queueLines(entry, queue) : null;
    const why = lines?.reason ?? reasonLine(job.wait_reason, queue);
    const parts: string[] = [];
    if (lines) parts.push($t(lines.position.key, lines.position.params) as string);
    if (why) parts.push($t(why.key, why.params) as string);
    return parts.join(" · ");
  }

  function saveLimit(field: "max_parallel" | "max_per_agent", raw: string) {
    const n = Math.round(Number(raw));
    if (!Number.isFinite(n)) return;
    void setJobLimits({ [field]: field === "max_parallel" ? Math.min(16, Math.max(1, n)) : Math.min(16, Math.max(0, n)) });
  }

  let agentId = $state("");
  let prompt = $state("");
  let workspace = $state<string | null>(null);
  let isolation = $state<Isolation>("auto");
  const isolations: Isolation[] = ["auto", "worktree", "shared"];
  let submitting = $state(false);

  let openId = $state<string | null>(null);
  let detail = $state<Job | null>(null);
  let diff = $state<JobDiffView | null>(null);
  const catchUps: CatchUp[] = ["once", "all_capped", "skip"];
  const concurrencies: TriggerConcurrency[] = ["skip_if_active", "queue"];

  let trKind = $state<"cron" | "webhook">("cron");
  let trName = $state("");
  let trCron = $state("*/15 * * * *");
  let trAgent = $state("");
  let trPrompt = $state("");
  let trWorkspace = $state<string | null>(null);
  let trCatchUp = $state<CatchUp>("once");
  let trConcurrency = $state<TriggerConcurrency>("skip_if_active");
  let trSaving = $state(false);

  onMount(() => {
    initJobsStore();
    void loadRoster();
  });

  // `?job=<id>` (a row of /llm/work): that job opened, whatever the filter.
  let linkedJob = $derived(page.url.searchParams.get("job"));
  let linkedRun = $derived(page.url.searchParams.get("run"));
  /** The link already followed: closing the row afterwards keeps it closed. */
  let followedJob: string | null = null;
  $effect(() => {
    const id = linkedJob;
    const row = id ? jobs.find((j) => j.id === id) : undefined;
    if (!row || followedJob === row.id) return;
    followedJob = row.id;
    untrack(() => {
      if (openId === row.id) return;
      filter = "all";
      toggle(row);
      requestAnimationFrame(() => document.getElementById(`job-${row.id}`)?.scrollIntoView({ block: "center" }));
    });
  });

  $effect(() => {
    if (agents.length === 0) return;
    if (!agentId) agentId = agents[0].id;
    if (!trAgent) trAgent = agents[0].id;
  });

  // The pending questions follow `llm://tool-ask`, `omni://tool-resolved` and
  // the job events (see initJobsStore): no timer here (A5).

  // The open row is what a "needs you" notification would point at: while
  // it is on screen, the backend does not send one (see $lib/llm/ui-focus).
  $effect(() => {
    setLlmFocus({ jobId: openId });
    return () => setLlmFocus({ jobId: null });
  });

  // A job event for the open row refreshes its full log, and its diff once
  // it has ended (the diff is recorded then; while it runs only on open).
  $effect(() => {
    const ev = getLastJobEvent();
    if (ev && ev.id === openId) {
      void refreshDetail(ev.id);
      if (!isActive(ev) && diff && !diff.complete) void refreshDiff(ev.id);
    }
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
    return (
      job.state === "queued" ||
      job.state === "waiting_workspace" ||
      job.state === "running" ||
      job.state === "waiting_approval"
    );
  }

  /** A job whose changes can be reviewed here (3.5): in review, or a
   *  finished worktree job whose branch can still be integrated. */
  function reviewable(job: Job): boolean {
    if (isActive(job)) return false;
    return job.state === "review" || (!!job.branch && !!job.worktree_path);
  }

  function isWaiting(job: Job): boolean {
    return job.state === "queued" || job.state === "waiting_workspace";
  }

  async function refreshDetail(id: string) {
    const job = await fetchJob(id);
    if (job && openId === id) detail = job;
  }

  async function refreshDiff(id: string) {
    const d = await fetchJobDiff(id);
    if (openId === id) diff = d;
  }

  function toggle(job: Job) {
    if (openId === job.id) {
      openId = null;
      detail = null;
      diff = null;
      return;
    }
    openId = job.id;
    detail = null;
    diff = null;
    void refreshDetail(job.id);
    void refreshDiff(job.id);
    void markJobSeen(job.id);
  }

  async function run() {
    if (!agentId || !prompt.trim()) return;
    submitting = true;
    const job = await submitJob(agentId, prompt.trim(), workspace, workspace ? isolation : null);
    submitting = false;
    if (job) prompt = "";
  }

  async function removeWorktree(id: string) {
    const out = await removeJobWorktree(id);
    if (!out) return;
    const saved = out.saved ? $t("llm.jobs.worktree.saved", { count: out.dirty.length }) + " " : "";
    showToast(
      "success",
      saved +
        (out.preserved_branch
          ? $t("llm.jobs.worktree.kept_branch", { branch: out.preserved_branch })
          : $t("llm.jobs.worktree.removed")),
    );
    if (openId === id) detail = await fetchJob(id);
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
      catch_up: trCatchUp,
      concurrency: trConcurrency,
      tz: "local",
    });
    trSaving = false;
    if (saved) {
      trName = "";
      trPrompt = "";
    }
  }

  function toggleTrigger(tr: Trigger) {
    void saveTrigger(triggerInput(tr, { enabled: !tr.enabled }));
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
      <h1 class="page-title">{$t("llm.surface.tasks")}</h1>
      <p class="page-lede">{$t("llm.jobs.lede")}</p>
    </div>
  </header>
  <SurfaceGuide text={$t("llm.surface.jobs_hint")} href="/help?article=tasks#guide" />

  <RunsPanel openRun={linkedRun} />

  <details class="creation-panel"><summary>{$t("llm.surface.create")}</summary>
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
      {#if workspace}
        <label class="field">
          <span class="field-label">{$t("llm.jobs.isolation.label")}</span>
          <select class="input" bind:value={isolation}>
            {#each isolations as i (i)}
              <option value={i}>{$t(`llm.jobs.isolation.${i}`)}</option>
            {/each}
          </select>
        </label>
      {/if}
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
    {#if queue}
      <div class="queue-bar" role="group" aria-label={$t("llm.jobs.queue.title")}>
        <span class="queue-summary">
          {$t("llm.jobs.queue.summary", { running: queue.limits.in_use, limit: queue.limits.max_parallel, queued: queue.queued.length })}
        </span>
        <label class="queue-limit">
          <span>{$t("llm.jobs.queue.max_parallel")}</span>
          <input
            class="input number"
            type="number"
            min="1"
            max="16"
            value={queue.limits.max_parallel}
            onchange={(e) => saveLimit("max_parallel", (e.currentTarget as HTMLInputElement).value)}
          />
        </label>
        <label class="queue-limit">
          <span>{$t("llm.jobs.queue.max_per_agent")}</span>
          <input
            class="input number"
            type="number"
            min="0"
            max="16"
            value={queue.limits.max_per_agent ?? 0}
            onchange={(e) => saveLimit("max_per_agent", (e.currentTarget as HTMLInputElement).value)}
          />
        </label>
      </div>
      <p class="hint">{$t("llm.jobs.queue.max_parallel_desc")} {$t("llm.jobs.queue.max_per_agent_desc")}</p>
    {/if}
    <div class="job-filters" role="group" aria-label={$t("llm.jobs.list_title")}>
      {#each filters as key}<button class="button" type="button" aria-pressed={filter === key} onclick={() => filter = key}>{$t(`llm.jobs.filter_${key}`)} · {jobs.filter(job => matches(job, key)).length}</button>{/each}
    </div>
    {#if visibleJobs.length === 0}
      <p class="hint">{$t(jobs.length ? "llm.jobs.no_matches" : "llm.jobs.empty")}</p>
    {:else}
      <div class="rows">
        {#each visibleJobs as job (job.id)}
          {@const ask = job.state === "waiting_approval" ? asks.find((a) => a.request_id === job.request_id) : undefined}
          <div class="row" id="job-{job.id}" class:open={openId === job.id}>
            <button type="button" class="row-head" aria-expanded={openId === job.id} onclick={() => toggle(job)}>
              <span class="pill {pickState(job.state)}">{job.state === "interrupted" ? $t("assist.runs.job_interrupted") : $t(`llm.jobs.state.${job.state}`)}</span>
              <span class="kind">{kindLabel(job.kind)}</span>
              {#if job.state === "failed" && job.failure_reason}
                <span class="reason">{$t(`llm.jobs.failure.${job.failure_reason}`)}</span>
              {/if}
              {#if isWaiting(job)}
                <span class="wait">{waitText(job)}</span>
              {/if}
              <span class="agent">{agentName(job.agent_id)}</span>
              <span class="line">{promptTitle(job.prompt).title}</span>
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
                {#if full.branch}
                  <div class="field-label">{$t("llm.jobs.worktree.title")}</div>
                  <div class="worktree">
                    <span class="chip" title={full.base_oid ?? ""}>
                      {$t("llm.jobs.worktree.chip", { branch: full.branch, base: (full.base_oid ?? "").slice(0, 7) })}
                    </span>
                    {#if full.worktree_path}
                      <code class="path" title={full.worktree_path}>{full.worktree_path}</code>
                    {:else}
                      <span class="hint">{$t("llm.jobs.worktree.gone")}</span>
                    {/if}
                  </div>
                {/if}
                {#if full.result}
                  <div class="field-label">{$t("llm.jobs.result")}</div>
                  <div class="text">{full.result}</div>
                {/if}
                {#if full.error}
                  <div class="field-label">{$t("llm.jobs.error")}</div>
                  <div class="text err">{full.error}</div>
                {/if}
                <div class="field-label">{$t("llm.jobs.diff.title")}</div>
                {#if !diff}
                  <p class="hint">{$t("llm.jobs.diff.loading")}</p>
                {:else if !diff.available}
                  <p class="hint">{$t(diffReasonKey(diff.reason))}</p>
                {:else}
                  {#if !diff.complete}<p class="hint">{$t("llm.jobs.diff.live")}</p>{/if}
                  {#if diff.files.length === 0}
                    <p class="hint">{$t("llm.jobs.diff.none")}</p>
                  {:else}
                    <p class="hint">
                      {$t("llm.jobs.diff.totals", { files: diff.file_count, additions: diff.additions, deletions: diff.deletions })}
                    </p>
                    <ul class="files">
                      {#each diff.files as f (f.path)}
                        <li>
                          <span class="badge st-{f.status}">{$t(`assist.runs.file_status.${f.status}`)}</span>
                          <code class="path" title={f.path}>{f.path}</code>
                          {#if f.binary}
                            <span class="dim">{$t("llm.jobs.diff.binary")}</span>
                          {:else}
                            {#if f.additions != null}<span class="plus">+{f.additions}</span>{/if}
                            {#if f.deletions != null}<span class="minus">−{f.deletions}</span>{/if}
                          {/if}
                        </li>
                      {/each}
                    </ul>
                    {#if diff.patch && !reviewable(full)}
                      <details class="patch">
                        <summary>{$t("assist.runs.show_diff")}</summary>
                        <DiffView patch={diff.patch} label={$t("llm.jobs.diff.title")} />
                      </details>
                    {/if}
                    {#if diff.clipped}<p class="hint">{$t("assist.runs.diff_clipped")}</p>{/if}
                    {#if reviewable(full) && diff.complete}
                      <JobReview
                        job={full}
                        files={diff.files}
                        onchanged={() => {
                          void refreshDetail(job.id);
                          void refreshDiff(job.id);
                        }}
                      />
                    {/if}
                  {/if}
                {/if}
                {#if full.proof}
                  {@const pr = full.proof}
                  <div class="field-label">{$t("llm.jobs.proof.title")}</div>
                  <div class="proof">
                    <code class="path" title={pr.stamp}>{pr.stamp}</code>
                    <p class="hint">
                      {$t("llm.jobs.proof.summary", {
                        files: pr.file_count,
                        tokens: (pr.tokens.billable ?? 0).toLocaleString(),
                        time: duration(full.started_ms, full.finished_ms) || "—",
                      })}{#if pr.cost_usd != null}{" · "}${pr.cost_usd.toFixed(4)}{/if}
                    </p>
                    {#each pr.checks as c, i (i)}
                      <p class="hint">
                        <span class={c.ok ? "plus" : "minus"}>{c.ok ? "✓" : "✗"}</span>
                        {$t("llm.jobs.proof.check", { command: c.command, code: c.exit_code ?? "?" })}
                      </p>
                    {/each}
                    <p class="hint">{$t("llm.jobs.proof.by_host")}</p>
                  </div>
                {/if}
                <div class="field-label">{$t("llm.jobs.log")}</div>
                <pre class="pre log">{full.log || $t("llm.jobs.log_empty")}</pre>
                {#if job.state === "interrupted"}
                  <p class="hint">{$t("assist.runs.job_interrupted_hint")}</p>
                {/if}
                {#if sandboxOf(full)}
                  <SandboxReview jobId={job.id} finished={!isActive(job)} />
                {/if}
                <div class="actions">
                  {#if job.state === "interrupted"}
                    <button type="button" class="button active" onclick={() => void resumeJob(job.id)}>
                      {$t("assist.runs.action_resume")}
                    </button>
                    <button type="button" class="button" onclick={() => void markJobDone(job.id)}>
                      {$t("assist.runs.action_mark_done")}
                    </button>
                    <button type="button" class="button" onclick={() => void discardJob(job.id)}>
                      {$t("assist.runs.action_discard")}
                    </button>
                  {:else if isActive(job)}
                    <button type="button" class="button" onclick={() => void cancelJob(job.id)}>
                      {$t("llm.jobs.cancel")}
                    </button>
                  {:else}
                    {#if job.state === "review"}
                      <button type="button" class="button active" onclick={() => void markJobDone(job.id)}>
                        {$t("llm.jobs.review.mark")}
                      </button>
                    {/if}
                    {#if full.worktree_path}
                      <button type="button" class="button" onclick={() => void removeWorktree(job.id)}>
                        {$t("llm.jobs.worktree.remove")}
                      </button>
                    {/if}
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
              <button
                type="button"
                class="button"
                aria-pressed={!!tr.muted}
                onclick={() => void muteTrigger(tr.id, !tr.muted)}
              >
                {tr.muted ? $t("assist.runs.routine_unmute") : $t("assist.runs.routine_mute")}
              </button>
              <button type="button" class="button" onclick={() => void fire(tr)}>{$t("llm.jobs.trigger.fire")}</button>
              <button type="button" class="button" onclick={() => void deleteTrigger(tr.id)}>
                {$t("llm.jobs.delete")}
              </button>
            </div>
            {#if tr.kind === "cron"}
              <code class="path">{tr.cron}</code>
              <p class="hint">
                {#if tr.enabled && tr.next_run_local}
                  {$t("assist.runs.routine_next", { when: tr.next_run_local, offset: tr.utc_offset ?? "" })}
                {/if}
                {#if tr.missed}· <span class="missed">{$t("llm.jobs.trigger.missed", { count: tr.missed })}</span>{/if}
                {#if tr.backlog}· {$t("llm.jobs.trigger.backlog", { count: tr.backlog })}{/if}
                {#if tr.requires_app_open}· {$t("assist.runs.routine_app_open")}{/if}
                {#if tr.muted}· {$t("assist.runs.routine_muted")}{/if}
                {#if tr.zone_known === false}· {$t("llm.jobs.trigger.zone_unknown", { zone: tr.tz ?? "" })}{/if}
              </p>
              <div class="policies">
                <label class="policy">
                  <span>{$t("llm.jobs.trigger.catch_up")}</span>
                  <select
                    class="input"
                    value={tr.catch_up ?? "once"}
                    onchange={(e) => void saveTrigger(triggerInput(tr, { catch_up: (e.currentTarget as HTMLSelectElement).value as CatchUp }))}
                  >
                    {#each catchUps as c (c)}
                      <option value={c}>{$t(`llm.jobs.trigger.catch_up_${c}`, { count: CATCH_UP_CAP })}</option>
                    {/each}
                  </select>
                </label>
                <label class="policy">
                  <span>{$t("llm.jobs.trigger.concurrency")}</span>
                  <select
                    class="input"
                    value={tr.concurrency ?? "skip_if_active"}
                    onchange={(e) => void saveTrigger(triggerInput(tr, { concurrency: (e.currentTarget as HTMLSelectElement).value as TriggerConcurrency }))}
                  >
                    {#each concurrencies as c (c)}
                      <option value={c}>{$t(`llm.jobs.trigger.concurrency_${c}`)}</option>
                    {/each}
                  </select>
                </label>
              </div>
            {:else}
              <div class="copyline">
                <code class="path">{hookUrl(tr)}</code>
                <button type="button" class="button" onclick={() => void copy(hookUrl(tr))}>{$t("llm.jobs.copy")}</button>
              </div>
              <div class="copyline" data-agent-sensitive>
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
        <div class="form-row">
          <label class="field">
            <span class="field-label">{$t("llm.jobs.trigger.catch_up")}</span>
            <select class="input" bind:value={trCatchUp}>
              {#each catchUps as c (c)}
                <option value={c}>{$t(`llm.jobs.trigger.catch_up_${c}`, { count: CATCH_UP_CAP })}</option>
              {/each}
            </select>
          </label>
          <label class="field">
            <span class="field-label">{$t("llm.jobs.trigger.concurrency")}</span>
            <select class="input" bind:value={trConcurrency}>
              {#each concurrencies as c (c)}
                <option value={c}>{$t(`llm.jobs.trigger.concurrency_${c}`)}</option>
              {/each}
            </select>
          </label>
        </div>
        <p class="hint">{$t("llm.jobs.trigger.policy_hint")}</p>
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
  .queue-bar { display: flex; flex-wrap: wrap; align-items: center; gap: var(--space-3); }
  .queue-summary { font-variant-numeric: tabular-nums; color: var(--text-dim); font-size: var(--text-sm); }
  .queue-limit { display: inline-flex; align-items: center; gap: var(--space-2); font-size: var(--text-sm); }
  .queue-limit .number { width: 7ch; text-align: right; }
  .wait { flex-shrink: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: var(--text-xs); color: var(--text-dim); }
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

  .proof {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .worktree {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .chip {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    padding: 2px var(--space-2);
    border-radius: var(--radius-sm);
    border: var(--hairline) solid var(--separator);
    color: var(--text-muted);
    white-space: nowrap;
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

  .policies {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
  }

  .policy {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-muted);
  }

  .missed {
    color: var(--orange);
  }

  /* What the job changed: one line per file, like the run detail. */
  .files {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .files li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    font-size: var(--text-xs);
  }

  .badge {
    flex-shrink: 0;
    padding: 0 6px;
    border-radius: var(--radius-sm);
    background: var(--fill-2);
    color: var(--text-muted);
  }

  .badge.st-A {
    color: var(--green);
  }

  .badge.st-D {
    color: var(--red);
  }

  .plus {
    color: var(--green);
    font-variant-numeric: tabular-nums;
  }

  .minus {
    color: var(--red);
    font-variant-numeric: tabular-nums;
  }

  .dim {
    color: var(--text-dim);
  }

  .patch summary {
    cursor: pointer;
    font-size: var(--text-sm);
    color: var(--text-muted);
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

  /* Why it failed, in words (a stall, a deadline, an unanswered question). */
  .reason {
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: var(--danger);
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
