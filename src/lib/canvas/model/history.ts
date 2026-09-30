// Undo/redo per origin (02-motor-do-canvas §4.6). Every transaction keeps its
// forward ops and their inverse. ⌘Z undoes only the user's own entries; when an
// agent touched the same record afterwards, that record is skipped (never
// overwrite the agent) and the caller is told so it can warn.

import { recordKey } from './ops';
import type { Op, Origin } from './types';

export const HISTORY_LIMIT = 200;

export interface HistoryEntry {
  seq: number;
  origin: Origin;
  forward: Op[];
  inverse: Op[];
  /** Selection before and after, restored with the entry (never an entry of its own). */
  selectionBefore?: string[];
  selectionAfter?: string[];
}

export interface UndoStep {
  /** Ops to apply now. */
  ops: Op[];
  /** Records left alone because an agent changed them later. */
  skipped: string[];
  selection?: string[];
}

export class History {
  private undoStack: HistoryEntry[] = [];
  private redoStack: HistoryEntry[] = [];
  private seq = 0;
  /** Last writer of each record: seq and origin. */
  private lastWrite = new Map<string, { seq: number; origin: Origin }>();
  /** Open mark: ops recorded while a gesture is in progress squash into one entry. */
  private mark: HistoryEntry | null = null;

  constructor(readonly limit = HISTORY_LIMIT) {}

  /** Records an applied transaction. Agent entries only feed the conflict map. */
  record(origin: Origin, forward: Op[], inverse: Op[], selectionBefore?: string[], selectionAfter?: string[]): void {
    if (forward.length === 0) return;
    const seq = ++this.seq;
    for (const op of forward) {
      if (op.op === 'set_camera') continue;
      this.lastWrite.set(recordKey(op), { seq, origin });
    }
    if (origin !== 'user') return;
    const fwd = forward.filter((o) => o.op !== 'set_camera');
    const inv = inverse.filter((o) => o.op !== 'set_camera');
    if (fwd.length === 0) return;
    this.redoStack = [];
    if (this.mark) {
      this.mark.forward.push(...fwd);
      // Inverse of (A then B) is (inv B then inv A).
      this.mark.inverse = [...inv, ...this.mark.inverse];
      this.mark.seq = seq;
      if (selectionAfter) this.mark.selectionAfter = selectionAfter;
      return;
    }
    this.push({ seq, origin, forward: fwd, inverse: inv, selectionBefore, selectionAfter });
  }

  private push(entry: HistoryEntry): void {
    this.undoStack.push(entry);
    if (this.undoStack.length > this.limit) this.undoStack.shift();
  }

  /** Opens a squash mark (pointerdown). Nested calls are ignored. */
  beginMark(selectionBefore?: string[]): void {
    if (this.mark) return;
    this.mark = { seq: this.seq, origin: 'user', forward: [], inverse: [], selectionBefore };
  }

  /** Closes the mark; an empty mark leaves no entry. */
  endMark(selectionAfter?: string[]): void {
    const m = this.mark;
    this.mark = null;
    if (!m || m.forward.length === 0) return;
    if (selectionAfter) m.selectionAfter = selectionAfter;
    this.push(m);
  }

  get marking(): boolean {
    return this.mark !== null;
  }

  canUndo(): boolean {
    return this.undoStack.length > 0;
  }

  canRedo(): boolean {
    return this.redoStack.length > 0;
  }

  private filterConflicts(entry: HistoryEntry, ops: Op[]): { ops: Op[]; skipped: string[] } {
    const skipped = new Set<string>();
    const out: Op[] = [];
    for (const op of ops) {
      const key = recordKey(op);
      const lw = this.lastWrite.get(key);
      if (lw && lw.origin !== 'user' && lw.seq > entry.seq) {
        skipped.add(key);
        continue;
      }
      out.push(op);
    }
    return { ops: out, skipped: Array.from(skipped) };
  }

  /** Pops the newest user entry; the caller applies `ops` without recording. */
  undo(): UndoStep | null {
    const entry = this.undoStack.pop();
    if (!entry) return null;
    const { ops, skipped } = this.filterConflicts(entry, entry.inverse);
    this.redoStack.push(entry);
    const seq = ++this.seq;
    for (const op of ops) this.lastWrite.set(recordKey(op), { seq, origin: 'user' });
    entry.seq = seq;
    return { ops, skipped, selection: entry.selectionBefore };
  }

  redo(): UndoStep | null {
    const entry = this.redoStack.pop();
    if (!entry) return null;
    const { ops, skipped } = this.filterConflicts(entry, entry.forward);
    const seq = ++this.seq;
    for (const op of ops) this.lastWrite.set(recordKey(op), { seq, origin: 'user' });
    entry.seq = seq;
    this.undoStack.push(entry);
    return { ops, skipped, selection: entry.selectionAfter };
  }

  clear(): void {
    this.undoStack = [];
    this.redoStack = [];
    this.lastWrite.clear();
    this.mark = null;
  }

  get sizes(): { undo: number; redo: number } {
    return { undo: this.undoStack.length, redo: this.redoStack.length };
  }
}
