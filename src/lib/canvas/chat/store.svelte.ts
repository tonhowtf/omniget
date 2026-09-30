// Chat face state per terminal node: which face shows (terminal or chat),
// the selected thread, the loaded window of each thread and unread counts.
// `canvas://chat` events are folded in by `startChatEvents()` (register.ts).

import {
  chatHistory,
  chatMarkRead,
  chatSelectThread,
  onChat,
  onChatRemoved,
  onChatState,
  THREADS,
  type ChatEntry,
  type ChatEvent,
  type ChatRemovedEvent,
  type ChatStateEvent,
  type ThreadInfo,
} from './client';

/** A request to show one entry (palette chat search): ChatFace scrolls it to the centre and marks `term`. */
export interface ChatFocusRequest {
  thread: number;
  entryId?: string;
  term?: string;
  at: number;
}
import type { UnlistenFn } from '@tauri-apps/api/event';

export class NodeChat {
  readonly nodeId: string;
  workspaceId = $state('');
  thread = $state(0);
  /** Entries of the selected thread, oldest first. */
  entries = $state.raw<ChatEntry[]>([]);
  hasOlder = $state(false);
  threads = $state.raw<ThreadInfo[]>([]);
  unread = $state<Record<number, number>>({});
  /** A user message is waiting for the agent's reply. */
  turnOpen = $state(false);
  /** Thread of the open (or last) turn: the only one selectable while it runs. */
  turnThread = $state(0);
  /** The last turn was interrupted (Esc) or ended without a reply. */
  turnInterrupted = $state(false);
  /** Set by `chats.openAt`; ChatFace consumes it. */
  focusRequest = $state<ChatFocusRequest | null>(null);
  loading = $state(false);
  error = $state('');
  loaded = false;

  constructor(nodeId: string) {
    this.nodeId = nodeId;
  }

  /** Without `thread`: the thread the app has selected (same as the MCP's `activeThread`). */
  async load(workspaceId: string, thread?: number): Promise<void> {
    this.workspaceId = workspaceId;
    this.loading = true;
    try {
      const page = await chatHistory(workspaceId, this.nodeId, { thread });
      this.thread = page.thread;
      this.entries = page.entries;
      this.hasOlder = page.hasOlder;
      this.threads = page.threads;
      this.turnOpen = page.turnOpen;
      this.turnThread = page.turnThread ?? page.thread;
      this.turnInterrupted = !!page.turnInterrupted;
      this.setUnread(page.unread);
      this.unread = { ...this.unread, [page.thread]: 0 };
      this.loaded = true;
      this.error = '';
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    } finally {
      this.loading = false;
    }
  }

  async older(): Promise<void> {
    if (!this.hasOlder || !this.entries.length) return;
    const page = await chatHistory(this.workspaceId, this.nodeId, {
      thread: this.thread,
      before: this.entries[0].id,
    });
    this.entries = [...page.entries, ...this.entries];
    this.hasOlder = page.hasOlder;
  }

  /** Unread counts of every thread without reading anything (header badge). */
  async peek(workspaceId: string): Promise<void> {
    this.workspaceId = workspaceId;
    try {
      const page = await chatHistory(workspaceId, this.nodeId, { limit: 1, peek: true });
      this.setUnread(page.unread);
      this.turnOpen = page.turnOpen;
      this.turnThread = page.turnThread ?? page.thread;
      if (!this.loaded) this.thread = page.activeThread;
    } catch {
      // no transcript yet
    }
  }

  private setUnread(list: number[] | undefined): void {
    if (!Array.isArray(list)) return;
    const u: Record<number, number> = {};
    list.forEach((n, i) => {
      if (n) u[i] = n;
    });
    this.unread = u;
  }

  /**
   * Selects a thread in the app (persisted: the MCP and the next message see
   * it). Refused with CHAT_TURN_ACTIVE while the agent answers on another
   * thread; `viewOnly` then still shows the thread without selecting it.
   */
  async select(thread: number, viewOnly = false): Promise<void> {
    if (thread < 0 || thread >= THREADS.length) return;
    try {
      await chatSelectThread(this.workspaceId, this.nodeId, thread);
    } catch (e) {
      if (!viewOnly) {
        this.error = e instanceof Error ? e.message : String(e);
        return;
      }
    }
    this.error = '';
    await this.load(this.workspaceId, thread);
  }

  /** Server state (thread selected elsewhere, turn opened/closed, unread). */
  applyState(ev: ChatStateEvent, visible: boolean): void {
    this.turnOpen = ev.turnOpen;
    this.turnThread = ev.turnThread;
    this.turnInterrupted = ev.turnInterrupted;
    this.setUnread(ev.unread);
    if (visible && this.loaded && (ev.unread?.[this.thread] ?? 0) > 0) {
      this.unread = { ...this.unread, [this.thread]: 0 };
      void chatMarkRead(this.workspaceId, this.nodeId, this.thread).catch(() => {});
    }
    if (this.loaded && ev.activeThread !== this.thread) void this.load(this.workspaceId, ev.activeThread);
  }

  /** Folds an event entry in (dedupes by id, replaces an optimistic one by clientMessageId). */
  receive(e: ChatEntry, visible: boolean): void {
    const i = this.threads.findIndex((t) => t.index === e.thread);
    if (i >= 0 && e.kind !== 'divider') {
      const t = { ...this.threads[i], count: this.threads[i].count + 1, lastTs: e.ts };
      this.threads = [...this.threads.slice(0, i), t, ...this.threads.slice(i + 1)];
    }
    if (e.role === 'user' && e.kind === 'message') {
      this.turnOpen = true;
      this.turnThread = e.thread;
    }
    if ((e.role === 'agent' && e.kind !== 'progress') || e.kind === 'interrupted') this.turnOpen = false;
    // Unread counts come from the server (`canvas://chat-state`).
    if (e.thread !== this.thread || !this.loaded) return;
    if (this.entries.some((x) => x.id === e.id)) return;
    const opt = e.clientMessageId ? this.entries.findIndex((x) => x.clientMessageId === e.clientMessageId) : -1;
    if (opt >= 0) {
      this.entries = [...this.entries.slice(0, opt), e, ...this.entries.slice(opt + 1)];
    } else {
      this.entries = [...this.entries, e];
    }
    void visible;
  }

  /** Entries left the transcript (Delete Message / Clear Chat History). */
  drop(ev: ChatRemovedEvent): void {
    if (ev.cleared) {
      if (ev.thread == null) this.unread = {};
      else this.unread = { ...this.unread, [ev.thread]: 0 };
      if (ev.thread == null || ev.thread === this.thread) this.turnOpen = false;
    }
    const gone = new Set(ev.removed);
    this.entries = this.entries.filter((e) => !gone.has(e.id));
    // Thread counts come from the server.
    if (this.loaded) void this.load(this.workspaceId, this.thread);
  }

  totalUnread(): number {
    return Object.values(this.unread).reduce((a, b) => a + b, 0);
  }
}

class ChatStore {
  private nodes = new Map<string, NodeChat>();
  /** Node ids showing the chat face. */
  faces = $state<Record<string, boolean>>({});

  of(nodeId: string): NodeChat {
    let c = this.nodes.get(nodeId);
    if (!c) {
      c = new NodeChat(nodeId);
      this.nodes.set(nodeId, c);
    }
    return c;
  }

  isChat(nodeId: string): boolean {
    return !!this.faces[nodeId];
  }

  setFace(nodeId: string, chat: boolean): void {
    this.faces = { ...this.faces, [nodeId]: chat };
    // Showing the chat reads the thread on screen.
    const c = this.nodes.get(nodeId);
    if (chat && c?.loaded && (c.unread[c.thread] ?? 0) > 0) {
      c.unread = { ...c.unread, [c.thread]: 0 };
      void chatMarkRead(c.workspaceId, nodeId, c.thread).catch(() => {});
    }
  }

  /**
   * Palette chat search result: shows the chat face of `nodeId` on `thread`
   * (selected, or only shown while a turn runs elsewhere) and asks ChatFace
   * to scroll `entryId` to the centre with `term` highlighted.
   */
  async openAt(nodeId: string, thread: number, entryId?: string, term?: string, workspaceId?: string): Promise<void> {
    const c = this.of(nodeId);
    if (workspaceId) c.workspaceId = workspaceId;
    c.focusRequest = { thread, entryId, term, at: Date.now() };
    this.setFace(nodeId, true);
    if (c.workspaceId) await c.select(thread, true);
  }

  /** ⌘J: flips the face of a node. */
  toggle(nodeId: string): boolean {
    const next = !this.faces[nodeId];
    this.setFace(nodeId, next);
    return next;
  }

  handle(ev: ChatEvent): void {
    // `of`, not `get`: a turn opened by the MCP or an agent before this
    // node's face ever loaded must still be known (Esc interrupts it).
    const c = this.of(ev.nodeId);
    if (!c.workspaceId) c.workspaceId = ev.workspaceId;
    c.receive(ev.message, this.isChat(ev.nodeId));
  }

  handleState(ev: ChatStateEvent): void {
    const c = this.of(ev.nodeId);
    if (!c.workspaceId) c.workspaceId = ev.workspaceId;
    c.applyState(ev, this.isChat(ev.nodeId));
  }

  handleRemoved(ev: ChatRemovedEvent): void {
    this.nodes.get(ev.nodeId)?.drop(ev);
  }
}

export const chats = new ChatStore();

let unlisten: Promise<UnlistenFn> | null = null;
let unlistenRemoved: Promise<UnlistenFn> | null = null;
let unlistenState: Promise<UnlistenFn> | null = null;
let users = 0;

/** Starts the single `canvas://chat` listener; returns the stop function. */
export function startChatEvents(): () => void {
  users++;
  if (!unlisten) unlisten = onChat((e) => chats.handle(e)).catch(() => () => {});
  if (!unlistenRemoved) unlistenRemoved = onChatRemoved((e) => chats.handleRemoved(e)).catch(() => () => {});
  if (!unlistenState) unlistenState = onChatState((e) => chats.handleState(e)).catch(() => () => {});
  return () => {
    users--;
    if (users <= 0 && unlisten) {
      const u = unlisten;
      unlisten = null;
      users = 0;
      void u.then((f) => f());
      const r = unlistenRemoved;
      unlistenRemoved = null;
      if (r) void r.then((f) => f());
      const st = unlistenState;
      unlistenState = null;
      if (st) void st.then((f) => f());
    }
  };
}
