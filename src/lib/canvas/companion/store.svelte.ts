// Live list of Omni's summaries for the canvas panel: loaded once from
// `canvas_companion_history`, then fed by `canvas://companion`. Reference
// counted so several panels share one listener.

import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
  EVENT_COMPANION,
  EVENT_COMPANION_DISMISS,
  asEntry,
  history,
  type CompanionEntry,
} from './api';

const CAP = 60;

class CompanionFeed {
  entries = $state.raw<CompanionEntry[]>([]);
  private unlisten: UnlistenFn[] = [];
  private refs = 0;

  async start(): Promise<void> {
    this.refs++;
    if (this.refs > 1) return;
    try {
      this.unlisten.push(
        await listen(EVENT_COMPANION, (e) => {
          const entry = asEntry(e.payload);
          if (entry) this.push(entry);
        }),
      );
      this.unlisten.push(
        await listen<{ id?: string }>(EVENT_COMPANION_DISMISS, (e) => {
          if (e.payload?.id) this.drop(e.payload.id);
        }),
      );
      const past = await history(undefined, CAP);
      const known = new Set(this.entries.map((x) => x.id));
      // History is newest first; the live list too.
      this.entries = [...this.entries, ...past.filter((x) => !known.has(x.id))].slice(0, CAP);
    } catch {
      // Outside Tauri: the list stays empty.
    }
  }

  stop(): void {
    this.refs = Math.max(0, this.refs - 1);
    if (this.refs > 0) return;
    for (const u of this.unlisten) u();
    this.unlisten = [];
  }

  push(entry: CompanionEntry): void {
    this.entries = [entry, ...this.entries.filter((x) => x.id !== entry.id)].slice(0, CAP);
  }

  drop(id: string): void {
    if (!this.entries.some((x) => x.id === id)) return;
    this.entries = this.entries.filter((x) => x.id !== id);
  }

  forWorkspace(workspaceId: string | undefined): CompanionEntry[] {
    return workspaceId ? this.entries.filter((x) => x.workspaceId === workspaceId) : this.entries;
  }
}

export const companionFeed = new CompanionFeed();
