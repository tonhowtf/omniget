// Selection and focus. Selection is semantic (Maestri): a selected terminal
// leaves inter-agent monitoring because the user took over, so every change is
// announced as `canvas://selection` for the Rust side to follow.
// Two focus states (02 §3.3): "canvas" (focused = null) and "inside a node".

import { emit } from '@tauri-apps/api/event';

export const SELECTION_EVENT = 'canvas://selection';

export class Selection {
  /** Node and drawing ids (UUIDs never collide across kinds). */
  ids = $state.raw<ReadonlySet<string>>(new Set());
  /** Node that owns keyboard and pointer right now, or null = the canvas. */
  focused = $state<string | null>(null);
  hovered = $state<string | null>(null);
  /** Id of the text/shape being edited in place. */
  editing = $state<string | null>(null);

  private scope: { workspaceId: string; floorId: string } = { workspaceId: '', floorId: '' };

  setScope(workspaceId: string, floorId: string): void {
    this.scope = { workspaceId, floorId };
  }

  private announce(): void {
    void emit(SELECTION_EVENT, { ...this.scope, ids: Array.from(this.ids) }).catch(() => {
      // Outside Tauri: nothing listens.
    });
  }

  has(id: string): boolean {
    return this.ids.has(id);
  }

  get size(): number {
    return this.ids.size;
  }

  list(): string[] {
    return Array.from(this.ids);
  }

  set(ids: Iterable<string>): void {
    const next = new Set(ids);
    if (next.size === this.ids.size && Array.from(next).every((id) => this.ids.has(id))) return;
    this.ids = next;
    this.announce();
  }

  add(ids: Iterable<string>): void {
    this.set([...this.ids, ...ids]);
  }

  toggle(id: string): void {
    const next = new Set(this.ids);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    this.set(next);
  }

  clear(): void {
    this.set([]);
    this.editing = null;
  }

  /** Drops ids that no longer exist in the document. */
  prune(exists: (id: string) => boolean): void {
    const next = Array.from(this.ids).filter(exists);
    if (next.length !== this.ids.size) this.set(next);
    if (this.focused && !exists(this.focused)) this.focused = null;
    if (this.editing && !exists(this.editing)) this.editing = null;
  }

  focus(id: string | null): void {
    this.focused = id;
  }
}
