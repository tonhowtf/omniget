import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { showToast } from "$lib/stores/toast-store.svelte";
import { addLog } from "$lib/stores/debug-store.svelte";

type JobState =
  | "queued"
  | "waiting_workspace"
  | "running"
  | "waiting_approval"
  | "done"
  | "review"
  | "done_unreported"
  | "failed"
  | "cancelled"
  | "interrupted";

export type Isolation = "shared" | "worktree" | "auto";

type WorktreeRemoval = {
  dirty: string[];
  saved: string | null;
  preserved_branch: string | null;
  branch_deleted: string | null;
  trashed: boolean;
};

type JobProof = {
  generated_by: "host";
  stamp: string;
  mode: "worktree" | "shared";
  branch: string | null;
  base: string | null;
  head: string | null;
  file_count: number;
  additions: number | null;
  deletions: number | null;
  checks: { command: string; exit_code: number | null; ok: boolean; at_ms: number }[];
  tokens: { input: number | null; output: number | null; cache_read: number | null; billable: number | null };
  cost_usd: number | null;
  duration_ms: number | null;
};

export type Job = {
  id: string;
  kind: "run" | "chat" | "loop" | "trigger" | "company";
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
  state_seq?: number;
  seen_ms?: number | null;
  failure_reason?: string | null;
  session_id?: string | null;
  attempt?: number;
  auto_resume?: boolean | null;
  deadline_min?: number | null;
  deadline_ms?: number | null;
  wait_reason?: string | null;
  diff_files?: FileChange[] | null;
  additions?: number | null;
  deletions?: number | null;
  diff_clipped?: boolean;
  diff_unavailable?: string | null;
  isolation?: Isolation;
  run_mode?: import("./llm-run-mode.svelte").RunMode;
  base_ref?: string | null;
  worktree_path?: string | null;
  branch?: string | null;
  base_oid?: string | null;
  head_oid?: string | null;
  proof?: JobProof | null;
  title?: string | null;
  outcome?: { status: "succeeded" | "failed"; summary: string } | null;
  usage?: {
    model: string;
    input_tokens: number;
    output_tokens: number;
    cache_read_tokens: number;
    cost_usd: number | null;
    calls: number;
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
  state: "running" | "done" | "failed" | "cancelled" | "interrupted" | "paused" | "blocked" | "budget_limited";
  rounds_done: number;
  conversation_id: string;
  created_ms: number;
  finished_ms: number | null;
  last_check: string | null;
  stop_reason: string | null;
  auto_resume?: boolean | null;
  run_mode?: import("./llm-run-mode.svelte").RunMode | null;
  max_cost_usd?: number | null;
  spent_usd?: number;
  spent_usd_estimated?: number;
  spent_tokens?: number;
  unpriced_rounds?: number;
  token_budget?: number | null;
  tokens_used?: number;
  note?: string | null;
  audits?: LoopAudit[];
  audit_usd?: number;
  rounds_guided?: number;
};

export type LoopAudit = {
  round: number;
  verdict: "continue" | "complete" | "blocked" | null;
  note: string;
  claimed_done: boolean;
  model: string;
  reason: string;
  error: string | null;
  cost_usd: number | null;
  tokens: number;
  ms: number;
  round_cost_usd: number | null;
  round_tokens: number;
  guided: boolean;
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
  muted?: boolean;
  next_run_ms?: number | null;
  next_run_local?: string | null;
  utc_offset?: string;
  requires_app_open?: boolean;
  catch_up?: CatchUp;
  concurrency?: TriggerConcurrency;
  tz?: string;
  zone_known?: boolean;
  missed?: number;
  last_missed_ms?: number | null;
  backlog?: number;
  last_run_id?: string | null;
  mission?: unknown;
  deadline_min?: number | null;
};

export type CatchUp = "skip" | "once" | "all_capped";
export const CATCH_UP_CAP = 25;
export type TriggerConcurrency = "skip_if_active" | "queue";

export type BridgeInfo = { base_url: string };

type QueuedEntry = {
  id: string;
  position: number;
  reason: "slot" | "agent_cap" | "workspace";
  holder?: string;
  folder?: string;
  detail: string;
  agent_id: string;
  kind: string | null;
  state: string;
  workspace: string | null;
};

type HeldEntry = { id: string; agent_id: string; kind: string | null; state: string; workspace: string | null };

type QueueState = {
  running: HeldEntry[];
  blocked: (HeldEntry & { reason: "approval" })[];
  queued: QueuedEntry[];
  limits: { max_parallel: number; max_per_agent: number | null; in_use: number; available: number; stagger_ms: number };
};

export type I18nLine = { key: string; params: Record<string, string | number> };

export function reasonLine(
  code: string | null | undefined,
  state: QueueState | null,
  holder?: string,
): I18nLine | null {
  if (!code) return null;
  const [kind, stored] = code.split(":", 2);
  if (kind === "workspace")
    return { key: "llm.jobs.queue.reason_workspace", params: { holder: holder ?? stored ?? "?" } };
  if (kind === "agent_cap")
    return { key: "llm.jobs.queue.reason_agent_cap", params: { cap: state?.limits.max_per_agent ?? "?" } };
  return { key: "llm.jobs.queue.reason_slot", params: { limit: state?.limits.max_parallel ?? "?" } };
}

export function queueLines(entry: QueuedEntry, state: QueueState): { position: I18nLine; reason: I18nLine } {
  return {
    position: { key: "llm.jobs.queue.position", params: { position: entry.position, total: state.queued.length } },
    reason: reasonLine(entry.reason, state, entry.holder) ?? { key: "llm.jobs.queue.reason_slot", params: { limit: state.limits.max_parallel } },
  };
}

export type PendingAsk = {
  id?: string;
  agent: string;
  request_id: string;
  tool_call_id: string;
  tool: string;
  preview: string;
  asked_ms?: number;
  deadline_ms?: number | null;
};

export type ApprovalDecision = "accept" | "accept_session" | "accept_always" | "decline" | "cancel";

export type Approval = {
  id: string;
  state: "pending" | "resolved" | "cancelled" | "stale" | "expired";
  decision: ApprovalDecision | null;
  job_id: string | null;
  run_id: string;
  agent: string;
  tool: string;
  preview: string;
  asked_ms: number;
  deadline_ms: number | null;
  resolved_ms: number | null;
  channel: string | null;
  decided_by: string | null;
  note: string | null;
};

type LoopInput = {
  agent_id: string;
  prompt: string;
  name?: string;
  workspace?: string | null;
  max_rounds?: number | null;
  max_minutes?: number | null;
  check_command?: string | null;
  auto_resume?: boolean | null;
  run_mode?: import("./llm-run-mode.svelte").RunMode | null;
  max_cost_usd?: number | null;
  token_budget?: number | null;
};

type TriggerInput = {
  id?: string;
  name?: string;
  kind: "cron" | "webhook";
  cron?: string | null;
  agent_id: string;
  prompt: string;
  workspace?: string | null;
  enabled: boolean;
  muted?: boolean;
  mission?: unknown;
  deadline_min?: number | null;
  catch_up?: CatchUp;
  concurrency?: TriggerConcurrency;
  tz?: string;
};

export function triggerInput(tr: Trigger, patch: Partial<TriggerInput>): TriggerInput {
  return {
    id: tr.id,
    name: tr.name,
    kind: tr.kind,
    cron: tr.cron,
    agent_id: tr.agent_id,
    prompt: tr.prompt,
    workspace: tr.workspace,
    enabled: tr.enabled,
    muted: tr.muted ?? false,
    mission: tr.mission,
    deadline_min: tr.deadline_min ?? null,
    catch_up: tr.catch_up ?? "once",
    concurrency: tr.concurrency ?? "skip_if_active",
    tz: tr.tz ?? "local",
    ...patch,
  };
}

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

type ResumeKind = "native" | "replay" | "new";

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
  state: "pending" | "approved" | "denied" | "expired" | "cancelled" | "stale";
  answer: string | null;
  version: number;
  created_ms: number;
  resolved_ms: number | null;
};

type RunEvent = {
  id: string;
  run_id: string;
  seq: number;
  kind: string;
  provider_id: string | null;
  payload: Record<string, unknown>;
  ts_ms: number;
};

export type CommandRun = { command: string; ok: boolean | null; output: string; is_test: boolean };

type RuntimeSession = {
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

export type FileChange = {
  path: string;
  status: string;
  additions: number | null;
  deletions: number | null;
  binary?: boolean;
};

export type JobDiffView = {
  job_id: string;
  state: string;
  available: boolean;
  reason: string | null;
  detail: string | null;
  files: FileChange[];
  file_count: number;
  additions: number;
  deletions: number;
  clipped: boolean;
  complete: boolean;
  patch: string;
};

const DIFF_REASONS = new Set(["no_workspace", "no_snapshot", "error"]);

export function diffReasonKey(reason: string | null | undefined): string {
  const code = reason ?? "no_snapshot";
  return `llm.jobs.diff.unavailable_${DIFF_REASONS.has(code) ? code : "error"}`;
}

export async function fetchJobDiff(id: string): Promise<JobDiffView | null> {
  try {
    return await invoke<JobDiffView>("llm_job_diff", { id });
  } catch (e) {
    fail(e);
    return null;
  }
}

export type RunDiff =
  | { available: true; workspace: string; diff: { files: FileChange[]; patch: string; clipped: boolean; complete: boolean } }
  | { available: false; reason: "no_workspace" | "no_changes" | "error"; workspace?: string; error?: string };

export type RunUpdate = {
  run_id: string;
  conversation_id: string;
  bot_id: string;
  state: RunState;
  error?: string;
  resume_kind?: ResumeKind;
};

let runs = $state<RunView[]>([]);
let staleAsks = $state<Approval[]>([]);
let readError = $state<string | null>(null);

export function getJobsReadError(): string | null {
  return readError;
}
let runTick = $state(0);
let runsReload: ReturnType<typeof setTimeout> | null = null;

export function getRuns(): RunView[] {
  return runs;
}
export function getStaleAsks(): Approval[] {
  return staleAsks;
}
export function getRunTick(): number {
  return runTick;
}

async function reloadRuns(): Promise<void> {
  try {
    const [list, stale] = await Promise.all([
      invoke<RunView[]>("assist_runs_list", { limit: 150 }),
      invoke<Approval[]>("llm_approvals_list", { state: "stale" }).catch(() => [] as Approval[]),
    ]);
    runs = list;
    staleAsks = stale;
  } catch (e) {
    addLog("warn", "system", `runs: could not read the run record: ${String(e)}`);
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

export async function respondApproval(id: string, decision: ApprovalDecision): Promise<boolean> {
  try {
    await invoke("llm_approval_respond", { id, decision });
    asks = asks.filter((a) => a.id !== id);
    staleAsks = staleAsks.filter((a) => a.id !== id);
    scheduleRunsReload();
    return true;
  } catch (e) {
    fail(e);
    scheduleRunsReload();
    void pollAsks();
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
let lastJobEvent = $state<Job | null>(null);
let queue = $state<QueueState | null>(null);
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
export function getQueue(): QueueState | null {
  return queue;
}
export function queuedEntry(id: string): QueuedEntry | undefined {
  return queue?.queued.find((q) => q.id === id);
}

async function reloadQueue(): Promise<void> {
  try {
    queue = await invoke<QueueState>("llm_jobs_state");
  } catch {
  }
}

export async function setJobLimits(limits: { max_parallel?: number; max_per_agent?: number }): Promise<void> {
  try {
    queue = await invoke<QueueState>("llm_jobs_limits_set", { limits });
  } catch (e) {
    fail(e);
  }
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

function mergeJobEvent(list: Job[], row: Job): Job[] {
  const current = list.find((j) => j.id === row.id);
  if (current && (row.state_seq ?? 0) < (current.state_seq ?? 0)) return list;
  return upsert(list, row);
}

export async function reloadJobs(): Promise<void> {
  try {
    jobs = await invoke<Job[]>("llm_jobs_list", { limit: 200 });
  } catch (e) {
    fail(e);
  }
}

async function reloadLoops(): Promise<void> {
  try {
    loops = await invoke<LoopDef[]>("llm_loops_list");
  } catch (e) {
    fail(e);
  }
}

async function reloadTriggers(): Promise<void> {
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
    const before = jobs.find((j) => j.id === ev.payload.id)?.state;
    jobs = mergeJobEvent(jobs, ev.payload);
    lastJobEvent = ev.payload;
    const now = jobs.find((j) => j.id === ev.payload.id)?.state;
    if (before !== now && (before === "waiting_approval" || now === "waiting_approval")) void pollAsks();
  }).catch(() => {});
  void listen("llm://tool-ask", () => void pollAsks()).catch(() => {});
  void listen<{ tool_call_id?: string }>("omni://tool-resolved", (ev) => {
    const id = ev.payload?.tool_call_id;
    if (id) asks = asks.filter((a) => a.tool_call_id !== id);
  }).catch(() => {});
  void pollAsks();
  void reloadQueue();
  void listen<QueueState>("llm://queue", (ev) => {
    queue = ev.payload;
  }).catch(() => {});
  void listen<LoopDef>("llm://loop", (ev) => {
    loops = upsert(loops, ev.payload);
  }).catch(() => {});
  void listen<Trigger>("llm://trigger", (ev) => {
    triggers = upsert(triggers, ev.payload);
  }).catch(() => {});
  void reloadRuns();
  void listen<RunUpdate>("assist://run", (ev) => {
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

export async function submitJob(
  agentId: string,
  prompt: string,
  workspace: string | null,
  isolation: Isolation | null = null,
  runMode: import("./llm-run-mode.svelte").RunMode | null = null,
): Promise<Job | null> {
  try {
    const job = await invoke<Job>("llm_job_submit", { agentId, prompt, workspace, isolation, runMode });
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

export async function removeJobWorktree(id: string): Promise<WorktreeRemoval | null> {
  try {
    const out = await invoke<WorktreeRemoval>("llm_job_remove_worktree", { id });
    const job = await invoke<Job | null>("llm_job_get", { id });
    if (job) jobs = upsert(jobs, job);
    return out;
  } catch (e) {
    fail(e);
    return null;
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
    readError = null;
  } catch (e) {
    readError = String(e);
    addLog("error", "system", `jobs: could not read the pending questions: ${String(e)}`);
  }
}

export async function markJobSeen(id: string): Promise<void> {
  try {
    const job = await invoke<Job>("llm_job_seen", { id });
    if (job) jobs = mergeJobEvent(jobs, job);
  } catch {
  }
}

export async function answerAsk(ask: PendingAsk, allow: boolean, always: boolean): Promise<void> {
  if (ask.id) {
    await respondApproval(ask.id, !allow ? "decline" : always ? "accept_always" : "accept");
    return;
  }
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

export function pickState(state: string): string {
  switch (state) {
    case "running":
    case "preparing":
      return "blue";
    case "waiting_approval":
    case "review":
    case "waiting_user":
    case "interrupted":
    case "unknown":
    case "blocked":
    case "budget_limited":
      return "orange";
    case "done":
    case "done_unreported":
    case "completed":
      return "green";
    case "failed":
      return "red";
    default:
      return "grey";
  }
}

export function auditOverhead(audits: LoopAudit[] | undefined): { audit: number; rounds: number; share: number | null } {
  let audit = 0;
  let rounds = 0;
  for (const a of audits ?? []) {
    audit += a.cost_usd ?? 0;
    rounds += a.round_cost_usd ?? 0;
  }
  return { audit, rounds, share: rounds > 0 ? audit / rounds : null };
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
