// Every backend call of the threads UI that is not the engine store lives
// here. The names are the ones the `t2-glue` worker registered
// (`commands::central::threads::*`); they are all in one place below.
//
// Each thread-level command is tried first; when the host answers "command
// … not found" (an older build without the glue) we fall back to the `vcs_*`
// commands on the thread's folder (worktree or project root). The result
// is cached per command name so a missing one costs one failed invoke.

import { invoke } from "@tauri-apps/api/core";
import {
  vcsCheckpointCapture,
  vcsCheckpointDelete,
  vcsCheckpointList,
  vcsCommit,
  vcsDiffFull,
  vcsDiffTurn,
  vcsPrCreate,
  vcsPrView,
  vcsPush,
  vcsRestoreTurn,
  vcsStatus,
  vcsWorktreeCreate,
  type CommitResult,
  type DiffOptions,
  type DiffResult,
  type PrCreateResult,
  type PrInfo,
  type PrRequest,
  type PushResult,
  type RepoStatus,
  type WorktreeInfo,
} from "$lib/central/vcs";
import type {
  CheckpointRow,
  CommitMessage,
  ExternalImport,
  InstanceLimitsView,
  RevertOutcome,
  UsageRow,
} from "$lib/central/threads/types";

/** Command names of the thread glue (t2-glue). Change here only. */
export const GLUE = {
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

export function isMissingCommand(e: unknown): boolean {
  const s = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  return /command\s+\S+\s+not\s+found|unknown command|not allowed\. (command|plugin)/i.test(s);
}

/** Tries the glue command; `undefined` means "not registered, use the fallback". */
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

/** True once we know the host has no thread-level glue for checkpoints. */
export function glueMissing(): boolean {
  return missing.has(GLUE.diff);
}

export type DiffScope = { kind: "turn"; turn: number } | { kind: "total" };

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

/**
 * Rewinds the conversation to `turnCount` turns and, if asked, the files to
 * the checkpoint of that turn. The host waits for `thread.reverted` and
 * gives back the dropped prompt (`RevertOutcome.prompt`). Fallback: restore
 * files via vcs, then the engine's `thread.revert`.
 */
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
  // Checkpoints of the dropped turns would be reused by the next turns.
  else await vcsCheckpointDelete(cwd, threadId, turnCount).catch(() => 0);
  await revertConversation(turnCount);
  return { threadId, turnCount, restoredFiles, prompt: null, attachments: [] };
}

/**
 * Folder a thread terminal starts in. The page already resolves it
 * (worktree ?? project root); `threads_terminal_open` is not called here
 * because it spawns its own PTY and the TerminalPanel spawns another one.
 * Use {@link threadTerminalOpen} once the panel can attach to a session id.
 */
export async function threadTerminalCwd(_threadId: string, fallbackCwd: string): Promise<string> {
  return fallbackCwd;
}

/** PTY session of the thread (id `thr-<thread>-<n>`, cwd = the thread folder). */
export type ThreadTerminal = { id: string; title: string; cwd: string | null; alive: boolean; cols: number; rows: number };

export function threadTerminalOpen(
  threadId: string,
  opts: { terminalId?: string; newTab?: boolean; cols?: number; rows?: number } = {},
): Promise<ThreadTerminal> {
  return invoke<ThreadTerminal>(GLUE.terminalOpen, { threadId, ...opts });
}

export function threadTerminalClose(threadId: string, terminalId: string): Promise<void> {
  return invoke<void>(GLUE.terminalClose, { threadId, terminalId });
}

export const threadCheckpoints = (threadId: string) => invoke<CheckpointRow[]>(GLUE.checkpoints, { threadId });
export const threadWorktreeCancel = (threadId: string) => invoke<boolean>(GLUE.worktreeCancel, { threadId });
export const threadGitStatus = (threadId: string) => invoke<RepoStatus>(GLUE.gitStatus, { threadId });
export const threadCommitMessage = (threadId: string) => invoke<CommitMessage>(GLUE.gitCommitMessage, { threadId });
export const threadPrCreate = (
  threadId: string,
  pr: { title?: string; body?: string; base?: string; draft?: boolean } = {},
) => invoke<PrCreateResult>(GLUE.gitPrCreate, { threadId, ...pr });
export const threadPrRefresh = (threadId: string) => invoke<PrInfo | null>(GLUE.gitPrRefresh, { threadId });
export const threadUsage = (threadId: string) => invoke<UsageRow[]>(GLUE.usage, { threadId });
export const threadLimits = () => invoke<InstanceLimitsView[]>(GLUE.limits);
export const threadImportExternal = (tool: string, sessionId: string) =>
  invoke<ExternalImport>(GLUE.importExternal, { tool, sessionId });
export const threadResumeExternal = (threadId: string, instanceId?: string, driver?: string) =>
  invoke<string>(GLUE.resumeExternal, { threadId, instanceId: instanceId ?? null, driver: driver ?? null });

export async function threadGitCommit(threadId: string, cwd: string, message?: string): Promise<CommitResult> {
  return (await tryGlue<CommitResult>(GLUE.gitCommit, { threadId, message: message ?? null })) ?? vcsCommit(cwd, message);
}

export async function threadGitPush(threadId: string, cwd: string): Promise<PushResult> {
  return (await tryGlue<PushResult>(GLUE.gitPush, { threadId })) ?? vcsPush(cwd);
}

export async function threadGitPr(threadId: string, cwd: string, request: PrRequest): Promise<PrCreateResult> {
  return (await tryGlue<PrCreateResult>(GLUE.gitPr, { threadId, request })) ?? vcsPrCreate(cwd, request);
}

// ── Checkpoints when the glue is absent ──────────────────────────────────

/**
 * Captures checkpoint `turn` unless it exists. Used only while the thread
 * glue is missing (then the host captures them itself): baseline 0 at thread
 * creation / before the first send, N at the end of turn N.
 */
export async function ensureCheckpoint(cwd: string, threadId: string, turn: number, overwrite = false): Promise<void> {
  if (present.has(GLUE.diff)) return;
  if (!glueMissing()) {
    // Probe once: a registered glue captures its own checkpoints.
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
    // No git and no shadow dir possible: diffs simply stay unavailable.
  }
}

// ── Worktree, status, PRs ────────────────────────────────────────────────

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

// ── @ file search ────────────────────────────────────────────────────────

type Hit = { path: string; size: number | null; is_dir: boolean };
const IGNORED = /[\\/](node_modules|\.git|target|dist|build|\.svelte-kit|\.next|\.venv|__pycache__)[\\/]/;

/**
 * Files of a thread whose path matches `query`. With `threadId` the host
 * answers from `git ls-files` of the thread folder (`threads_files_search`);
 * otherwise (or on an older build) Spotlight / fd / Everything under `cwd`.
 */
export async function searchFiles(
  cwd: string,
  query: string,
  limit = 40,
  threadId?: string,
): Promise<{ path: string; isDir: boolean }[]> {
  const q = query.trim();
  if (q.length < 1) return [];
  if (threadId) {
    try {
      const glued = await tryGlue<string[]>(GLUE.filesSearch, { threadId, query: q, limit });
      if (glued) return glued.map((path) => ({ path, isDir: false }));
    } catch {
      // fall through to the name search
    }
  }
  try {
    const hits = await invoke<Hit[]>("tool_file_search", { query: q, folder: cwd, limit: 300 });
    const root = cwd.replace(/[\\/]+$/, "");
    return hits
      .filter((h) => !IGNORED.test(h.path) && h.path.startsWith(root))
      .map((h) => ({ path: h.path.slice(root.length + 1), isDir: h.is_dir }))
      .sort((a, b) => a.path.length - b.path.length)
      .slice(0, limit);
  } catch {
    return [];
  }
}
