<script lang="ts">
  /**
   * Activity → agent runs: every turn a bot took, from the durable record
   * (`assist::runs`), with a plain state (working, waiting for you,
   * interrupted, failed, done), the questions waiting for an answer, and the
   * review of one run on click. Live through `assist://run`.
   */
  import { onMount } from "svelte";
  import { t } from "$lib/i18n";
  import { getAgents, loadRoster } from "$lib/stores/llm-store.svelte";
  import {
    answerPermission,
    duration,
    getPermissions,
    getRuns,
    initJobsStore,
    shortTime,
    type RunView,
  } from "$lib/stores/llm-jobs-store.svelte";
  import RunDetail from "./RunDetail.svelte";
  import { matchesFilter, promptTitle, runStatus, statusTone, type RunFilter } from "./run-status";

  let { conversationId = null }: { conversationId?: string | null } = $props();

  let agents = $derived(getAgents());
  let all = $derived(getRuns().filter((r) => !conversationId || r.conversation_id === conversationId));
  let pending = $derived(getPermissions().filter((p) => all.some((r) => r.id === p.run_id)));
  let filter = $state<RunFilter>("all");
  const filters: RunFilter[] = ["all", "active", "attention", "done"];
  let visible = $derived(all.filter((r) => matchesFilter(r, filter)));
  let openId = $state<string | null>(null);

  onMount(() => {
    initJobsStore();
    void loadRoster();
  });

  function agentName(id: string): string {
    return agents.find((a) => a.id === id)?.name ?? id;
  }

  function toggle(run: RunView) {
    openId = openId === run.id ? null : run.id;
  }
</script>

<section class="stack" aria-labelledby="runs-title">
  <h2 class="section-header-title" id="runs-title">{$t("assist.runs.title")}</h2>
  <p class="hint">{$t("assist.runs.lede")}</p>

  {#if pending.length > 0}
    <div class="asks" role="region" aria-label={$t("assist.runs.permissions_title")}>
      <div class="field-label">{$t("assist.runs.permissions_title")}</div>
      {#each pending as p (p.id)}
        {@const run = all.find((r) => r.id === p.run_id)}
        <div class="ask">
          <div class="ask-title">
            {run ? agentName(run.bot_id) : ""} · {$t("assist.runs.permission_label", { action: p.action })}
          </div>
          <pre class="pre">{p.preview}</pre>
          <div class="actions">
            <button type="button" class="button active" onclick={() => void answerPermission(p.id, "once")}>
              {$t("assist.runs.answer_once")}
            </button>
            <button type="button" class="button" onclick={() => void answerPermission(p.id, "always")}>
              {$t("assist.runs.answer_always")}
            </button>
            <button type="button" class="button" onclick={() => void answerPermission(p.id, "deny")}>
              {$t("assist.runs.answer_deny")}
            </button>
          </div>
        </div>
      {/each}
    </div>
  {/if}

  <div class="filters" role="group" aria-label={$t("assist.runs.title")}>
    {#each filters as key (key)}
      <button class="button" type="button" aria-pressed={filter === key} onclick={() => (filter = key)}>
        {$t(`assist.runs.filter_${key}`)} · {all.filter((r) => matchesFilter(r, key)).length}
      </button>
    {/each}
  </div>

  {#if visible.length === 0}
    <p class="hint">{$t(all.length ? "assist.runs.no_matches" : "assist.runs.empty")}</p>
  {:else}
    <div class="rows">
      {#each visible as run (run.id)}
        {@const status = runStatus(run.state, run.error)}
        {@const head = promptTitle(run.input_preview)}
        <div class="row" class:open={openId === run.id}>
          <button type="button" class="row-head" aria-expanded={openId === run.id} onclick={() => toggle(run)}>
            <span class="pill {statusTone(status)}">
              {#if status === "working"}<span class="pulse" aria-hidden="true"></span>{/if}
              {$t(`assist.runs.status.${status}`)}
            </span>
            <span class="agent">{agentName(run.bot_id)}</span>
            {#if head.mission}<span class="tag" title={head.mission}>{$t("llm.jobs.kind.mission")}</span>{/if}
            <span class="line">{head.title}</span>
            {#if run.pending_permissions > 0}
              <span class="tag warn">{$t("assist.runs.waiting_count", { count: run.pending_permissions })}</span>
            {/if}
            {#if run.resume_kind === "replay"}
              <span class="tag" title={$t("assist.runs.resume.replay_hint")}>{$t("assist.runs.resume.replay")}</span>
            {/if}
            <span class="time">
              {shortTime(run.created_ms)}
              {#if run.ended_ms}· {duration(run.started_ms ?? run.created_ms, run.ended_ms)}{/if}
              {#if run.tool_calls > 0}· {$t("assist.runs.tool_calls", { count: run.tool_calls })}{/if}
            </span>
          </button>
          {#if openId === run.id}
            <RunDetail runId={run.id} />
          {/if}
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .stack {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    margin-bottom: var(--space-5);
    min-width: 0;
  }

  .hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }

  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .filters [aria-pressed="true"] {
    color: var(--accent-hi);
    background: var(--accent-soft);
  }

  .asks {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .ask {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--orange) 8%, transparent);
  }

  .ask-title {
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .pre {
    margin: 0;
    padding: var(--space-2);
    max-height: 200px;
    overflow: auto;
    border-radius: var(--radius-sm);
    background: var(--fill-2);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
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
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2) var(--space-3);
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

  .row-head:focus-visible {
    outline: var(--focus-ring);
    outline-offset: -2px;
  }

  .agent {
    flex-shrink: 0;
    font-size: var(--text-xs);
    font-weight: 500;
    color: var(--text-muted);
  }

  .line {
    flex: 1;
    min-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .time {
    margin-left: auto;
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }

  .tag {
    flex-shrink: 0;
    padding: 0 6px;
    border-radius: 999px;
    font-size: var(--text-xs);
    color: var(--text-muted);
    background: var(--fill-2);
  }

  .tag.warn {
    color: var(--orange);
    background: color-mix(in srgb, var(--orange) 14%, transparent);
  }

  .pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
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

  .pulse {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    animation: pulse 1.2s ease-in-out infinite;
  }

  @keyframes pulse {
    50% {
      opacity: 0.3;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .pulse {
      animation: none;
    }
  }
</style>
