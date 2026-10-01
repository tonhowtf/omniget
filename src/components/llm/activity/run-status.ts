import type { RunState, RunView } from "$lib/stores/llm-jobs-store.svelte";

type RunStatus = "queued" | "working" | "waiting" | "interrupted" | "failed" | "done" | "cancelled";

const CANCELLED_ERROR = /\b(ERR_LLM_CANCELLED|EXTERNAL_EXECUTION_CANCELLED|ERR_JOB_CANCELLED)\b|^(job )?cancel+ed\b/i;

export function runStatus(state: RunState, error?: string | null): RunStatus {
  if (state === "failed" && error && CANCELLED_ERROR.test(error.trim())) return "cancelled";
  switch (state) {
    case "queued":
      return "queued";
    case "preparing":
    case "running":
      return "working";
    case "waiting_user":
      return "waiting";
    case "interrupted":
    case "unknown":
      return "interrupted";
    case "failed":
      return "failed";
    case "completed":
      return "done";
    case "cancelled":
      return "cancelled";
  }
}

export function statusTone(status: RunStatus): string {
  switch (status) {
    case "working":
      return "blue";
    case "waiting":
    case "interrupted":
      return "orange";
    case "done":
      return "green";
    case "failed":
      return "red";
    default:
      return "grey";
  }
}

export type RunFilter = "all" | "active" | "attention" | "done";

export function matchesFilter(run: RunView, filter: RunFilter): boolean {
  const s = runStatus(run.state, run.error);
  switch (filter) {
    case "active":
      return s === "queued" || s === "working" || s === "waiting";
    case "attention":
      return (
        s === "waiting" ||
        s === "failed" ||
        (s === "interrupted" && !run.resolution) ||
        run.pending_permissions > 0
      );
    case "done":
      return s === "done" || s === "cancelled" || (s === "interrupted" && !!run.resolution);
    default:
      return true;
  }
}

export function diffLineKind(line: string): "add" | "del" | "hunk" | "file" | "ctx" {
  if (line.startsWith("+++") || line.startsWith("---") || line.startsWith("diff --git")) return "file";
  if (line.startsWith("@@")) return "hunk";
  if (line.startsWith("+")) return "add";
  if (line.startsWith("-")) return "del";
  return "ctx";
}

export function parseResolution(resolution: string | null): { kind: string; ref: string | null } | null {
  if (!resolution) return null;
  const i = resolution.indexOf(":");
  if (i < 0) return { kind: resolution, ref: null };
  return { kind: resolution.slice(0, i), ref: resolution.slice(i + 1) };
}

export function secondsLeft(deadlineMs: number, nowMs: number): number {
  return Math.max(0, Math.round((deadlineMs - nowMs) / 1000));
}

export function promptTitle(text: string): { title: string; mission: string | null } {
  const lines = text.split("\n").map((l) => l.trim()).filter(Boolean);
  const head = /^\[Mission ([A-Za-z0-9-]+) — objective\]$/.exec(lines[0] ?? "");
  if (head) return { title: lines[1] ?? "", mission: head[1] };
  return { title: lines[0] ?? "", mission: null };
}
