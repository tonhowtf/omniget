export interface BudgetIncident {
  id: string;
  state: string;
  observed: number;
  limit: number;
}

export interface PolicyStatus {
  id: string;
  metric: string;
  window: string;
  amount: number;
  enabled: boolean;
  warn_pct?: number | null;
  observed?: number | null;
  incident?: BudgetIncident | null;
}

export interface Meter {
  policyId: string;
  metric: string;
  window: string;
  spent: number;
  limit: number;
  ratio: number;
  tone: "ok" | "warn" | "paused";
  incidentId: string | null;
}

const DEFAULT_WARN_PCT = 80;

function blocks(i: BudgetIncident | null | undefined): i is BudgetIncident {
  return !!i && (i.state === "open" || i.state === "paused");
}

export function pickMeter(policies: PolicyStatus[]): Meter | null {
  const live = policies.filter((p) => p && p.enabled && p.amount > 0);
  const hit = live.find((p) => blocks(p.incident));
  if (hit && hit.incident) {
    return {
      policyId: hit.id,
      metric: hit.metric,
      window: hit.window,
      spent: Math.max(hit.observed ?? 0, hit.incident.observed),
      limit: hit.amount,
      ratio: 1,
      tone: "paused",
      incidentId: hit.incident.id,
    };
  }
  let best: Meter | null = null;
  for (const p of live) {
    if (typeof p.observed !== "number" || !Number.isFinite(p.observed)) continue;
    const ratio = Math.min(1, Math.max(0, p.observed / p.amount));
    if (best && ratio <= best.ratio) continue;
    best = {
      policyId: p.id,
      metric: p.metric,
      window: p.window,
      spent: p.observed,
      limit: p.amount,
      ratio,
      tone: ratio * 100 >= (p.warn_pct ?? DEFAULT_WARN_PCT) ? "warn" : "ok",
      incidentId: null,
    };
  }
  return best;
}

export function raiseTo(m: Pick<Meter, "metric" | "limit" | "spent">): number | null {
  const doubled = m.limit * 2;
  const wanted = doubled > m.spent ? doubled : Math.ceil(m.spent * 1.5);
  const next = m.metric === "quota_pct" ? Math.min(100, wanted) : wanted;
  return next > Math.max(m.spent, m.limit) ? next : null;
}
