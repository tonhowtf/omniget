import type { Job, LoopDef, PendingAsk, PermissionRequest, RunView } from "$lib/stores/llm-jobs-store.svelte";
import type { MissionRow } from "$lib/stores/assist-missions-store.svelte";
import type { ApprovalRow, ThreadRow, UserInputRow } from "$lib/central/threads/types";
import type { BudgetIncident } from "$lib/llm/budget";

type WorkSource = "job" | "loop" | "mission" | "thread" | "run" | "ask" | "budget";
type WorkPill = "approval" | "input" | "decision" | "working" | "failed" | "queued" | "review" | "ready" | "stopped";
type WorkSection = "needs_you" | "running" | "queued" | "review" | "done";

const WORK_SECTIONS: WorkSection[] = ["needs_you", "running", "queued", "review", "done"];

export type WorkItem = {
  key: string;
  source: WorkSource;
  id: string;
  title: string;
  pill: WorkPill;
  section: WorkSection;
  href: string;
  at: number;
  turn: string;
  detail: string | null;
  resumed?: number;
};

type WorkInput = {
  jobs: Job[];
  loops: LoopDef[];
  asks: PendingAsk[];
  runs: RunView[];
  permissions: Pick<PermissionRequest, "run_id" | "tool_call_id" | "action" | "state">[];
  missions: MissionRow[];
  threads: ThreadRow[];
  threadApprovals: ApprovalRow[];
  threadInputs: UserInputRow[];
  incidents?: Pick<BudgetIncident, "id" | "scope" | "state" | "metric" | "opened_ms">[];
  now: number;
  doneWindowMs?: number;
  doneLimit?: number;
};

const DONE_WINDOW_MS = 3 * 24 * 60 * 60 * 1000;
const DONE_LIMIT = 20;

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

function sectionOf(pill: WorkPill): WorkSection {
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

function jobPill(state: Job["state"]): WorkPill {
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

export function titleOf(text: string | null | undefined, max = 90): string {
  const lines = (text ?? "").split("\n").map((l) => l.trim()).filter(Boolean);
  const head = /^\[Mission [A-Za-z0-9-]+ — objective\]$/.test(lines[0] ?? "");
  const line = (head ? lines[1] : lines[0]) ?? "";
  return line.length > max ? `${line.slice(0, max - 1)}…` : line;
}

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

export function deriveWork(input: WorkInput): WorkItem[] {
  const window = input.doneWindowMs ?? DONE_WINDOW_MS;
  const doneLimit = input.doneLimit ?? DONE_LIMIT;
  const items = new Map<string, WorkItem>();
  const asksByRequest = new Map<string, PendingAsk>();
  for (const a of input.asks) if (!asksByRequest.has(a.request_id)) asksByRequest.set(a.request_id, a);
  const claimedAsks = new Set<string>();

  const missions = input.missions.filter((m) => m.state !== "draft");
  const missionOf = (j: Job) => missions.find((m) => jobInMission(j, m));

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
      case "blocked":
      case "budget_limited":
        pill = "decision";
        break;
      case "paused":
        pill = "queued";
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
    const ended = j.state === "failed" || j.state === "interrupted";
    const detail = ask?.tool ?? (ended ? (j.failure_reason ?? null) : null);
    const at = jobAt(j);

    const loop = j.loop_id ? items.get(`loop:${j.loop_id}`) : undefined;
    if (loop) {
      if (isActiveJob(j.state)) raise(loop, pill, turn, detail, at);
      continue;
    }
    const mission = missionOf(j);
    if (mission) {
      const item = items.get(`mission:${mission.id}`);
      if (item && isActiveJob(j.state)) raise(item, pill, turn, detail, at);
      continue;
    }
    if (j.kind === "chat") {
      if (!isActiveJob(j.state)) continue;
      items.set(`job:${j.id}`, make("job", j.id, titleOf(j.prompt), pill, `/llm?conversation=${encodeURIComponent(j.conversation_id)}`, at, turn, detail));
      continue;
    }
    const company = j.kind === "company" && pill !== "approval" ? /^company:([^:]+):/.exec(j.conversation_id)?.[1] : undefined;
    const href = company ? `/llm/company/${encodeURIComponent(company)}/activity` : `/llm/jobs?job=${encodeURIComponent(j.id)}`;
    const item = make("job", j.id, titleOf(j.prompt), pill, href, at, turn, detail);
    if ((j.attempt ?? 0) > 0) item.resumed = j.attempt;
    items.set(`job:${j.id}`, item);
  }

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

  for (const a of input.asks) {
    if (claimedAsks.has(a.tool_call_id) || threadRequests.has(a.tool_call_id)) continue;
    claimedAsks.add(a.tool_call_id);
    items.set(`ask:${a.tool_call_id}`, make("ask", a.tool_call_id, a.preview ? titleOf(a.preview) : a.tool, "approval", "/llm", input.now, a.tool_call_id, a.tool));
  }

  for (const inc of input.incidents ?? []) {
    if (inc.state !== "open") continue;
    items.set(
      `budget:${inc.id}`,
      make("budget", inc.id, inc.scope, "decision", `/llm/usage?incident=${encodeURIComponent(inc.id)}#budgets`, inc.opened_ms, inc.id, inc.metric),
    );
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

export function notificationKey(item: Pick<WorkItem, "key" | "turn" | "pill">): string {
  return `${item.key}|${item.turn}|${item.pill}`;
}

function notifiable(item: WorkItem, sinceMs: number): boolean {
  if (item.section === "needs_you") return true;
  if (item.pill !== "failed" && item.pill !== "ready" && item.pill !== "review") return false;
  if (item.source !== "job" && item.source !== "loop" && item.source !== "mission") return false;
  return item.at >= sinceMs;
}

export function freshNotifications(items: WorkItem[], seen: ReadonlySet<string>, sinceMs: number): WorkItem[] {
  return items.filter((i) => notifiable(i, sinceMs) && !seen.has(notificationKey(i)));
}

export function rememberSeen(seen: readonly string[], added: readonly string[], cap = 500): string[] {
  const next = seen.filter((k) => !added.includes(k)).concat(added);
  return next.length > cap ? next.slice(next.length - cap) : next;
}

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

export function permissionsOfAsks(asks: PendingAsk[]): WorkInput["permissions"] {
  return asks.map((a) => ({ run_id: a.request_id, tool_call_id: a.tool_call_id, action: a.tool, state: "pending" as const }));
}
