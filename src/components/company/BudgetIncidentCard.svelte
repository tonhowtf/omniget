<script lang="ts">
  import { t } from "$lib/i18n";
  import { rpc, errorText, type Incident, type PolicyStatus } from "$lib/company/rpc";
  import BudgetBar from "./BudgetBar.svelte";
  import { relTime } from "./company-ui";
  import { formatMetric, scopeName } from "./budget-format";
  import type { Project, Staff } from "$lib/company/rpc";

  let {
    incident,
    status = null,
    staff,
    projects,
  }: { incident: Incident; status?: PolicyStatus | null; staff: Staff[]; projects: Project[] } = $props();

  let policy = $derived(status?.policy ?? null);
  let used = $derived(status?.used ?? incident.observed);
  let raise = $state<number | null>(null);
  let busy = $state(false);
  let error = $state("");

  $effect(() => {
    if (raise === null) {
      const base = Math.max(incident.limit_value, used);
      raise = Math.ceil(base * 1.5 * 100) / 100 || 1;
    }
  });

  let valid = $derived(typeof raise === "number" && raise > used);

  async function resolve(withRaise: boolean) {
    busy = true;
    error = "";
    try {
      await rpc("budget.resolve", { incident: incident.id, raise_to: withRaise ? raise : null });
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<article class="incident surface-card">
  <header class="head">
    <span class="kind" class:hard={incident.kind === "hard"}>
      {incident.kind === "hard" ? $t("company.inbox.incident_hard") : $t("company.inbox.incident_soft")}
    </span>
    <span class="who">{relTime(incident.created_ms, $t)}</span>
  </header>
  <h3 class="title">
    {policy ? scopeName(policy, staff, projects, $t) : $t("company.inbox.budget")}
  </h3>
  <p class="line">
    {$t("company.inbox.incident_line", {
      used: policy ? formatMetric(policy.metric, used) : used.toFixed(2),
      limit: policy ? formatMetric(policy.metric, incident.limit_value) : incident.limit_value.toFixed(2),
      window: incident.window_key,
    })}
  </p>
  <BudgetBar pct={incident.limit_value > 0 ? (used / incident.limit_value) * 100 : 100} warn={policy?.warn_pct ?? 80} />
  <div class="actions">
    <label class="raise">
      <span class="field-label">{$t("company.inbox.new_limit")}</span>
      <input class="input" type="number" min="0" step="any" bind:value={raise} />
    </label>
    <span class="spacer"></span>
    <button type="button" class="btn btn-secondary btn-sm" disabled={busy} onclick={() => resolve(false)}>{$t("company.inbox.keep_paused")}</button>
    <button type="button" class="btn btn-primary btn-sm" disabled={busy || !valid} onclick={() => resolve(true)}>{$t("company.inbox.raise_resume")}</button>
  </div>
  {#if !valid}<p class="field-hint">{$t("company.inbox.raise_above", { used: used.toFixed(2) })}</p>{/if}
  {#if error}<p class="field-error" role="alert">{error}</p>{/if}
</article>

<style>
  .incident {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4);
  }
  .head {
    display: flex;
    align-items: center;
    font-size: var(--text-xs);
  }
  .kind {
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: var(--track-caps);
    color: var(--orange);
  }
  .kind.hard {
    color: var(--red);
  }
  .who {
    margin-left: auto;
    color: var(--text-dim);
  }
  .title {
    margin: 0;
    font-size: var(--text-md);
    font-weight: 600;
  }
  .line {
    margin: 0;
    font-size: var(--text-sm);
    color: var(--text-muted);
    font-variant-numeric: tabular-nums;
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--space-2);
  }
  .raise {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .raise .input {
    width: 140px;
  }
  .spacer {
    flex: 1;
  }
</style>
