/**
 * Agent jobs, loops and triggers. One load on first use, then the backend
 * pushes `llm://job` and `llm://loop`; rows are upserted by id.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { showToast } from "$lib/stores/toast-store.svelte";

export type JobState =
  | "queued"
  | "running"
  | "waiting_approval"
  | "done"
  | "failed"
  | "cancelled"
  /** The app closed while it ran: its result is unknown, a person decides. */
  | "interrupted";

export type Job = {
  id: string;
  kind: "run" | "chat" | "loop" | "trigger";
  agent_id: string;
  conversation_id: string;
  prompt: string;
  workspace: string | null;
  state: JobState;
  loop_id: string | null;
  trigger_id: string | null;
  request_id: string | null;
  created_ms: number;
  started_ms: number | null;
  finished_ms: number | null;
  result: string | null;
  error: string | null;
  log: string;
  /** Summed over the model calls of the job; `cost_usd` is null for local models and CLI accounts. */
  usage?: {
    model: string;
    input_tokens: number;
    output_tokens: number;
    cache_read_tokens: number;
    cost_usd: number | null;
    calls: number;
    /** Context pruning, per model request of the job. Measured, never a ratio. */
    prune?: {
      omitted: number;
      requests: {
        request: number;
        omitted: number;
        est_tokens_before: number;
        est_tokens_after: number;
        input_tokens: number | null;
      }[];
    };
  } | null;
};

/** `ollama/qwen3:8b · 8.2k in · 1.7k out · $0.0123` for a job row. */
export function usageLabel(job: Job): string {
  const u = job.usage;
  if (!u) return "";
  const k = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n));
  const parts = [u.model, `${k(u.input_tokens)} in`, `${k(u.output_tokens)} out`];
  if (u.cost_usd != null) parts.push(`$${u.cost_usd.toFixed(4)}`);
  const last = u.prune?.requests.at(-1);
  if (last && last.omitted > 0) {
    parts.push(`${last.omitted} omitted, est. ${k(last.est_tokens_before)} → ${k(last.est_tokens_after)}`);
  }
  return parts.filter(Boolean).join(" · ");
}

export type LoopDef = {
  id: string;
  name: string;
  agent_id: string;
  prompt: string;
  workspace: string | null;
  max_rounds: number | null;
  max_minutes: number | null;
  check_command: string | null;
  state: "running" | "done" | "failed" | "cancelled" | "interrupted";
  rounds_done: number;
  conversation_id: string;
  created_ms: number;
  finished_ms: number | null;
  last_check: string | null;
  stop_reason: string | null;
};

export type Trigger = {
  id: string;
  name: string;
  kind: "cron" | "webhook";
  cron: string | null;
  agent_id: string;
  prompt: string;
  workspace: string | null;
  enabled: boolean;
  last_fired_ms: number | null;
  fire_count: number;
  created_ms: number;
  /** Silenced: runs, never notifies. */
  muted?: boolean;
  /** Next run in the user's time zone (computed by the backend). */
  next_run_ms?: number | null;
  next_run_local?: string | null;
  utc_offset?: string;
  /** Routines only run while OmniGet is open. Always true today. */
  requires_app_open?: boolean;
};

export type BridgeInfo = { base_url: string; token: string };

export type PendingAsk = {
  agent: string;
  request_id: string;
  tool_call_id: string;
  tool: string;
  preview: string;
};

export type LoopInput = {
  agent_id: string;
  prompt: string;
  name?: string;
  workspace?: string | null;
  max_rounds?: number | null;
  max_minutes?: number | null;
  check_command?: string | null;
};

export type TriggerInput = {
  id?: string;
  name?: string;
  kind: "cron" | "webhook";
  cron?: string | null;
  agent_id: string;
  prompt: string;
  workspace?: string | null;
  enabled: boolean;
};

// ── Runs (durable record of every agent turn: `assist::runs`) ──────────

export type RunState =
  | "queued"
  | "preparing"
  | "running"
  | "waiting_user"
  | "completed"
  | "failed"
  | "cancelled"
  | "interrupted"
  | "unknown";

export type ResumeKind = "native" | "replay" | "new";

export type RunView = {
  id: string;
  session_id: string | null;
  conversation_id: string;
  bot_id: string;
  parent_run_id: string | null;
  state: RunState;
  resume_kind: ResumeKind | null;
  runtime: string | null;
  cwd: string | null;
  instance_id: string;
  launch_id: string | null;
  pid: number | null;
  input_preview: string;
  summary: string | null;
  error: string | null;
  resolution: string | null;
  tokens_in: number;
  tokens_out: number;
  /** `null` = unknown cost, never shown as free. */
  cost_usd: number | null;
  events_dropped: number;
  created_ms: number;
  started_ms: number | null;
  ended_ms: number | null;
  updated_ms: number;
  pending_permissions: number;
  live: boolean;
  actions: ("cancel" | "continue" | "mark_done" | "discard")[];
  tool_calls: number;
};

export type PermissionRequest = {
  id: string;
  run_id: string;
  tool_call_id: string;
  action: string;
  scope: string;
  preview: string;
  options: string[];
  deadline_ms: number;
  state: "pending" | "approved" | "denied" | "expired" | "cancelled";
  answer: string | null;
  version: number;
  created_ms: number;
  resolved_ms: number | null;
};

export type RunEvent = {
  id: string;
  run_id: string;
  seq: number;
  kind: string;
  provider_id: string | null;
  payload: Record<string, unknown>;
  ts_ms: number;
};

export type CommandRun = { command: string; ok: boolean | null; output: string; is_test: boolean };

export type RuntimeSession = {
  id: string;
  runtime: string;
  account: string | null;
  exe_version: string | null;
  cwd: string | null;
  context_kind: string;
  provider_handle: string | null;
  generation: number;
  native_resume: boolean;
};

export type RunDetail = RunView & {
  events: RunEvent[];
  permissions: PermissionRequest[];
  commands: CommandRun[];
  files_touched: string[];
  session: RuntimeSession | null;
};

export type FileChange = { path: string; status: string; additions: number | null; deletions: number | null };

export type RunDiff =
  | { available: true; workspace: string; diff: { files: FileChange[]; patch: string; clipped: boolean; complete: boolean } }
  | { available: false; reason: "no_workspace" | "no_changes" | "error"; workspace?: string; error?: string };

/** `assist://run` payload. */
export type RunUpdate = {
  run_id: string;
  conversation_id: string;
  bot_id: string;
  state: RunState;
  error?: string;
  resume_kind?: ResumeKind;
};

let runs = $state<RunView[]>([]);
let permissions = $state<PermissionRequest[]>([]);
/** Bumped on every `assist://run`, so an open detail can refresh. */
let runTick = $state(0);
let lastRunUpdate = $state<RunUpdate | null>(null);
let runsReload: ReturnType<typeof setTimeout> | null = null;

export function getRuns(): RunView[] {
  return runs;
}
export function getPermissions(): PermissionRequest[] {
  return permissions;
}
export function getRunTick(): number {
  return runTick;
}
export function getLastRunUpdate(): RunUpdate | null {
  return lastRunUpdate;
}

export async function reloadRuns(): Promise<void> {
  try {
    const [list, pending] = await Promise.all([
      invoke<RunView[]>("assist_runs_list", { limit: 150 }),
      invoke<PermissionRequest[]>("assist_permissions_pending"),
    ]);
    runs = list;
    permissions = pending;
  } catch {
    // The run record may be unavailable (no assistant database): the rest
    // of the page still works.
  }
}

function scheduleRunsReload() {
  if (runsReload) return;
  runsReload = setTimeout(() => {
    runsReload = null;
    void reloadRuns();
  }, 250);
}

export async function fetchRun(id: string): Promise<RunDetail | null> {
  try {
    return await invoke<RunDetail>("assist_run_get", { runId: id });
  } catch (e) {
    fail(e);
    return null;
  }
}

export async function fetchRunDiff(id: string): Promise<RunDiff | null> {
  try {
    return await invoke<RunDiff>("assist_run_diff", { runId: id });
  } catch (e) {
    fail(e);
    return null;
  }
}

/** `answer`: once | always | deny. A late answer is refused (expired). */
export async function answerPermission(id: string, answer: "once" | "always" | "deny"): Promise<boolean> {
  try {
    await invoke("assist_permission_answer", { permissionId: id, answer });
    permissions = permissions.filter((p) => p.id !== id);
    scheduleRunsReload();
    return true;
  } catch (e) {
    fail(e);
    scheduleRunsReload();
    return false;
  }
}

export async function cancelRun(id: string): Promise<void> {
  try {
    await invoke("assist_run_cancel", { runId: id });
  } catch (e) {
    fail(e);
  }
  scheduleRunsReload();
}

export async function resolveRun(id: string, action: "continue" | "mark_done" | "discard"): Promise<boolean> {
  try {
    await invoke("assist_run_resolve", { runId: id, action });
    scheduleRunsReload();
    return true;
  } catch (e) {
    fail(e);
    return false;
  }
}

// ── Recovery of interrupted jobs and Loops (explicit, never automatic) ──

export async function resumeJob(id: string): Promise<void> {
  try {
    jobs = upsert(jobs, await invoke<Job>("llm_job_resume", { id }));
  } catch (e) {
    fail(e);
  }
}

export async function markJobDone(id: string): Promise<void> {
  try {
    jobs = upsert(jobs, await invoke<Job>("llm_job_mark_done", { id }));
  } catch (e) {
    fail(e);
  }
}

export async function discardJob(id: string): Promise<void> {
  try {
    jobs = upsert(jobs, await invoke<Job>("llm_job_discard", { id }));
  } catch (e) {
    fail(e);
  }
}

export async function resumeLoop(id: string): Promise<void> {
  try {
    loops = upsert(loops, await invoke<LoopDef>("llm_loop_resume", { id }));
  } catch (e) {
    fail(e);
  }
}

export async function settleLoop(id: string, done: boolean): Promise<void> {
  try {
    loops = upsert(loops, await invoke<LoopDef>("llm_loop_settle", { id, done }));
  } catch (e) {
    fail(e);
  }
}

export async function muteTrigger(id: string, muted: boolean): Promise<void> {
  try {
    triggers = upsert(triggers, await invoke<Trigger>("llm_trigger_mute", { id, muted }));
  } catch (e) {
    fail(e);
  }
}

let jobs = $state<Job[]>([]);
let loops = $state<LoopDef[]>([]);
let triggers = $state<Trigger[]>([]);
let bridge = $state<BridgeInfo | null>(null);
let asks = $state<PendingAsk[]>([]);
/** Last `llm://job` payload, so an open row can refresh its log. */
let lastJobEvent = $state<Job | null>(null);
let started = false;

export function getJobs(): Job[] {
  return jobs;
}
export function getLoops(): LoopDef[] {
  return loops;
}
export function getTriggers(): Trigger[] {
  return triggers;
}
export function getBridge(): BridgeInfo | null {
  return bridge;
}
export function getAsks(): PendingAsk[] {
  return asks;
}
export function getLastJobEvent(): Job | null {
  return lastJobEvent;
}

function fail(e: unknown) {
  showToast("error", String(e));
}

function upsert<T extends { id: string }>(list: T[], row: T): T[] {
  const i = list.findIndex((x) => x.id === row.id);
  if (i < 0) return [row, ...list];
  const next = list.slice();
  next[i] = row;
  return next;
}

export async function reloadJobs(): Promise<void> {
  try {
    jobs = await invoke<Job[]>("llm_jobs_list", { limit: 200 });
  } catch (e) {
    fail(e);
  }
}

export async function reloadLoops(): Promise<void> {
  try {
    loops = await invoke<LoopDef[]>("llm_loops_list");
  } catch (e) {
    fail(e);
  }
}

export async function reloadTriggers(): Promise<void> {
  try {
    const out = await invoke<{ triggers: Trigger[]; bridge: BridgeInfo }>("llm_triggers_list");
    triggers = out.triggers ?? [];
    bridge = out.bridge ?? null;
  } catch (e) {
    fail(e);
  }
}

export function initJobsStore(): void {
  if (started) return;
  started = true;
  void reloadJobs();
  void reloadLoops();
  void reloadTriggers();
  void listen<Job>("llm://job", (ev) => {
    jobs = upsert(jobs, ev.payload);
    lastJobEvent = ev.payload;
  }).catch(() => {});
  void listen<LoopDef>("llm://loop", (ev) => {
    loops = upsert(loops, ev.payload);
  }).catch(() => {});
  void reloadRuns();
  void listen<RunUpdate>("assist://run", (ev) => {
    lastRunUpdate = ev.payload;
    runTick += 1;
    const i = runs.findIndex((r) => r.id === ev.payload.run_id);
    if (i >= 0) {
      const next = runs.slice();
      next[i] = {
        ...next[i],
        state: ev.payload.state,
        error: ev.payload.error ?? next[i].error,
        resume_kind: ev.payload.resume_kind ?? next[i].resume_kind,
      };
      runs = next;
    }
    scheduleRunsReload();
  }).catch(() => {});
}

export async function fetchJob(id: string): Promise<Job | null> {
  try {
    return await invoke<Job | null>("llm_job_get", { id });
  } catch (e) {
    fail(e);
    return null;
  }
}

export async function submitJob(agentId: string, prompt: string, workspace: string | null): Promise<Job | null> {
  try {
    const job = await invoke<Job>("llm_job_submit", { agentId, prompt, workspace });
    jobs = upsert(jobs, job);
    return job;
  } catch (e) {
    fail(e);
    return null;
  }
}

export async function cancelJob(id: string): Promise<void> {
  try {
    jobs = upsert(jobs, await invoke<Job>("llm_job_cancel", { id }));
  } catch (e) {
    fail(e);
  }
}

export async function deleteJob(id: string): Promise<void> {
  try {
    await invoke("llm_job_delete", { id });
    jobs = jobs.filter((j) => j.id !== id);
  } catch (e) {
    fail(e);
  }
}

export async function createLoop(def: LoopInput): Promise<LoopDef | null> {
  try {
    const loop = await invoke<LoopDef>("llm_loop_create", { def });
    loops = upsert(loops, loop);
    return loop;
  } catch (e) {
    fail(e);
    return null;
  }
}

export async function cancelLoop(id: string): Promise<void> {
  try {
    loops = upsert(loops, await invoke<LoopDef>("llm_loop_cancel", { id }));
  } catch (e) {
    fail(e);
  }
}

export async function deleteLoop(id: string): Promise<void> {
  try {
    await invoke("llm_loop_delete", { id });
    loops = loops.filter((l) => l.id !== id);
  } catch (e) {
    fail(e);
  }
}

export async function saveTrigger(trigger: TriggerInput): Promise<Trigger | null> {
  try {
    const saved = await invoke<Trigger>("llm_trigger_save", { trigger });
    triggers = upsert(triggers, saved);
    return saved;
  } catch (e) {
    fail(e);
    return null;
  }
}

export async function deleteTrigger(id: string): Promise<void> {
  try {
    await invoke("llm_trigger_delete", { id });
    triggers = triggers.filter((x) => x.id !== id);
  } catch (e) {
    fail(e);
  }
}

export async function fireTrigger(id: string): Promise<Job | null> {
  try {
    const job = await invoke<Job>("llm_trigger_fire", { id });
    jobs = upsert(jobs, job);
    return job;
  } catch (e) {
    fail(e);
    return null;
  }
}

export async function pollAsks(): Promise<void> {
  try {
    asks = await invoke<PendingAsk[]>("llm_tool_asks_pending");
  } catch {
    asks = [];
  }
}

export async function answerAsk(ask: PendingAsk, allow: boolean, always: boolean): Promise<void> {
  try {
    await invoke("llm_tool_answer", {
      requestId: ask.request_id,
      toolCallId: ask.tool_call_id,
      allow,
      always,
    });
    asks = asks.filter((a) => a.tool_call_id !== ask.tool_call_id);
  } catch (e) {
    fail(e);
  }
}

/** Shared by both pages. */
export function pickState(state: string): string {
  switch (state) {
    case "running":
    case "preparing":
      return "blue";
    case "waiting_approval":
    case "waiting_user":
    case "interrupted":
    case "unknown":
      return "orange";
    case "done":
    case "completed":
      return "green";
    case "failed":
      return "red";
    default:
      return "grey";
  }
}

export function shortTime(ms: number | null): string {
  if (!ms) return "";
  const d = new Date(ms);
  const sameDay = new Date().toDateString() === d.toDateString();
  return sameDay
    ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })
    : d.toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
}

export function duration(fromMs: number | null, toMs: number | null): string {
  if (!fromMs || !toMs || toMs < fromMs) return "";
  const s = Math.round((toMs - fromMs) / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

export async function pickFolder(): Promise<string | null> {
  try {
    const { open } = await import("@tauri-apps/plugin-dialog");
    const out = await open({ directory: true, multiple: false });
    return typeof out === "string" ? out : null;
  } catch (e) {
    fail(e);
    return null;
  }
}
