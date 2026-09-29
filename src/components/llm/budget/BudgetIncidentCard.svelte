<script lang="ts">
  /**
   * One budget incident (3.3, 80#P3): a scope crossed its hard ceiling (or
   * someone refused a checkpoint), so it starts nothing. Two answers: raise
   * the ceiling above what is used and resume (paused Loops start again), or
   * keep it paused. The same answers exist in the CLI:
   * `omniget agent budget raise <id> <amount>`.
   */
  import { t } from "$lib/i18n";
  import {
    formatAmount,
    resolveIncident,
    scopeLabel,
    suggestedRaise,
    type BudgetIncident,
  } from "$lib/stores/llm-budget-store.svelte";

  let { incident, highlight = false }: { incident: BudgetIncident; highlight?: boolean } = $props();

  let amount = $state<number | null>(null);
  let busy = $state(false);
  let raiseTo = $derived(amount ?? suggestedRaise(incident));
  let tooLow = $derived(!(raiseTo > incident.observed));

  async function raise() {
    if (busy || tooLow) return;
    busy = true;
    await resolveIncident(incident.id, "raise", raiseTo);
    busy = false;
  }

  async function keep() {
    if (busy) return;
    busy = true;
    await resolveIncident(incident.id, "keep_paused");
    busy = false;
  }
</script>

<article class="incident surface-card" class:highlight id={`incident-${incident.id}`} aria-live="polite">
  <div class="head">
    <span class="pill orange">{$t(incident.state === "paused" ? "llm.budget.incident.kept" : "llm.budget.incident.open")}</span>
    <strong class="scope">{scopeLabel(incident.scope)}</strong>
    <span class="dim">{$t(`llm.budget.window.${incident.window}`)}</span>
  </div>
  <p class="line">
    {$t(incident.kind === "checkpoint" ? "llm.budget.incident.checkpoint_line" : "llm.budget.incident.hard_line", {
      used: formatAmount(incident.metric, incident.observed),
      limit: formatAmount(incident.metric, incident.limit),
    })}
  </p>
  <div class="actions">
    <label class="field">
      <span class="field-label">{$t("llm.budget.incident.new_limit")}</span>
      <input
        class="input number"
        type="number"
        min="0"
        step="any"
        value={raiseTo}
        disabled={busy}
        oninput={(e) => (amount = Number((e.currentTarget as HTMLInputElement).value))}
      />
    </label>
    <button type="button" class="button active" disabled={busy || tooLow} onclick={raise}>
      {$t("llm.budget.incident.raise")}
    </button>
    {#if incident.state === "open"}
      <button type="button" class="button" disabled={busy} onclick={keep}>{$t("llm.budget.incident.keep")}</button>
    {/if}
  </div>
  {#if tooLow}<p class="hint danger">{$t("llm.budget.incident.too_low")}</p>{/if}
  <p class="hint"><code>omniget agent budget raise {incident.id} {raiseTo}</code></p>
</article>

<style>
  .incident {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4);
    border-left: 3px solid var(--warning, #d97706);
  }
  .incident.highlight {
    box-shadow: 0 0 0 2px var(--warning, #d97706);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .scope {
    font-family: var(--font-mono, monospace);
  }
  .line {
    margin: 0;
  }
  .actions {
    display: flex;
    align-items: flex-end;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .number {
    width: 12ch;
  }
  .dim,
  .hint {
    color: var(--text-dim);
    font-size: var(--text-sm);
  }
  .hint {
    margin: 0;
  }
  .danger {
    color: var(--danger);
  }
</style>
