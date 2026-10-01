// Notes API (contract-doc "Commands"): real .md files owned by note nodes.
// Revision = first 16 hex of sha256(text); writes carry `ifRevision` and a
// mismatch comes back as `CANVAS_NOTE_CONFLICT: <current>` (Maestri wire 412).

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { CanvasNode } from '../model/types';

const NOTE_EVENT = 'canvas://note';

interface NoteContent {
  nodeId: string;
  fileName?: string;
  path: string;
  displayName: string;
  text: string;
  revision: string;
  isExternal: boolean;
  lastModifiedAt?: string;
  preview: string;
}

interface NoteSummary {
  nodeId?: string;
  floorId?: string;
  fileName?: string;
  path: string;
  displayName: string;
  revision: string;
  isExternal: boolean;
  lastModifiedAt?: string;
  preview: string;
}

interface NoteEvent {
  workspaceId: string;
  nodeId: string;
  revision: string;
  origin: string;
}

interface NoteData {
  fileName?: string;
  path?: string;
  isExternal?: boolean;
  hasCustomName?: boolean;
  isContentLocked?: boolean;
  customColor?: string;
  /** Default face when not being edited (New Item Defaults → note render). */
  mode?: 'raw' | 'formatted';
  /** Legacy/in-memory text (placeholder notes, memory mode). */
  text?: string;
}

export function noteData(node: CanvasNode): NoteData {
  return (node.data ?? {}) as NoteData;
}

/** Id of the fichário holding this note, when it is inside one. */
export function stackIdOf(node: CanvasNode | undefined): string | undefined {
  const v = (node as (CanvasNode & { stackId?: unknown }) | undefined)?.stackId;
  return typeof v === 'string' && v ? v : undefined;
}

export function hasFile(node: CanvasNode): boolean {
  const d = noteData(node);
  return !!d.fileName || !!d.path;
}

export class NoteConflict extends Error {
  constructor(readonly current: string) {
    super(`CANVAS_NOTE_CONFLICT: ${current}`);
  }
}

export class NoteLocked extends Error {}

function classify(err: unknown): Error {
  const msg = err instanceof Error ? err.message : String(err);
  const m = /^CANVAS_NOTE_CONFLICT:\s*(\S*)/.exec(msg);
  if (m) return new NoteConflict(m[1]);
  if (msg.startsWith('CANVAS_NOTE_LOCKED')) return new NoteLocked(msg);
  return err instanceof Error ? err : new Error(msg);
}

export async function noteRead(workspaceId: string, floorId: string, nodeId: string): Promise<NoteContent> {
  return invoke<NoteContent>('canvas_note_read', { workspaceId, floorId, nodeId });
}

export async function noteWrite(
  workspaceId: string,
  floorId: string,
  nodeId: string,
  text: string,
  ifRevision?: string,
): Promise<NoteContent> {
  try {
    return await invoke<NoteContent>('canvas_note_write', {
      workspaceId,
      floorId,
      nodeId,
      text,
      ifRevision: ifRevision ?? null,
      origin: 'user',
    });
  } catch (err) {
    throw classify(err);
  }
}

/**
 * Maestri "Move to…": writes the note to `dest` (absolute, `~/…`, or relative
 * to the workspace folder) and makes it an external note there; deleting the
 * node later keeps that file.
 */
export async function noteMove(
  workspaceId: string,
  floorId: string,
  nodeId: string,
  dest: string,
): Promise<NoteContent> {
  return invoke<NoteContent>('canvas_note_move', { workspaceId, floorId, nodeId, dest });
}

export async function noteList(workspaceId: string): Promise<NoteSummary[]> {
  try {
    const list = await invoke<NoteSummary[]>('canvas_note_list', { workspaceId });
    return Array.isArray(list) ? list : [];
  } catch {
    return [];
  }
}

type NoteListener = (e: NoteEvent) => void;
const noteListeners = new Set<NoteListener>();
let noteUnlisten: Promise<UnlistenFn | null> | null = null;

/** One Tauri listener shared by every mounted note. */
export function onNoteEvent(fn: NoteListener): () => void {
  noteListeners.add(fn);
  if (!noteUnlisten) {
    noteUnlisten = listen<NoteEvent>(NOTE_EVENT, (e) => {
      for (const l of noteListeners) l(e.payload);
    }).catch(() => null);
  }
  return () => {
    noteListeners.delete(fn);
    if (noteListeners.size === 0 && noteUnlisten) {
      const p = noteUnlisten;
      noteUnlisten = null;
      void p.then((u) => u?.());
    }
  };
}

/** Note colours (Maestri sticky palette). `undefined` = default surface. */
export const NOTE_COLORS: Array<{ id: string; css: string }> = [
  { id: 'yellow', css: '#f6d860' },
  { id: 'orange', css: '#f4a261' },
  { id: 'red', css: '#ef6f6c' },
  { id: 'pink', css: '#f28ab2' },
  { id: 'purple', css: '#b392f0' },
  { id: 'blue', css: '#6cb6ff' },
  { id: 'teal', css: '#5ccfb9' },
  { id: 'green', css: '#8fd16a' },
  { id: 'gray', css: '#a0a4ab' },
];

export function noteColorCss(id: string | undefined): string | undefined {
  if (!id) return undefined;
  return NOTE_COLORS.find((c) => c.id === id)?.css ?? (id.startsWith('#') ? id : undefined);
}

/** `.md`, `.markdown`, `.txt` dragged in become external notes (Maestri). */
export function isNoteFile(path: string): boolean {
  return /\.(md|markdown|txt)$/i.test(path);
}
