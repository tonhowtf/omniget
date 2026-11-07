import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { CanvasNode } from '../model/types';

export interface CanvasAccount {
  id: string;
  label: string;
  cli: 'claude' | 'codex' | string;
  disabled: boolean;
}

export interface PortalSession {
  id: string;
  label: string;
  color?: string;
  createdAt: number;
}

export interface SessionUser {
  workspaceId: string;
  workspaceName?: string;
  floorId: string;
  nodeId: string;
  title?: string | null;
}

interface Overview {
  accounts: CanvasAccount[];
  sessions: PortalSession[];
  namedSessionsSupported: boolean;
  running: Record<string, string | null>;
}

type BadgeKind = 'environment' | 'normal' | 'restart' | 'invalid';

export interface AccountBadge {
  kind: BadgeKind;
  id: string | null;
  label: string;
  color: string;
  reason?: 'missing' | 'disabled' | 'cli';
}

const PALETTE = ['#4f8cff', '#f2994a', '#27ae60', '#bb6bd9', '#eb5757', '#2bb5b5', '#d4a72c', '#e0609b'];

export function colorFor(id: string): string {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return PALETTE[h % PALETTE.length];
}

export const ACCOUNT_AGENTS = new Set(['claude', 'codex', 'shell']);

export function cliOfAgent(agent: string): string | null {
  return agent === 'claude' || agent === 'codex' ? agent : null;
}

function accountOf(node: CanvasNode): string | null {
  const v = node.data?.account;
  return typeof v === 'string' && v.trim() ? v.trim() : null;
}

export function sessionOf(node: CanvasNode): string | null {
  const v = node.data?.session;
  return typeof v === 'string' && v.trim() ? v.trim() : null;
}

class AccountsStore {
  accounts = $state.raw<CanvasAccount[]>([]);
  sessions = $state.raw<PortalSession[]>([]);
  supported = $state(true);
  running = $state.raw<Record<string, string | null>>({});
  loaded = $state(false);
  private inflight: Promise<void> | null = null;
  private listening = false;

  load(force = false): Promise<void> {
    this.listen();
    if (this.loaded && !force) return Promise.resolve();
    if (this.inflight) return this.inflight;
    this.inflight = invoke<Overview>('canvas_accounts_overview')
      .then((o) => {
        this.accounts = o.accounts ?? [];
        this.sessions = o.sessions ?? [];
        this.supported = o.namedSessionsSupported !== false;
        this.running = o.running ?? {};
        this.loaded = true;
      })
      .catch(() => {})
      .finally(() => {
        this.inflight = null;
      });
    return this.inflight;
  }

  private listen() {
    if (this.listening || typeof window === 'undefined') return;
    this.listening = true;
    void listen<{ terminalId: string; account: string | null }>('canvas://accounts', (e) => {
      this.running = { ...this.running, [e.payload.terminalId]: e.payload.account ?? null };
    }).catch(() => {});
    void listen<{ sessions: PortalSession[] }>('canvas://portal-sessions', (e) => {
      this.sessions = e.payload.sessions ?? [];
    }).catch(() => {});
    window.addEventListener('focus', () => void this.load(true));
  }

  forCli(cli: string | null): CanvasAccount[] {
    return cli ? this.accounts.filter((a) => a.cli === cli) : this.accounts;
  }

  session(id: string | null | undefined): PortalSession | undefined {
    return id ? this.sessions.find((s) => s.id === id) : undefined;
  }

  terminalBadge(node: CanvasNode, terminalId: string, alive: boolean): AccountBadge {
    const id = accountOf(node);
    const agent = typeof node.data?.agent === 'string' ? (node.data.agent as string) : 'shell';
    const ran = alive && terminalId in this.running ? this.running[terminalId] : undefined;
    const restart = ran !== undefined && (ran ?? null) !== id;
    if (!id) {
      return { kind: restart ? 'restart' : 'environment', id: null, label: '', color: 'var(--text-dim, #98989d)' };
    }
    const acc = this.accounts.find((a) => a.id === id);
    const color = colorFor(id);
    if (!acc) return { kind: this.loaded ? 'invalid' : 'normal', id, label: id, color, reason: 'missing' };
    if (acc.disabled) return { kind: 'invalid', id, label: acc.label, color, reason: 'disabled' };
    const cli = cliOfAgent(agent);
    if ((cli && acc.cli !== cli) || !ACCOUNT_AGENTS.has(agent)) {
      return { kind: 'invalid', id, label: acc.label, color, reason: 'cli' };
    }
    return { kind: restart ? 'restart' : 'normal', id, label: acc.label, color };
  }

  sessionBadge(node: CanvasNode): AccountBadge | null {
    const id = sessionOf(node);
    if (!id) return null;
    const s = this.session(id);
    const color = s?.color || colorFor(id);
    if (!s) return { kind: this.loaded ? 'invalid' : 'normal', id, label: id, color, reason: 'missing' };
    return { kind: 'normal', id, label: s.label, color };
  }

  async createSession(label: string, color?: string): Promise<PortalSession> {
    const s = await invoke<PortalSession>('canvas_portal_session_create', { label, color: color ?? null });
    this.sessions = [...this.sessions.filter((x) => x.id !== s.id), s];
    return s;
  }

  async updateSession(id: string, patch: { label?: string; color?: string }): Promise<PortalSession> {
    const s = await invoke<PortalSession>('canvas_portal_session_update', {
      id,
      label: patch.label ?? null,
      color: patch.color ?? null,
    });
    this.sessions = this.sessions.map((x) => (x.id === id ? s : x));
    return s;
  }

  usage(id: string): Promise<SessionUser[]> {
    return invoke<SessionUser[]>('canvas_portal_session_usage', { id });
  }

  async deleteSession(id: string): Promise<SessionUser[]> {
    const r = await invoke<{ portals: SessionUser[] }>('canvas_portal_session_delete', { id });
    this.sessions = this.sessions.filter((x) => x.id !== id);
    return r.portals ?? [];
  }
}

export const canvasAccounts = new AccountsStore();
