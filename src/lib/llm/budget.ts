/**
 * Budget policies and incidents (3.3): the shapes the backend sends and the
 * pure formatting the budgets panel, the incident card and the Work list
 * share. The live state is `$lib/stores/llm-budget-store.svelte`.
 */

export type BudgetMetric = "usd" | "weighted_tokens" | "quota_pct";
export type BudgetWindow = "turn" | "day" | "week" | "5h" | "7d" | "run";
export const BUDGET_METRICS: BudgetMetric[] = ["usd", "weighted_tokens", "quota_pct"];
export const BUDGET_WINDOWS: BudgetWindow[] = ["turn", "day", "week", "5h", "7d", "run"];
export const SCOPE_KINDS = ["global", "agent", "account", "loop", "job", "mission"] as const;
export type ScopeKind = (typeof SCOPE_KINDS)[number];

export type BudgetIncident = {
  id: string;
  policy_id: string;
  /** `global`, `agent:<id>`, `loop:<id>`, … */
  scope: string;
  metric: BudgetMetric;
  window: BudgetWindow;
  observed: number;
  limit: number;
  /** `open` needs someone; `paused` was kept paused on purpose; `resolved` lets work go on. */
  state: "open" | "paused" | "resolved";
  /** `hard` (the ceiling) or `checkpoint` (a "continue?" answered no). */
  kind: "hard" | "checkpoint";
  agent: string | null;
  opened_ms: number;
  resolved_ms: number | null;
  resolution: string | null;
};

export type BudgetPolicy = {
  id: string;
  scope: string;
  metric: BudgetMetric;
  window: BudgetWindow;
  amount: number;
  warn_pct: number | null;
  hard_stop: boolean;
  enabled: boolean;
  created_ms: number;
  updated_ms: number;
  /** Where it stands now; `null` = a quota policy with no reading yet. */
  observed: number | null;
  source: "ledger" | "real" | "none";
  incident: BudgetIncident | null;
};

export function formatAmount(metric: BudgetMetric, v: number): string {
  switch (metric) {
    case "usd":
      return `US$ ${v < 1 ? v.toFixed(4) : v.toFixed(2)}`;
    case "weighted_tokens":
      return v >= 1e6 ? `${(v / 1e6).toFixed(2)} M` : v >= 1e3 ? `${(v / 1e3).toFixed(1)} k` : String(Math.round(v));
    case "quota_pct":
      return `${v.toFixed(1)}%`;
  }
}

/** `loop:l123` → `loop l123`; `global` stays. */
export function scopeLabel(scope: string): string {
  const i = scope.indexOf(":");
  return i < 0 ? scope : `${scope.slice(0, i)} ${scope.slice(i + 1)}`;
}

/** Share of the ceiling used, 0..100 (for the bar). */
export function usedShare(observed: number | null, amount: number): number {
  if (observed === null || !(amount > 0)) return 0;
  return Math.max(0, Math.min(100, (observed / amount) * 100));
}

/** A suggested new ceiling for "raise and resume": half again above what is used. */
export function suggestedRaise(i: Pick<BudgetIncident, "metric" | "observed" | "limit">): number {
  const base = Math.max(i.observed, i.limit);
  const next = base * 1.5;
  if (i.metric === "quota_pct") return Math.min(100, Math.ceil(next));
  if (i.metric === "weighted_tokens") return Math.ceil(next / 1000) * 1000;
  return Math.ceil(next * 100) / 100;
}
