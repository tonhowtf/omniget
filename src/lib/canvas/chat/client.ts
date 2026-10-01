import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export const THREADS = [
  { key: 'blue', color: '#3B82F6' },
  { key: 'purple', color: '#8B5CF6' },
  { key: 'pink', color: '#EC4899' },
  { key: 'red', color: '#EF4444' },
  { key: 'orange', color: '#F97316' },
  { key: 'yellow', color: '#EAB308' },
  { key: 'green', color: '#22C55E' },
] as const;

type ThreadKey = (typeof THREADS)[number]['key'];

export interface ChatAttachment {
  id: string;
  filename: string;
  path: string;
  isImage: boolean;
  contentType?: string;
  size: number;
}

export interface ChatEntry {
  id: string;
  thread: number;
  role: 'user' | 'agent';
  kind: 'message' | 'progress' | 'final' | 'divider' | 'interrupted';
  ts: string;
  text: string;
  attachments?: ChatAttachment[];
  authorName?: string;
  agentType?: string;
  clientMessageId?: string;
}

export interface ThreadInfo {
  index: number;
  key: ThreadKey;
  color: string;
  count: number;
  lastTs?: string;
}

interface HistoryPage {
  nodeId: string;
  thread: number;
  entries: ChatEntry[];
  hasOlder: boolean;
  threads: ThreadInfo[];
  activeThread: number;
  turnOpen: boolean;
  turnThread: number;
  turnInterrupted: boolean;
  unread: number[];
  unreadCount: number;
}

export interface ChatStateEvent {
  workspaceId: string;
  nodeId: string;
  activeThread: number;
  turnThread: number;
  turnOpen: boolean;
  turnInterrupted: boolean;
  unread: number[];
  unreadCount: number;
}

const CHAT_STATE_EVENT = 'canvas://chat-state';

export function onChatState(cb: (e: ChatStateEvent) => void): Promise<UnlistenFn> {
  return listen<ChatStateEvent>(CHAT_STATE_EVENT, (e) => cb(e.payload));
}

export const chatSelectThread = (workspaceId: string, nodeId: string, thread: number) =>
  invoke<unknown>('canvas_chat_select_thread', { workspaceId, nodeId, thread });

export const chatMarkRead = (workspaceId: string, nodeId: string, thread?: number) =>
  invoke<unknown>('canvas_chat_mark_read', { workspaceId, nodeId, thread: thread ?? null });

export const chatInterrupt = (workspaceId: string, nodeId: string) =>
  invoke<ChatEntry | null>('canvas_chat_interrupt', { workspaceId, nodeId });

export interface ChatSearchHit {
  workspaceId: string;
  floorId: string | null;
  nodeId: string;
  thread: number;
  entryId: string;
  role: 'user' | 'agent';
  ts: string;
  snippet: string;
}

export const chatSearch = (query: string, limit?: number) =>
  invoke<ChatSearchHit[]>('canvas_chat_search', { query, limit: limit ?? null });

export interface ChatEvent {
  workspaceId: string;
  nodeId: string;
  thread: number;
  message: ChatEntry;
}

const CHAT_EVENT = 'canvas://chat';

export const chatHistory = (
  workspaceId: string,
  nodeId: string,
  opts: { thread?: number; before?: string; limit?: number; peek?: boolean } = {},
) =>
  invoke<HistoryPage>('canvas_chat_history', {
    workspaceId,
    nodeId,
    thread: opts.thread ?? null,
    before: opts.before ?? null,
    limit: opts.limit ?? null,
    peek: opts.peek ?? null,
  });

export const chatSend = (
  workspaceId: string,
  nodeId: string,
  thread: number,
  text: string,
  attachments: string[] = [],
  clientMessageId?: string,
) =>
  invoke<ChatEntry>('canvas_chat_send', {
    workspaceId,
    nodeId,
    thread,
    text,
    attachments,
    clientMessageId: clientMessageId ?? null,
  });

export const chatClear = (workspaceId: string, nodeId: string, thread: number) =>
  invoke<ChatEntry>('canvas_chat_clear', { workspaceId, nodeId, thread });

interface ChatRemoved {
  removed: string[];
  attachmentsRemoved: number;
  remaining: number;
}

export interface ChatRemovedEvent {
  workspaceId: string;
  nodeId: string;
  removed: string[];
  thread: number | null;
  cleared: boolean;
}

const CHAT_REMOVED_EVENT = 'canvas://chat-removed';

export const chatDeleteEntry = (workspaceId: string, nodeId: string, entryId: string, thread?: number) =>
  invoke<ChatRemoved>('canvas_chat_delete_entry', { workspaceId, nodeId, entryId, thread: thread ?? null });

export const chatWipe = (workspaceId: string, nodeId: string, thread?: number) =>
  invoke<ChatRemoved>('canvas_chat_wipe', { workspaceId, nodeId, thread: thread ?? null });

export function onChatRemoved(cb: (e: ChatRemovedEvent) => void): Promise<UnlistenFn> {
  return listen<ChatRemovedEvent>(CHAT_REMOVED_EVENT, (e) => cb(e.payload));
}

export const attachmentSave = (workspaceId: string, name: string, dataBase64: string) =>
  invoke<string>('canvas_attachment_save', { workspaceId, name, dataBase64 });

export function onChat(cb: (e: ChatEvent) => void): Promise<UnlistenFn> {
  return listen<ChatEvent>(CHAT_EVENT, (e) => cb(e.payload));
}

export function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onerror = () => reject(r.error);
    r.onload = () => {
      const s = String(r.result ?? '');
      const i = s.indexOf('base64,');
      resolve(i >= 0 ? s.slice(i + 7) : s);
    };
    r.readAsDataURL(blob);
  });
}
