import { invoke } from "@tauri-apps/api/core";

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

type FileStatus = "added" | "deleted" | "modified" | "renamed" | "binary";
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

export type ReviewComment = {
  path: string;
  side: "old" | "new" | "mixed";
  start: number;
  end: number;
  rangeLabel: string;
  diff: string;
  text: string;
  language: string;
};

export function vcsError(e: unknown): { code: string | null; message: string } {
  const s = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  const m = /^(ERR_[A-Z_]+):\s*([\s\S]*)$/.exec(s);
  return m ? { code: m[1], message: m[2] } : { code: null, message: s };
}

export const vcsStatus = (path: string) => invoke<RepoStatus>("vcs_status", { path });
export const vcsLog = (path: string, n = 50, rev?: string) => invoke<LogEntry[]>("vcs_log", { path, n, rev: rev ?? null });
export const vcsCommitSuggest = (path: string) => invoke<CommitSuggestion>("vcs_commit_suggest", { path });
export const vcsCommit = (path: string, message?: string | null, files?: string[] | null) =>
  invoke<CommitResult>("vcs_commit", { path, message: message ?? null, files: files ?? null });
export const vcsPush = (path: string) => invoke<PushResult>("vcs_push", { path });
export const vcsHost = (path: string) => invoke<HostInfo>("vcs_host", { path });
export const vcsPrCreate = (path: string, request: PrRequest) =>
  invoke<PrCreateResult>("vcs_pr_create", { path, request });
export const vcsPrView = (path: string, reference?: string) =>
  invoke<PrInfo | null>("vcs_pr_view", { path, reference: reference ?? null });

export function compactCount(n: number): string {
  if (n < 1000) return String(n);
  if (n < 10000) return (n / 1000).toFixed(1).replace(/\.0$/, "") + "k";
  if (n < 1_000_000) return Math.round(n / 1000) + "k";
  return (n / 1_000_000).toFixed(1).replace(/\.0$/, "") + "M";
}

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
