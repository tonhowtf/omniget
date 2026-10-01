import type { ProjectRow, ThreadRow } from "$lib/central/threads/types";

export type SidebarStatus = "approval" | "input" | "working" | "failed" | "done" | "idle";

const STATUS_RANK: Record<SidebarStatus, number> = {
  approval: 0,
  input: 1,
  working: 2,
  failed: 3,
  done: 4,
  idle: 5,
};

function hasUnseenCompletion(t: ThreadRow): boolean {
  if (t.activeTurnId) return false;
  if (!t.lastVisitedAt || !t.latestTurnId) return false;
  return t.updatedAt > t.lastVisitedAt && t.turnCount > 0;
}

export function sidebarStatus(t: ThreadRow): SidebarStatus {
  if (t.pendingApprovalCount > 0) return "approval";
  if (t.pendingUserInputCount > 0) return "input";
  if (t.activeTurnId || t.sessionStatus === "starting" || t.sessionStatus === "running") {
    if (t.activeTurnId) return "working";
  }
  if (t.sessionStatus === "error" || t.lastError) return "failed";
  if (hasUnseenCompletion(t)) return "done";
  return "idle";
}

export function recedes(t: ThreadRow, status: SidebarStatus, selected: boolean): boolean {
  if (selected) return false;
  if (status === "input" || status === "done" || status === "failed") return false;
  if (status === "approval") return false;
  return status === "idle";
}

function isSnoozed(t: ThreadRow, now = Date.now()): boolean {
  return !!t.snoozedUntil && Date.parse(t.snoozedUntil) > now;
}

export function isImported(t: ThreadRow): boolean {
  return t.projectId === "prj_imported" || t.threadId.startsWith("imp_");
}

export function isExternalHarness(t: ThreadRow): boolean {
  return t.driver !== "native";
}

export type Section = { key: "pinned" | "active" | "snoozed" | "archived"; threads: ThreadRow[] };

type ProjectGroup = { project: ProjectRow | null; projectId: string; sections: Section[]; count: number };

function byPriorityThenRecent(a: ThreadRow, b: ThreadRow): number {
  const ra = STATUS_RANK[sidebarStatus(a)];
  const rb = STATUS_RANK[sidebarStatus(b)];
  const ua = ra <= 3 ? ra : 9;
  const ub = rb <= 3 ? rb : 9;
  if (ua !== ub) return ua - ub;
  return (b.latestUserMessageAt ?? b.updatedAt).localeCompare(a.latestUserMessageAt ?? a.updatedAt);
}

function matchesQuery(t: ThreadRow, project: ProjectRow | undefined, q: string): boolean {
  if (!q) return true;
  const s = q.toLowerCase();
  return (
    t.title.toLowerCase().includes(s) ||
    (t.branch ?? "").toLowerCase().includes(s) ||
    (project?.title ?? "").toLowerCase().includes(s) ||
    t.driver.toLowerCase().includes(s) ||
    (t.model ?? "").toLowerCase().includes(s)
  );
}

export function groupThreads(
  projects: ProjectRow[],
  threads: ThreadRow[],
  opts: { query: string; showArchived: boolean; now?: number },
): ProjectGroup[] {
  const now = opts.now ?? Date.now();
  const byId = new Map(projects.map((p) => [p.projectId, p]));
  const groups = new Map<string, ThreadRow[]>();
  for (const p of projects) groups.set(p.projectId, []);
  for (const t of threads) {
    if (!matchesQuery(t, byId.get(t.projectId), opts.query)) continue;
    if (!groups.has(t.projectId)) groups.set(t.projectId, []);
    groups.get(t.projectId)!.push(t);
  }
  const out: ProjectGroup[] = [];
  for (const [projectId, list] of groups) {
    const pinned: ThreadRow[] = [];
    const active: ThreadRow[] = [];
    const snoozed: ThreadRow[] = [];
    const archived: ThreadRow[] = [];
    for (const t of list) {
      if (t.archivedAt) archived.push(t);
      else if (isSnoozed(t, now) && sidebarStatus(t) !== "approval" && sidebarStatus(t) !== "input") snoozed.push(t);
      else if (t.pinnedAt) pinned.push(t);
      else active.push(t);
    }
    pinned.sort((a, b) => (a.pinnedAt ?? "").localeCompare(b.pinnedAt ?? ""));
    active.sort(byPriorityThenRecent);
    snoozed.sort((a, b) => (a.snoozedUntil ?? "").localeCompare(b.snoozedUntil ?? ""));
    archived.sort((a, b) => (b.archivedAt ?? "").localeCompare(a.archivedAt ?? ""));
    const sections: Section[] = [
      { key: "pinned", threads: pinned },
      { key: "active", threads: active },
      { key: "snoozed", threads: snoozed },
    ];
    if (opts.showArchived) sections.push({ key: "archived", threads: archived });
    const count = pinned.length + active.length + snoozed.length + (opts.showArchived ? archived.length : 0);
    if (opts.query && count === 0) continue;
    out.push({ project: byId.get(projectId) ?? null, projectId, sections, count });
  }
  const rankOf = (g: ProjectGroup) =>
    Math.min(9, ...g.sections.flatMap((s) => s.threads.map((t) => STATUS_RANK[sidebarStatus(t)])));
  const recentOf = (g: ProjectGroup) =>
    g.sections.flatMap((s) => s.threads.map((t) => t.updatedAt)).sort().at(-1) ?? g.project?.updatedAt ?? "";
  out.sort((a, b) => {
    const ra = Math.min(rankOf(a), 4);
    const rb = Math.min(rankOf(b), 4);
    if (ra !== rb) return ra - rb;
    return recentOf(b).localeCompare(recentOf(a));
  });
  return out;
}

export function snoozePresets(now = new Date()): { key: string; until: Date }[] {
  const out: { key: string; until: Date }[] = [];
  out.push({ key: "1h", until: new Date(now.getTime() + 3600_000) });
  out.push({ key: "3h", until: new Date(now.getTime() + 3 * 3600_000) });
  const evening = new Date(now);
  evening.setHours(18, 0, 0, 0);
  if (evening.getTime() - now.getTime() > 3600_000) out.push({ key: "evening", until: evening });
  const tomorrow = new Date(now);
  tomorrow.setDate(tomorrow.getDate() + 1);
  tomorrow.setHours(9, 0, 0, 0);
  out.push({ key: "tomorrow", until: tomorrow });
  const monday = new Date(now);
  const add = ((8 - monday.getDay()) % 7) || 7;
  monday.setDate(monday.getDate() + add);
  monday.setHours(9, 0, 0, 0);
  out.push({ key: "next_week", until: monday });
  return out;
}
