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
  tags: string[];
  fields: [string, string][];
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
  seq: number;
  quote_id: string | null;
  reactions: WaReaction[];
}

export interface WaReaction {
  emoji: string;
  count: number;
  mine: boolean;
}

export type RuleTrigger =
  | { kind: "keyword"; mode: "contains" | "exact" | "starts_with"; words: string[] }
  | { kind: "first_message" }
  | { kind: "outside_hours"; open_minute: number; close_minute: number }
  | { kind: "any" };

export type RuleAction =
  | { kind: "reply"; text: string }
  | { kind: "react"; emoji: string }
  | { kind: "ai"; instructions: string; fallback: string | null; draft: boolean };

export interface WaRule {
  id: number;
  name: string;
  enabled: boolean;
  trigger: RuleTrigger;
  chats: string[];
  action: RuleAction;
  cooldown_minutes: number;
  tag: string | null;
  follow_ups: { after_minutes: number; text: string }[];
  needs_tag: string | null;
  ask: { question: string; field: string } | null;
}

export interface WaLimits {
  enabled: boolean;
  per_hour: number;
  per_day: number;
  delay_min_ms: number;
  delay_max_ms: number;
  manual_pause_minutes: number;
}

export interface WaRulesState {
  limits: WaLimits;
  rules: WaRule[];
  sent_hour: number;
  sent_day: number;
  opt_outs: { chat: string; name: string; ts: number }[];
  stats: { rule_id: number; sent: number; skipped: number; failed: number }[];
}

export interface WaRuleRun {
  id: number;
  rule_id: number;
  chat: string;
  chat_name: string;
  ts: number;
  outcome: "queued" | "sent" | "failed" | "skipped";
  detail: string | null;
}

export interface WaRuleTest {
  rule: WaRule | null;
  reply: string | null;
  opt_out: boolean;
}

export interface WaScheduled {
  id: number;
  chat: string;
  text: string;
  reply_to: string | null;
  at: number;
  created: number;
  status: "pending" | "sent" | "failed" | "missed" | "cancelled";
  error: string | null;
  sent_id: string | null;
  sent_at: number | null;
  rule_id: number | null;
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
  "not_linked",
  "time_in_past",
  "chat_not_found",
  "reply_not_found",
  "message_not_found",
  "invalid_reaction",
  "invalid_rule_name",
  "invalid_rule_words",
  "invalid_rule_text",
  "rule_not_found",
  "invalid_rule_follow_ups",
  "invalid_tag",
  "invalid_field",
  "invalid_rule_hours",
  "file_too_large",
  "file_not_found",
  "upload_failed",
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

export const status = () => invoke<WaStatus>("phonechat_status");
export const connect = () => invoke<WaStatus>("phonechat_connect");
export const disconnect = () => invoke<WaStatus>("phonechat_disconnect");
export const pairPhone = (phone: string) => invoke<string>("phonechat_pair_phone", { phone });
export const logout = () => invoke<WaStatus>("phonechat_logout");
export const chats = (query?: string) => invoke<WaChat[]>("phonechat_chats", { query: query || null });
export const messages = (chat: string, before?: number | null, limit?: number) =>
  invoke<WaMessage[]>("phonechat_messages", { chat, before: before ?? null, limit: limit ?? null });
export const markRead = (chat: string) => invoke<number>("phonechat_mark_read", { chat });
export const sendText = (chat: string, text: string, replyTo?: string | null) =>
  invoke<WaMessage>("phonechat_send_text", { chat, text, replyTo: replyTo ?? null });
export const react = (chat: string, id: string, emoji: string) => invoke<void>("phonechat_react", { chat, id, emoji });
export const sendFile = (chat: string, path: string, caption?: string | null) =>
  invoke<WaMessage>("phonechat_send_file", { chat, path, caption: caption || null });
export const tag = (chat: string, name: string, on: boolean) => invoke<boolean>("phonechat_tag", { chat, tag: name, on });
export const field = (chat: string, key: string, value: string) => invoke<void>("phonechat_field", { chat, key, value });
export const contactsExport = (path: string) => invoke<number>("phonechat_contacts_export", { path });
export const draft = (chat: string) => invoke<string | null>("phonechat_draft", { chat });
export const draftClear = (chat: string) => invoke<boolean>("phonechat_draft_clear", { chat });
export const rules = () => invoke<WaRulesState>("phonechat_rules");
export const ruleSave = (rule: WaRule) => invoke<WaRule>("phonechat_rule_save", { rule });
export const ruleEnable = (id: number, enabled: boolean) => invoke<WaRule>("phonechat_rule_enable", { id, enabled });
export const ruleDelete = (id: number) => invoke<boolean>("phonechat_rule_delete", { id });
export const rulesLimits = (limits: WaLimits) => invoke<WaLimits>("phonechat_rules_limits", { limits });
export const ruleTest = (text: string) => invoke<WaRuleTest>("phonechat_rule_test", { text, chat: null });
export const ruleRuns = (limit = 30) => invoke<WaRuleRun[]>("phonechat_rule_runs", { limit });
export const optIn = (chat: string) => invoke<boolean>("phonechat_opt_in", { chat });
export const schedule = (chat: string, text: string, at: number, replyTo?: string | null) =>
  invoke<WaScheduled>("phonechat_schedule", { chat, text, at, replyTo: replyTo ?? null });
export const scheduled = (includeDone = false) => invoke<WaScheduled[]>("phonechat_scheduled", { includeDone });
export const cancelScheduled = (id: number) => invoke<boolean>("phonechat_cancel_scheduled", { id });
export const requestOlder = (chat: string) => invoke<void>("phonechat_request_older", { chat });
export const mediaStats = (chat: string) => invoke<WaMediaStats>("phonechat_media_stats", { chat });
export const defaultDir = () => invoke<string>("phonechat_default_dir");
export const downloadMedia = (chat: string, messageId: string, outDir?: string | null) =>
  invoke<WaSavedFile>("phonechat_download_media", { chat, messageId, outDir: outDir || null });
export const downloadChatMedia = (chat: string, kinds: MediaKind[], outDir?: string | null) =>
  invoke<WaDownloadJob>("phonechat_download_chat_media", { chat, kinds, outDir: outDir || null });
export const cancelDownload = (jobId: string) => invoke<boolean>("phonechat_cancel_download", { jobId });
export const exportName = (chat: string, format: ExportFormat) => invoke<string>("phonechat_export_name", { chat, format });
export const exportChat = (chat: string, format: ExportFormat, path: string, labels: WaExportLabels) =>
  invoke<number>("phonechat_export", { chat, format, path, labels });
export const reveal = (path: string) => invoke<void>("reveal_file", { path });
export const openPath = (path: string) => invoke<void>("open_path_default", { path });
