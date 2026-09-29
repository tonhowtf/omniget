/**
 * "Work" as one unit (roadmap 3.9, S24, 82#P8): jobs, loops, missions, the
 * Central's threads and the chat's durable runs folded into one list of
 * items, each with one derived pill and one section.
 *
 * Pure on purpose: the store (`llm-work-store.svelte.ts`) feeds it what the
 * other stores already keep live from their events (`llm://job` with
 * `state_seq`, `llm://tool-ask`, `threads://event`, `assist://mission`,
 * `assist://run`), so the "needs you" badge follows an event as soon as the
 * source store applies it, with no timer of its own.
 *
 * Pill priority (82#P8, t3code `resolveSidebarThreadStatus`): approval >
 * input > decision > working > failed > queued > review > ready/stopped.
 * When two sources describe the same work (a loop and its round job, a
 * mission and its turns, a job and the durable run of the same
 * conversation, a thread and the broker's copy of its ask) the item keeps
 * the stronger pill and the work is counted once.
 */
import type { Job, LoopDef, PendingAsk, PermissionRequest, RunView } from "$lib/stores/llm-jobs-store.svelte";
import type { MissionRow } from "$lib/stores/assist-missions-store.svelte";
import type { ApprovalRow, ThreadRow, UserInputRow } from "$lib/central/threads/types";

export type WorkSource = "job" | "loop" | "mission" | "thread" | "run" | "ask";
export type WorkPill = "approval" | "input" | "decision" | "working" | "failed" | "queued" | "review" | "ready" | "stopped";
export type WorkSection = "needs_you" | "running" | "queued" | "review" | "done";

export const WORK_SECTIONS: WorkSection[] = ["needs_you", "running", "queued", "review", "done"];

export type WorkItem = {
  /** `source:id`, unique in the list. */
  key: string;
  source: WorkSource;
  id: string;
  title: string;
  pill: WorkPill;
  section: WorkSection;
  /** Where the detail lives (every item links to the page that owns it). */
  href: string;
  /** Last activity (epoch ms): the order inside a section. */
  at: number;
  /**
   * What makes this state of the item new: the pending request, the job's
   * `state_seq`, the thread's turn. Part of the notification key, so the
   * same (item, turn, status) is announced once.
   */
  turn: string;
  /** The tool waiting for an answer, a failure reason. */
  detail: string | null;
  /**
   * Times the host started this job again by itself after the app closed or
   * the watchdog stopped it (3.1). Such a job runs with its old
   * `failure_reason` still set: the count says why, the pill says where.
   */
  resumed?: number;
};

export type WorkInput = {
  jobs: Job[];
  loops: LoopDef[];
  asks: PendingAsk[];
  runs: RunView[];
  /**
   * Pending asks of the durable runs. Since 2.2 every broker ask is a row of
   * `runs_permissions` and `llm_tool_asks_pending` returns them all, so the
   * store derives these from the asks (`permissionsOfAsks`).
   */
  permissions: Pick<PermissionRequest, "run_id" | "tool_call_id" | "action" | "state">[];
  missions: MissionRow[];
  threads: ThreadRow[];
  threadApprovals: ApprovalRow[];
  threadInputs: UserInputRow[];
  now: number;
  /** How long a finished item stays under "done recently". */
  doneWindowMs?: number;
  /** At most this many items under "done recently". */
  doneLimit?: number;
};

export const DONE_WINDOW_MS = 3 * 24 * 60 * 60 * 1000;
export const DONE_LIMIT = 20;

const RANK: Record<WorkPill, number> = {
  approval: 0,
  input: 1,
  decision: 2,
  working: 3,
  failed: 4,
  queued: 5,
  review: 6,
  ready: 7,
  stopped: 8,
};

export function pillRank(pill: WorkPill): number {
  return RANK[pill];
}

export function sectionOf(pill: WorkPill): WorkSection {
  switch (pill) {
    case "approval":
    case "input":
    case "decision":
      return "needs_you";
    case "working":
      return "running";
    case "queued":
      return "queued";
    case "review":
      return "review";
    default:
      return "done";
  }
}

/** The pill of one job by itself. */
export function jobPill(state: Job["state"]): WorkPill {
  switch (state) {
    case "waiting_approval":
      return "approval";
    case "interrupted":
      return "decision";
    case "running":
      return "working";
    case "queued":
    case "waiting_workspace":
      return "queued";
    case "review":
      return "review";
    case "failed":
      return "failed";
    case "cancelled":
      return "stopped";
    default:
      return "ready";
  }
}

function isActiveJob(state: Job["state"]): boolean {
  return state === "queued" || state === "waiting_workspace" || state === "running" || state === "waiting_approval";
}

/** First meaningful line, without the mission header a mission turn carries. */
export function titleOf(text: string | null | undefined, max = 90): string {
  const lines = (text ?? "").split("\n").map((l) => l.trim()).filter(Boolean);
  const head = /^\[Mission [A-Za-z0-9-]+ — objective\]$/.test(lines[0] ?? "");
  const line = (head ? lines[1] : lines[0]) ?? "";
  return line.length > max ? `${line.slice(0, max - 1)}…` : line;
}

/** Same rule as `jobOfMission` in the missions store (kept here so this file stays pure). */
function jobInMission(job: Pick<Job, "conversation_id">, mission: Pick<MissionRow, "conversation_id">): boolean {
  const conv = mission.conversation_id;
  if (!conv) return false;
  return job.conversation_id === conv || job.conversation_id.startsWith(`${conv}-`);
}

function iso(ts: string | null | undefined): number {
  if (!ts) return 0;
  const n = Date.parse(ts);
  return Number.isFinite(n) ? n : 0;
}

function make(
  source: WorkSource,
  id: string,
  title: string,
  pill: WorkPill,
  href: string,
  at: number,
  turn: string,
  detail: string | null = null,
): WorkItem {
  return { key: `${source}:${id}`, source, id, title, pill, section: sectionOf(pill), href, at, turn, detail };
}

/** Raises an item to a stronger pill (never lowers it). */
function raise(item: WorkItem, pill: WorkPill, turn: string, detail: string | null, at: number): void {
  if (RANK[pill] < RANK[item.pill]) {
    item.pill = pill;
    item.section = sectionOf(pill);
    item.turn = turn;
    item.detail = detail ?? item.detail;
  }
  if (at > item.at) item.at = at;
}

function jobAt(j: Job): number {
  return j.finished_ms ?? j.started_ms ?? j.created_ms;
}

/**
 * Every source folded into one list, ordered by section, then pill, then
 * the most recent first. Done items older than the window are left out, and
 * at most `doneLimit` of them stay.
 */
export function deriveWork(input: WorkInput): WorkItem[] {
  const window = input.doneWindowMs ?? DONE_WINDOW_MS;
  const doneLimit = input.doneLimit ?? DONE_LIMIT;
  const items = new Map<string, WorkItem>();
  const asksByRequest = new Map<string, PendingAsk>();
  for (const a of input.asks) if (!asksByRequest.has(a.request_id)) asksByRequest.set(a.request_id, a);
  /** Asks some item already accounts for (by tool call id). */
  const claimedAsks = new Set<string>();

  const missions = input.missions.filter((m) => m.state !== "draft");
  const missionOf = (j: Job) => missions.find((m) => jobInMission(j, m));

  // Missions first, so their turns fold into them.
  for (const m of missions) {
    let pill: WorkPill;
    switch (m.state) {
      case "queued":
      case "paused":
        pill = "queued";
        break;
      case "running":
      case "verifying":
        pill = "working";
        break;
      case "blocked":
        pill = "decision";
        break;
      case "partial":
        pill = "review";
        break;
      case "failed":
        pill = "failed";
        break;
      case "cancelled":
        pill = "stopped";
        break;
      default:
        pill = "ready";
    }
    const at = m.finished_ms ?? m.updated_ms ?? m.created_ms;
    items.set(
      `mission:${m.id}`,
      make("mission", m.id, titleOf(m.objective), pill, `/llm/missions?id=${encodeURIComponent(m.id)}`, at, m.block?.code ?? "", m.block?.code ?? null),
    );
  }

  for (const l of input.loops) {
    let pill: WorkPill;
    switch (l.state) {
      case "running":
        pill = "working";
        break;
      case "interrupted":
        pill = "decision";
        break;
      case "failed":
        pill = "failed";
        break;
      case "cancelled":
        pill = "stopped";
        break;
      default:
        pill = "ready";
    }
    items.set(
      `loop:${l.id}`,
      make("loop", l.id, l.name?.trim() || titleOf(l.prompt), pill, `/llm/loops?loop=${encodeURIComponent(l.id)}`, l.finished_ms ?? l.created_ms, String(l.rounds_done), l.stop_reason),
    );
  }

  for (const j of input.jobs) {
    const pill = jobPill(j.state);
    const ask = j.state === "waiting_approval" && j.request_id ? asksByRequest.get(j.request_id) : undefined;
    if (ask) claimedAsks.add(ask.tool_call_id);
    const turn = pill === "approval" ? (ask?.tool_call_id ?? j.request_id ?? String(j.state_seq ?? 0)) : String(j.state_seq ?? 0);
    // A failure reason only while it is the job's state: an auto-resumed
    // job (3.1) runs with its `app_restart`/`stalled` still on the row.
    const ended = j.state === "failed" || j.state === "interrupted";
    const detail = ask?.tool ?? (ended ? (j.failure_reason ?? null) : null);
    const at = jobAt(j);

    // A loop round: the loop is the unit, its round can only raise it.
    const loop = j.loop_id ? items.get(`loop:${j.loop_id}`) : undefined;
    if (loop) {
      if (isActiveJob(j.state)) raise(loop, pill, turn, detail, at);
      continue;
    }
    // A mission turn: the same, the mission is the unit.
    const mission = missionOf(j);
    if (mission) {
      const item = items.get(`mission:${mission.id}`);
      if (item && isActiveJob(j.state)) raise(item, pill, turn, detail, at);
      continue;
    }
    // A chat turn mirrored as a job lives in its conversation: it is work
    // while it runs or waits, and history once it ended.
    if (j.kind === "chat") {
      if (!isActiveJob(j.state)) continue;
      items.set(`job:${j.id}`, make("job", j.id, titleOf(j.prompt), pill, `/llm?conversation=${encodeURIComponent(j.conversation_id)}`, at, turn, detail));
      continue;
    }
    const item = make("job", j.id, titleOf(j.prompt), pill, `/llm/jobs?job=${encodeURIComponent(j.id)}`, at, turn, detail);
    if ((j.attempt ?? 0) > 0) item.resumed = j.attempt;
    items.set(`job:${j.id}`, item);
  }

  // Threads of the Central. Their pending requests are the thread's own.
  const threadRequests = new Set<string>();
  for (const a of input.threadApprovals) if (a.status === "pending") threadRequests.add(a.requestId);
  for (const u of input.threadInputs) if (u.status === "pending") threadRequests.add(u.requestId);
  for (const t of input.threads) {
    if (t.archivedAt) continue;
    const at = iso(t.updatedAt);
    const snoozed = iso(t.snoozedUntil) > input.now;
    const href = `/llm/threads?thread=${encodeURIComponent(t.threadId)}`;
    const title = t.title || t.threadId;
    if (t.status === "approval") {
      const req = input.threadApprovals.find((a) => a.threadId === t.threadId && a.status === "pending");
      items.set(`thread:${t.threadId}`, make("thread", t.threadId, title, "approval", href, at, req?.requestId ?? t.activeTurnId ?? "", req?.requestType ?? null));
    } else if (t.status === "input") {
      const req = input.threadInputs.find((a) => a.threadId === t.threadId && a.status === "pending");
      items.set(`thread:${t.threadId}`, make("thread", t.threadId, title, "input", href, at, req?.requestId ?? t.activeTurnId ?? "", null));
    } else if (t.status === "running") {
      items.set(`thread:${t.threadId}`, make("thread", t.threadId, title, "working", href, at, t.activeTurnId ?? ""));
    } else if (snoozed) {
      continue;
    } else if (t.status === "error") {
      items.set(`thread:${t.threadId}`, make("thread", t.threadId, title, "failed", href, at, t.latestTurnId ?? "", t.lastError));
    } else if (t.turnCount > 0) {
      items.set(`thread:${t.threadId}`, make("thread", t.threadId, title, "ready", href, at, t.latestTurnId ?? ""));
    }
  }

  // The chat's durable runs: only what waits for a person. A run whose
  // conversation already has a live job item raises that item instead.
  const pendingByRun = new Map<string, WorkInput["permissions"]>();
  for (const p of input.permissions) {
    if (p.state !== "pending") continue;
    const list = pendingByRun.get(p.run_id) ?? [];
    list.push(p);
    pendingByRun.set(p.run_id, list);
  }
  for (const r of input.runs) {
    const perms = (pendingByRun.get(r.id) ?? []).filter((p) => !claimedAsks.has(p.tool_call_id) && !threadRequests.has(p.tool_call_id));
    const waiting = perms.length > 0 || (r.state === "waiting_user" && r.pending_permissions > 0);
    const undecided = (r.state === "interrupted" || r.state === "unknown") && !r.resolution && r.actions.length > 0;
    if (!waiting && !undecided) continue;
    const pill: WorkPill = waiting ? "approval" : "decision";
    const first = perms[0];
    const turn = first?.tool_call_id ?? r.id;
    for (const p of perms) claimedAsks.add(p.tool_call_id);
    const liveJob = input.jobs.find((j) => j.conversation_id === r.conversation_id && isActiveJob(j.state));
    const owner = liveJob ? items.get(`job:${liveJob.id}`) : undefined;
    if (owner) {
      raise(owner, pill, turn, first?.action ?? null, r.updated_ms);
      continue;
    }
    const href = waiting ? `/llm?conversation=${encodeURIComponent(r.conversation_id)}` : `/llm/jobs?run=${encodeURIComponent(r.id)}`;
    items.set(`run:${r.id}`, make("run", r.id, titleOf(r.input_preview), pill, href, r.updated_ms, turn, first?.action ?? null));
  }

  // A question nobody above owns (the pet, an old chat turn): still counted,
  // unless it is the broker's copy of a thread's request.
  for (const a of input.asks) {
    if (claimedAsks.has(a.tool_call_id) || threadRequests.has(a.tool_call_id)) continue;
    claimedAsks.add(a.tool_call_id);
    items.set(`ask:${a.tool_call_id}`, make("ask", a.tool_call_id, a.preview ? titleOf(a.preview) : a.tool, "approval", "/llm", input.now, a.tool_call_id, a.tool));
  }

  const all = [...items.values()];
  const live = all.filter((i) => i.section !== "done");
  const done = all
    .filter((i) => i.section === "done" && input.now - i.at <= window)
    .sort((a, b) => b.at - a.at)
    .slice(0, doneLimit);
  const order = (a: WorkItem, b: WorkItem) =>
    WORK_SECTIONS.indexOf(a.section) - WORK_SECTIONS.indexOf(b.section) || RANK[a.pill] - RANK[b.pill] || b.at - a.at;
  return [...live, ...done].sort(order);
}

export type WorkSummary = {
  needsYou: number;
  running: number;
  queued: number;
  review: number;
  /** The strongest pill across the list: what the header shows. */
  top: WorkPill | null;
};

export function summarize(items: WorkItem[]): WorkSummary {
  const count = (s: WorkSection) => items.filter((i) => i.section === s).length;
  let top: WorkPill | null = null;
  for (const i of items) {
    if (i.section === "done") continue;
    if (top === null || RANK[i.pill] < RANK[top]) top = i.pill;
  }
  return { needsYou: count("needs_you"), running: count("running"), queued: count("queued"), review: count("review"), top };
}

export function groupBySection(items: WorkItem[]): { section: WorkSection; items: WorkItem[] }[] {
  return WORK_SECTIONS.map((section) => ({ section, items: items.filter((i) => i.section === section) }));
}

// ── Notifications: once per (item, turn, status) (82#P8) ────────────────

export function notificationKey(item: Pick<WorkItem, "key" | "turn" | "pill">): string {
  return `${item.key}|${item.turn}|${item.pill}`;
}

/**
 * Whether this state of an item is worth telling the person about: every
 * "needs you"; a failure, a finish or a finish waiting for review of
 * background work (a job, a loop, a mission) that happened after `sinceMs`. A chat thread going idle is not
 * news, and what ended before this session is history, not a notification.
 */
export function notifiable(item: WorkItem, sinceMs: number): boolean {
  if (item.section === "needs_you") return true;
  if (item.pill !== "failed" && item.pill !== "ready" && item.pill !== "review") return false;
  if (item.source !== "job" && item.source !== "loop" && item.source !== "mission") return false;
  return item.at >= sinceMs;
}

/**
 * The items to announce now: notifiable and not announced before. `seen`
 * is the persisted set of keys, so reopening the app does not repeat any.
 */
export function freshNotifications(items: WorkItem[], seen: ReadonlySet<string>, sinceMs: number): WorkItem[] {
  return items.filter((i) => notifiable(i, sinceMs) && !seen.has(notificationKey(i)));
}

/** The seen keys with `added`, keeping only the newest `cap`. */
export function rememberSeen(seen: readonly string[], added: readonly string[], cap = 500): string[] {
  const next = seen.filter((k) => !added.includes(k)).concat(added);
  return next.length > cap ? next.slice(next.length - cap) : next;
}

/** The color family of a pill, the same names the job and mission lists use. */
export function pillTone(pill: WorkPill): "blue" | "orange" | "green" | "red" | "grey" {
  switch (pill) {
    case "approval":
    case "input":
    case "decision":
    case "review":
      return "orange";
    case "working":
      return "blue";
    case "failed":
      return "red";
    case "ready":
      return "green";
    default:
      return "grey";
  }
}

/**
 * The asks as the durable runs' pending rows (2.2): the broker writes each ask
 * with `run_id` = the turn's request id and the same `tool_call_id`, which is
 * also a thread's `requestId`, so the dedupe above holds.
 */
export function permissionsOfAsks(asks: PendingAsk[]): WorkInput["permissions"] {
  return asks.map((a) => ({ run_id: a.request_id, tool_call_id: a.tool_call_id, action: a.tool, state: "pending" as const }));
}
