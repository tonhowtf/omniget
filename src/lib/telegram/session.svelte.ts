import { get } from "svelte/store";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { t } from "$lib/i18n";
import { showToast } from "$lib/stores/toast-store.svelte";
import {
  tg,
  tg2,
  tg3,
  errText,
  isUnknownCommand,
  type BatchFileStatus,
  type ChatPageOffset,
  type ChatScanEvent,
  type RealtimeState,
  type ChatRef,
  type PlaybackEntry,
  type TelegramChat,
  type WatchRule,
  type WatchRunEvent,
} from "./api";

export function failText(e: unknown): string {
  return isUnknownCommand(e) ? (get(t)("telegram.common.not_ready") as string) : errText(e);
}

export const chatsState = $state<{
  chats: TelegramChat[];
  next: ChatPageOffset | null;
  loading: boolean;
  loaded: boolean;
  error: string | null;
}>({ chats: [], next: null, loading: false, loaded: false, error: null });

let sessionCheck: Promise<unknown> | null = null;

export function ensureSession(force = false): Promise<unknown> {
  if (force || !sessionCheck) {
    const p = tg.checkSession();
    sessionCheck = p;
    p.catch(() => {
      if (sessionCheck === p) sessionCheck = null;
    });
  }
  return sessionCheck as Promise<unknown>;
}

export async function loadChats(reset = false): Promise<void> {
  if (chatsState.loading) return;
  if (!reset && chatsState.loaded && !chatsState.next) return;
  chatsState.loading = true;
  chatsState.error = null;
  try {
    await ensureSession();
    const page = await tg.listChatsPage(reset ? null : chatsState.next, 100);
    const seen = new Set(reset ? [] : chatsState.chats.map((c) => `${c.chat_type}:${c.id}`));
    const fresh = page.chats.filter((c) => !seen.has(`${c.chat_type}:${c.id}`));
    chatsState.chats = reset ? page.chats : [...chatsState.chats, ...fresh];
    chatsState.next = page.next_offset;
    chatsState.loaded = true;
  } catch (e) {
    chatsState.error = errText(e);
  } finally {
    chatsState.loading = false;
  }
}

export function resetSession(): void {
  sessionCheck = null;
  chatsState.chats = [];
  chatsState.next = null;
  chatsState.loaded = false;
  chatsState.error = null;
  thumbs.clear();
  for (const k of Object.keys(batches)) delete batches[Number(k)];
  for (const k of Object.keys(uploads)) delete uploads[Number(k)];
  for (const k of Object.keys(scans)) delete scans[k];
  recentState.items = [];
  recentState.loaded = false;
}

export function chatRef(id: number, chatType: string, title?: string | null): TelegramChat {
  const hit = chatsState.chats.find((c) => c.id === id && c.chat_type === chatType);
  if (hit) return hit;
  return {
    id,
    chat_type: chatType,
    title: title || String(id),
    peer_hash: 0,
    unread_count: 0,
    is_muted: false,
    is_pinned: false,
    is_verified: false,
    is_online: false,
  };
}

export const recentState = $state<{ items: PlaybackEntry[]; loaded: boolean; unavailable: boolean }>({
  items: [],
  loaded: false,
  unavailable: false,
});

export async function loadRecent(): Promise<void> {
  try {
    await ensureSession();
    recentState.items = await tg2.playbackRecent(12);
    recentState.unavailable = false;
  } catch (e) {
    recentState.items = [];
    recentState.unavailable = isUnknownCommand(e);
  } finally {
    recentState.loaded = true;
  }
}

export function notePlayback(chat: ChatRef, entry: Omit<PlaybackEntry, "chat_id" | "chat_type" | "chat_title">): void {
  const rest = recentState.items.filter((r) => !(r.chat_id === chat.id && r.chat_type === chat.chat_type && r.message_id === entry.message_id));
  const row: PlaybackEntry = { ...entry, chat_id: chat.id, chat_type: chat.chat_type, chat_title: chat.title };
  recentState.items = [row, ...rest].slice(0, 24);
}

export function resumeOf(chat: ChatRef, messageId: number): PlaybackEntry | undefined {
  return recentState.items.find((r) => r.chat_id === chat.id && r.chat_type === chat.chat_type && r.message_id === messageId);
}

const thumbs = new Map<string, Promise<string | null>>();
let inflight = 0;
const waiting: (() => void)[] = [];

async function slot<T>(fn: () => Promise<T>): Promise<T> {
  if (inflight >= 3) await new Promise<void>((r) => waiting.push(r));
  inflight += 1;
  try {
    return await fn();
  } finally {
    inflight -= 1;
    waiting.shift()?.();
  }
}

export function thumbnail(chat: TelegramChat, messageId: number): Promise<string | null> {
  const key = `${chat.chat_type}:${chat.id}:${messageId}`;
  let p = thumbs.get(key);
  if (!p) {
    p = slot(() => tg.thumbnail(chat, messageId))
      .then((b64) => (b64 ? `data:image/jpeg;base64,${b64}` : null))
      .catch(() => null);
    thumbs.set(key, p);
  }
  return p;
}

export interface Batch {
  id: number;
  chatTitle: string;
  total: number;
  finished: boolean;
  cancelled: boolean;
  files: Record<number, { status: string; percent: number; error: string | null }>;
}

export const batches = $state<Record<number, Batch>>({});

export function trackBatch(id: number, chatTitle: string, messageIds: number[]): void {
  const files: Batch["files"] = {};
  for (const m of messageIds) files[m] = { status: "queued", percent: 0, error: null };
  batches[id] = { id, chatTitle, total: messageIds.length, finished: false, cancelled: false, files };
}

export function trackBatchCount(id: number, chatTitle: string, count: number): void {
  batches[id] = { id, chatTitle, total: count, finished: count === 0, cancelled: false, files: {} };
}

export function batchCounts(b: Batch): { done: number; failed: number; percent: number } {
  let done = 0;
  let failed = 0;
  let sum = 0;
  for (const f of Object.values(b.files)) {
    if (f.status === "error") failed += 1;
    else if (f.status === "done" || f.status === "duplicate" || f.status.startsWith("skipped") || f.status === "resumed_skip") done += 1;
    sum += f.status === "error" ? 100 : f.percent;
  }
  return { done, failed, percent: b.total ? sum / b.total : 0 };
}

export function fileStatus(messageId: number): { status: string; percent: number } | null {
  let out: { status: string; percent: number } | null = null;
  for (const b of Object.values(batches)) {
    const f = b.files[messageId];
    if (f) out = f;
  }
  return out;
}

export const watchState = $state<{ rules: WatchRule[]; loaded: boolean; loading: boolean; error: string | null; lastRun: Record<string, WatchRunEvent> }>({
  rules: [],
  loaded: false,
  loading: false,
  error: null,
  lastRun: {},
});

export async function loadWatches(): Promise<void> {
  watchState.loading = true;
  try {
    await ensureSession();
    watchState.rules = await tg2.watchList();
    watchState.error = null;
  } catch (e) {
    watchState.error = failText(e);
  } finally {
    watchState.loading = false;
    watchState.loaded = true;
  }
}

export const realtimeState = $state<{ state: RealtimeState | null }>({ state: null });

export async function loadRealtime(): Promise<void> {
  try {
    realtimeState.state = await tg3.realtimeGet();
  } catch {
    realtimeState.state = null;
  }
}

export interface Scan {
  id: string;
  chatId: number;
  chatTitle: string;
  kind: string;
  scanned: number;
  matched: number;
  done: boolean;
  cancelled: boolean;
}

export const scans = $state<Record<string, Scan>>({});

export function trackScan(id: string, chatId: number, chatTitle: string, kind: string): void {
  scans[id] = { id, chatId, chatTitle, kind, scanned: 0, matched: 0, done: false, cancelled: false };
}

export function finishScan(id: string): void {
  const s = scans[id];
  if (s) s.done = true;
  setTimeout(() => {
    if (scans[id]?.done) delete scans[id];
  }, 2500);
}

export function expireScan(id: string, ms = 12000): void {
  setTimeout(() => {
    const s = scans[id];
    if (s && !s.done && s.scanned === 0) delete scans[id];
  }, ms);
}

export async function cancelScan(id: string): Promise<void> {
  const s = scans[id];
  if (s) s.cancelled = true;
  try {
    await tg3.scanCancel(id);
  } catch {
  }
  finishScan(id);
}

function onScan(p: ChatScanEvent): void {
  const id = p.scan_id ?? Object.values(scans).find((s) => s.chatId === p.chat_id && !s.done)?.id;
  if (!id) return;
  let s = scans[id];
  if (!s) {
    const mine = Object.values(scans).find((x) => x.kind === "backfill" && x.chatId === p.chat_id && !x.done && x.scanned === 0);
    if (mine) {
      delete scans[mine.id];
      scans[id] = { ...mine, id };
      s = scans[id];
    }
  }
  if (!s) {
    if (p.done) return;
    const rule = watchState.rules.find((r) => r.chatId === p.chat_id);
    scans[id] = {
      id,
      chatId: p.chat_id,
      chatTitle: rule?.chatTitle ?? chatsState.chats.find((c) => c.id === p.chat_id)?.title ?? String(p.chat_id),
      kind: "backfill",
      scanned: p.scanned,
      matched: p.matched,
      done: false,
      cancelled: false,
    };
    return;
  }
  s.scanned = Math.max(s.scanned, p.scanned);
  s.matched = Math.max(s.matched, p.matched);
  if (p.cancelled) s.cancelled = true;
  if (p.done) finishScan(id);
}

export interface Upload {
  id: number;
  title: string;
  percent: number;
  done: boolean;
  error: string | null;
}

export const uploads = $state<Record<number, Upload>>({});

let unlisteners: UnlistenFn[] = [];

export async function attachSessionEvents(): Promise<void> {
  detachSessionEvents();
  try {
    unlisteners.push(
      await listen<BatchFileStatus>("telegram-batch-file-status", (ev) => {
        const p = ev.payload;
        const b = batches[p.batch_id];
        if (!b) return;
        b.files[p.message_id] = { status: p.status, percent: p.percent, error: p.error };
      }),
    );
    type Complete = { id: number; platform?: string; success?: boolean; error?: string | null; duplicate_count?: number };
    const complete = (ev: { payload: Complete }) => {
      if (ev.payload.platform && ev.payload.platform !== "telegram") return;
      const b = batches[ev.payload.id];
      if (!b) return;
      b.finished = true;
      const dups = ev.payload.duplicate_count ?? 0;
      if (dups > 0) showToast("info", get(t)("telegram.dedupe.batch_toast", { count: dups, chat: b.chatTitle }) as string);
      if (ev.payload.success === false && ev.payload.error && !b.cancelled) showToast("error", ev.payload.error);
    };
    unlisteners.push(await listen<Complete>("telegram-download-complete", complete));
    unlisteners.push(await listen<ChatScanEvent>("telegram-chat-scan", (ev) => onScan(ev.payload)));
    unlisteners.push(
      await listen<Partial<RealtimeState>>("telegram-realtime", (ev) => {
        const prev = realtimeState.state;
        realtimeState.state = {
          triggered: 0,
          last_event_at: null,
          ...prev,
          enabled: ev.payload.enabled ?? prev?.enabled ?? true,
          connected: ev.payload.connected ?? prev?.connected ?? false,
        };
      }),
    );
    unlisteners.push(
      await listen<WatchRunEvent>("telegram-watch-run", (ev) => {
        const p = ev.payload;
        watchState.lastRun[p.id] = p;
        const rule = watchState.rules.find((r) => r.id === p.id);
        if (p.queued > 0) {
          const msg = get(t)("telegram.watch.run_toast", { count: p.queued, chat: rule?.chatTitle ?? "" }) as string;
          showToast("success", msg);
        }
        if (watchState.loaded) void loadWatches();
      }),
    );
    type UploadProgress = { id: number; title?: string; percent?: number; kind?: string };
    unlisteners.push(
      await listen<UploadProgress>("telegram-upload-progress", (ev) => {
        const p = ev.payload;
        if (p.kind === "batch") return;
        const u = uploads[p.id];
        if (u) u.percent = p.percent ?? u.percent;
        else uploads[p.id] = { id: p.id, title: p.title ?? "", percent: p.percent ?? 0, done: false, error: null };
      }),
    );
    type UploadComplete = {
      id: number;
      title?: string;
      success?: boolean;
      error?: string | null;
      kind?: string;
      count?: number;
      uploaded?: number;
      failed?: number;
    };
    unlisteners.push(
      await listen<UploadComplete>("telegram-upload-complete", (ev) => {
        const p = ev.payload;
        const ok = p.success !== false;
        if (p.kind === "batch") {
          const tr = get(t);
          if (ok) showToast("success", tr("telegram.upload.batch_done", { count: p.uploaded ?? p.count ?? 0 }) as string);
          else if (p.error) showToast("error", p.error);
          return;
        }
        uploads[p.id] = {
          id: p.id,
          title: p.title ?? uploads[p.id]?.title ?? "",
          percent: ok ? 100 : (uploads[p.id]?.percent ?? 0),
          done: true,
          error: ok ? null : (p.error ?? "failed"),
        };
      }),
    );
    unlisteners.push(
      await listen<{ kind?: string; message?: string }>("telegram-toast", (ev) => {
        const k = ev.payload.kind === "error" ? "error" : ev.payload.kind === "success" ? "success" : "info";
        if (ev.payload.message) showToast(k, ev.payload.message);
      }),
    );
  } catch {
  }
}

export function detachSessionEvents(): void {
  for (const u of unlisteners) u();
  unlisteners = [];
}
