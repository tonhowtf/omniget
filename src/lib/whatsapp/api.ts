import { invoke } from "@tauri-apps/api/core";

export type WaState = "idle" | "starting" | "pairing" | "connected" | "offline" | "logged_out";

export type MessageKind =
  | "text"
  | "image"
  | "video"
  | "audio"
  | "voice"
  | "document"
  | "sticker"
  | "location"
  | "contact"
  | "poll";

export type MediaKind = "image" | "video" | "audio" | "document" | "sticker";

export type ExportFormat = "json" | "txt" | "html";

export interface WaAccount {
  jid: string;
  push_name: string | null;
  platform: string | null;
}

export interface WaStatus {
  state: WaState;
  linked: boolean;
  account: WaAccount | null;
  qr_svg: string | null;
  qr_timeout_secs: number | null;
  pair_code: string | null;
  error: string | null;
  sync_progress: number | null;
}

export interface WaChat {
  jid: string;
  name: string;
  is_group: boolean;
  last_ts: number | null;
  unread: number;
  archived: boolean;
  last_kind: MessageKind | null;
  last_text: string | null;
  last_from_me: boolean;
  media_count: number;
  message_count: number;
}

export interface WaMessage {
  id: string;
  chat: string;
  from_me: boolean;
  sender: string | null;
  sender_name: string | null;
  ts: number;
  kind: MessageKind;
  text: string | null;
  mime: string | null;
  size: number | null;
  file_name: string | null;
  duration: number | null;
  thumb: string | null;
  saved_path: string | null;
  edited: boolean;
  deleted: boolean;
}

export interface WaMediaStats {
  image: number;
  video: number;
  audio: number;
  document: number;
  sticker: number;
  image_bytes: number;
  video_bytes: number;
  audio_bytes: number;
  document_bytes: number;
  sticker_bytes: number;
  saved: number;
}

export interface WaSavedFile {
  message_id: string;
  path: string;
  skipped: boolean;
}

export interface WaDownloadJob {
  job_id: string;
  total: number;
  dir: string;
}

export interface WaDownloadProgress {
  job_id: string | null;
  chat: string;
  message_id: string;
  stage: "start" | "file" | "retrying" | "done" | "error";
  index?: number;
  total?: number;
  done?: number;
  skipped?: number;
  failed?: number;
  bytes?: number;
  path?: string;
  error?: string;
}

export interface WaDownloadDone {
  job_id: string;
  chat: string;
  total: number;
  done: number;
  skipped: number;
  failed: number;
  bytes: number;
  dir: string;
  cancelled: boolean;
  errors: Record<string, number>;
}

export interface WaExportLabels {
  you: string;
  media_omitted: string;
  attached: string;
  exported_from: string;
  messages: string;
  deleted: string;
  edited: string;
}

export const MEDIA_KINDS: MediaKind[] = ["image", "video", "audio", "document", "sticker"];

export function errText(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

const KNOWN_ERRORS = [
  "offline",
  "not_running",
  "invalid_phone",
  "qr_expired",
  "stream_replaced",
  "temporary_ban",
  "client_outdated",
  "media_expired",
  "media_gone",
  "retry_timeout",
  "media_retry_failed",
  "no_media",
  "no_anchor",
  "empty_filter",
  "empty_message",
] as const;

export type KnownError = (typeof KNOWN_ERRORS)[number];

export function knownError(raw: string | null | undefined): KnownError | null {
  if (!raw) return null;
  const hit = KNOWN_ERRORS.find((k) => raw === k || raw.startsWith(`${k}:`));
  return hit ?? null;
}

export function isMedia(kind: MessageKind): boolean {
  return kind === "image" || kind === "video" || kind === "audio" || kind === "voice" || kind === "document" || kind === "sticker";
}

export function formatBytes(bytes: number | null | undefined): string {
  const b = bytes ?? 0;
  if (b < 1024) return `${b} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = b / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

export function formatDuration(secs: number | null | undefined): string {
  const s = Math.max(0, Math.round(secs ?? 0));
  const m = Math.floor(s / 60);
  return `${m}:${String(s % 60).padStart(2, "0")}`;
}

export const status = () => invoke<WaStatus>("whatsapp_status");
export const connect = () => invoke<WaStatus>("whatsapp_connect");
export const disconnect = () => invoke<WaStatus>("whatsapp_disconnect");
export const pairPhone = (phone: string) => invoke<string>("whatsapp_pair_phone", { phone });
export const logout = () => invoke<WaStatus>("whatsapp_logout");
export const chats = (query?: string) => invoke<WaChat[]>("whatsapp_chats", { query: query || null });
export const messages = (chat: string, before?: number | null, limit?: number) =>
  invoke<WaMessage[]>("whatsapp_messages", { chat, before: before ?? null, limit: limit ?? null });
export const markRead = (chat: string) => invoke<number>("whatsapp_mark_read", { chat });
export const sendText = (chat: string, text: string) => invoke<WaMessage>("whatsapp_send_text", { chat, text });
export const requestOlder = (chat: string) => invoke<void>("whatsapp_request_older", { chat });
export const mediaStats = (chat: string) => invoke<WaMediaStats>("whatsapp_media_stats", { chat });
export const defaultDir = () => invoke<string>("whatsapp_default_dir");
export const downloadMedia = (chat: string, messageId: string, outDir?: string | null) =>
  invoke<WaSavedFile>("whatsapp_download_media", { chat, messageId, outDir: outDir || null });
export const downloadChatMedia = (chat: string, kinds: MediaKind[], outDir?: string | null) =>
  invoke<WaDownloadJob>("whatsapp_download_chat_media", { chat, kinds, outDir: outDir || null });
export const cancelDownload = (jobId: string) => invoke<boolean>("whatsapp_cancel_download", { jobId });
export const exportName = (chat: string, format: ExportFormat) => invoke<string>("whatsapp_export_name", { chat, format });
export const exportChat = (chat: string, format: ExportFormat, path: string, labels: WaExportLabels) =>
  invoke<number>("whatsapp_export", { chat, format, path, labels });
export const reveal = (path: string) => invoke<void>("reveal_file", { path });
export const openPath = (path: string) => invoke<void>("open_path_default", { path });
