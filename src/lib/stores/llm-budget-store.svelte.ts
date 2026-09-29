/**
 * Budget policies and incidents (roadmap 3.3, S18, D5).
 *
 * The backend (`llm_budget.rs` over `core::llm::budget::policy`) owns the
 * ceilings, the ledger and the incidents; this store reads `llm_budget_state`
 * once and again on every `llm://budget` event (a policy saved or removed,
 * an incident opened or settled, a warning). No polling.
 *
 * An open incident is "needs you" work (`/llm/work`): the scope is paused
 * until someone raises the ceiling or keeps it paused.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { get } from "svelte/store";
import { t } from "$lib/i18n";
import { showToast } from "$lib/stores/toast-store.svelte";
import { addLog } from "$lib/stores/debug-store.svelte";
import { formatAmount, scopeLabel } from "$lib/llm/budget";

export { formatAmount, scopeLabel, suggestedRaise, usedShare } from "$lib/llm/budget";

import type { BudgetIncident, BudgetMetric, BudgetPolicy, BudgetWindow } from "$lib/llm/budget";
export type { BudgetIncident, BudgetMetric, BudgetPolicy, BudgetWindow, ScopeKind } from "$lib/llm/budget";
export { BUDGET_METRICS, BUDGET_WINDOWS, SCOPE_KINDS } from "$lib/llm/budget";

export type PolicyInput = {
  id?: string;
  scope: string;
  metric: BudgetMetric;
  window: BudgetWindow;
  amount: number;
  warn_pct?: number | null;
  hard_stop?: boolean;
  enabled?: boolean;
};

type BudgetEvent = {
  kind: "incident_opened" | "incident_changed" | "warn" | "policy_saved" | "policy_removed";
  incident?: BudgetIncident;
  policy?: Omit<BudgetPolicy, "observed" | "source" | "incident">;
  observed?: number;
};

let policies = $state<BudgetPolicy[]>([]);
let incidents = $state<BudgetIncident[]>([]);
let readError = $state<string | null>(null);
let started = false;

export async function reloadBudget(): Promise<void> {
  try {
    const v = await invoke<{ policies: BudgetPolicy[]; incidents: BudgetIncident[] }>("llm_budget_state");
    policies = v.policies ?? [];
    incidents = v.incidents ?? [];
    readError = null;
  } catch (e) {
    readError = String(e);
    addLog("error", "system", `budget: state read failed: ${String(e)}`);
  }
}

/** Idempotent: the /llm layout and the budgets panel both call it. */
export function initBudgetStore(): void {
  if (started) return;
  started = true;
  void reloadBudget();
  void listen<BudgetEvent>("llm://budget", (e) => {
    const ev = e.payload;
    if (ev?.kind === "warn" && ev.policy) {
      const tr = get(t);
      showToast(
        "info",
        tr("llm.budget.warn_toast", {
          scope: scopeLabel(ev.policy.scope),
          used: formatAmount(ev.policy.metric, ev.observed ?? 0),
          limit: formatAmount(ev.policy.metric, ev.policy.amount),
        }) as string,
      );
    }
    void reloadBudget();
  }).catch((err) => {
    readError = String(err);
  });
}

export function getBudgetPolicies(): BudgetPolicy[] {
  return policies;
}

export function getBudgetIncidents(): BudgetIncident[] {
  return incidents;
}

/** Open or kept-paused incidents: the scopes that start nothing now. */
export function getBlockingIncidents(): BudgetIncident[] {
  return incidents.filter((i) => i.state === "open" || i.state === "paused");
}

export function getBudgetReadError(): string | null {
  return readError;
}

function fail(e: unknown): void {
  showToast("error", String(e));
  addLog("error", "system", `budget: ${String(e)}`);
}

export async function saveBudgetPolicy(policy: PolicyInput): Promise<boolean> {
  try {
    await invoke("llm_budget_policy_save", { policy });
    await reloadBudget();
    return true;
  } catch (e) {
    fail(e);
    return false;
  }
}

export async function deleteBudgetPolicy(id: string): Promise<void> {
  try {
    await invoke("llm_budget_policy_delete", { id });
    await reloadBudget();
  } catch (e) {
    fail(e);
  }
}

/** "Raise and resume" (`raise` with the new ceiling) or "keep paused". */
export async function resolveIncident(id: string, action: "raise" | "keep_paused", amount?: number): Promise<boolean> {
  try {
    await invoke("llm_budget_incident_resolve", { id, action, amount: amount ?? null });
    await reloadBudget();
    return true;
  } catch (e) {
    fail(e);
    return false;
  }
}
