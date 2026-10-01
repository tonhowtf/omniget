<script lang="ts">
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import BudgetIncidentCard from "./BudgetIncidentCard.svelte";
  import {
    BUDGET_METRICS,
    BUDGET_WINDOWS,
    SCOPE_KINDS,
    deleteBudgetPolicy,
    formatAmount,
    getBlockingIncidents,
    getBudgetPolicies,
    getBudgetReadError,
    initBudgetStore,
    reloadBudget,
    saveBudgetPolicy,
    scopeLabel,
    usedShare,
    type BudgetMetric,
    type BudgetWindow,
    type ScopeKind,
  } from "$lib/stores/llm-budget-store.svelte";

  let policies = $derived(getBudgetPolicies());
  let blocking = $derived(getBlockingIncidents());
  let readError = $derived(getBudgetReadError());
  let linked = $derived(page.url.searchParams.get("incident"));

  let kind = $state<ScopeKind>("global");
  let scopeId = $state("");
  let metric = $state<BudgetMetric>("usd");
  let win = $state<BudgetWindow>("day");
  let amount = $state<number | null>(null);
  let warn = $state<number | null>(80);
  let soft = $state(false);
  let saving = $state(false);

  let windows = $derived(metric === "quota_pct" ? (["5h", "7d"] as BudgetWindow[]) : BUDGET_WINDOWS);
  $effect(() => {
    if (!windows.includes(win)) win = windows[0];
  });
  let scope = $derived(kind === "global" ? "global" : `${kind}:${scopeId.trim()}`);
  let valid = $derived((kind === "global" || scopeId.trim().length > 0) && typeof amount === "number" && amount > 0);

  onMount(() => {
    initBudgetStore();
  });

  async function add() {
    if (!valid || saving || amount === null) return;
    saving = true;
    const ok = await saveBudgetPolicy({
      scope,
      metric,
      window: win,
      amount,
      warn_pct: warn && warn > 0 && warn < 100 ? warn : null,
      hard_stop: !soft,
    });
    saving = false;
    if (ok) {
      amount = null;
      scopeId = "";
    }
  }
</script>

<section class="card surface-card wide budgets" id="budgets" aria-labelledby="budgets-title">
  <div class="card-head">
    <h2 class="card-title" id="budgets-title">{$t("llm.budget.title")}</h2>
    <button type="button" class="btn btn-secondary" onclick={() => void reloadBudget()}>{$t("llm.budget.refresh")}</button>
  </div>
  <p class="lede">{$t("llm.budget.lede")}</p>

  {#if readError}
    <p class="hint danger">{$t("llm.budget.read_error")} <code>{readError}</code></p>
  {/if}

  {#if blocking.length}
    <div class="incidents">
      {#each blocking as incident (incident.id)}
        <BudgetIncidentCard {incident} highlight={incident.id === linked} />
      {/each}
    </div>
  {/if}

  {#if policies.length === 0}
    <p class="hint">{$t("llm.budget.empty")}</p>
  {:else}
    <ul class="policies">
      {#each policies as p (p.id)}
        <li class="policy" class:paused={!!p.incident}>
          <div class="p-head">
            <strong class="scope">{scopeLabel(p.scope)}</strong>
            <span class="dim">{$t(`llm.budget.metric.${p.metric}`)} · {$t(`llm.budget.window.${p.window}`)}</span>
            <span class="tag">{$t(p.hard_stop ? "llm.budget.hard" : "llm.budget.soft")}</span>
            {#if p.incident}<span class="pill orange">{$t("llm.budget.paused")}</span>{/if}
            <button type="button" class="link" onclick={() => void deleteBudgetPolicy(p.id)}>{$t("llm.budget.remove")}</button>
          </div>
          <div class="bar" role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(usedShare(p.observed, p.amount))} aria-label={scopeLabel(p.scope)}>
            <span class="fill" style={`width: ${usedShare(p.observed, p.amount)}%`}></span>
          </div>
          <div class="p-foot dim">
            {#if p.observed === null}
              {$t("llm.budget.no_reading")}
            {:else}
              {formatAmount(p.metric, p.observed)} / {formatAmount(p.metric, p.amount)}
            {/if}
            {#if p.warn_pct}· {$t("llm.budget.warn_at", { pct: p.warn_pct })}{/if}
            {#if p.metric === "usd" && p.scope.startsWith("account:")}· {$t("llm.budget.estimate_note")}{/if}
          </div>
        </li>
      {/each}
    </ul>
  {/if}

  <details class="add">
    <summary>{$t("llm.budget.add")}</summary>
    <div class="form-row">
      <label class="field">
        <span class="field-label">{$t("llm.budget.scope")}</span>
        <select class="input" bind:value={kind}>
          {#each SCOPE_KINDS as k (k)}<option value={k}>{$t(`llm.budget.scope_kind.${k}`)}</option>{/each}
        </select>
      </label>
      {#if kind !== "global"}
        <label class="field grow">
          <span class="field-label">{$t("llm.budget.scope_id")}</span>
          <input class="input mono" type="text" bind:value={scopeId} placeholder={kind === "account" ? "max-1" : "id"} />
        </label>
      {/if}
      <label class="field">
        <span class="field-label">{$t("llm.budget.metric_label")}</span>
        <select class="input" bind:value={metric}>
          {#each BUDGET_METRICS as m (m)}<option value={m}>{$t(`llm.budget.metric.${m}`)}</option>{/each}
        </select>
      </label>
      <label class="field">
        <span class="field-label">{$t("llm.budget.window_label")}</span>
        <select class="input" bind:value={win}>
          {#each windows as w (w)}<option value={w}>{$t(`llm.budget.window.${w}`)}</option>{/each}
        </select>
      </label>
    </div>
    <div class="form-row">
      <label class="field">
        <span class="field-label">{$t("llm.budget.amount")}</span>
        <input class="input number" type="number" min="0" step="any" bind:value={amount} />
      </label>
      <label class="field">
        <span class="field-label">{$t("llm.budget.warn_label")}</span>
        <input class="input number" type="number" min="0" max="99" bind:value={warn} />
      </label>
      <label class="check">
        <input type="checkbox" bind:checked={soft} />
        {$t("llm.budget.soft_label")}
      </label>
      <button type="button" class="button active" disabled={!valid || saving} onclick={add}>{$t("llm.budget.save")}</button>
    </div>
    <p class="hint">{$t(soft ? "llm.budget.soft_hint" : "llm.budget.hard_hint")}</p>
    {#if metric === "quota_pct"}<p class="hint">{$t("llm.budget.quota_hint")}</p>{/if}
  </details>
</section>

<style>
  .budgets {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .lede,
  .hint {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
  }
  .danger {
    color: var(--danger);
  }
  .incidents {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .policies {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .policy {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: var(--space-2) 0;
    border-bottom: 1px solid var(--border-subtle, rgba(127, 127, 127, 0.2));
  }
  .p-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .scope {
    font-family: var(--font-mono, monospace);
  }
  .dim {
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .tag {
    font-size: var(--text-xs, 11px);
    padding: 1px 6px;
    border-radius: 999px;
    background: var(--surface-2, rgba(127, 127, 127, 0.15));
  }
  .link {
    margin-left: auto;
    background: none;
    border: 0;
    color: var(--accent);
    cursor: pointer;
    font-size: var(--text-sm);
  }
  .bar {
    height: 6px;
    border-radius: 3px;
    background: var(--surface-2, rgba(127, 127, 127, 0.15));
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    background: var(--accent);
  }
  .paused .fill {
    background: var(--warning, #d97706);
  }
  .add summary {
    cursor: pointer;
    font-weight: 600;
  }
  .form-row {
    display: flex;
    gap: var(--space-2);
    align-items: flex-end;
    flex-wrap: wrap;
    margin-top: var(--space-2);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .grow {
    flex: 1;
    min-width: 12ch;
  }
  .number {
    width: 11ch;
  }
  .mono {
    font-family: var(--font-mono, monospace);
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-sm);
  }
</style>
