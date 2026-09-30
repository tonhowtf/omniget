// Workspaces list shared by the sidebar, the palette and the file tree
// (contract-doc "Workspace"). Follows `canvas://workspaces`.

import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export const WORKSPACES_EVENT = 'canvas://workspaces';

export interface FloorMeta {
  id: string;
  name: string;
  color?: string;
  branch?: string;
  isGitIsolated?: boolean;
  path?: string;
  createdAt?: string;
}

export interface WorkspaceSummary {
  id: string;
  name: string;
  icon?: string;
  color?: string;
  workingDirectory?: string;
  activeFloorId: string;
  floors: FloorMeta[];
  isPinned?: boolean;
  groupName?: string;
  folderName?: string;
  sortIndex?: number;
  createdAt?: string;
  updatedAt?: string;
  nodeCount?: number;
  terminalCount?: number;
  /** Mirror of the workspace environment (`envs::save`); null/absent = this machine. */
  environment?: { kind: string; name?: string } | null;
  /** Lifted and docked nodes of this workspace. */
  panels?: { lifted?: string | null; docked?: Array<{ nodeId: string; edge: 'left' | 'right'; width: number }> } | null;
}

export interface WorkspaceInput {
  name: string;
  icon?: string;
  color?: string;
  workingDirectory?: string;
}

class WorkspacesState {
  list = $state.raw<WorkspaceSummary[]>([]);
  loaded = $state(false);
  error = $state<string | null>(null);
  private unlisten: Promise<UnlistenFn | null> | null = null;
  private users = 0;

  byId(id: string): WorkspaceSummary | undefined {
    return this.list.find((w) => w.id === id);
  }

  /** Starts following changes; call the returned function to stop. */
  use(): () => void {
    this.users++;
    if (!this.unlisten) {
      this.unlisten = listen<WorkspaceSummary[]>(WORKSPACES_EVENT, (e) => {
        if (Array.isArray(e.payload)) this.set(e.payload);
      }).catch(() => null);
    }
    if (!this.loaded) void this.refresh();
    return () => {
      this.users--;
      if (this.users <= 0 && this.unlisten) {
        const p = this.unlisten;
        this.unlisten = null;
        void p.then((u) => u?.());
      }
    };
  }

  private set(list: WorkspaceSummary[]): void {
    this.list = list.map((w) => ({ ...w, floors: Array.isArray(w.floors) ? w.floors : [] }));
    this.loaded = true;
  }

  async refresh(): Promise<void> {
    try {
      this.set(await invoke<WorkspaceSummary[]>('canvas_workspaces'));
      this.error = null;
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    }
  }

  async create(input: WorkspaceInput): Promise<WorkspaceSummary | null> {
    try {
      const ws = await invoke<WorkspaceSummary>('canvas_workspace_create', { input });
      await this.refresh();
      return this.byId(ws.id) ?? ws;
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
      return null;
    }
  }

  async update(workspaceId: string, patch: Partial<WorkspaceSummary>): Promise<void> {
    // Optimistic, so renames and reorders feel instant.
    this.list = this.list.map((w) => (w.id === workspaceId ? { ...w, ...patch } : w));
    try {
      await invoke('canvas_workspace_update', { workspaceId, patch });
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
      await this.refresh();
    }
  }

  async remove(workspaceId: string): Promise<boolean> {
    try {
      await invoke('canvas_workspace_delete', { workspaceId });
      this.list = this.list.filter((w) => w.id !== workspaceId);
      return true;
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
      return false;
    }
  }

  /** Writes a new order as `sortIndex` 0..n. */
  async reorder(ids: string[]): Promise<void> {
    const pos = new Map(ids.map((id, i) => [id, i]));
    this.list = this.list
      .map((w) => ({ ...w, sortIndex: pos.get(w.id) ?? w.sortIndex }))
      .sort((a, b) => (a.sortIndex ?? 1e9) - (b.sortIndex ?? 1e9));
    await Promise.all(
      ids.map((id, i) => invoke('canvas_workspace_update', { workspaceId: id, patch: { sortIndex: i } }).catch(() => {})),
    );
  }
}

export const workspaces = new WorkspacesState();
