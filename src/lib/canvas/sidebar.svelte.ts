// Workspace sidebar helpers (study 100 §A): sidebar order with groups and
// folders, fixed jump numbers, the ⌘⌘-number jump mode, the mini sidebar
// toggle, custom icons (emoji or image file) and "Open in IDE".

import { invoke } from '@tauri-apps/api/core';
import type { WorkspaceSummary } from './palette/workspaces.svelte';

/** Runtime and extra fields the backend adds to a workspace summary. */
export interface WorkspaceExtra {
  ide?: string;
  jumpNumber?: number | null;
  syncAgentFiles?: boolean;
  loaded?: boolean;
  iconRevision?: string;
  agentFiles?: { synced: boolean; divergent: string[]; files: Array<{ name: string; exists: boolean; lines?: number }> } | null;
  terminals?: Array<{ nodeId: string; floorId: string; title?: string; terminalId?: string; agent?: string; running: boolean }>;
}

export type Ws = WorkspaceSummary & WorkspaceExtra;

export const asWs = (w: WorkspaceSummary): Ws => w as Ws;

export interface WsSection {
  /** Divider label (Maestri "Group"); '' for workspaces outside any group. */
  group: string;
  /** Items in sidebar order: plain rows and folders (Maestri "Folder"). */
  items: Array<{ kind: 'ws'; ws: Ws } | { kind: 'folder'; name: string; key: string; list: Ws[] }>;
}

/**
 * Sidebar layout: workspaces without a group first, then one section per
 * group in the order its first member appears; inside, folders sit where
 * their first member is. The flat order (what ⌘↑/⌘↓ and ⌃N follow) is the
 * visual order.
 */
export function layoutWorkspaces(list: WorkspaceSummary[]): { sections: WsSection[]; flat: Ws[] } {
  const groups = new Map<string, Ws[]>();
  for (const w of list as Ws[]) {
    const g = w.groupName?.trim() ?? '';
    if (!groups.has(g)) groups.set(g, []);
    groups.get(g)!.push(w);
  }
  const order = [...groups.keys()].sort((a, b) => (a === '' ? -1 : b === '' ? 1 : 0));
  const sections: WsSection[] = [];
  const flat: Ws[] = [];
  for (const g of order) {
    const members = groups.get(g)!;
    const items: WsSection['items'] = [];
    const folders = new Map<string, { kind: 'folder'; name: string; key: string; list: Ws[] }>();
    for (const w of members) {
      const f = w.folderName?.trim();
      if (!f) {
        items.push({ kind: 'ws', ws: w });
        continue;
      }
      let folder = folders.get(f);
      if (!folder) {
        folder = { kind: 'folder', name: f, key: `${g}\u0000${f}`, list: [] };
        folders.set(f, folder);
        items.push(folder);
      }
      folder.list.push(w);
    }
    for (const it of items) {
      if (it.kind === 'ws') flat.push(it.ws);
      else flat.push(...it.list);
    }
    sections.push({ group: g, items });
  }
  return { sections, flat };
}

/**
 * Jump number → workspace: fixed `jumpNumber`s first, then the others by
 * sidebar position on the numbers still free (Maestri "number" setting).
 */
export function jumpTargets(flat: Ws[]): Map<number, Ws> {
  const out = new Map<number, Ws>();
  for (const w of flat) {
    const n = w.jumpNumber;
    if (typeof n === 'number' && n >= 1 && n <= 9 && !out.has(n)) out.set(n, w);
  }
  const fixed = new Set([...out.values()].map((w) => w.id));
  flat.forEach((w, i) => {
    if (fixed.has(w.id) || i >= 9) return;
    if (!out.has(i + 1)) out.set(i + 1, w);
  });
  return out;
}

export function numberOf(targets: Map<number, Ws>, id: string): number | undefined {
  for (const [n, w] of targets) if (w.id === id) return n;
  return undefined;
}

// ---- icons -----------------------------------------------------------

export type WsIcon = { kind: 'text'; text: string } | { kind: 'image'; revision: string };

const EMOJI = /\p{Extended_Pictographic}/u;

/** How to draw a workspace icon: letters, an emoji, or an image file. */
export function iconOf(w: Ws): WsIcon {
  const raw = (w.icon ?? '').trim();
  if (raw.startsWith('file:') || /^\/.+\.(png|jpe?g|gif|webp|svg|ico)$/i.test(raw)) {
    return { kind: 'image', revision: w.iconRevision ?? '' };
  }
  const icon = raw.startsWith('emoji:') ? raw.slice(6).trim() : raw;
  if (icon && EMOJI.test(icon)) return { kind: 'text', text: icon };
  if (icon && icon.length <= 4) return { kind: 'text', text: icon.toUpperCase() };
  return { kind: 'text', text: (w.name.trim().charAt(0) || '?').toUpperCase() };
}

class IconCache {
  urls = $state<Record<string, string>>({});
  private loading = new Set<string>();

  /** Data URL of an image icon, loaded once per revision. */
  url(w: Ws): string | undefined {
    const key = `${w.id}@${w.iconRevision ?? ''}`;
    const hit = this.urls[key];
    if (hit !== undefined) return hit || undefined;
    if (!this.loading.has(key)) {
      this.loading.add(key);
      invoke<{ dataUrl: string } | null>('canvas_workspace_icon', { workspaceId: w.id })
        .then((r) => (this.urls = { ...this.urls, [key]: r?.dataUrl ?? '' }))
        .catch(() => (this.urls = { ...this.urls, [key]: '' }));
    }
    return undefined;
  }
}

export const wsIcons = new IconCache();

// ---- mini sidebar, jump mode --------------------------------------------

const MINI_KEY = 'omniget.canvas.miniSidebar';

class SidebarUi {
  /** Closed sidebar shows the mini rail of icons (View → Toggle Mini Sidebar). */
  mini = $state(read(MINI_KEY) !== '0');
  /** ⌘ tapped twice: the next digit jumps and the icons show their numbers. */
  jumpArmed = $state(false);
  /** Last sidebar announcement for screen readers (Alt+arrows reorder). */
  announce = $state('');

  toggleMini(): void {
    this.mini = !this.mini;
    try {
      localStorage.setItem(MINI_KEY, this.mini ? '1' : '0');
    } catch {
      // Private mode.
    }
  }
}

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

export const sidebarUi = new SidebarUi();

// ---- IDE -------------------------------------------------------------------

export const IDE_LABELS: Record<string, string> = {
  vscode: 'VS Code',
  'vscode-insiders': 'VS Code Insiders',
  cursor: 'Cursor',
  windsurf: 'Windsurf',
  zed: 'Zed',
  idea: 'IntelliJ IDEA',
  webstorm: 'WebStorm',
  pycharm: 'PyCharm',
  goland: 'GoLand',
  rustrover: 'RustRover',
  xcode: 'Xcode',
  sublime: 'Sublime Text',
};

/** Opens the workspace folder in its editor (remembered per workspace). */
export async function openInIde(workspaceId: string, ide?: string): Promise<{ ide: string; path: string }> {
  return invoke('canvas_workspace_open_ide', { workspaceId, ide: ide ?? null });
}

/** Editors found on this machine. */
export async function installedIdes(): Promise<string[]> {
  try {
    return await invoke<string[]>('canvas_ides');
  } catch {
    return [];
  }
}

/** Status of CLAUDE.md / AGENTS.md / GEMINI.md, optionally making one win. */
export async function agentFiles(
  workspaceId: string,
  resolve?: string,
  confirm = false,
): Promise<{ status: WorkspaceExtra['agentFiles']; text: string | null; resolve?: { changes: Array<{ file: string; linesLost: number; linesAdded: number; lost: string[] }> } | null }> {
  return invoke('canvas_workspace_agent_files', { workspaceId, resolve: resolve ?? null, confirm });
}
