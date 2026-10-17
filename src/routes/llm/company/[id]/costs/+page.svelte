<script lang="ts">
  import { page } from "$app/state";
  import { t } from "$lib/i18n";
  import { showToast } from "$lib/stores/toast-store.svelte";
  import {
    rpc,
    errorText,
    type BudgetPolicy,
    type CostSummary,
    type Incident,
    type PolicyStatus,
    type Project,
    type Staff,
  } from "$lib/company/rpc";
  import CostBars from "$components/company/CostBars.svelte";
  import BudgetBar from "$components/company/BudgetBar.svelte";
  import BudgetPolicyDialog from "$components/company/BudgetPolicyDialog.svelte";
  import { formatMetric, scopeName } from "$components/company/budget-format";
  import { tokens, watchCompany, dateTime } from "$components/company/company-format";
  import { usd } from "$components/company/company-ui";

  type Win = "day" | "month" | "lifetime";

  let company = $derived(page.params.id ?? "");
  let win = $state<Win>("month");
  let summary = $state<CostSummary | null>(null);
  let budgets = $state<PolicyStatus[]>([]);
  let incidents = $state<Incident[]>([]);
  let staff = $state<Staff[]>([]);
  let projects = $state<Project[]>([]);
  let loadError = $state("");
  let breakdown = $state<"agent" | "project" | "model">("agent");
  let dialog = $state<{ policy: BudgetPolicy | null } | null>(null);

  async function load() {
    const c = company;
    const w = win;
    if (!c) return;
    try {
      const [s, b, i, st, pr] = await Promise.all([
        rpc<CostSummary>("cost.summary", { company: c, window: w }),
        rpc<PolicyStatus[]>("budget.status", { company: c }),
        rpc<Incident[]>("budget.incidents", { company: c, open_only: false }),
        rpc<Staff[]>("staff.list", { company: c }),
        rpc<Project[]>("project.list", { company: c }),
      ]);
      if (c !== company || w !== win) return;
      summary = s;
      budgets = b;
      incidents = i;
      staff = st;
      projects = pr;
      loadError = "";
    } catch (e) {
      loadError = errorText(e);
    }
  }

  $effect(() => {
    void [company, win];
    void load();
  });
  $effect(() =>
    watchCompany(company, (batch) => {
      if (batch.some((c) => ["run", "cost", "budget", "incident", "staff", "project"].includes(c.entity))) void load();
    }),
  );

  let rows = $derived(
    !summary ? [] : breakdown === "agent" ? summary.by_agent : breakdown === "project" ? summary.by_project : summary.by_model,
  );

  function policyName(id: string): string {
    const p = budgets.find((b) => b.policy.id === id)?.policy;
    return p ? `${scopeName(p, staff, projects, $t)} · ${$t(`company.costs.metric.${p.metric}`)} / ${$t(`company.costs.window.${p.window}`)}` : id;
  }

  async function remove(p: BudgetPolicy) {
    if (!(await confirm($t("company.costs.delete_confirm")))) return;
    try {
      await rpc("budget.delete", { company, policy_id: p.id });
    } catch (e) {
      showToast("error", errorText(e));
    }
  }
</script>

<svelte:head><title>{$t("company.costs.title")}</title></svelte:head>

<div class="costs-page">
  <div class="bar">
    <div class="segmented" role="tablist" aria-label={$t("company.costs.window_label")}>
      {#each ["day", "month", "lifetime"] as w (w)}
        <button type="button" class="segmented-btn" role="tab" aria-selected={win === w} onclick={() => (win = w as Win)}>
          {$t(`company.costs.window.${w}`)}
        </button>
      {/each}
    </div>
    {#if summary}<span class="dim">{summary.window_key}</span>{/if}
  </div>

  {#if loadError}<p class="field-error" role="alert">{loadError}</p>{/if}

  {#if !summary}
    <div class="skeleton" style="height: 96px"></div>
  {:else}
    <div class="stats">
      <div class="stat surface-card">
        <span class="stat-label">{$t("company.costs.total")}</span>
        <span class="stat-value">US$ {usd(summary.cost_usd)}</span>
      </div>
      <div class="stat surface-card">
        <span class="stat-label">{$t("company.costs.tokens")}</span>
        <span class="stat-value">{tokens(summary.tokens)}</span>
      </div>
      <div class="stat surface-card">
        <span class="stat-label">{$t("company.costs.runs")}</span>
        <span class="stat-value">{summary.runs}</span>
      </div>
      <div class="stat surface-card">
        <span class="stat-label">{$t("company.costs.unpriced")}</span>
        <span class="stat-value">{summary.unpriced_events}</span>
        <span class="stat-hint">{$t("company.costs.unpriced_hint")}</span>
      </div>
    </div>

    <section class="section">
      <div class="section-head">
        <h2 class="section-header-title">{$t("company.costs.breakdown")}</h2>
        <div class="segmented">
          {#each ["agent", "project", "model"] as b (b)}
            <button type="button" class="segmented-btn" aria-pressed={breakdown === b} class:active={breakdown === b} onclick={() => (breakdown = b as typeof breakdown)}>
              {$t(`company.costs.by.${b}`)}
            </button>
          {/each}
        </div>
      </div>
      <CostBars {rows} />
    </section>
  {/if}

  <section class="section">
    <div class="section-head">
      <h2 class="section-header-title">{$t("company.costs.policies")}</h2>
      <button type="button" class="btn btn-primary btn-sm" onclick={() => (dialog = { policy: null })}>{$t("company.costs.new_policy")}</button>
    </div>
    {#if budgets.length === 0}
      <div class="empty-state small">
        <img class="empty-state-art" src="/emoji/package.png" alt="" width="72" height="72" draggable="false" />
        <p class="empty-state-title">{$t("company.costs.no_policies")}</p>
        <p class="empty-state-hint">{$t("company.costs.no_policies_hint")}</p>
      </div>
    {:else}
      <div class="group">
        {#each budgets as b (b.policy.id)}
          {@const p = b.policy}
          <div class="group-row policy">
            <div class="group-row-content">
              <span class="group-row-title">
                {scopeName(p, staff, projects, $t)}
                <span class="dim">· {$t(`company.costs.metric.${p.metric}`)} / {$t(`company.costs.window.${p.window}`)}</span>
                {#if p.hard_stop}<span class="tag">{$t("company.costs.hard_stop_tag")}</span>{/if}
                {#if !p.enabled}<span class="tag">{$t("company.costs.disabled")}</span>{/if}
                {#if b.open_incident}<span class="tag tag-danger">{$t("company.costs.state.incident")}</span>
                {:else if b.pct >= p.warn_pct}<span class="tag tag-warning">{$t("company.costs.state.warning")}</span>
                {:else}<span class="tag tag-success">{$t("company.costs.state.ok")}</span>{/if}
              </span>
              <BudgetBar pct={b.pct} warn={p.warn_pct} />
              <span class="group-row-sub num">
                {$t("company.costs.usage", {
                  used: formatMetric(p.metric, b.used),
                  limit: formatMetric(p.metric, p.amount),
                  pct: Math.round(b.pct),
                  warn: p.warn_pct,
                })}
              </span>
            </div>
            <div class="group-row-trailing">
              <button type="button" class="btn btn-ghost btn-sm" onclick={() => (dialog = { policy: p })}>{$t("company.common.edit")}</button>
              <button type="button" class="btn btn-ghost btn-sm" onclick={() => remove(p)}>{$t("company.common.delete")}</button>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </section>

  <section class="section">
    <h2 class="section-header-title">{$t("company.costs.incidents")}</h2>
    {#if incidents.length === 0}
      <p class="dim">{$t("company.costs.no_incidents")}</p>
    {:else}
      <div class="group">
        {#each incidents as i (i.id)}
          <div class="group-row">
            <span class="tag" class:tag-danger={i.kind === "hard"} class:tag-warning={i.kind === "soft"}>
              {i.kind === "hard" ? $t("company.inbox.incident_hard") : $t("company.inbox.incident_soft")}
            </span>
            <div class="group-row-content">
              <span class="group-row-title">{policyName(i.policy_id)}</span>
              <span class="group-row-sub num">{i.window_key} · {i.observed.toFixed(2)} / {i.limit_value.toFixed(2)}</span>
            </div>
            <div class="group-row-trailing">
              <span class="tag" class:tag-danger={i.state === "open"}>{$t(`company.costs.incident_state.${i.state}`)}</span>
              <span class="dim num">{dateTime(i.resolved_ms ?? i.created_ms)}</span>
            </div>
          </div>
        {/each}
      </div>
    {/if}
  </section>
</div>

{#if dialog}
  <BudgetPolicyDialog {company} {staff} {projects} policy={dialog.policy} onclose={() => (dialog = null)} />
{/if}

<style>
  .costs-page {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    max-width: 1000px;
  }
  .bar,
  .section-head {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .section-head {
    justify-content: space-between;
  }
  .stats {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: var(--space-3);
  }
  .stat {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: var(--space-3) var(--space-4);
  }
  .stat-label {
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .stat-value {
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: 600;
    letter-spacing: var(--track-tight);
    font-variant-numeric: tabular-nums;
  }
  .stat-hint {
    font-size: var(--text-xs);
    color: var(--text-dim);
  }
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .policy .group-row-content {
    gap: 6px;
  }
  .group-row-title {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }
  .dim {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-dim);
  }
  .num {
    font-variant-numeric: tabular-nums;
  }
  .empty-state.small {
    padding: var(--space-5);
  }
</style>
