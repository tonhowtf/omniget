// Invokes tipados dos comandos `vcs_*` da Central (git das threads): status,
// worktree por thread, checkpoints por turno, diffs, restauração e as ações
// do botão dividido (commit, push, PR). Os erros chegam como "CODE: mensagem".

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

const WORKTREE_SETUP_EVENT = "central://vcs/worktree-setup";

// ---- tipos (espelham os structs Rust, campos em snake_case) ----

type StatusFile = {
  path: string;
  orig_path?: string;
  index: string;
  worktree: string;
  kind: "modified" | "added" | "deleted" | "renamed" | "copied" | "typechange" | "untracked" | "conflict" | "ignored";
};

type RemoteInfo = { name: string; url: string; host_kind: HostKind };
type HostKind = "github" | "gitlab" | "bitbucket" | "azure" | "forgejo" | "unknown" | "none";

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

type WorktreeRequest = {
  repo: string;
  thread: string;
  base?: string | null;
  branch?: string | null;
  start_from_origin?: boolean;
  submodules?: "recursive" | "top-level" | "none" | null;
  run_setup?: boolean;
};

type SetupStageId = "fetch" | "checkout" | "submodules" | "setup-script";
type SetupStageState = "pending" | "running" | "done" | "skipped" | "warning" | "failed";
type SetupStage = {
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

type RestoreResult = { turn: number; files: string[]; dropped_later: number };

export type ChangeSummary = { path: string; status: FileStatus; additions: number; deletions: number };
type CommitSuggestion = { subject: string; body: string; files: ChangeSummary[] };
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

// ---- worktrees ----

export const vcsWorktreeCreate = (request: WorktreeRequest) => invoke<WorktreeInfo>("vcs_worktree_create", { request });
export const vcsWorktreeCancel = (thread: string) => invoke<boolean>("vcs_worktree_cancel", { thread });

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

// Same rules as `core::vcs::actions::suggest_commit_message`, so the commit
// dialog can redo the message for the files left checked.
const isDoc = (p: string) => {
  const l = p.toLowerCase();
  return /\.(md|mdx|txt|rst)$/.test(l) || l.startsWith("docs/");
};
const isTest = (p: string) => {
  const l = p.toLowerCase();
  return (
    l.includes("/tests/") || l.startsWith("tests/") || l.includes("/test/") || l.includes(".test.") ||
    l.includes(".spec.") || l.includes("_test.") || l.endsWith("_tests.rs")
  );
};
const CONFIG_NAMES = new Set([
  "package.json", "cargo.toml", "cargo.lock", "pnpm-lock.yaml", "package-lock.json", "yarn.lock",
  "tsconfig.json", ".gitignore", "go.mod", "go.sum", "pyproject.toml", "requirements.txt",
]);
const isConfig = (p: string) => {
  const name = (p.split("/").pop() ?? p).toLowerCase();
  return CONFIG_NAMES.has(name) || (name.startsWith(".") && (name.endsWith("rc") || name.endsWith(".json") || name.endsWith(".yml") || name.endsWith(".yaml")));
};
const baseName = (p: string) => p.split("/").pop() ?? p;
function commonDir(paths: string[]): string {
  let parts: string[] | null = null;
  for (const p of paths) {
    const dirs = p.split("/").slice(0, -1);
    if (!parts) parts = dirs;
    else {
      let i = 0;
      while (i < parts.length && i < dirs.length && parts[i] === dirs[i]) i++;
      parts = parts.slice(0, i);
    }
  }
  return (parts ?? []).join("/");
}

export function suggestCommitMessage(changes: ChangeSummary[]): string {
  if (!changes.length) return "Update files";
  const paths = changes.map((c) => c.path);
  const prefix = paths.every(isDoc) ? "docs: " : paths.every(isTest) ? "test: " : paths.every(isConfig) ? "chore: " : "";
  const all = (s: string) => changes.every((c) => c.status === s);
  const verb = all("added") ? "Add" : all("deleted") ? "Remove" : all("renamed") ? "Rename" : "Update";
  let target: string;
  if (changes.length === 1) {
    const p = changes[0].path;
    target = p.length <= 50 ? p : baseName(p);
  } else {
    const dir = commonDir(paths);
    if (dir) target = `${dir} (${changes.length} files)`;
    else {
      const names = paths.map(baseName).filter((n, i, a) => i === 0 || a[i - 1] !== n);
      target = names.length <= 3 && names.reduce((s, n) => s + n.length + 2, 0) <= 50 ? names.join(", ") : `${changes.length} files`;
    }
  }
  let subject = prefix ? `${prefix}${verb.toLowerCase()} ${target}` : `${verb} ${target}`;
  if ([...subject].length > 72) subject = [...subject].slice(0, 71).join("") + "…";
  if (changes.length < 2) return subject;
  const mark = (s: string) => (s === "added" ? "A" : s === "deleted" ? "D" : s === "renamed" ? "R" : "M");
  const lines = changes.slice(0, 20).map((c) => `- ${mark(c.status)} ${c.path} (+${c.additions} -${c.deletions})`);
  if (changes.length > 20) lines.push(`- … and ${changes.length - 20} more`);
  return `${subject}\n\n${lines.join("\n")}`;
}
