<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import { errorText, type Run, type Staff, type Wake } from "$lib/company/rpc";
  import StatusDot from "./StatusDot.svelte";
  import { duration, microsToUsd, relTime } from "./company-ui";
  import { dateTime, jobHref, runTokens, staffName, tokens } from "./company-format";

  let {
    run,
    staff,
    issueRef = null,
    wakes = [],
    showAgent = true,
  }: {
    run: Run;
    staff: Staff[];
    issueRef?: { identifier: string; href: string } | null;
    wakes?: Wake[];
    showAgent?: boolean;
  } = $props();

  let open = $state(false);
  let stopping = $state(false);
  let live = $derived(run.state === "running" || run.state === "queued");

  async function stop() {
    stopping = true;
    try {
      await invoke("company_run_stop", { runId: run.id });
    } catch (e) {
      showToast("error", errorText(e));
    } finally {
      stopping = false;
    }
  }
</script>

<div class="run" class:open>
  <button type="button" class="list-row interactive compact head" aria-expanded={open} onclick={() => (open = !open)}>
    <StatusDot status={run.state} kind="run" label />
    {#if showAgent}<span class="agent">{staffName(staff, run.staff_id)}</span>{/if}
    {#if issueRef}<span class="ident">{issueRef.identifier}</span>{/if}
    <span class="src">{run.source}{run.wake_count > 1 ? ` ×${run.wake_count}` : ""}</span>
    <span class="line">{run.error || run.summary || run.liveness || ""}</span>
    {#if run.attempt > 1}<span class="num">{$t("company.activity.attempt", { count: run.attempt })}</span>{/if}
    <span class="num">{$t("company.common.cost", { amount: microsToUsd(run.cost_micros) })}</span>
    <span class="num">{tokens(runTokens(run))}</span>
    <span class="num w">{duration(run.started_ms, run.finished_ms)}</span>
    <time class="num w" title={dateTime(run.created_ms)}>{relTime(run.created_ms, $t)}</time>
  </button>
  {#if open}
    <div class="detail">
      <dl class="facts">
        <dt>{$t("company.activity.source")}</dt>
        <dd>{run.source}</dd>
        {#if issueRef}
          <dt>{$t("company.activity.issue")}</dt>
          <dd><a href={issueRef.href}>{issueRef.identifier}</a></dd>
        {/if}
        <dt>{$t("company.activity.tokens")}</dt>
        <dd class="tab">
          {$t("company.activity.tokens_split", {
            input: tokens(run.input_tokens),
            cached: tokens(run.cached_tokens),
            output: tokens(run.output_tokens),
          })}
        </dd>
        {#if run.liveness}
          <dt>{$t("company.activity.liveness")}</dt>
          <dd>{run.liveness}</dd>
        {/if}
        {#if run.failure_class}
          <dt>{$t("company.activity.failure")}</dt>
          <dd>{run.failure_class}</dd>
        {/if}
        <dt>{$t("company.activity.started")}</dt>
        <dd>{dateTime(run.started_ms ?? run.created_ms)}</dd>
        {#if run.job_id}
          <dt>{$t("company.activity.job")}</dt>
          <dd><a class="mono" href={jobHref(run.job_id)}>{run.job_id}</a></dd>
        {/if}
      </dl>
      {#if run.error}<p class="err">{run.error}</p>{/if}
      {#if run.summary}<p class="summary">{run.summary}</p>{/if}
      {#if wakes.length > 0}
        <div class="field-label">{$t("company.activity.coalesced", { count: wakes.length })}</div>
        <ul class="wakes">
          {#each wakes as w (w.id)}
            <li><span class="src">{w.source}</span> {w.reason} <span class="num">{relTime(w.created_ms, $t)}</span></li>
          {/each}
        </ul>
      {/if}
      {#if live}
        <div>
          <button type="button" class="btn btn-destructive btn-sm" disabled={stopping} onclick={stop}>
            {$t("company.activity.stop")}
          </button>
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .head {
    border-radius: 0;
    gap: var(--space-2);
    font-size: var(--text-sm);
  }
  .run.open {
    background: var(--fill-1);
  }
  .agent {
    font-weight: 600;
    flex-shrink: 0;
    max-width: 140px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ident,
  .mono {
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .src {
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: var(--text-muted);
    padding: 0 6px;
    border-radius: var(--radius-full);
    background: var(--fill-2);
  }
  .line {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-dim);
  }
  .num {
    flex-shrink: 0;
    font-size: var(--text-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .w {
    min-width: 56px;
    text-align: right;
  }
  .detail {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: 0 var(--space-3) var(--space-3);
  }
  .facts {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 2px var(--space-3);
    margin: 0;
    font-size: var(--text-sm);
  }
  .facts dt {
    color: var(--text-dim);
  }
  .facts dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .tab {
    font-variant-numeric: tabular-nums;
  }
  .err {
    margin: 0;
    color: var(--red);
    font-size: var(--text-sm);
    white-space: pre-wrap;
    user-select: text;
  }
  .summary {
    margin: 0;
    font-size: var(--text-sm);
    white-space: pre-wrap;
    user-select: text;
  }
  .wakes {
    margin: 0;
    padding-left: var(--space-4);
    font-size: var(--text-sm);
  }
</style>
