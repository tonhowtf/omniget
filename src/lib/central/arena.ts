// Arena da Central (T9): tipos e wrappers dos comandos `arena_*`, o evento
// ao vivo `central://arena` e helpers puros de exibição. O estado ao vivo de
// cada coluna (rodando / aprovação / pergunta) vem do `threadsStore`; o que a
// arena mede ao fim (tempo, tokens, custo, testes, diff) vem daqui.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { DiffOptions, DiffResult } from "$lib/central/vcs";
import type { AccessMode } from "$lib/central/threads/types";

export const ARENA_EVENT = "central://arena";

export type EntryStatus = "preparing" | "running" | "testing" | "done" | "failed" | "interrupted";
export type TestStatus = "none" | "running" | "passed" | "failed" | "error";
export type ArenaStatus = "running" | "review" | "applied" | "discarded";
export type ApplyStrategy = "merge" | "squash";

export type EntrySpec = {
  instanceId: string;
  driver: string;
  model?: string | null;
  agentId?: string | null;
  label?: string | null;
};

export type ArenaCreate = {
  projectId?: string | null;
  workspaceRoot?: string | null;
  title?: string | null;
  prompt: string;
  baseBranch?: string | null;
  verifyCommand?: string | null;
  verifyTimeoutSecs?: number | null;
  runtimeMode?: AccessMode | null;
  entries: EntrySpec[];
  sourceThreadId?: string | null;
  sourcePlanId?: string | null;
};

export type ArenaEntry = {
  entryId: string;
  arenaId: string;
  position: number;
  threadId: string;
  instanceId: string;
  driver: string;
  model: string | null;
  label: string;
  status: EntryStatus;
  startedAt: string | null;
  turnStartedAt: string | null;
  finishedAt: string | null;
  durationMs: number | null;
  turns: number;
  inputTokens: number;
  outputTokens: number;
  cachedTokens: number;
  reasoningTokens: number;
  costUsd: number | null;
  branch: string | null;
  worktreePath: string | null;
  commitSha: string | null;
  filesChanged: number;
  additions: number;
  deletions: number;
  testStatus: TestStatus;
  testExit: number | null;
  testMs: number | null;
  testTail: string | null;
  votes: number;
  winner: boolean;
  error: string | null;
  cleanedAt: string | null;
};

export type ArenaView = {
  arenaId: string;
  title: string;
  prompt: string;
  projectId: string;
  repoRoot: string;
  baseBranch: string | null;
  baseCommit: string;
  verifyCommand: string | null;
  verifyTimeoutSecs: number;
  runtimeMode: AccessMode;
  sourceThreadId: string | null;
  sourcePlanId: string | null;
  status: ArenaStatus;
  winnerEntryId: string | null;
  appliedStrategy: ApplyStrategy | null;
  appliedCommit: string | null;
  appliedBranch: string | null;
  appliedAt: string | null;
  createdAt: string;
  updatedAt: string;
  entries: ArenaEntry[];
};

export type ScoreRow = {
  driver: string;
  model: string | null;
  runs: number;
  wins: number;
  votes: number;
  testsPassed: number;
  testsRun: number;
  failures: number;
  avgDurationMs: number | null;
  avgCostUsd: number | null;
  avgTokens: number | null;
  avgLines: number | null;
  lastAt: string | null;
};

export type ApplyPreview = {
  arenaId: string;
  entryId: string;
  targetBranch: string;
  targetHead: string;
  winnerCommit: string;
  checkedOutAt: string | null;
  checkoutDirty: boolean;
  alreadyApplied: boolean;
  conflicts: string[];
  filesChanged: number;
  additions: number;
  deletions: number;
};

export type ApplyResult = {
  arena: ArenaView;
  commit: string;
  targetBranch: string;
  strategy: ApplyStrategy;
  updated: "ref" | "checkout";
};

export type CleanupResult = {
  arena: ArenaView;
  archived: string[];
  branchesDeleted: string[];
  errors: string[];
};

export type ArenaEventPayload = { arenaId: string; arena?: ArenaView; deleted?: boolean };

// ── Comandos ────────────────────────────────────────────────────────────

export const arenaCreate = (request: ArenaCreate) => invoke<ArenaView>("arena_create", { request });
export const arenaList = (limit = 100, projectId?: string | null) =>
  invoke<ArenaView[]>("arena_list", { limit, projectId: projectId ?? null });
export const arenaGet = (arenaId: string) => invoke<ArenaView>("arena_get", { arenaId });
export const arenaDiff = (arenaId: string, entryId: string, options?: DiffOptions) =>
  invoke<DiffResult>("arena_diff", { arenaId, entryId, options: options ?? null });
export const arenaVote = (arenaId: string, entryId: string, delta: 1 | -1 = 1) =>
  invoke<ArenaView>("arena_vote", { arenaId, entryId, delta });
export const arenaPick = (arenaId: string, entryId: string | null) =>
  invoke<ArenaView>("arena_pick", { arenaId, entryId });
export const arenaVerify = (arenaId: string, entryId?: string | null, command?: string | null) =>
  invoke<ArenaView>("arena_verify", { arenaId, entryId: entryId ?? null, command: command ?? null });
export const arenaStop = (arenaId: string) => invoke<ArenaView>("arena_stop", { arenaId });
export const arenaApplyPreview = (arenaId: string, entryId: string, targetBranch?: string | null) =>
  invoke<ApplyPreview>("arena_apply_preview", { arenaId, entryId, targetBranch: targetBranch ?? null });
export const arenaApply = (input: {
  arenaId: string;
  entryId: string;
  strategy: ApplyStrategy;
  targetBranch?: string | null;
  touchCheckout?: boolean;
  message?: string | null;
}) =>
  invoke<ApplyResult>("arena_apply", {
    arenaId: input.arenaId,
    entryId: input.entryId,
    strategy: input.strategy,
    targetBranch: input.targetBranch ?? null,
    confirm: true,
    touchCheckout: input.touchCheckout ?? false,
    message: input.message ?? null,
  });
export const arenaCleanup = (arenaId: string, scope: "losers" | "all" = "losers", deleteBranches = false) =>
  invoke<CleanupResult>("arena_cleanup", { arenaId, scope, deleteBranches });
export const arenaDelete = (arenaId: string) => invoke<void>("arena_delete", { arenaId });
export const arenaScoreboard = (projectId?: string | null) =>
  invoke<ScoreRow[]>("arena_scoreboard", { projectId: projectId ?? null });

export function onArena(cb: (p: ArenaEventPayload) => void): Promise<UnlistenFn> {
  return listen<ArenaEventPayload>(ARENA_EVENT, (e) => cb(e.payload));
}

/** `"CODE: msg"` → partes. */
export function arenaError(e: unknown): { code: string | null; message: string } {
  const s = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  const m = /^([A-Z][A-Z0-9_]+):\s*([\s\S]*)$/.exec(s);
  return m ? { code: m[1], message: m[2] } : { code: null, message: s };
}

// ── Helpers puros ───────────────────────────────────────────────────────

export const ACTIVE: EntryStatus[] = ["preparing", "running", "testing"];

export function isActive(e: Pick<ArenaEntry, "status">): boolean {
  return ACTIVE.includes(e.status);
}

export function formatMs(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms)) return "—";
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${String(s % 60).padStart(2, "0")}s`;
  return `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, "0")}m`;
}

export function formatCost(usd: number | null | undefined): string {
  if (usd == null || !Number.isFinite(usd)) return "—";
  if (usd === 0) return "$0";
  if (usd < 0.01) return `$${usd.toFixed(4)}`;
  return `$${usd.toFixed(2)}`;
}

export function formatTokens(n: number | null | undefined): string {
  if (n == null || !Number.isFinite(n)) return "—";
  if (n < 1000) return String(Math.round(n));
  if (n < 1_000_000) return `${(n / 1000).toFixed(n < 10_000 ? 1 : 0)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

/** Duração ao vivo: do início do turno até agora, ou a medida final. */
export function elapsedOf(e: ArenaEntry, now: number): number | null {
  if (e.status === "preparing" || e.status === "running") {
    const from = e.turnStartedAt ?? e.startedAt;
    return from ? now - Date.parse(from) : null;
  }
  return e.durationMs;
}

/**
 * Destaques por coluna para o placar da arena: o mais rápido, o mais barato,
 * o menor diff e o mais votado entre as entradas terminadas.
 */
export function highlights(entries: ArenaEntry[]): Record<string, string[]> {
  const done = entries.filter((e) => e.status === "done");
  const out: Record<string, string[]> = {};
  const mark = (id: string | undefined, tag: string) => {
    if (!id) return;
    (out[id] ??= []).push(tag);
  };
  const best = <T>(list: ArenaEntry[], key: (e: ArenaEntry) => T | null, cmp: (a: T, b: T) => number) => {
    let win: ArenaEntry | undefined;
    let val: T | null = null;
    let tie = false;
    for (const e of list) {
      const v = key(e);
      if (v == null) continue;
      if (val == null || cmp(v, val) < 0) {
        win = e;
        val = v;
        tie = false;
      } else if (cmp(v, val) === 0) tie = true;
    }
    return tie ? undefined : win;
  };
  if (done.length < 2) return out;
  mark(best(done, (e) => e.durationMs, (a, b) => a - b)?.entryId, "fastest");
  mark(best(done, (e) => e.costUsd, (a, b) => a - b)?.entryId, "cheapest");
  mark(best(done.filter((e) => e.filesChanged > 0), (e) => e.additions + e.deletions, (a, b) => a - b)?.entryId, "smallest");
  mark(best(done.filter((e) => e.votes > 0), (e) => -e.votes, (a, b) => a - b)?.entryId, "voted");
  return out;
}

/** Texto do prompt a partir de um plano aprovado. */
export function promptFromPlan(markdown: string, lead: string): string {
  return `${lead}\n\n${markdown.trim()}\n`;
}
