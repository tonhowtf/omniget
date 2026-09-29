/**
 * Review of a job's changes (3.5): hunks with ids and the discard of one,
 * notes that go back to the agent, integrate or discard a worktree job's
 * branch, and the walkthrough on demand. Typed invokes of `llm_job_*` and
 * the pure pieces the panel needs (tested in job-review.test.ts).
 */
import { invoke } from "@tauri-apps/api/core";
import type { FileDiff, FileStatus, ReviewComment } from "$lib/central/vcs";
import type { Job } from "$lib/stores/llm-jobs-store.svelte";

export type Hunk = {
  /** `path:sha1(@@ line + body)[..8]`, computed by the backend only. */
  id: string;
  path: string;
  /** Position among the hunks of its file (the viewer's hunk index). */
  index: number;
  header: string;
  old_start: number;
  old_lines: number;
  new_start: number;
  new_lines: number;
  additions: number;
  deletions: number;
  /** The hunk alone, byte for byte: what a discard sends back. */
  patch: string;
  /** False when the diff's size limit cut it: shown, never acted on. */
  complete: boolean;
};

export type HunksView = {
  job_id: string;
  state: string;
  complete: boolean;
  clipped: boolean;
  mode: "worktree" | "shared";
  /** A hunk can be discarded now (the job ended, its folder is there). */
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
  /** Why the merge cannot start (dirty checkout, detached HEAD…), or null. */
  blocker: string | null;
  commits: string[];
  up_to_date: boolean;
  worktree_dirty: string[];
};

export type IntegrateOutcome = {
  status: "merged" | "up_to_date" | "conflict" | "failed";
  strategy: "merge" | "squash";
  target_branch: string;
  commit: string | null;
  conflicts: string[];
  output: string;
  clean: boolean;
};

export type WalkthroughStop = { title: string; hunks: string[]; importance: "critical" | "normal" | "context"; prose: string };
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

/**
 * The files of a job's diff as the Central's DiffViewer takes them: the
 * counts from the job's file list, the patch of each file from the hunks
 * view (the same bytes the hunk ids were computed from).
 */
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

/** The hunk the viewer shows as hunk `index` of `path`. */
export function hunkAt(view: HunksView, path: string, index: number): Hunk | null {
  return view.hunks.find((h) => h.path === path && h.index === index) ?? null;
}

/**
 * A note from a selection of the viewer. Lines of the new version; a
 * selection of removed lines only has no new line to point at, so it
 * becomes a note on the file that says which removed lines it means.
 */
export function noteFromComment(c: ReviewComment): { path: string; start_line: number | null; end_line: number | null; body: string } {
  if (c.side === "old") {
    return { path: c.path, start_line: null, end_line: null, body: `(on removed lines ${c.rangeLabel}) ${c.text.trim()}` };
  }
  return { path: c.path, start_line: c.start, end_line: c.end, body: c.text.trim() };
}

/** `L3`, `L3–9` or `file` for a note. */
export function noteRange(n: Pick<JobNote, "start_line" | "end_line">): string {
  if (n.start_line == null) return "file";
  if (n.end_line == null || n.end_line === n.start_line) return `L${n.start_line}`;
  return `L${n.start_line}–${n.end_line}`;
}

/** The error code of a backend message (`CODE: detail`). */
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
