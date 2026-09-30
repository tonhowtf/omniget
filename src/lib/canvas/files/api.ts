// File tree / editor / git-lite commands (src-tauri commands/canvas/files.rs).
// Every call carries the workspace and the tree root; Rust confines paths to
// that root. Relative paths use `/`.

import { invoke } from '@tauri-apps/api/core';
import type { BranchInfo, DiffResult, RepoStatus } from '$lib/central/vcs';

export interface FileEntry {
  name: string;
  path: string;
  isDir: boolean;
  isSymlink: boolean;
  hidden: boolean;
  size: number;
  modifiedMs?: number;
}

export interface FileText {
  path: string;
  absPath: string;
  size: number;
  revision: string;
  binary: boolean;
  truncated: boolean;
  text?: string;
  dataUrl?: string;
  mime?: string;
}

export interface SearchHit {
  path: string;
  isDir: boolean;
  line?: number;
  preview?: string;
}

export interface FileTreeData {
  rootPath: string;
  currentPath?: string;
  showsHiddenFiles?: boolean;
  pinnedPaths?: string[];
  viewMode?: string;
  expanded?: string[];
  /** Maestri sort options (Files view). */
  sortBy?: 'name' | 'type' | 'modified' | 'size';
  sortReverse?: boolean;
  foldersFirst?: boolean;
  /** Remote tree (Maestri §K9): `"workspace"` (the workspace's SSH/Docker
   *  environment), `"docker:name"`, `"ssh:host"` or a record. Absent = here. */
  environment?: unknown;
}

/** Editor node = a `file` node with `view: "editor"` (no new node kind on the wire). */
export interface FileEditorData {
  path: string;
  displayName?: string;
  view?: 'editor';
  /** Root of the tree it was opened from (the confinement root). */
  rootPath?: string;
  line?: number;
  isImage?: boolean;
  isVideo?: boolean;
  /** Open files of this editor (absolute paths); `path` is the active one. */
  tabs?: string[];
  /** Select these lines (1-based, inclusive) once, like `line`. */
  selectLines?: { from: number; to?: number };
  /** Same as the tree's: the file lives on a remote environment. */
  environment?: unknown;
}

export class FileConflict extends Error {
  constructor(readonly current: string) {
    super(`CANVAS_FILE_CONFLICT: ${current}`);
  }
}

function err(e: unknown): Error {
  const msg = e instanceof Error ? e.message : String(e);
  const m = /^CANVAS_FILE_CONFLICT:\s*(\S*)/.exec(msg);
  if (m) return new FileConflict(m[1]);
  return e instanceof Error ? e : new Error(msg);
}

async function call<T>(cmd: string, args: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw err(e);
  }
}

export const filesList = (workspaceId: string, root: string, dir = '', showHidden = false) =>
  call<FileEntry[]>('canvas_files_list', { workspaceId, root, dir, showHidden });

export const filesRead = (workspaceId: string, root: string, path: string) =>
  call<FileText>('canvas_files_read', { workspaceId, root, path });

export const filesWrite = (workspaceId: string, root: string, path: string, text: string, ifRevision?: string) =>
  call<FileText>('canvas_files_write', { workspaceId, root, path, text, ifRevision: ifRevision ?? null });

export const filesCreate = (workspaceId: string, root: string, path: string, dir = false) =>
  call<FileEntry>('canvas_files_create', { workspaceId, root, path, dir });

export const filesRename = (workspaceId: string, root: string, from: string, to: string) =>
  call<string>('canvas_files_rename', { workspaceId, root, from, to });

export const filesDelete = (workspaceId: string, root: string, path: string) =>
  call<string>('canvas_files_delete', { workspaceId, root, path });

export const filesSearch = (workspaceId: string, root: string, query: string, content = false, showHidden = false) =>
  call<SearchHit[]>('canvas_files_search', { workspaceId, root, query, content, showHidden, limit: 200 });

export const filesGitStatus = (workspaceId: string, root: string) =>
  call<RepoStatus>('canvas_files_git_status', { workspaceId, root });

export const filesGitDiff = (workspaceId: string, root: string, path?: string) =>
  call<DiffResult>('canvas_files_git_diff', { workspaceId, root, path: path ?? null });

export type GitAction =
  | 'status'
  | 'branches'
  | 'commit'
  | 'pull'
  | 'fetch'
  | 'push'
  | 'sync'
  | 'checkout'
  | 'newBranch'
  | 'deleteBranch'
  | 'merge'
  | 'mergeAbort'
  | 'stashPush'
  | 'stashApply'
  | 'stashPop'
  | 'stashDrop'
  | 'stashList'
  | 'stage'
  | 'unstage'
  | 'discard'
  | 'conflict'
  | 'removeLock';

export interface GitOpArgs {
  message?: string;
  /** Relative to the tree root. */
  paths?: string[];
  branch?: string;
  start?: string;
  force?: boolean;
  index?: number;
  untracked?: boolean;
}

export interface ConflictView {
  path: string;
  currentLabel: string;
  incomingLabel: string;
  base?: string;
  current?: string;
  incoming?: string;
}

export interface GitOpResult {
  action: GitAction;
  ok: boolean;
  output: string;
  conflicts?: string[];
  sha?: string;
  branches?: BranchInfo[];
  stashes?: string[];
  conflict?: ConflictView;
  status?: RepoStatus;
}

export const filesGitOp = (workspaceId: string, root: string, action: GitAction, args: GitOpArgs = {}) =>
  call<GitOpResult>('canvas_files_git_op', { workspaceId, root, action, args });

export function joinPath(root: string, rel: string): string {
  if (!rel) return root;
  const sep = root.includes('\\') && !root.includes('/') ? '\\' : '/';
  return `${root.replace(/[\\/]+$/, '')}${sep}${rel.split('/').join(sep)}`;
}

export function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

export function dirName(rel: string): string {
  const i = rel.lastIndexOf('/');
  return i < 0 ? '' : rel.slice(0, i);
}

/** Relative path of `abs` inside `root`, or null when outside. */
export function relativeTo(root: string, abs: string): string | null {
  const r = root.replace(/[\\/]+$/, '').replace(/\\/g, '/');
  const a = abs.replace(/\\/g, '/');
  if (a === r) return '';
  if (!a.startsWith(`${r}/`)) return null;
  return a.slice(r.length + 1);
}

export const IMAGE_RE = /\.(png|jpe?g|gif|webp|svg|bmp|ico|avif)$/i;
export const VIDEO_RE = /\.(mp4|mov|webm|mkv|m4v)$/i;

/** Git status letter for a tree row: M, A, D, R, U (untracked), C (conflict). */
export function statusLetter(kind: string): string {
  switch (kind) {
    case 'modified':
    case 'typechange':
      return 'M';
    case 'added':
    case 'copied':
      return 'A';
    case 'deleted':
      return 'D';
    case 'renamed':
      return 'R';
    case 'untracked':
      return 'U';
    case 'conflict':
      return 'C';
    default:
      return '';
  }
}

/** Where a symbol is declared under the root (editor F12 / ⌘-click). */
export const filesDefinition = (workspaceId: string, root: string, symbol: string, from?: string | null) =>
  call<SearchHit[]>('canvas_files_definition', { workspaceId, root, symbol, from: from ?? null, limit: 20 });

// ---- remote trees (SSH, Docker, sandbox) -------------------------------

/** A tree/editor whose `data.environment` names somewhere else than here. */
export function isRemoteEnv(env: unknown): boolean {
  if (env == null || env === '') return false;
  const k =
    typeof env === 'object' ? String((env as { kind?: unknown }).kind ?? '') : String(env).split(':')[0].toLowerCase();
  return k !== '' && k !== 'local' && k !== 'tmux' && k !== 'local-tmux';
}

export const filesRemote = <T>(workspaceId: string, args: Record<string, unknown>) =>
  call<T>('canvas_files_remote', { workspaceId, args });

/** One file API over a local root or a remote environment. */
export interface FileSource {
  remote: boolean;
  list(dir?: string, showHidden?: boolean): Promise<FileEntry[]>;
  read(path: string): Promise<FileText>;
  write(path: string, text: string, ifRevision?: string): Promise<FileText>;
  create(path: string, dir?: boolean): Promise<unknown>;
  rename(from: string, to: string): Promise<unknown>;
  remove(path: string): Promise<unknown>;
  search(query: string, content?: boolean, showHidden?: boolean): Promise<SearchHit[]>;
}

export function fileSource(workspaceId: string, root: string, environment?: unknown): FileSource {
  if (!isRemoteEnv(environment)) {
    return {
      remote: false,
      list: (dir = '', hidden = false) => filesList(workspaceId, root, dir, hidden),
      read: (path) => filesRead(workspaceId, root, path),
      write: (path, text, rev) => filesWrite(workspaceId, root, path, text, rev),
      create: (path, dir) => filesCreate(workspaceId, root, path, dir),
      rename: (from, to) => filesRename(workspaceId, root, from, to),
      remove: (path) => filesDelete(workspaceId, root, path),
      search: (q, content, hidden) => filesSearch(workspaceId, root, q, content, hidden),
    };
  }
  const r = <T>(action: string, extra: Record<string, unknown> = {}) =>
    filesRemote<T>(workspaceId, { environment, root, action, ...extra });
  return {
    remote: true,
    list: (dir = '', hidden = false) =>
      r<{ root: string; entries: FileEntry[] }>('list', { path: dir, showHidden: hidden }).then((x) => x.entries),
    read: (path) => r<FileText>('read', { path }),
    write: (path, text, rev) => r<FileText>('write', { path, text, ifRevision: rev ?? null }),
    create: (path, dir) => r('create', { path, dir: !!dir }),
    rename: (from, to) => r('rename', { path: from, to }),
    remove: (path) => r('delete', { path }),
    search: (q, content, hidden) =>
      r<{ hits: SearchHit[] }>('search', { query: q, content: !!content, showHidden: !!hidden }).then((x) => x.hits),
  };
}

/** The absolute folder a remote tree shows (resolves an empty root). */
export const remoteRoot = (workspaceId: string, environment: unknown, root: string) =>
  filesRemote<{ root: string }>(workspaceId, { environment, root, action: 'resolve' }).then((x) => x.root);
