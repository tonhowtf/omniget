// Thread detail → flat list of timeline rows (T3 `MessagesTimeline.logic.ts`):
// turn folds ("Worked for 2m 14s"), tool groups with one-line summaries, a
// live row with a stable key, answered questions, plan cards and a per-turn
// footer with changed files, tokens and cost. Pure: UI state (what is
// expanded) comes in as sets, so the virtual list can key rows by id.

import type {
  ActivityRow,
  MessageRow,
  PlanRow,
  ThreadDetail,
  TurnDetail,
  UsageRow,
  UserInputRow,
} from "$lib/central/threads/types";
import {
  approvalEntry,
  diffStats,
  isQuietActivity,
  splitUnifiedDiff,
  summarize,
  toWorkEntry,
  type DiffFileStat,
  type SummaryPart,
  type WorkEntry,
} from "./work";

export type ChangedFile = { path: string; status: string; additions: number; deletions: number };

export type Row =
  | { kind: "loose-head"; id: string }
  | { kind: "user"; id: string; turn: TurnDetail | null; message: MessageRow; revertTo: number | null }
  | { kind: "assistant"; id: string; message: MessageRow; terminal: boolean; turnId: string | null }
  | { kind: "reasoning"; id: string; message: MessageRow; preview: string }
  | { kind: "work"; id: string; entry: WorkEntry; depth: number }
  | { kind: "group"; id: string; entries: WorkEntry[]; summary: SummaryPart[]; open: boolean; failed: boolean; depth: number }
  | { kind: "live"; id: string; turnId: string; current: WorkEntry | null; entries: WorkEntry[]; open: boolean }
  | {
      kind: "fold";
      id: string;
      turnId: string;
      durationMs: number;
      interrupted: boolean;
      failed: boolean;
      open: boolean;
      summary: SummaryPart[];
    }
  | { kind: "working"; id: string; turnId: string; since: string; phase: "thinking" | "working" | "pending" }
  | { kind: "agents"; id: string; entries: WorkEntry[]; open: boolean; live: boolean }
  | { kind: "question"; id: string; input: UserInputRow }
  | { kind: "plan"; id: string; plan: PlanRow; turnId: string | null; actionable: boolean }
  | { kind: "compaction"; id: string }
  | { kind: "error"; id: string; entry: WorkEntry }
  | {
      kind: "meta";
      id: string;
      turn: TurnDetail;
      usage: UsageRow | null;
      files: ChangedFile[];
      durationMs: number | null;
      answer: string;
      /** Turn error not already shown by an error row. */
      error: string | null;
    };

export type DeriveOptions = {
  expandedFolds: Set<string>;
  collapsedFolds: Set<string>;
  expandedGroups: Set<string>;
  /** Turn ids interrupted in this session: they auto-expand. */
  interruptedHere: Set<string>;
  /** Latest turn in the thread (for plan actionability). */
  latestTurnId: string | null;
  planMode: boolean;
};

type Item =
  | { t: "msg"; at: string; order: number; m: MessageRow }
  | { t: "work"; at: string; order: number; e: WorkEntry }
  | { t: "q"; at: string; order: number; u: UserInputRow };

const LIVE_STATES = new Set(["pending", "running"]);

export function isLiveTurn(t: TurnDetail): boolean {
  return LIVE_STATES.has(t.turn.state);
}

export function turnDurationMs(t: TurnDetail, now = Date.now()): number {
  const start = Date.parse(t.turn.startedAt ?? t.turn.requestedAt);
  const endIso = t.turn.completedAt;
  const end = endIso ? Date.parse(endIso) : now;
  if (Number.isNaN(start) || Number.isNaN(end)) return t.usage?.durationMs ?? 0;
  return Math.max(0, end - start);
}

/** One-line plaintext preview of a thought. */
export function thoughtPreview(text: string): string {
  const plain = text
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/[`*_#>~[\]()]/g, "")
    .replace(/\s+/g, " ")
    .trim();
  return plain.length > 140 ? plain.slice(0, 139) + "…" : plain;
}

/** Output messages (`<turn>:<item>`) belong to the activity `<turn>:item:<item>`. */
function outputKey(messageId: string): string | null {
  const i = messageId.lastIndexOf(":");
  if (i < 0) return null;
  return `${messageId.slice(0, i)}:item:${messageId.slice(i + 1)}`;
}

/** Files changed in a turn: driver diff first, then edit entries. */
export function changedFilesOf(turn: TurnDetail): { files: ChangedFile[]; patches: DiffFileStat[] } {
  const diffAct = turn.activities.find((a) => a.kind === "turn.diff.updated");
  const unified = diffAct && typeof diffAct.payload.unifiedDiff === "string" ? (diffAct.payload.unifiedDiff as string) : null;
  if (unified) {
    const patches = splitUnifiedDiff(unified);
    if (patches.length) return { files: patches.map(({ patch: _p, ...f }) => f), patches };
  }
  const map = new Map<string, ChangedFile>();
  for (const a of turn.activities) {
    if (isQuietActivity(a)) continue;
    const e = toWorkEntry(a);
    if (e.kind !== "edit" || e.status === "failed" || e.status === "declined") continue;
    const st = diffStats(e.diff);
    for (const p of e.paths.length ? e.paths : []) {
      const cur = map.get(p) ?? { path: p, status: "modified", additions: 0, deletions: 0 };
      cur.additions += st.add;
      cur.deletions += st.del;
      if (e.diff?.startsWith("--- /dev/null")) cur.status = "added";
      map.set(p, cur);
    }
  }
  return { files: [...map.values()], patches: [] };
}

function itemsOf(turn: { messages: MessageRow[]; activities: ActivityRow[]; userInputs?: UserInputRow[]; approvals?: TurnDetail["approvals"] }): Item[] {
  const items: Item[] = [];
  const outputs = new Map<string, string>();
  for (const m of turn.messages) {
    if (m.role === "output") {
      const k = outputKey(m.messageId);
      if (k) outputs.set(k, (outputs.get(k) ?? "") + m.text);
    }
  }
  let order = 0;
  for (const m of turn.messages) {
    if (m.role === "output") continue;
    if (m.role === "plan") continue; // shown as the plan card
    items.push({ t: "msg", at: m.createdAt, order: order++, m });
  }
  for (const a of turn.activities) {
    if (isQuietActivity(a)) continue;
    const e = toWorkEntry(a, outputs.get(a.activityId) ?? null);
    items.push({ t: "work", at: a.createdAt, order: a.sequence ?? order++, e });
  }
  for (const ap of turn.approvals ?? []) {
    if (ap.status === "pending") continue;
    items.push({ t: "work", at: ap.createdAt, order: order++, e: approvalEntry(ap) });
  }
  for (const u of turn.userInputs ?? []) {
    if (u.status === "pending") continue;
    items.push({ t: "q", at: u.createdAt, order: order++, u });
  }
  items.sort((a, b) => {
    if (a.t === "msg" && a.m.role === "user" && !(b.t === "msg" && b.m.role === "user")) return -1;
    if (b.t === "msg" && b.m.role === "user" && !(a.t === "msg" && a.m.role === "user")) return 1;
    const c = a.at.localeCompare(b.at);
    return c !== 0 ? c : a.order - b.order;
  });
  return items;
}

/** Groups a run of work entries: 1 entry → plain row, ≥2 → summary toggle. */
function pushWorkRun(out: Row[], run: WorkEntry[], opts: DeriveOptions, depth: number) {
  if (!run.length) return;
  const agents = run.filter((e) => e.agent);
  const tools = run.filter((e) => !e.agent);
  if (tools.length === 1) out.push({ kind: "work", id: tools[0].id, entry: tools[0], depth });
  else if (tools.length > 1) {
    const id = `group:${tools[0].id}`;
    const open = opts.expandedGroups.has(id);
    out.push({
      kind: "group",
      id,
      entries: tools,
      summary: summarize(tools),
      open,
      failed: tools.at(-1)?.status === "failed",
      depth,
    });
    if (open) for (const e of tools) out.push({ kind: "work", id: `${id}/${e.id}`, entry: e, depth: depth + 1 });
  }
  if (agents.length) {
    const id = `agents:${agents[0].id}`;
    out.push({ kind: "agents", id, entries: agents, open: opts.expandedGroups.has(id), live: agents.some((a) => a.status === "running") });
  }
}

/** Items → rows, grouping consecutive work; messages and errors break runs. */
function rowsOf(items: Item[], opts: DeriveOptions, turn: TurnDetail | null, depth: number, terminalId: string | null): Row[] {
  const out: Row[] = [];
  let run: WorkEntry[] = [];
  const flush = () => {
    pushWorkRun(out, run, opts, depth);
    run = [];
  };
  for (const it of items) {
    if (it.t === "work") {
      if (it.e.kind === "error" || it.e.kind === "warning") {
        flush();
        out.push({ kind: "error", id: it.e.id, entry: it.e });
      } else if (it.e.kind === "compaction") {
        flush();
        out.push({ kind: "compaction", id: it.e.id });
      } else run.push(it.e);
      continue;
    }
    flush();
    if (it.t === "q") {
      out.push({ kind: "question", id: `q:${it.u.requestId}`, input: it.u });
      continue;
    }
    const m = it.m;
    if (m.role === "user") {
      out.push({ kind: "user", id: m.messageId, turn, message: m, revertTo: turn ? turn.turn.ordinal - 1 : null });
    } else if (m.role === "reasoning") {
      if (!m.text.trim() && !m.streaming) continue;
      out.push({ kind: "reasoning", id: m.messageId, message: m, preview: thoughtPreview(m.text) });
    } else {
      out.push({ kind: "assistant", id: m.messageId, message: m, terminal: m.messageId === terminalId, turnId: turn?.turn.turnId ?? null });
    }
  }
  flush();
  return out;
}

function planRows(turn: TurnDetail, opts: DeriveOptions): Row[] {
  const rows: Row[] = [];
  for (const p of turn.plans) {
    const md = p.markdown ?? turn.messages.filter((m) => m.role === "plan").map((m) => m.text).join("");
    if (!md.trim()) continue;
    rows.push({
      kind: "plan",
      id: `plan:${p.planId}`,
      plan: { ...p, markdown: md },
      turnId: turn.turn.turnId,
      actionable: opts.planMode && turn.turn.turnId === opts.latestTurnId && !isLiveTurn(turn),
    });
  }
  if (!turn.plans.some((p) => p.markdown)) {
    const streamed = turn.messages.filter((m) => m.role === "plan").map((m) => m.text).join("");
    if (streamed.trim() && !rows.length) {
      rows.push({
        kind: "plan",
        id: `plan:${turn.turn.turnId}:stream`,
        plan: { planId: `${turn.turn.turnId}:stream`, threadId: turn.turn.threadId, turnId: turn.turn.turnId, explanation: null, steps: null, markdown: streamed, updatedAt: turn.turn.requestedAt },
        turnId: turn.turn.turnId,
        actionable: opts.planMode && turn.turn.turnId === opts.latestTurnId && !isLiveTurn(turn),
      });
    }
  }
  return rows;
}

function deriveTurn(turn: TurnDetail, opts: DeriveOptions): Row[] {
  const items = itemsOf(turn);
  const id = turn.turn.turnId;
  const live = isLiveTurn(turn);
  const userItems = items.filter((i) => i.t === "msg" && i.m.role === "user");
  const rest = items.filter((i) => !(i.t === "msg" && i.m.role === "user"));
  const out: Row[] = [];
  for (const u of userItems) if (u.t === "msg") out.push({ kind: "user", id: u.m.messageId, turn, message: u.m, revertTo: turn.turn.ordinal - 1 });

  if (live) {
    // Trailing work collapses into one live row with a stable key.
    let cut = rest.length;
    while (cut > 0) {
      const it = rest[cut - 1];
      if (it.t === "work" && !it.e.agent && it.e.kind !== "error" && it.e.kind !== "warning" && it.e.kind !== "compaction") cut--;
      else break;
    }
    const lastAssistant = [...rest].reverse().find((i) => i.t === "msg" && i.m.role === "assistant");
    out.push(...rowsOf(rest.slice(0, cut), opts, turn, 0, lastAssistant && lastAssistant.t === "msg" ? lastAssistant.m.messageId : null));
    const trailing = rest.slice(cut).map((i) => (i.t === "work" ? i.e : null)).filter((e): e is WorkEntry => !!e);
    const liveId = `live:${id}`;
    if (trailing.length) {
      const current = [...trailing].reverse().find((e) => e.status === "running") ?? trailing.at(-1) ?? null;
      const open = opts.expandedGroups.has(liveId);
      out.push({ kind: "live", id: liveId, turnId: id, current, entries: trailing, open });
      if (open) for (const e of trailing) out.push({ kind: "work", id: `${liveId}/${e.id}`, entry: e, depth: 1 });
    }
    const streaming = rest.some((i) => i.t === "msg" && i.m.streaming && i.m.role === "assistant");
    out.push(...planRows(turn, opts));
    out.push({
      kind: "working",
      id: `working:${id}`,
      turnId: id,
      since: turn.turn.startedAt ?? turn.turn.requestedAt,
      phase: turn.turn.state === "pending" ? "pending" : trailing.some((e) => e.status === "running") || streaming ? "working" : "thinking",
    });
    return out;
  }

  // Settled turn: fold everything before the terminal answer.
  let terminalIdx = -1;
  for (let i = rest.length - 1; i >= 0; i--) {
    const it = rest[i];
    if (it.t === "msg" && it.m.role === "assistant" && it.m.text.trim()) {
      terminalIdx = i;
      break;
    }
  }
  const interrupted = turn.turn.state === "interrupted" || turn.turn.state === "cancelled";
  const failed = turn.turn.state === "failed";
  let before = terminalIdx >= 0 ? rest.slice(0, terminalIdx) : rest;
  let after = terminalIdx >= 0 ? rest.slice(terminalIdx + 1) : [];
  // A single ordinary trailing tool joins the fold.
  if (after.length === 1 && after[0].t === "work" && !after[0].e.agent && after[0].e.kind !== "error" && after[0].e.kind !== "warning") {
    before = [...before, after[0]];
    after = [];
  }
  // Questions, subagent runs and errors never fold.
  const foldable = before.filter((i) => !(i.t === "q" || (i.t === "work" && (i.e.agent || i.e.kind === "error" || i.e.kind === "warning"))));
  const pinned = before.filter((i) => !foldable.includes(i));
  // Reasoning after the answer folds too.
  const afterReasoning = after.filter((i) => i.t === "msg" && i.m.role === "reasoning");
  after = after.filter((i) => !afterReasoning.includes(i));
  const hidden = [...foldable, ...afterReasoning];
  const worthFolding = hidden.some((i) => i.t === "work") || hidden.filter((i) => i.t === "msg").length > 1 || (hidden.length > 0 && terminalIdx >= 0);
  const terminalId = terminalIdx >= 0 && rest[terminalIdx].t === "msg" ? (rest[terminalIdx] as { m: MessageRow }).m.messageId : null;

  if (hidden.length && worthFolding) {
    const foldId = `fold:${id}`;
    const autoOpen = (interrupted && opts.interruptedHere.has(id)) || terminalIdx < 0;
    const open = opts.collapsedFolds.has(foldId) ? false : autoOpen || opts.expandedFolds.has(foldId);
    out.push({
      kind: "fold",
      id: foldId,
      turnId: id,
      durationMs: turnDurationMs(turn),
      interrupted,
      failed,
      open,
      summary: summarize(hidden.filter((i): i is Extract<Item, { t: "work" }> => i.t === "work").map((i) => i.e)),
    });
    if (open) out.push(...rowsOf(hidden, opts, turn, 1, null));
  } else {
    out.push(...rowsOf(hidden, opts, turn, 0, null));
  }
  out.push(...rowsOf(pinned, opts, turn, 0, null));
  if (terminalIdx >= 0) out.push(...rowsOf([rest[terminalIdx]], opts, turn, 0, terminalId));
  out.push(...rowsOf(after, opts, turn, 0, null));
  out.push(...planRows(turn, opts));

  const { files } = changedFilesOf(turn);
  const answer = terminalIdx >= 0 && rest[terminalIdx].t === "msg" ? (rest[terminalIdx] as { m: MessageRow }).m.text : "";
  const shownError = rest.some((i) => i.t === "work" && i.e.kind === "error");
  const error = turn.turn.state === "failed" && !shownError ? turn.turn.errorMessage : null;
  out.push({ kind: "meta", id: `meta:${id}`, turn, usage: turn.usage, files, durationMs: turnDurationMs(turn), answer, error });
  return out;
}

/** Settled turns are cached by a cheap signature, so streaming only re-derives the live turn. */
const turnCache = new Map<string, { sig: string; rows: Row[] }>();

function turnSig(turn: TurnDetail, optsKey: string): string {
  let text = 0;
  for (const m of turn.messages) text += m.text.length;
  let act = 0;
  for (const a of turn.activities) act += a.sequence ?? 0;
  return [
    turn.turn.state,
    turn.turn.completedAt ?? "",
    turn.messages.length,
    text,
    turn.activities.length,
    act,
    turn.approvals.map((a) => a.status).join(""),
    turn.userInputs.map((u) => u.status).join(""),
    turn.plans.map((p) => p.updatedAt).join(""),
    turn.usage?.outputTokens ?? "",
    turn.usage?.costUsd ?? "",
    optsKey,
  ].join("|");
}

export function deriveRows(detail: ThreadDetail | undefined, opts: DeriveOptions): Row[] {
  if (!detail) return [];
  const rows: Row[] = [];
  if (detail.looseMessages.length || detail.looseActivities.length) {
    rows.push(...rowsOf(itemsOf({ messages: detail.looseMessages, activities: detail.looseActivities }), opts, null, 0, null));
  }
  const optsKey = [
    [...opts.expandedFolds].join(","),
    [...opts.collapsedFolds].join(","),
    [...opts.expandedGroups].join(","),
    [...opts.interruptedHere].join(","),
    opts.latestTurnId ?? "",
    opts.planMode ? 1 : 0,
  ].join(";");
  for (const turn of detail.turns) {
    if (isLiveTurn(turn)) {
      rows.push(...deriveTurn(turn, opts));
      continue;
    }
    const key = `${detail.threadId}:${turn.turn.turnId}`;
    const sig = turnSig(turn, optsKey);
    const hit = turnCache.get(key);
    if (hit && hit.sig === sig) {
      rows.push(...hit.rows);
      continue;
    }
    const derived = deriveTurn(turn, opts);
    turnCache.set(key, { sig, rows: derived });
    if (turnCache.size > 4000) turnCache.delete(turnCache.keys().next().value as string);
    rows.push(...derived);
  }
  return rows;
}

/** Latest context usage we know of: the most recent turn with usage. */
export function latestUsage(detail: ThreadDetail | undefined): UsageRow | null {
  if (!detail) return null;
  for (let i = detail.turns.length - 1; i >= 0; i--) if (detail.turns[i].usage) return detail.turns[i].usage;
  return null;
}

/** Latest plan steps (Codex plan / Claude TodoWrite) across turns. */
export function latestPlan(detail: ThreadDetail | undefined): { steps: PlanRow | null; proposed: PlanRow | null } {
  let steps: PlanRow | null = null;
  let proposed: PlanRow | null = null;
  for (const t of detail?.turns ?? []) {
    for (const p of t.plans) {
      if (p.steps && p.steps.length) steps = p;
      else if (p.steps && p.steps.length === 0) steps = null;
      if (p.markdown) proposed = p;
    }
  }
  return { steps, proposed };
}

/** Every subagent / task entry in the thread. */
export function tasksOf(detail: ThreadDetail | undefined): WorkEntry[] {
  const map = new Map<string, WorkEntry>();
  for (const t of detail?.turns ?? []) {
    for (const a of t.activities) {
      if (!a.kind.startsWith("task.") && a.payload.itemType !== "collab_agent_tool_call") continue;
      const e = toWorkEntry(a);
      map.set(e.agent?.taskId ?? e.id, e);
    }
  }
  return [...map.values()];
}

/** Rate-limit windows reported by the driver (latest). */
export function rateLimitsOf(detail: ThreadDetail | undefined): { id: string; label: string; usedPercent?: number; resetsAt?: string }[] {
  for (const t of [...(detail?.turns ?? [])].reverse()) {
    const a = [...t.activities].reverse().find((x) => x.kind === "account.rate-limits.updated");
    if (a && Array.isArray(a.payload.windows)) return a.payload.windows as { id: string; label: string; usedPercent?: number; resetsAt?: string }[];
  }
  const loose = [...(detail?.looseActivities ?? [])].reverse().find((x) => x.kind === "account.rate-limits.updated");
  if (loose && Array.isArray(loose.payload.windows)) return loose.payload.windows as { id: string; label: string; usedPercent?: number; resetsAt?: string }[];
  return [];
}

/** User prompts of the thread, newest last (for ↑ history). */
export function userPrompts(detail: ThreadDetail | undefined): string[] {
  const out: string[] = [];
  for (const t of detail?.turns ?? []) for (const m of t.messages) if (m.role === "user" && m.text.trim()) out.push(m.text);
  const dedup: string[] = [];
  for (const p of out) if (dedup.at(-1) !== p) dedup.push(p);
  return dedup;
}
