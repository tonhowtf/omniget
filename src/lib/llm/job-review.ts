import { invoke } from "@tauri-apps/api/core";
import type { FileDiff, FileStatus, ReviewComment } from "$lib/central/vcs";
import type { Job } from "$lib/stores/llm-jobs-store.svelte";

export type Hunk = {
  id: string;
  path: string;
  index: number;
  header: string;
  old_start: number;
  old_lines: number;
  new_start: number;
  new_lines: number;
  additions: number;
  deletions: number;
  patch: string;
  complete: boolean;
};

export type HunksView = {
  job_id: string;
  state: string;
  complete: boolean;
  clipped: boolean;
  mode: "worktree" | "shared";
  actionable: boolean;
  hunks: Hunk[];
  files: { path: string; patch: string }[];
};

export type JobNote = {
  id: string;
  job_id: string;
  path: string;
  start_line: number | null;
  end_line: number | null;
  body: string;
  created_ms: number;
  sent_ms: number | null;
  sent_job: string | null;
  head_oid: string | null;
};

export type IntegratePreview = {
  job_id: string;
  branch: string;
  target_branch: string | null;
  blocker: string | null;
  commits: string[];
  up_to_date: boolean;
  worktree_dirty: string[];
};

type IntegrateOutcome = {
  status: "merged" | "up_to_date" | "conflict" | "failed";
  strategy: "merge" | "squash";
  target_branch: string;
  commit: string | null;
  conflicts: string[];
  output: string;
  clean: boolean;
};

type WalkthroughStop = { title: string; hunks: string[]; importance: "critical" | "normal" | "context"; prose: string };
export type Walkthrough = {
  key: string;
  title: string;
  focus: string;
  stops: WalkthroughStop[];
  uncovered: string[];
  skipped: string[];
  raw: string | null;
  model: string;
  created_ms: number;
  cost_usd: number | null;
  tokens: number | null;
  cached: boolean;
};

const STATUS: Record<string, FileStatus> = { A: "added", D: "deleted", M: "modified", R: "renamed" };

export function viewerFiles(
  files: { path: string; status: string; additions: number | null; deletions: number | null; binary?: boolean }[],
  view: HunksView,
): FileDiff[] {
  const patches = new Map(view.files.map((f) => [f.path, f.patch]));
  return files.map((f) => {
    const patch = patches.get(f.path) ?? "";
    const binary = !!f.binary;
    return {
      path: f.path,
      status: binary ? "binary" : (STATUS[f.status] ?? "modified"),
      additions: f.additions ?? 0,
      deletions: f.deletions ?? 0,
      binary,
      patch,
      truncated: !binary && !patch && (f.additions ?? 0) + (f.deletions ?? 0) > 0,
    };
  });
}

export function hunkAt(view: HunksView, path: string, index: number): Hunk | null {
  return view.hunks.find((h) => h.path === path && h.index === index) ?? null;
}

export function noteFromComment(c: ReviewComment): { path: string; start_line: number | null; end_line: number | null; body: string } {
  if (c.side === "old") {
    return { path: c.path, start_line: null, end_line: null, body: `(on removed lines ${c.rangeLabel}) ${c.text.trim()}` };
  }
  return { path: c.path, start_line: c.start, end_line: c.end, body: c.text.trim() };
}

export function noteRange(n: Pick<JobNote, "start_line" | "end_line">): string {
  if (n.start_line == null) return "file";
  if (n.end_line == null || n.end_line === n.start_line) return `L${n.start_line}`;
  return `L${n.start_line}–${n.end_line}`;
}

export function errorCode(e: unknown): string {
  const text = String(e);
  const m = /^([A-Z][A-Z0-9_]+):/.exec(text);
  return m ? m[1] : "";
}

export const fetchHunks = (id: string) => invoke<HunksView>("llm_job_hunks", { id });
export const discardHunk = (id: string, hunk: Hunk) =>
  invoke<{ hunk: Hunk; job: Job }>("llm_job_discard_hunk", { id, hunkId: hunk.id, patch: hunk.patch });
export const fetchNotes = (id: string) => invoke<JobNote[]>("llm_job_notes_list", { id });
export const addNote = (id: string, n: ReturnType<typeof noteFromComment>) =>
  invoke<JobNote>("llm_job_note_add", { id, path: n.path, startLine: n.start_line, endLine: n.end_line, body: n.body });
export const deleteNote = (id: string, noteId: string) => invoke("llm_job_note_delete", { id, noteId });
export const sendNotes = (id: string) => invoke<Job>("llm_job_notes_send", { id });
export const fetchIntegratePreview = (id: string) => invoke<IntegratePreview>("llm_job_integrate_preview", { id });
export const integrateJob = (id: string, strategy: "merge" | "squash") =>
  invoke<{ outcome: IntegrateOutcome; job: Job }>("llm_job_integrate", { id, strategy });
export const discardWork = (id: string) =>
  invoke<{ branch: string; tip: string | null; job: Job }>("llm_job_discard_worktree", { id });
export const fetchWalkthrough = (id: string, refresh = false) => invoke<Walkthrough>("llm_job_walkthrough", { id, refresh });
