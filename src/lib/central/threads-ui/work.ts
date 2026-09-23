// Activity → work entry (T3 `deriveWorkLogEntries` + `toolGroupAction`):
// classify each tool item, pull the command, output, touched paths and a
// mini diff out of the payload, and summarize a run of entries in one line.

import type { ActivityRow, ApprovalRow } from "$lib/central/threads/types";

export type WorkKind =
  | "command"
  | "read"
  | "edit"
  | "web"
  | "search"
  | "mcp"
  | "dynamic"
  | "agent"
  | "image"
  | "approval"
  | "error"
  | "warning"
  | "info"
  | "compaction"
  | "other";

export type WorkStatus = "running" | "done" | "failed" | "declined" | "stopped";

export type WorkEntry = {
  id: string;
  turnId: string | null;
  kind: WorkKind;
  status: WorkStatus;
  /** Tool / program name ("git", "Read", "mcp__github__search"). */
  name: string;
  /** Human label (title, query, first path…). */
  label: string;
  command: string | null;
  cwd: string | null;
  output: string | null;
  paths: string[];
  /** Unified diff of an edit, when the payload carries one (or can build one). */
  diff: string | null;
  detail: string | null;
  /** Raw JSON shown for MCP / dynamic tools. */
  raw: unknown;
  /** Subagent / task fields. */
  agent: { taskId: string; title: string; status: string; model: string | null; summary: string | null; tokens: number | null } | null;
  severe: boolean;
  createdAt: string;
  updatedAt: string;
};

type Obj = Record<string, unknown>;
const isObj = (v: unknown): v is Obj => !!v && typeof v === "object" && !Array.isArray(v);
const str = (v: unknown): string | null => (typeof v === "string" && v.length > 0 ? v : null);

/** Activities that feed side surfaces, not the timeline (T3 "quiet timeline"). */
export function isQuietActivity(a: ActivityRow): boolean {
  switch (a.kind) {
    case "tool.progress":
    case "tool.summary":
    case "thread.token-usage.updated":
    case "usage.updated":
    case "account.rate-limits.updated":
    case "account.updated":
    case "mcp.status.updated":
    case "mcp.oauth.completed":
    case "auth.status":
    case "turn.diff.updated":
    case "thread.metadata.updated":
    case "session.configured":
    case "thread.state.changed":
    case "files.persisted":
    case "thread.realtime.started":
    case "thread.realtime.item-added":
    case "thread.realtime.closed":
      return true;
    case "hook.started":
    case "hook.progress":
    case "hook.completed": {
      const code = a.payload.exitCode;
      return !(typeof code === "number" && code !== 0);
    }
  }
  const itemType = str(a.payload.itemType);
  if (itemType === "plan" || itemType === "user_message") return true;
  return false;
}

const PATH_KEYS = ["path", "file_path", "filePath", "relativePath", "filename", "newPath", "oldPath", "notebook_path", "target_file"];
const NEST_KEYS = ["item", "result", "input", "data", "changes", "files", "edits", "patch", "patches", "operations", "output"];

/** Bounded recursive scan for path-like fields (depth ≤ 4, ≤ 12 files). */
export function collectPaths(v: unknown, out: string[] = [], depth = 0): string[] {
  if (depth > 4 || out.length >= 12 || v == null) return out;
  if (Array.isArray(v)) {
    for (const x of v) {
      if (typeof x === "string" && depth > 0 && /[\\/.]/.test(x) && !/\s/.test(x) && x.length < 400) {
        if (!out.includes(x)) out.push(x);
      } else collectPaths(x, out, depth + 1);
      if (out.length >= 12) break;
    }
    return out;
  }
  if (!isObj(v)) return out;
  for (const k of PATH_KEYS) {
    const s = str(v[k]);
    if (s && s.length < 400 && !out.includes(s)) out.push(s);
  }
  for (const k of NEST_KEYS) if (k in v) collectPaths(v[k], out, depth + 1);
  // `changes` in Codex is `{ "<path>": {...} }`.
  if (isObj(v.changes)) for (const k of Object.keys(v.changes)) if (!out.includes(k) && out.length < 12) out.push(k);
  return out;
}

function commandOf(input: unknown, payload: Obj): string | null {
  const direct = str(payload.command);
  if (direct) return direct;
  if (!isObj(input)) return null;
  const c = input.command ?? input.cmd ?? input.script ?? input.commandLine;
  if (typeof c === "string") return c;
  if (Array.isArray(c)) return unwrapShell(c.map(String));
  if (Array.isArray(input.argv)) return unwrapShell(input.argv.map(String));
  return null;
}

/** ["bash","-lc","git status"] → "git status". */
function unwrapShell(argv: string[]): string {
  if (argv.length >= 3 && /(^|\/)(ba|z|da|fi)?sh$|pwsh|powershell|cmd(\.exe)?$/i.test(argv[0]) && /^(-l?c|\/c|-Command)$/i.test(argv[1])) {
    return argv.slice(2).join(" ");
  }
  return argv.join(" ");
}

export function programName(command: string): string {
  const first = command.trim().replace(/^(sudo|env|time|npx|pnpm exec)\s+/, "").split(/\s+/)[0] ?? "";
  return first.split(/[\\/]/).at(-1) ?? first;
}

function outputOf(data: unknown, payload: Obj): string | null {
  const o = isObj(data) ? data.output : undefined;
  if (typeof o === "string") return o.replace(/\n?<exited with exit code \d+>\s*$/, "");
  if (isObj(o)) {
    const parts = [str(o.aggregatedOutput), str(o.stdout), str(o.stderr), str(o.content), str(o.text)].filter(Boolean);
    if (parts.length) return parts.join("\n");
    try {
      return JSON.stringify(o, null, 2);
    } catch {
      return null;
    }
  }
  if (Array.isArray(o)) {
    const texts = o
      .map((x) => (isObj(x) ? (str(x.text) ?? (isObj(x.content) ? str(x.content.text) : null)) : typeof x === "string" ? x : null))
      .filter(Boolean);
    if (texts.length) return texts.join("\n");
  }
  return str(payload.aggregatedOutput);
}

/** A unified diff for edit tools: explicit patch, or old/new strings, or new content. */
function diffOf(input: unknown, data: unknown, path: string | null): string | null {
  const probe = [input, isObj(data) ? data.output : null, data];
  for (const p of probe) {
    if (!isObj(p)) continue;
    const d = str(p.diff) ?? str(p.patch) ?? str(p.unifiedDiff) ?? str(p.unified_diff);
    if (d) return d;
  }
  if (!isObj(input)) return null;
  const file = path ?? "file";
  const oldS = str(input.old_string) ?? str(input.oldString) ?? str(input.search);
  const newS = typeof input.new_string === "string" ? input.new_string : typeof input.newString === "string" ? input.newString : typeof input.replace === "string" ? input.replace : null;
  if (oldS != null && newS != null) return synthDiff(file, oldS, newS);
  if (Array.isArray(input.edits)) {
    const chunks = input.edits
      .filter(isObj)
      .map((e) => {
        const o = str(e.old_string) ?? str(e.oldString) ?? "";
        const n = typeof e.new_string === "string" ? e.new_string : typeof e.newString === "string" ? e.newString : "";
        return synthHunk(o, n);
      });
    if (chunks.length) return `--- a/${file}\n+++ b/${file}\n${chunks.join("\n")}`;
  }
  const content = typeof input.content === "string" ? input.content : typeof input.contents === "string" ? input.contents : null;
  if (content != null) return synthDiff(file, "", content);
  return null;
}

function synthHunk(oldS: string, newS: string): string {
  const a = oldS === "" ? [] : oldS.split("\n");
  const b = newS === "" ? [] : newS.split("\n");
  return [`@@ -1,${a.length} +1,${b.length} @@`, ...a.map((l) => "-" + l), ...b.map((l) => "+" + l)].join("\n");
}

function synthDiff(file: string, oldS: string, newS: string): string {
  return `--- ${oldS === "" ? "/dev/null" : "a/" + file}\n+++ b/${file}\n${synthHunk(oldS, newS)}`;
}

export function diffStats(diff: string | null): { add: number; del: number } {
  if (!diff) return { add: 0, del: 0 };
  let add = 0;
  let del = 0;
  for (const line of diff.split("\n")) {
    if (line.startsWith("+++") || line.startsWith("---")) continue;
    if (line.startsWith("+")) add++;
    else if (line.startsWith("-")) del++;
  }
  return { add, del };
}

export type DiffFileStat = { path: string; status: string; additions: number; deletions: number; patch: string };

/** Splits a multi-file unified diff (`turn.diff.updated`) into per-file stats. */
export function splitUnifiedDiff(diff: string): DiffFileStat[] {
  const out: DiffFileStat[] = [];
  let cur: DiffFileStat | null = null;
  let buf: string[] = [];
  const flush = () => {
    if (cur) {
      cur.patch = buf.join("\n");
      out.push(cur);
    }
    buf = [];
  };
  for (const line of diff.split("\n")) {
    const git = /^diff --git a\/(.+?) b\/(.+)$/.exec(line);
    if (git) {
      flush();
      cur = { path: git[2], status: "modified", additions: 0, deletions: 0, patch: "" };
      buf.push(line);
      continue;
    }
    if (line.startsWith("--- ") && (!cur || buf.some((l) => l.startsWith("@@")))) {
      flush();
      cur = { path: "", status: "modified", additions: 0, deletions: 0, patch: "" };
    }
    if (!cur) continue;
    buf.push(line);
    if (line.startsWith("+++ ")) {
      const p = line.slice(4).replace(/^b\//, "").trim();
      if (p !== "/dev/null") cur.path = p;
      else cur.status = "deleted";
    } else if (line.startsWith("--- ")) {
      const p = line.slice(4).replace(/^a\//, "").trim();
      if (p === "/dev/null") cur.status = "added";
      else if (!cur.path) cur.path = p;
    } else if (line.startsWith("new file")) cur.status = "added";
    else if (line.startsWith("deleted file")) cur.status = "deleted";
    else if (line.startsWith("rename to ")) {
      cur.status = "renamed";
      cur.path = line.slice(10);
    } else if (line.startsWith("+")) cur.additions++;
    else if (line.startsWith("-")) cur.deletions++;
  }
  flush();
  return out.filter((f) => f.path);
}

function classify(itemType: string | null, name: string, kind: string): WorkKind {
  const n = name.toLowerCase();
  if (kind.startsWith("task.")) return "agent";
  if (kind === "runtime.error") return "error";
  if (kind === "runtime.warning" || kind === "config.warning" || kind === "deprecation.notice" || kind === "tool.denied") return "warning";
  if (kind === "model.rerouted" || kind.startsWith("hook.")) return "info";
  switch (itemType) {
    case "command_execution":
      return "command";
    case "file_change":
      return "edit";
    case "web_search":
      return /grep|code/.test(n) ? "search" : "web";
    case "image_view":
      return "image";
    case "mcp_tool_call":
      return "mcp";
    case "collab_agent_tool_call":
      return "agent";
    case "context_compaction":
      return "compaction";
    case "error":
      return "error";
  }
  if (n.startsWith("mcp__") || n.startsWith("mcp:")) return "mcp";
  if (/^(bash|shell|shell_exec|exec|exec_command|run_command|terminal|run_terminal_cmd|powershell)$/.test(n)) return "command";
  if (/(multi_?edit|str_replace|edit|write|patch|apply|create_file|delete_file|notebook_?edit)/.test(n)) return "edit";
  if (/(web_?search|web_?fetch|fetch_url|browse|http_get)/.test(n)) return "web";
  if (/(grep|glob|search|find|list_dir|ls|codebase)/.test(n)) return "search";
  if (/(read|view|cat|open_file|get_file)/.test(n)) return "read";
  if (/^(task|agent|spawn|delegate)/.test(n)) return "agent";
  if (itemType === "dynamic_tool_call") return "dynamic";
  return "other";
}

function statusOf(a: ActivityRow): WorkStatus {
  const s = str(a.payload.status) ?? "";
  if (s === "failed") return "failed";
  if (s === "declined") return "declined";
  if (s === "completed") return "done";
  if (s === "inProgress") return "running";
  if (a.kind === "item.started" || a.kind === "item.updated") return "running";
  if (a.kind === "task.started" || a.kind === "task.progress" || a.kind === "task.updated") {
    const ts = (str(a.payload.status) ?? "").toLowerCase();
    if (/fail|error/.test(ts)) return "failed";
    if (/stop|cancel|interrupt/.test(ts)) return "stopped";
    if (/complete|done|success/.test(ts)) return "done";
    return "running";
  }
  if (a.kind === "task.completed") {
    const ts = (str(a.payload.status) ?? "").toLowerCase();
    if (/fail|error/.test(ts) || a.payload.error) return "failed";
    return "done";
  }
  if (a.tone === "error") return "failed";
  return "done";
}

/** Output text that says the tool failed even when the provider said "completed". */
export function looksLikeFailure(text: string | null): boolean {
  if (!text) return false;
  const tail = text.slice(-2000);
  return /command not found|ENOENT|no such file or directory|is not recognized as the name of a cmdlet|exit code [1-9]\d*|Traceback \(most recent call last\)/i.test(tail);
}

export function toWorkEntry(a: ActivityRow, outputText?: string | null): WorkEntry {
  const p = a.payload ?? {};
  const itemType = str(p.itemType);
  const data = p.data;
  const input = isObj(data) ? data.input : undefined;
  const name = str(p.toolName) ?? str(p.lastToolName) ?? str(p.hookName) ?? a.summary ?? "";
  const kind = classify(itemType, name, a.kind);
  const paths = collectPaths({ input, data: isObj(data) ? data : undefined, ...p });
  const command = kind === "command" ? commandOf(input, p) : null;
  let output = outputOf(data, p) ?? null;
  if (outputText) output = output ? output : outputText;
  const diff = kind === "edit" ? diffOf(input, data, paths[0] ?? null) : null;
  let status = statusOf(a);
  if (status === "done" && kind === "command" && looksLikeFailure(output)) status = "failed";
  const title = str(p.title);
  let label = title ?? "";
  if (kind === "command" && command) label = command;
  else if ((kind === "read" || kind === "edit" || kind === "image") && paths[0]) label = paths[0];
  else if (kind === "web" || kind === "search") {
    const q = isObj(input) ? (str(input.query) ?? str(input.pattern) ?? str(input.url) ?? str(input.q)) : null;
    label = q ?? title ?? name;
  } else if (!label) label = a.summary || name;
  const agent =
    kind === "agent" && a.kind.startsWith("task.")
      ? {
          taskId: str(p.taskId) ?? a.activityId,
          title: str(p.title) ?? str(p.description) ?? a.summary,
          status: str(p.status) ?? (a.kind === "task.completed" ? "completed" : "running"),
          model: str(p.model),
          summary: str(p.summary) ?? str(p.error),
          tokens: isObj(p.usage) && typeof p.usage.outputTokens === "number"
            ? ((p.usage.inputTokens as number) ?? 0) + (p.usage.outputTokens as number)
            : null,
        }
      : null;
  const detail =
    kind === "error" || kind === "warning" || kind === "info"
      ? (str(p.detail) ?? str(p.message) ?? str(p.reason) ?? str(p.output) ?? null)
      : str(p.detail);
  return {
    id: a.activityId,
    turnId: a.turnId,
    kind,
    status,
    name: kind === "command" && command ? programName(command) : name,
    label,
    command,
    cwd: isObj(input) ? str(input.cwd) ?? str(input.workdir) : null,
    output,
    paths,
    diff,
    detail: detail && detail !== label ? detail : null,
    raw: kind === "mcp" || kind === "dynamic" || kind === "other" ? (data ?? p) : null,
    agent,
    severe: kind === "error" || a.kind === "runtime.error",
    createdAt: a.createdAt,
    updatedAt: a.updatedAt,
  };
}

export function approvalEntry(ap: ApprovalRow): WorkEntry {
  const d = ap.detail ?? {};
  return {
    id: `approval:${ap.requestId}`,
    turnId: ap.turnId,
    kind: "approval",
    status: ap.decision === "decline" || ap.decision === "cancel" ? "declined" : "done",
    name: d.toolName ?? ap.requestType,
    label: d.command ?? d.paths?.[0] ?? d.toolName ?? ap.requestType,
    command: d.command ?? null,
    cwd: d.cwd ?? null,
    output: null,
    paths: d.paths ?? [],
    diff: d.diff ?? null,
    detail: ap.decision ?? ap.resolution ?? null,
    raw: null,
    agent: null,
    severe: false,
    createdAt: ap.createdAt,
    updatedAt: ap.resolvedAt ?? ap.createdAt,
  };
}

// ── One-line summaries ──────────────────────────────────────────────────

export type SummaryPart = { key: string; count: number };

/** "Ran 3 commands, read 2 files, changed 1 file" as i18n parts (distinct files for edits). */
export function summarize(entries: WorkEntry[]): SummaryPart[] {
  const commands = entries.filter((e) => e.kind === "command").length;
  const reads = new Set(entries.filter((e) => e.kind === "read" || e.kind === "image").flatMap((e) => (e.paths.length ? e.paths.slice(0, 1) : [e.id]))).size;
  const edits = new Set(entries.filter((e) => e.kind === "edit").flatMap((e) => (e.paths.length ? e.paths : [e.id]))).size;
  const web = entries.filter((e) => e.kind === "web").length;
  const search = entries.filter((e) => e.kind === "search").length;
  const tools = entries.filter((e) => e.kind === "mcp" || e.kind === "dynamic" || e.kind === "other").length;
  const agents = entries.filter((e) => e.kind === "agent").length;
  const updates = entries.filter((e) => e.kind === "approval" || e.kind === "info").length;
  const parts: SummaryPart[] = [];
  if (commands) parts.push({ key: "commands", count: commands });
  if (reads) parts.push({ key: "reads", count: reads });
  if (edits) parts.push({ key: "edits", count: edits });
  if (search) parts.push({ key: "searches", count: search });
  if (web) parts.push({ key: "web", count: web });
  if (tools) parts.push({ key: "tools", count: tools });
  if (agents) parts.push({ key: "agents", count: agents });
  if (updates) parts.push({ key: "updates", count: updates });
  return parts;
}

/** Tense-aware verb key for a single entry ("Running", "Ran"…); failures get a badge. */
export function entryVerbKey(e: WorkEntry): string {
  return `llm.central.threads.work.${e.kind}.${e.status === "running" ? "running" : "done"}`;
}

/** Unified diff → `FileDiff[]` for the DiffViewer (headerless hunks go under `fallbackPath`). */
export function toFileDiffs(diff: string | null | undefined, fallbackPath = "file"): import("$lib/central/vcs").FileDiff[] {
  if (!diff || !diff.trim()) return [];
  const files = splitUnifiedDiff(diff);
  if (files.length) {
    return files.map((f) => ({
      path: f.path,
      status: (["added", "deleted", "renamed"].includes(f.status) ? f.status : "modified") as "added" | "deleted" | "renamed" | "modified",
      additions: f.additions,
      deletions: f.deletions,
      binary: false,
      patch: f.patch,
      truncated: false,
    }));
  }
  const st = diffStats(diff);
  return [{ path: fallbackPath, status: "modified", additions: st.add, deletions: st.del, binary: false, patch: diff, truncated: false }];
}
