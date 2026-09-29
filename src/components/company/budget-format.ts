// Formatting of budget policies (scope names and metric values).
import type { BudgetPolicy, Project, Staff } from "$lib/company/rpc";
import { usd } from "./company-ui";
import { tokens } from "./company-format";

type Translate = (key: string, vars?: Record<string, unknown>) => string;

export function formatMetric(metric: string, value: number): string {
  if (metric === "usd") return `US$ ${usd(value)}`;
  if (metric === "tokens") return tokens(value);
  return String(Math.round(value));
}

export function scopeName(p: BudgetPolicy, staff: Staff[], projects: Project[], t: Translate): string {
  if (p.scope_type === "agent") return staff.find((s) => s.id === p.scope_id)?.name ?? p.scope_id;
  if (p.scope_type === "project") return projects.find((x) => x.id === p.scope_id)?.name ?? p.scope_id;
  return t("company.costs.scope.company");
}
