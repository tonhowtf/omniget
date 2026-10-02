import { invoke } from '@tauri-apps/api/core';

export const ACTIVITY_EVENT = 'canvas://activity';
const KEEP = 500;

export type ActorKind = 'person' | 'client' | 'agent' | 'system';

export interface ActivityEntry {
  id: number;
  seq: number;
  rev: number;
  at: number;
  workspaceId: string;
  floorId: string;
  origin: string;
  actor: string;
  actorKind: ActorKind;
  verb: string;
  target: string | null;
  targetKind: string | null;
  count: number;
  summary: string;
  nodeIds: string[];
  undo: boolean;
  via?: string;
}

export type ActivityTab = 'activity' | 'mcp';

export const VERB_DEFAULTS: Record<string, string> = {
  created: 'created',
  deleted: 'deleted',
  moved: 'moved',
  resized: 'resized',
  renamed: 'renamed',
  edited: 'edited',
  wrote: 'wrote in',
  corded: 'connected',
  uncorded: 'disconnected',
  changed: 'changed',
  drew: 'drew',
  erased: 'erased',
  grouped: 'grouped',
  quoted: 'got a price for',
  submitted: 'started',
  generated: 'finished',
  failed: 'hit a failure in',
  uncertain: 'lost track of',
};

export function initials(name: string): string {
  const parts = name.split(/[\s·_:-]+/).filter(Boolean);
  const s = parts.length > 1 ? parts[0][0] + parts[1][0] : name.slice(0, 2);
  return s.toUpperCase();
}

const PALETTE = ['#7c5cff', '#ff6b6b', '#2fb4ff', '#21c08b', '#ffb020', '#ff5fa2', '#00b8a9', '#f2711c'];

export function baseOrigin(origin: string): string {
  return origin.endsWith(':undo') ? origin.slice(0, -5) : origin;
}

export function isAi(e: Pick<ActivityEntry, 'actorKind'>): boolean {
  return e.actorKind === 'client' || e.actorKind === 'agent';
}

export function actorColor(origin: string): string {
  const key = baseOrigin(origin);
  let h = 0;
  for (const c of key) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return PALETTE[h % PALETTE.length];
}

const PRESENCE_KEY = 'omniget.canvas.aiPresence';

function readPresence(): boolean {
  try {
    return localStorage.getItem(PRESENCE_KEY) !== '0';
  } catch {
    return true;
  }
}

export function reducedMotion(): boolean {
  return typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
}

class AiActivity {
  open = $state(false);
  tab = $state<ActivityTab>('activity');
  mcpNodeId = $state<string | null>(null);
  follow = $state(false);
  followOrigin = $state<string | null>(null);
  aiOnly = $state(false);
  #presence = $state(readPresence());
  entries = $state.raw<ActivityEntry[]>([]);

  get presence(): boolean {
    return this.#presence;
  }

  set presence(v: boolean) {
    this.#presence = v;
    try {
      localStorage.setItem(PRESENCE_KEY, v ? '1' : '0');
    } catch {
    }
  }
  private workspaceId = '';
  private loading = 0;

  show(tab: ActivityTab = 'activity', nodeId: string | null = null): void {
    this.tab = tab;
    this.mcpNodeId = tab === 'mcp' ? nodeId : this.mcpNodeId;
    this.open = true;
  }

  toggle(tab: ActivityTab = 'activity'): void {
    if (this.open && this.tab === tab) this.open = false;
    else this.show(tab);
  }

  toggleFollow(): void {
    this.follow = !this.follow;
  }

  async load(workspaceId: string): Promise<void> {
    if (!workspaceId) return;
    if (workspaceId !== this.workspaceId) {
      this.workspaceId = workspaceId;
      this.entries = [];
    }
    const ticket = ++this.loading;
    try {
      const list = await invoke<ActivityEntry[]>('canvas_activity', { workspaceId, floorId: null, since: null, limit: KEEP });
      if (ticket !== this.loading || workspaceId !== this.workspaceId) return;
      const known = new Map(this.entries.map((e) => [e.id, e]));
      for (const e of list) known.set(e.id, e);
      this.entries = [...known.values()].sort((a, b) => a.id - b.id).slice(-KEEP);
    } catch {
      return;
    }
  }

  push(e: ActivityEntry): boolean {
    if (!e || e.workspaceId !== this.workspaceId) return false;
    const i = this.entries.findIndex((x) => x.id === e.id);
    if (i >= 0) {
      const next = this.entries.slice();
      next[i] = e;
      this.entries = next;
    } else {
      this.entries = [...this.entries, e].slice(-KEEP);
    }
    return true;
  }

  recent(now: number, windowMs: number): Array<{ origin: string; name: string; last: ActivityEntry }> {
    const byActor = new Map<string, ActivityEntry>();
    for (let i = this.entries.length - 1; i >= 0; i--) {
      const e = this.entries[i];
      if (now - e.at > windowMs) continue;
      if (!isAi(e)) continue;
      const key = baseOrigin(e.origin);
      if (!byActor.has(key)) byActor.set(key, e);
    }
    return [...byActor].map(([origin, last]) => ({ origin, name: last.actor, last })).sort((a, b) => b.last.at - a.last.at);
  }

  actors(): Array<{ origin: string; name: string }> {
    const seen = new Map<string, string>();
    for (const e of this.entries) if (isAi(e)) seen.set(baseOrigin(e.origin), e.actor);
    return [...seen].map(([origin, name]) => ({ origin, name }));
  }
}

export const aiActivity = new AiActivity();
