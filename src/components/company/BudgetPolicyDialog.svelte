<script lang="ts">
  import { untrack } from "svelte";
  import { t } from "$lib/i18n";
  import { rpc, errorText, type BudgetPolicy, type Project, type Staff } from "$lib/company/rpc";

  let {
    company,
    staff,
    projects,
    policy = null,
    onclose,
  }: { company: string; staff: Staff[]; projects: Project[]; policy?: BudgetPolicy | null; onclose: () => void } = $props();

  const initial = untrack(() => policy);
  const editing = !!initial;
  let scopeType = $state<"company" | "agent" | "project">(initial?.scope_type ?? "company");
  let scopeId = $state(initial && initial.scope_type !== "company" ? initial.scope_id : "");
  let metric = $state<"usd" | "tokens" | "runs">(initial?.metric ?? "usd");
  let win = $state<"day" | "month" | "lifetime">(initial?.window ?? "month");
  let amount = $state<number | null>(initial?.amount ?? null);
  let warnPct = $state<number>(initial?.warn_pct ?? 80);
  let hardStop = $state(initial?.hard_stop ?? true);
  let busy = $state(false);
  let error = $state("");

  let options = $derived(
    scopeType === "agent"
      ? staff.filter((s) => s.status !== "terminated").map((s) => ({ id: s.id, name: s.name }))
      : scopeType === "project"
        ? projects.map((p) => ({ id: p.id, name: p.name }))
        : [],
  );

  $effect(() => {
    if (scopeType !== "company" && !options.some((o) => o.id === scopeId)) scopeId = options[0]?.id ?? "";
  });

  let valid = $derived(
    typeof amount === "number" && amount > 0 && (scopeType === "company" || !!scopeId) && warnPct >= 1 && warnPct <= 100,
  );

  async function submit(e: Event) {
    e.preventDefault();
    if (!valid || busy) return;
    busy = true;
    error = "";
    try {
      await rpc("budget.set", {
        company,
        policy: {
          scope_type: scopeType,
          scope_id: scopeType === "company" ? null : scopeId,
          metric,
          window: win,
          amount,
          warn_pct: Math.round(warnPct),
          hard_stop: hardStop,
        },
      });
      onclose();
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }
</script>

<div class="dialog-backdrop" role="dialog" aria-modal="true" aria-labelledby="policy-title" tabindex="-1" onkeydown={(e) => e.key === "Escape" && onclose()} onclick={(e) => e.target === e.currentTarget && onclose()}>
  <form
    class="dialog-card"
    onsubmit={submit}
   
  >
    <h2 class="dialog-title" id="policy-title">{editing ? $t("company.costs.edit_policy") : $t("company.costs.new_policy")}</h2>
    <div class="grid">
      <label class="field">
        <span class="field-label">{$t("company.costs.scope_label")}</span>
        <select class="input" bind:value={scopeType} disabled={editing}>
          <option value="company">{$t("company.costs.scope.company")}</option>
          <option value="agent">{$t("company.costs.scope.agent")}</option>
          <option value="project">{$t("company.costs.scope.project")}</option>
        </select>
      </label>
      {#if scopeType !== "company"}
        <label class="field">
          <span class="field-label">{scopeType === "agent" ? $t("company.costs.scope.agent") : $t("company.costs.scope.project")}</span>
          <select class="input" bind:value={scopeId} disabled={editing}>
            {#each options as o (o.id)}<option value={o.id}>{o.name}</option>{/each}
          </select>
        </label>
      {/if}
      <label class="field">
        <span class="field-label">{$t("company.costs.metric_label")}</span>
        <select class="input" bind:value={metric} disabled={editing}>
          {#each ["usd", "tokens", "runs"] as m (m)}<option value={m}>{$t(`company.costs.metric.${m}`)}</option>{/each}
        </select>
      </label>
      <label class="field">
        <span class="field-label">{$t("company.costs.window_label")}</span>
        <select class="input" bind:value={win} disabled={editing}>
          {#each ["day", "month", "lifetime"] as w (w)}<option value={w}>{$t(`company.costs.window.${w}`)}</option>{/each}
        </select>
      </label>
      <label class="field">
        <span class="field-label">{$t("company.costs.amount")}</span>
        <input class="input" type="number" min="0" step="any" bind:value={amount} />
      </label>
      <label class="field">
        <span class="field-label">{$t("company.costs.warn_pct")}</span>
        <input class="input" type="number" min="1" max="100" bind:value={warnPct} />
      </label>
    </div>
    <label class="check">
      <input type="checkbox" class="checkbox" bind:checked={hardStop} />
      <span>
        {$t("company.costs.hard_stop")}
        <span class="field-hint">{$t("company.costs.hard_stop_hint")}</span>
      </span>
    </label>
    {#if error}<p class="field-error" role="alert">{error}</p>{/if}
    <div class="dialog-actions">
      <button type="button" class="btn btn-secondary" onclick={onclose}>{$t("company.common.cancel")}</button>
      <button type="submit" class="btn btn-primary" disabled={!valid || busy}>{$t("company.common.save")}</button>
    </div>
  </form>
</div>

<style>
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
  }
  .check {
    display: flex;
    gap: var(--space-2);
    align-items: flex-start;
    font-size: var(--text-sm);
  }
  .check .field-hint {
    display: block;
  }
</style>
