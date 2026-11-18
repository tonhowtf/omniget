import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import * as api from "./api";
import type { WaDownloadDone, WaDownloadProgress, WaStatus } from "./api";

type Listener<T> = (payload: T) => void;

export const wa = $state<{
  status: WaStatus | null;
  loading: boolean;
  error: string | null;
  chatsVersion: number;
  lastMessageChat: string | null;
  messageVersion: number;
  scheduledVersion: number;
  rulesVersion: number;
  draftVersion: number;
  syncChats: number;
  syncMessages: number;
}>({
  status: null,
  loading: true,
  error: null,
  chatsVersion: 0,
  lastMessageChat: null,
  messageVersion: 0,
  scheduledVersion: 0,
  rulesVersion: 0,
  draftVersion: 0,
  syncChats: 0,
  syncMessages: 0,
});

const progressListeners = new Set<Listener<WaDownloadProgress>>();
const doneListeners = new Set<Listener<WaDownloadDone>>();
let unlisten: UnlistenFn[] = [];
let attached = 0;
let chatsTimer: ReturnType<typeof setTimeout> | null = null;

function bumpChats() {
  if (chatsTimer) return;
  chatsTimer = setTimeout(() => {
    chatsTimer = null;
    wa.chatsVersion += 1;
  }, 400);
}

export async function refreshStatus(): Promise<WaStatus | null> {
  try {
    wa.status = await api.status();
    wa.error = null;
  } catch (e) {
    wa.error = api.errText(e);
  } finally {
    wa.loading = false;
  }
  return wa.status;
}

export async function attach(): Promise<void> {
  attached += 1;
  if (attached > 1) return;
  unlisten = await Promise.all([
    listen<WaStatus>("phonechat-status", (e) => {
      wa.status = e.payload;
      wa.loading = false;
    }),
    listen("phonechat-chats-changed", () => bumpChats()),
    listen<{ chat: string }>("phonechat-message", (e) => {
      wa.lastMessageChat = e.payload.chat;
      wa.messageVersion += 1;
    }),
    listen("phonechat-scheduled-changed", () => {
      wa.scheduledVersion += 1;
    }),
    listen("phonechat-draft", () => {
      wa.draftVersion += 1;
    }),
    listen("phonechat-rules-changed", () => {
      wa.rulesVersion += 1;
    }),
    listen<{ progress: number | null; chats: number; messages: number }>("phonechat-sync", (e) => {
      wa.syncChats += e.payload.chats;
      wa.syncMessages += e.payload.messages;
    }),
    listen<WaDownloadProgress>("phonechat-download-progress", (e) => {
      for (const l of progressListeners) l(e.payload);
    }),
    listen<WaDownloadDone>("phonechat-download-done", (e) => {
      for (const l of doneListeners) l(e.payload);
    }),
  ]);
}

export function detach(): void {
  attached = Math.max(0, attached - 1);
  if (attached > 0) return;
  for (const u of unlisten) u();
  unlisten = [];
  if (chatsTimer) clearTimeout(chatsTimer);
  chatsTimer = null;
}

export function onDownloadProgress(l: Listener<WaDownloadProgress>): () => void {
  progressListeners.add(l);
  return () => progressListeners.delete(l);
}

export function onDownloadDone(l: Listener<WaDownloadDone>): () => void {
  doneListeners.add(l);
  return () => doneListeners.delete(l);
}
