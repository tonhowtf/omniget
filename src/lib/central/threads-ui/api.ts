import { invoke } from "@tauri-apps/api/core";
import {
  vcsCheckpointCapture,
  vcsCheckpointDelete,
  vcsCheckpointList,
  
  vcsDiffFull,
  vcsDiffTurn,
  
  vcsPrView,
  
  vcsRestoreTurn,
  vcsStatus,
  vcsWorktreeCreate,
  
  type DiffOptions,
  type DiffResult,
  
  type PrInfo,
  
  
  type RepoStatus,
  type WorktreeInfo,
} from "$lib/central/vcs";
import type {
  
  
  
  
  RevertOutcome,
  
} from "$lib/central/threads/types";

const GLUE = {
  checkpoints: "threads_checkpoints",
  diff: "threads_diff",
  revertToTurn: "threads_revert_to_turn",
  worktreeCancel: "threads_worktree_cancel",
  gitStatus: "threads_git_status",
  gitCommitMessage: "threads_git_commit_message",
  gitCommit: "threads_git_commit",
  gitPush: "threads_git_push",
  gitPrCreate: "threads_git_pr_create",
  gitPr: "threads_git_pr",
  gitPrRefresh: "threads_git_pr_refresh",
  filesSearch: "threads_files_search",
  terminalOpen: "threads_terminal_open",
  terminalClose: "threads_terminal_close",
  usage: "threads_usage",
  limits: "threads_limits",
  importExternal: "threads_import_external",
  resumeExternal: "threads_resume_external",
} as const;

const missing = new Set<string>();
const present = new Set<string>();

function isMissingCommand(e: unknown): boolean {
  const s = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  return /command\s+\S+\s+not\s+found|unknown command|not allowed\. (command|plugin)/i.test(s);
}

async function tryGlue<T>(name: string, args: Record<string, unknown>): Promise<T | undefined> {
  if (missing.has(name)) return undefined;
  try {
    const out = await invoke<T>(name, args);
    present.add(name);
    return out;
  } catch (e) {
    if (isMissingCommand(e)) {
      missing.add(name);
      return undefined;
    }
    present.add(name);
    throw e;
  }
}

function glueMissing(): boolean {
  return missing.has(GLUE.diff);
}

type DiffScope = { kind: "turn"; turn: number } | { kind: "total" };

export async function threadDiff(threadId: string, cwd: string, scope: DiffScope, options?: DiffOptions): Promise<DiffResult> {
  const glued = await tryGlue<DiffResult>(GLUE.diff, {
    threadId,
    turn: scope.kind === "turn" ? scope.turn : null,
    scope: scope.kind,
    options: options ?? null,
  });
  if (glued) return glued;
  return scope.kind === "turn" ? vcsDiffTurn(cwd, threadId, scope.turn, options) : vcsDiffFull(cwd, threadId, undefined, options);
}

export async function threadRevertToTurn(
  threadId: string,
  cwd: string,
  turnCount: number,
  restoreFiles: boolean,
  revertConversation: (turnCount: number) => Promise<unknown>,
): Promise<RevertOutcome> {
  const glued = await tryGlue<RevertOutcome>(GLUE.revertToTurn, { threadId, turn: turnCount, restoreFiles });
  if (glued !== undefined) return glued;
  let restoredFiles: string[] = [];
  if (restoreFiles) restoredFiles = (await vcsRestoreTurn(cwd, threadId, turnCount, { confirm: true, dropLater: true })).files;
  else await vcsCheckpointDelete(cwd, threadId, turnCount).catch(() => 0);
  await revertConversation(turnCount);
  return { threadId, turnCount, restoredFiles, prompt: null, attachments: [] };
}

export async function threadTerminalCwd(_threadId: string, fallbackCwd: string): Promise<string> {
  return fallbackCwd;
}

type ThreadTerminal = { id: string; title: string; cwd: string | null; alive: boolean; cols: number; rows: number };

function threadTerminalOpen(
  threadId: string,
  opts: { terminalId?: string; newTab?: boolean; cols?: number; rows?: number } = {},
): Promise<ThreadTerminal> {
  return invoke<ThreadTerminal>(GLUE.terminalOpen, { threadId, ...opts });
}

export async function ensureCheckpoint(cwd: string, threadId: string, turn: number, overwrite = false): Promise<void> {
  if (present.has(GLUE.diff)) return;
  if (!glueMissing()) {
    await tryGlue(GLUE.diff, { threadId: "__probe__", turn: 0, scope: "turn", options: { stat_only: true } }).catch(() => undefined);
    if (!glueMissing()) return;
  }
  try {
    if (!overwrite) {
      const list = await vcsCheckpointList(cwd, threadId);
      if (list.some((c) => c.turn === turn)) return;
    }
    await vcsCheckpointCapture(cwd, threadId, turn);
  } catch {
  }
}

export const createWorktree = (repo: string, thread: string, base?: string | null) =>
  vcsWorktreeCreate({ repo, thread, base: base ?? null, run_setup: true }) as Promise<WorktreeInfo>;

export async function repoStatus(cwd: string): Promise<RepoStatus | null> {
  try {
    return await vcsStatus(cwd);
  } catch {
    return null;
  }
}

export async function prLookup(cwd: string, reference?: string): Promise<PrInfo | null> {
  try {
    return await vcsPrView(cwd, reference);
  } catch {
    return null;
  }
}

export async function searchFiles(
  cwd: string,
  query: string,
  limit = 40,
  threadId?: string,
): Promise<{ path: string; isDir: boolean }[]> {
  const q = query.trim();
  if (q.length < 1 || !threadId) return [];
  try {
    const glued = await tryGlue<string[]>(GLUE.filesSearch, { threadId, query: q, limit });
    return (glued ?? []).map((path) => ({ path, isDir: false }));
  } catch {
    return [];
  }
}
