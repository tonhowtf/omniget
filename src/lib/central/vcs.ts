// Invokes tipados dos comandos `vcs_*` da Central (git das threads): status,
// worktree por thread, checkpoints por turno, diffs, restauração e as ações
// do botão dividido (commit, push, PR). Os erros chegam como "CODE: mensagem".

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export const WORKTREE_SETUP_EVENT = "central://vcs/worktree-setup";

// ---- tipos (espelham os structs Rust, campos em snake_case) ----

export type StatusFile = {
  path: string;
  orig_path?: string;
  index: string;
  worktree: string;
  kind: "modified" | "added" | "deleted" | "renamed" | "copied" | "typechange" | "untracked" | "conflict" | "ignored";
};

export type RemoteInfo = { name: string; url: string; host_kind: HostKind };
export type HostKind = "github" | "gitlab" | "bitbucket" | "azure" | "forgejo" | "unknown" | "none";

export type RepoStatus = {
  is_repo: boolean;
  root: string | null;
  git_dir: string | null;
  common_dir: string | null;
  linked_worktree: boolean;
  omniget_worktree: boolean;
  branch: string | null;
  head: string | null;
  detached: boolean;
  upstream: string | null;
  ahead: number;
  behind: number;
  remote: RemoteInfo | null;
  dirty: boolean;
  staged: number;
  unstaged: number;
  untracked: number;
  conflicts: number;
  files: StatusFile[];
  base_branch: string | null;
  index_locked: boolean;
};

export type BranchInfo = {
  name: string;
  remote: boolean;
  current: boolean;
  oid: string;
  upstream: string | null;
  updated_at: number;
  worktree: string | null;
};

export type LogEntry = {
  oid: string;
  short: string;
  author: string;
  email: string;
  at: number;
  subject: string;
  refs: string;
};

export type ProjectScript = { name: string; command: string; run_on_worktree_create: boolean; run_async: boolean };
export type ProjectConfig = {
  source: string | null;
  worktree_submodules: "recursive" | "top-level" | "none" | null;
  default_thread_env_mode: "local" | "worktree" | null;
  scripts: ProjectScript[];
};

export type WorktreeRequest = {
  repo: string;
  thread: string;
  base?: string | null;
  branch?: string | null;
  start_from_origin?: boolean;
  submodules?: "recursive" | "top-level" | "none" | null;
  run_setup?: boolean;
};

export type SetupStageId = "fetch" | "checkout" | "submodules" | "setup-script";
export type SetupStageState = "pending" | "running" | "done" | "skipped" | "warning" | "failed";
export type SetupStage = {
  id: SetupStageId;
  state: SetupStageState;
  detail: string | null;
  percent: number | null;
  tail: string[];
};
export type SetupSnapshot = {
  thread: string;
  phase: "running" | "done" | "failed" | "cancelled";
  path: string | null;
  branch: string | null;
  base: string | null;
  stages: SetupStage[];
  error: string | null;
};

export type WorktreeInfo = {
  thread: string;
  path: string;
  branch: string;
  base: string;
  base_commit: string;
  repo_root: string;
  reused: boolean;
};

export type WorktreeEntry = {
  path: string;
  head: string | null;
  branch: string | null;
  thread: string | null;
  omniget: boolean;
  prunable: boolean;
  main: boolean;
};

export type RemoveResult = {
  path: string;
  removed: boolean;
  branch_deleted: string | null;
  checkpoints_deleted: number;
};

export type Checkpoint = {
  thread: string;
  turn: number;
  ref: string;
  commit: string;
  tree: string;
  at: number;
  shadow: boolean;
};

export type DiffOptions = {
  ignore_whitespace?: boolean;
  max_bytes?: number;
  max_file_bytes?: number;
  context?: number;
  stat_only?: boolean;
};

export type FileStatus = "added" | "deleted" | "modified" | "renamed" | "binary";
export type FileDiff = {
  path: string;
  old_path?: string;
  status: FileStatus;
  additions: number;
  deletions: number;
  binary: boolean;
  patch: string;
  truncated: boolean;
};
export type DiffResult = {
  from: string;
  to: string;
  files: FileDiff[];
  additions: number;
  deletions: number;
  truncated: boolean;
};

export type RestoreResult = { turn: number; files: string[]; dropped_later: number };

export type ChangeSummary = { path: string; status: FileStatus; additions: number; deletions: number };
export type CommitSuggestion = { subject: string; body: string; files: ChangeSummary[] };
export type CommitResult = {
  status: "created" | "skipped_no_changes";
  sha: string | null;
  subject: string;
  branch: string | null;
};
export type PushResult = {
  status: "pushed" | "skipped_up_to_date";
  remote: string;
  branch: string;
  upstream: string;
  set_upstream: boolean;
};

export type HostInfo = {
  kind: HostKind;
  host: string | null;
  cli: "gh" | "glab" | null;
  cli_path: string | null;
  authenticated: boolean;
  available: boolean;
  reason: "no-remote" | "provider-unsupported" | "cli-missing" | "cli-unauthenticated" | null;
  term: "PR" | "MR";
};

export type PrInfo = {
  number: number;
  title: string;
  url: string;
  state: "open" | "closed" | "merged";
  draft: boolean;
  base: string;
  head: string;
  updated_at: string | null;
  review_decision: string | null;
  host: "github" | "gitlab";
};
export type PrRequest = { title: string; body?: string; base?: string | null; draft?: boolean };
export type PrCreateResult = { status: "created" | "opened_existing"; pr: PrInfo };

/** Uma seleção de linhas no DiffViewer, pronta para virar contexto no chat. */
export type ReviewComment = {
  path: string;
  /** "old" (só linhas removidas), "new" (adicionadas/contexto) ou "mixed". */
  side: "old" | "new" | "mixed";
  start: number;
  end: number;
  /** Ex.: "+12 to +18", "-4", "L3 to L9". */
  rangeLabel: string;
  /** Mini hunk autocontido (@@ … @@ + linhas com marcador). */
  diff: string;
  /** Texto do comentário (vazio em "adicionar ao chat" direto). */
  text: string;
  /** Linguagem para a cerca de código. */
  language: string;
};

// ---- códigos de erro ----

export const VCS_ERR = {
  gitMissing: "ERR_VCS_GIT_MISSING",
  timeout: "ERR_VCS_TIMEOUT",
  exit: "ERR_VCS_EXIT",
  notRepo: "ERR_VCS_NOT_REPO",
  invalid: "ERR_VCS_INVALID",
  unsafe: "ERR_VCS_UNSAFE",
  notFound: "ERR_VCS_NOT_FOUND",
  unavailable: "ERR_VCS_UNAVAILABLE",
  cancelled: "ERR_VCS_CANCELLED",
} as const;

/** Separa "CODE: mensagem" do erro de um invoke. */
export function vcsError(e: unknown): { code: string | null; message: string } {
  const s = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  const m = /^(ERR_[A-Z_]+):\s*([\s\S]*)$/.exec(s);
  return m ? { code: m[1], message: m[2] } : { code: null, message: s };
}

// ---- repositório ----

export const vcsStatus = (path: string) => invoke<RepoStatus>("vcs_status", { path });
export const vcsBranches = (path: string) => invoke<BranchInfo[]>("vcs_branches", { path });
export const vcsLog = (path: string, n = 50, rev?: string) => invoke<LogEntry[]>("vcs_log", { path, n, rev: rev ?? null });
export const vcsProjectConfig = (path: string) => invoke<ProjectConfig>("vcs_project_config", { path });

// ---- worktrees ----

export const vcsWorktreeCreate = (request: WorktreeRequest) => invoke<WorktreeInfo>("vcs_worktree_create", { request });
export const vcsWorktreeCancel = (thread: string) => invoke<boolean>("vcs_worktree_cancel", { thread });
export const vcsWorktreeRenameBranch = (path: string, name: string) =>
  invoke<string>("vcs_worktree_rename_branch", { path, name });
export const vcsWorktreeRemove = (
  path: string,
  opts: { force?: boolean; deleteBranch?: boolean; thread?: string } = {},
) =>
  invoke<RemoveResult>("vcs_worktree_remove", {
    path,
    force: opts.force ?? false,
    deleteBranch: opts.deleteBranch ?? false,
    thread: opts.thread ?? null,
  });
export const vcsWorktreeList = (path: string) => invoke<WorktreeEntry[]>("vcs_worktree_list", { path });
export const vcsWorktreeCleanup = (path: string) => invoke<string[]>("vcs_worktree_cleanup", { path });

/** Assina o progresso do setup de worktree (filtra pela thread se dada). */
export function onWorktreeSetup(cb: (s: SetupSnapshot) => void, thread?: string): Promise<UnlistenFn> {
  return listen<SetupSnapshot>(WORKTREE_SETUP_EVENT, (ev) => {
    if (!thread || ev.payload.thread === thread) cb(ev.payload);
  });
}

// ---- checkpoints e diffs ----

export const vcsCheckpointCapture = (path: string, thread: string, turn: number) =>
  invoke<Checkpoint>("vcs_checkpoint_capture", { path, thread, turn });
export const vcsCheckpointList = (path: string, thread: string) =>
  invoke<Checkpoint[]>("vcs_checkpoint_list", { path, thread });
/** Apaga os checkpoints depois de `afterTurn`; sem ele, todos. */
export const vcsCheckpointDelete = (path: string, thread: string, afterTurn?: number) =>
  invoke<number>("vcs_checkpoint_delete", { path, thread, afterTurn: afterTurn ?? null });

export const vcsDiffTurn = (path: string, thread: string, turn: number, options?: DiffOptions) =>
  invoke<DiffResult>("vcs_diff_turn", { path, thread, turn, options: options ?? null });
/** Base (turno 0) → `toTurn`, ou → a árvore viva se omitido. */
export const vcsDiffFull = (path: string, thread: string, toTurn?: number, options?: DiffOptions) =>
  invoke<DiffResult>("vcs_diff_full", { path, thread, toTurn: toTurn ?? null, options: options ?? null });
export const vcsDiffRange = (path: string, thread: string, fromTurn: number, toTurn?: number, options?: DiffOptions) =>
  invoke<DiffResult>("vcs_diff_range", { path, thread, fromTurn, toTurn: toTurn ?? null, options: options ?? null });

/** Fora de uma worktree do OmniGet exige `confirm: true` (senão ERR_VCS_UNSAFE). */
export const vcsRestoreTurn = (
  path: string,
  thread: string,
  turn: number,
  opts: { confirm?: boolean; dropLater?: boolean } = {},
) =>
  invoke<RestoreResult>("vcs_restore_turn", {
    path,
    thread,
    turn,
    confirm: opts.confirm ?? false,
    dropLater: opts.dropLater ?? false,
  });

// ---- ações ----

export const vcsWorkingChanges = (path: string) => invoke<ChangeSummary[]>("vcs_working_changes", { path });
export const vcsCommitSuggest = (path: string) => invoke<CommitSuggestion>("vcs_commit_suggest", { path });
export const vcsCommit = (path: string, message?: string | null, files?: string[] | null) =>
  invoke<CommitResult>("vcs_commit", { path, message: message ?? null, files: files ?? null });
export const vcsPush = (path: string) => invoke<PushResult>("vcs_push", { path });
export const vcsHost = (path: string) => invoke<HostInfo>("vcs_host", { path });
export const vcsPrCreate = (path: string, request: PrRequest) =>
  invoke<PrCreateResult>("vcs_pr_create", { path, request });
export const vcsPrView = (path: string, reference?: string) =>
  invoke<PrInfo | null>("vcs_pr_view", { path, reference: reference ?? null });

// ---- utilidades ----

/** 1234 → "1.2k". */
export function compactCount(n: number): string {
  if (n < 1000) return String(n);
  if (n < 10000) return (n / 1000).toFixed(1).replace(/\.0$/, "") + "k";
  if (n < 1_000_000) return Math.round(n / 1000) + "k";
  return (n / 1_000_000).toFixed(1).replace(/\.0$/, "") + "M";
}
