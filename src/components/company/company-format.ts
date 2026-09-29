// Small formatting helpers shared by the company issue, inbox, costs and
// activity screens (times and money come from ./company-ui). Status and
// priority names live under `company.common.*`.
import { onCompanyChange, type Change, type IssueStatus, type Priority, type Run, type Staff } from "$lib/company/rpc";

export const BOARD_COLUMNS: IssueStatus[] = ["backlog", "todo", "in_progress", "in_review", "blocked", "done"];
export const LIST_ORDER: IssueStatus[] = ["in_progress", "in_review", "blocked", "todo", "backlog", "done", "cancelled"];
export const PRIORITIES: Priority[] = ["critical", "high", "medium", "low"];

export const issueStatusKey = (s: string) => `company.common.status.${s}`;
export const priorityKey = (p: string) => `company.common.priority.${p}`;
export const runStateKey = (s: string) => `company.common.run_state.${s}`;
export const staffStatusKey = (s: string) => `company.common.staff_status.${s}`;

/** Status → color token, the same palette as the rest of /llm. */
export function issueStatusColor(s: string): string {
  switch (s) {
    case "in_progress":
      return "var(--blue)";
    case "in_review":
      return "var(--purple)";
    case "blocked":
      return "var(--red)";
    case "done":
      return "var(--green)";
    case "todo":
      return "var(--text-muted)";
    default:
      return "var(--text-faint)";
  }
}

export function priorityColor(p: string): string {
  switch (p) {
    case "critical":
      return "var(--red)";
    case "high":
      return "var(--orange)";
    case "medium":
      return "var(--text-muted)";
    default:
      return "var(--text-faint)";
  }
}

export function runStateColor(s: string): string {
  switch (s) {
    case "running":
      return "var(--blue)";
    case "queued":
      return "var(--text-muted)";
    case "succeeded":
      return "var(--green)";
    case "failed":
    case "timed_out":
      return "var(--red)";
    default:
      return "var(--text-faint)";
  }
}




const compact = new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 });
export function tokens(v: number): string {
  return compact.format(v || 0);
}

export function runTokens(r: Run): number {
  return (r.input_tokens || 0) + (r.output_tokens || 0);
}


export function staffName(staff: Staff[], id: string | null | undefined): string {
  if (!id) return "";
  return staff.find((s) => s.id === id)?.name ?? id.slice(0, 8);
}

export function dateTime(ms: number | null | undefined): string {
  if (!ms) return "";
  return new Date(ms).toLocaleString(undefined, { dateStyle: "short", timeStyle: "short" });
}

/** Where a run's job opens; the jobs page lists every job. */
export function jobHref(jobId: string): string {
  return `/llm/jobs?job=${encodeURIComponent(jobId)}`;
}

/** Live updates for one company; returns the cleanup (use inside `$effect`). */
export function watchCompany(company: string, fn: (batch: Change[]) => void): () => void {
  let off: (() => void) | undefined;
  let alive = true;
  void onCompanyChange(company, fn).then((u) => {
    if (alive) off = u;
    else u();
  });
  return () => {
    alive = false;
    off?.();
  };
}
