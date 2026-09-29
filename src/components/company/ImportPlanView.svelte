<script lang="ts">
  /** What an import (template or package) is about to create. */
  import type { ImportPlan } from "$lib/company/rpc";
  import { t } from "$lib/i18n";

  let { plan }: { plan: ImportPlan } = $props();
  let counts = $derived(
    (
      [
        ["goals", plan.goals],
        ["projects", plan.projects],
        ["routines", plan.routines],
        ["budgets", plan.budgets],
        ["issues", plan.issues],
      ] as const
    ).filter(([, n]) => n > 0),
  );
</script>

<div class="plan">
  <div class="plan-head">
    <span class="prefix">{plan.issue_prefix}</span>
    <strong>{plan.company}</strong>
  </div>
  {#if plan.staff.length}
    <div class="field-label">{$t("company.list.plan.staff", { count: plan.staff.length })}</div>
    <div class="chips">{#each plan.staff as s (s)}<span class="tag">{s}</span>{/each}</div>
  {/if}
  {#if counts.length}
    <ul class="counts">
      {#each counts as [k, n] (k)}<li><span class="num">{n}</span> {$t(`company.list.plan.${k}`)}</li>{/each}
    </ul>
  {/if}
  {#if plan.secrets_to_fill.length}
    <div class="notice notice-warning">
      <div class="notice-body">
        <div class="notice-title">{$t("company.list.plan.secrets")}</div>
        <div class="notice-text">{plan.secrets_to_fill.join(", ")}</div>
      </div>
    </div>
  {/if}
  {#each plan.warnings as w, i (i)}<p class="warn">{w}</p>{/each}
</div>

<style>
  .plan { display: flex; flex-direction: column; gap: var(--space-2); padding: var(--space-3); border-radius: var(--radius-md); background: var(--fill-1); }
  .plan-head { display: flex; align-items: center; gap: var(--space-2); }
  .prefix { font-family: var(--font-mono); font-size: var(--text-xs); padding: 1px 6px; border-radius: var(--radius-xs); background: var(--fill-2); color: var(--text-muted); }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; }
  .counts { display: flex; flex-wrap: wrap; gap: 4px var(--space-4); margin: 0; padding: 0; list-style: none; font-size: var(--text-sm); color: var(--text-muted); }
  .num { font-variant-numeric: tabular-nums; font-weight: 600; color: var(--text); }
  .warn { margin: 0; font-size: var(--text-xs); color: var(--warning); }
</style>
