// Typed wrappers for the `/telegram` superpower. Everything the engine does
// goes through `telegram_call(command, args)`; the connection layer has its
// own `telegram_net_*` commands. Types mirror the serde shapes in
// `omniget-core/src/core/telegram/net/{route,manager,doctor}.rs`.

import { invoke } from "@tauri-apps/api/core";

type RouteKind = "direct" | "socks5" | "http" | "mt_proxy" | "shadowsocks" | "trojan";

type RouteSource =
  | { type: "direct" }
  | { type: "manual" }
  | { type: "link" }
  | { type: "system" }
  | { type: "env"; var: string }
  | { type: "local_app"; app: string }
  | { type: "clash"; config: string }
  | { type: "subscription"; url: string }
  | { type: "public_list"; list: string };

export interface Route {
  id: string;
  kind: RouteKind;
  name: string;
  host: string;
  port: number;
  username?: string;
  password?: string;
  secret?: string;
  cipher?: string;
  sni?: string;
  insecure: boolean;
  source: RouteSource;
}

export interface Health {
  ok: boolean | null;
  rtt_ms: number | null;
  connect_ms: number | null;
  error: string | null;
  tested_at: number;
  fails_in_row: number;
}

export interface RouteView {
  route: Route;
  health: Health;
  saved: boolean;
  active: boolean;
  share_link: string | null;
}

export interface NetOpts {
  prefer_ipv6: boolean;
  prefer_doh: boolean;
  connect_timeout_secs: number;
}

interface SubscriptionEntry {
  url: string;
  name: string;
  updated_at: number;
  count: number;
  unsupported: [string, number][];
}

export type Phase = "idle" | "scanning" | "connected" | "failed";
export type Mode = "auto" | "fixed";

export interface NetStatus {
  phase: Phase;
  mode: Mode;
  active: RouteView | null;
  routes: RouteView[];
  scanned_at: number;
  clock_offset_secs: number | null;
  opts: NetOpts;
  use_rescue: boolean;
  subscriptions: SubscriptionEntry[];
  bridge_port: number | null;
}

export type StepState = "ok" | "warn" | "fail" | "skip";

interface DoctorStep {
  id: string;
  state: StepState;
  hint: string;
  detail: string;
}

export interface DoctorReport {
  steps: DoctorStep[];
  status: NetStatus;
  verdict: string;
}

export interface AddLinksResult {
  added: Route[];
  unsupported: string[];
}

/** Protocols only a full proxy core (Clash/mihomo, v2rayN) can speak. */
export const NEEDS_CLASH = new Set(["vmess", "vless", "hysteria2", "hy2", "hysteria", "tuic", "ssr", "wireguard", "anytls"]);

const RESCUE_LIST = "telegram-rescue";

// ---- connection layer ----

export const net = {
  status: () => invoke<NetStatus>("telegram_net_status"),
  scan: () => invoke<NetStatus>("telegram_net_scan"),
  doctor: () => invoke<DoctorReport>("telegram_net_doctor"),
  addLinks: (text: string) => invoke<AddLinksResult>("telegram_net_add_links", { text }),
  addRoute: (route: Route) => invoke<Route>("telegram_net_add_route", { route }),
  removeRoute: (id: string) => invoke<void>("telegram_net_remove_route", { id }),
  keep: (id: string) => invoke<void>("telegram_net_keep", { id }),
  removeFailed: () => invoke<number>("telegram_net_remove_failed"),
  test: (id: string) => invoke<RouteView>("telegram_net_test", { id }),
  setMode: (mode: Mode, fixedRoute?: string) =>
    invoke<void>("telegram_net_set_mode", { mode, fixedRoute: fixedRoute ?? null }),
  setOpts: (opts: NetOpts, useRescue?: boolean) =>
    invoke<void>("telegram_net_set_opts", { opts, useRescue: useRescue ?? null }),
  addSubscription: (url: string, name?: string) =>
    invoke<SubscriptionEntry>("telegram_net_add_subscription", { url, name: name ?? null }),
  removeSubscription: (url: string) => invoke<void>("telegram_net_remove_subscription", { url }),
  refreshSubscriptions: () =>
    invoke<[string, { Ok?: number; Err?: string }][]>("telegram_net_refresh_subscriptions"),
  mihomoStatus: () => invoke<MihomoStatus>("telegram_net_mihomo_status"),
  mihomoSet: (on: boolean) => invoke<MihomoStatus>("telegram_net_mihomo_set", { on }),
  mihomoRestart: () => invoke<MihomoStatus>("telegram_net_mihomo_restart"),
  mihomoRemoveNode: (key: string) => invoke<void>("telegram_net_mihomo_remove_node", { key }),
};

/** The optional mihomo core that speaks VLESS/Hysteria2/VMess/TUIC… for us. */
export interface MihomoStatus {
  enabled: boolean | null;
  supported_platform: boolean;
  installed: boolean;
  installing: boolean;
  running: boolean;
  pinned_version: string;
  version: string | null;
  error: string | null;
  nodes: { key: string; name: string; kind: string; server: string; port: number; route_id: string | null }[];
  kinds: Record<string, number>;
}

// ---- engine ----

export function call<T = unknown>(command: string, args: Record<string, unknown> = {}): Promise<T> {
  return invoke<T>("telegram_call", { command, args });
}

/** True when the dispatcher does not know the command yet. */
export function isUnknownCommand(e: unknown): boolean {
  return /unknown telegram command|not implemented|unknown command/i.test(errText(e));
}

export function errText(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

export interface TelegramChat {
  id: number;
  title: string;
  chat_type: string;
  peer_hash: number;
  last_message?: string;
  last_message_date?: number;
  unread_count: number;
  is_muted: boolean;
  is_pinned: boolean;
  is_verified: boolean;
  is_online: boolean;
}

export interface ChatPageOffset {
  date: number;
  id: number;
  peer_id: number;
  peer_type: string;
  peer_hash: number;
}

interface ChatPage {
  chats: TelegramChat[];
  next_offset: ChatPageOffset | null;
}

export interface TelegramMediaItem {
  message_id: number;
  file_name: string;
  file_size: number;
  media_type: string;
  date: number;
  caption?: string;
  grouped_id?: number;
}

export interface TelegramSelf {
  user_id: number;
  username: string | null;
  first_name: string;
  last_name: string | null;
  phone: string | null;
}

export interface AccountProfile {
  id: string;
  label: string;
  phone_redacted?: string | null;
  user_id?: number | null;
  created_at: number;
  updated_at: number;
}

export interface QrStart {
  svg: string;
  expires: number;
}

export interface BatchFileStatus {
  batch_id: number;
  message_id: number;
  status: string;
  percent: number;
  error: string | null;
}

/** One pasted t.me link after resolving. Fields are optional on purpose:
 * a link can point to a chat, a message or a message without media. */
export interface ResolvedLink {
  link: string;
  ok?: boolean;
  error?: string | null;
  chat_id?: number;
  chat_type?: string;
  chat_title?: string;
  message_id?: number;
  file_name?: string;
  file_size?: number;
  media_type?: string;
}

export const tg = {
  checkSession: () => call<string>("telegram_check_session"),
  qrStart: () => call<QrStart>("telegram_qr_start"),
  qrPoll: () => call<string>("telegram_qr_poll"),
  sendCode: (phone: string) => call<null>("telegram_send_code", { phone }),
  verifyCode: (code: string) => call<string>("telegram_verify_code", { code }),
  verify2fa: (password: string) => call<string>("telegram_verify_2fa", { password }),
  logout: () => call<null>("telegram_logout"),
  getSelf: () => call<TelegramSelf>("telegram_get_self"),
  listChatsPage: (offset: ChatPageOffset | null, limit = 100) =>
    call<ChatPage>("telegram_list_chats_page", {
      offsetDate: offset?.date ?? 0,
      offsetId: offset?.id ?? 0,
      offsetPeer: offset
        ? { peer_id: offset.peer_id, peer_type: offset.peer_type, peer_hash: offset.peer_hash }
        : null,
      limit,
    }),
  listMedia: (chat: TelegramChat, mediaType: string | null, offset: number, limit = 60) =>
    call<TelegramMediaItem[]>("telegram_list_media", {
      chatId: chat.id,
      chatType: chat.chat_type,
      mediaType: mediaType ?? undefined,
      offset,
      limit,
    }),
  searchMedia: (chat: TelegramChat, query: string, mediaType: string | null, limit = 60) =>
    call<TelegramMediaItem[]>("telegram_search_media", {
      chatId: chat.id,
      chatType: chat.chat_type,
      query,
      mediaType: mediaType ?? undefined,
      limit,
    }),
  thumbnail: (chat: TelegramChat, messageId: number) =>
    call<string>("telegram_get_thumbnail", { chatId: chat.id, chatType: chat.chat_type, messageId }),
  downloadBatch: (chat: TelegramChat, items: TelegramMediaItem[], outputDir: string) =>
    call<number>("telegram_download_batch", {
      chatId: chat.id,
      chatType: chat.chat_type,
      chatTitle: chat.title,
      items: items.map((i) => ({ message_id: i.message_id, file_name: i.file_name, file_size: i.file_size })),
      outputDir,
      resume: true,
    }),
  cancelBatch: (batchId: number) => call<null>("telegram_cancel_batch", { batchId }),
  // The backend answers `{ items: [{url, …}], errors: [{url, error}] }`;
  // the screen wants one row per link, failures included.
  resolveLinks: async (text: string): Promise<ResolvedLink[]> => {
    const r = await call<{
      items?: (Omit<ResolvedLink, "link"> & { url: string })[];
      errors?: { url: string; error: string }[];
    }>("telegram_resolve_links", { text });
    const ok = (r?.items ?? []).map(({ url, ...rest }) => ({ ...rest, link: url, ok: true }));
    const bad = (r?.errors ?? []).map((e) => ({ link: e.url, ok: false, error: e.error }));
    return [...ok, ...bad];
  },
  downloadLinks: (text: string, outputDir: string) =>
    call<{ batch_id: number; count: number }>("telegram_download_links", { text, outputDir }),
  exportChat: (chat: TelegramChat, format: string, withMedia: boolean, outputDir: string) =>
    call<{ export_id: number }>("telegram_export_chat", {
      chatId: chat.id,
      chatType: chat.chat_type,
      chatTitle: chat.title,
      format,
      includeMedia: withMedia,
      outputDir,
    }),
  accountsList: () => call<AccountProfile[] | { profiles: AccountProfile[] }>("telegram_accounts_list"),
  accountsSaveCurrent: (label: string, phone?: string | null, userId?: number | null) =>
    call<AccountProfile>("telegram_accounts_save_current", { label, phone: phone ?? null, userId: userId ?? null }),
  accountsRestore: (id: string) => call<{ ok: boolean; needs_restart?: boolean }>("telegram_accounts_restore", { id }),
  accountsRemove: (id: string) => call<unknown>("telegram_accounts_remove", { id }),
  accountsRename: (id: string, label: string) => call<AccountProfile>("telegram_accounts_rename", { id, label }),
};

// ---- round 2: player, auto-save, whole-chat download, upload, forward, clone ----

/** Any chat reference the backend needs: `{chatId, chatType}`. */
export interface ChatRef {
  id: number;
  chat_type: string;
  title: string;
}

export interface StreamInfo {
  url: string;
  mime: string;
  file_name: string;
  file_size: number;
  duration_sec?: number | null;
  width?: number | null;
  height?: number | null;
  /** False when the container (MKV, AVI, some WebM) does not play in the webview. */
  playable?: boolean | null;
  /** HLS copy with the video untouched and the audio in AAC (fast). Only when `playable` is false. */
  remux_url?: string | null;
  /** HLS copy fully converted to H.264/AAC (slow); for when the remux fails too. */
  transcode_url?: string | null;
  /** File extension of the original. */
  container?: string | null;
}

interface PlaybackState {
  position_sec: number;
  duration_sec: number;
  finished: boolean;
}

/** One "continue watching" row. Only the ids and positions are certain;
 * the rest is shown when the backend sends it. */
export interface PlaybackEntry extends PlaybackState {
  chat_id: number;
  chat_type: string;
  message_id: number;
  chat_title?: string | null;
  file_name?: string | null;
  media_type?: string | null;
  updated_at?: number | null;
}

export type ExternalPlayer = "default" | "vlc" | "iina" | "mpv";

export const MEDIA_TYPES = ["video", "photo", "document", "audio", "gif", "round_video", "voice"] as const;

/** Filters shared by auto-save rules and the whole-chat download. Sizes in bytes. */
export interface MediaFilter {
  mediaTypes?: string[];
  fromDate?: number;
  toDate?: number;
  minSize?: number;
  maxSize?: number;
  keyword?: string;
  extInclude?: string[];
  extExclude?: string[];
  fromMessageId?: number;
  toMessageId?: number;
  limit?: number;
}

export interface WatchRule {
  id: string;
  chatId: number;
  chatType: string;
  chatTitle: string;
  enabled: boolean;
  mediaTypes: string[];
  minSize?: number | null;
  maxSize?: number | null;
  keyword?: string | null;
  extInclude?: string[] | null;
  extExclude?: string[] | null;
  outputDir?: string | null;
  template?: string | null;
  lastMessageId: number;
  createdAt: number;
  lastRunAt?: number | null;
  savedCount: number;
  lastResult?: string | null;
  lastError?: string | null;
  /** Skip files already saved anywhere (same Telegram file); null follows the global setting. */
  dedupe?: boolean | null;
  /** Add each saved file to the Watch library. */
  addToWatch?: boolean | null;
  /** Files this rule did not download again because they were already saved. */
  duplicateCount?: number | null;
}

export type WatchRuleDraft = Omit<WatchRule, "id" | "lastMessageId" | "createdAt" | "lastRunAt" | "savedCount" | "duplicateCount">;

export interface WatchRunEvent {
  id: string;
  found: number;
  queued: number;
  error?: string | null;
  /** "realtime" | "interval" | "manual" | "backfill". */
  trigger?: string | null;
  /** Downloads avoided because the file was already saved. */
  duplicates?: number | null;
}

export interface ChatDownloadArgs {
  filter: MediaFilter;
  template?: string;
  outputDir?: string;
  skipSame?: boolean;
  takeout?: boolean;
  /** Skip files already saved in any folder (downloaded.sqlite). */
  dedupe?: boolean;
  /** Lets the screen follow and cancel the scan (`telegram-chat-scan`). */
  scanId?: string;
}

export interface ChatDownloadPreview {
  count: number;
  total_bytes: number;
  scanned?: number;
  output_dir?: string;
  scan_id?: string;
  /** True when the count came from the on-disk history index (no full re-read). */
  from_index?: boolean;
  /** Each sample also carries its planned path (relative to `output_dir`) and whether it is already there. */
  sample: (TelegramMediaItem & { path?: string; exists?: boolean })[];
}

interface UploadArgs {
  paths: string[];
  caption?: string;
  asDocument?: boolean;
  deleteAfter?: boolean;
  groupAlbums?: boolean;
}

interface CloneOptions {
  delay_ms: number;
  batch_size: number;
  limit: number | null;
  drop_author: boolean;
  drop_captions: boolean;
}

export interface CloneSession {
  id: string;
  source_chat_id: number;
  source_chat_type: string;
  source_title: string;
  dest_chat_id: number;
  dest_chat_type: string;
  dest_title: string;
  last_message_id: number;
  total_collected: number;
  cloned_count: number;
  failed_count: number;
  status: string;
  options: CloneOptions;
  created_at: number;
  updated_at: number;
  error?: string | null;
}

export interface CloneProgress {
  session_id: string;
  stage: string;
  total: number;
  current: number;
  failed: number;
  last_message_id: number;
  error?: string | null;
}

const chatArgs = (c: ChatRef) => ({ chatId: c.id, chatType: c.chat_type });

/** Rules may come back in camelCase (contract) or snake_case (serde default). */
function normRule(r: Record<string, unknown>): WatchRule {
  const g = (camel: string, snake: string) => (r[camel] !== undefined ? r[camel] : r[snake]);
  return {
    id: String(r.id ?? ""),
    chatId: Number(g("chatId", "chat_id") ?? 0),
    chatType: String(g("chatType", "chat_type") ?? ""),
    chatTitle: String(g("chatTitle", "chat_title") ?? ""),
    enabled: Boolean(r.enabled ?? true),
    mediaTypes: (g("mediaTypes", "media_types") as string[] | undefined) ?? [],
    minSize: (g("minSize", "min_size") as number | null | undefined) ?? null,
    maxSize: (g("maxSize", "max_size") as number | null | undefined) ?? null,
    keyword: (r.keyword as string | null | undefined) ?? null,
    extInclude: (g("extInclude", "ext_include") as string[] | null | undefined) ?? null,
    extExclude: (g("extExclude", "ext_exclude") as string[] | null | undefined) ?? null,
    outputDir: (g("outputDir", "output_dir") as string | null | undefined) ?? null,
    template: (r.template as string | null | undefined) ?? null,
    lastMessageId: Number(g("lastMessageId", "last_message_id") ?? 0),
    createdAt: Number(g("createdAt", "created_at") ?? 0),
    lastRunAt: (g("lastRunAt", "last_run_at") as number | null | undefined) ?? null,
    savedCount: Number(g("savedCount", "saved_count") ?? 0),
    lastResult: (g("lastResult", "last_result") as string | null | undefined) ?? null,
    lastError: (g("lastError", "last_error") as string | null | undefined) ?? null,
    dedupe: (r.dedupe as boolean | null | undefined) ?? null,
    addToWatch: (g("addToWatch", "add_to_watch") as boolean | null | undefined) ?? null,
    duplicateCount: (g("duplicateCount", "duplicate_count") as number | null | undefined) ?? null,
  };
}

function normEntry(r: Record<string, unknown>): PlaybackEntry {
  const g = (camel: string, snake: string) => (r[snake] !== undefined ? r[snake] : r[camel]);
  return {
    chat_id: Number(g("chatId", "chat_id") ?? 0),
    chat_type: String(g("chatType", "chat_type") ?? ""),
    message_id: Number(g("messageId", "message_id") ?? 0),
    position_sec: Number(g("positionSec", "position_sec") ?? 0),
    duration_sec: Number(g("durationSec", "duration_sec") ?? 0),
    finished: Boolean(r.finished),
    chat_title: (g("chatTitle", "chat_title") as string | null | undefined) ?? null,
    file_name: (g("fileName", "file_name") as string | null | undefined) ?? null,
    media_type: (g("mediaType", "media_type") as string | null | undefined) ?? null,
    updated_at: (g("updatedAt", "updated_at") as number | null | undefined) ?? null,
  };
}

export const tg2 = {
  streamUrl: (c: ChatRef, messageId: number) => call<StreamInfo>("telegram_stream_url", { ...chatArgs(c), messageId }),
  playbackGet: (c: ChatRef, messageId: number) =>
    call<PlaybackState | null>("telegram_playback_get", { ...chatArgs(c), messageId }),
  playbackSet: (c: ChatRef, messageId: number, positionSec: number, durationSec: number, extra: Record<string, unknown> = {}) =>
    call<unknown>("telegram_playback_set", { ...chatArgs(c), messageId, positionSec, durationSec, ...extra }),
  playbackRecent: async (limit = 12): Promise<PlaybackEntry[]> => {
    const r = await call<Record<string, unknown>[] | { items: Record<string, unknown>[] }>("telegram_playback_recent", { limit });
    return listOf<Record<string, unknown>>(r, "items").map(normEntry);
  },
  openExternal: (url: string, player: ExternalPlayer = "default") => call<unknown>("telegram_open_external", { url, player }),

  watchList: async (): Promise<WatchRule[]> => {
    const r = await call<Record<string, unknown>[] | { rules: Record<string, unknown>[] }>("telegram_watch_list");
    return listOf<Record<string, unknown>>(r, "rules").map(normRule);
  },
  watchAdd: async (rule: WatchRuleDraft, backfill: number, scanId?: string) =>
    normRule(
      await call<Record<string, unknown>>("telegram_watch_add", {
        rule,
        backfill: backfill > 0 ? backfill : undefined,
        scanId: backfill > 0 ? scanId : undefined,
      }),
    ),
  watchUpdate: (id: string, patch: Partial<WatchRuleDraft>) => call<unknown>("telegram_watch_update", { id, patch }),
  watchRemove: (id: string) => call<unknown>("telegram_watch_remove", { id }),
  watchRun: (id?: string) => call<unknown>("telegram_watch_run", id ? { id } : {}),
  watchSettings: (intervalMin?: number) =>
    call<{ interval_min: number }>("telegram_watch_settings", intervalMin ? { intervalMin } : {}),
  externalPlayers: () => call<ExternalPlayer[]>("telegram_external_players"),

  previewChatDownload: (c: ChatRef, a: ChatDownloadArgs) =>
    call<ChatDownloadPreview>("telegram_preview_chat_download", { ...chatArgs(c), chatTitle: c.title, ...a }),
  downloadChat: (c: ChatRef, a: ChatDownloadArgs) =>
    call<{ batch_id: number | null; count: number }>("telegram_download_chat", { ...chatArgs(c), chatTitle: c.title, ...a }),

  uploadPaths: (c: ChatRef, a: UploadArgs) =>
    call<{ batch_id: number; count: number }>("telegram_upload_paths", { ...chatArgs(c), ...a }),

  forward: (from: ChatRef, to: ChatRef, messageIds: number[], dropAuthor: boolean, dropCaptions: boolean) =>
    call<{ ok: boolean }>("telegram_forward_messages", {
      fromChatId: from.id,
      fromChatType: from.chat_type,
      toChatId: to.id,
      toChatType: to.chat_type,
      messageIds,
      dropAuthor,
      dropCaptions,
    }),

  cloneStart: (a: {
    source: ChatRef;
    dest?: ChatRef | null;
    destTitle?: string;
    resumeId?: string;
    options: CloneOptions;
  }) =>
    call<{ session_id: string; dest_chat_id: number; dest_chat_type: string; dest_title: string }>("telegram_clone_start", {
      sourceId: a.source.id,
      sourceType: a.source.chat_type,
      sourceTitle: a.source.title,
      destId: a.dest?.id,
      destType: a.dest?.chat_type,
      destTitle: a.dest ? a.dest.title : a.destTitle || undefined,
      resumeId: a.resumeId,
      options: a.options,
    }),
  cloneList: () => call<CloneSession[]>("telegram_clone_list"),
  clonePause: (sessionId: string) => call<unknown>("telegram_clone_pause", { sessionId }),
  cloneCancel: (sessionId: string) => call<unknown>("telegram_clone_cancel", { sessionId }),
  cloneDelete: (sessionId: string) => call<unknown>("telegram_clone_delete", { sessionId }),
};

// ---- round 3: fast scans, dedupe, media search, transcripts, Watch, approvals ----

/** `telegram_realtime_get/set` and the `telegram-realtime` event. */
export interface RealtimeState {
  enabled: boolean;
  /** The Telegram update channel is being read right now. */
  connected: boolean;
  last_event_at?: number | null;
  /** How many times a rule ran because of a live message. */
  triggered?: number;
}

/** `telegram-chat-scan` payload. */
export interface ChatScanEvent {
  scan_id?: string | null;
  chat_id: number;
  scanned: number;
  matched: number;
  done?: boolean;
  cancelled?: boolean;
}

export interface DedupeStats {
  enabled: boolean;
  /** Files in the register of saved files. */
  files: number;
  bytes: number;
  /** Downloads avoided because the file was already saved. */
  avoided: number;
  avoided_bytes: number;
}

/** `telegram-transcribe-progress` payload. */
export interface TranscribeProgress {
  chat_id: number;
  message_id: number;
  /** "audio" | "chunk" | "done". */
  stage: string;
  done: number;
  total: number;
}

interface MediaSearch {
  query?: string;
  mediaType?: string | null;
  fromDate?: number;
  toDate?: number;
  minSize?: number;
  maxSize?: number;
  limit?: number;
}

interface TranscriptSegment {
  start: number;
  end: number;
  text: string;
}

export interface Transcript {
  text: string;
  segments: TranscriptSegment[];
  lang?: string | null;
  /** "local" (the app's own engine) or "telegram" (Premium). */
  source?: string | null;
  cached?: boolean;
}

export interface ApprovalRequest {
  id: string;
  tool: string;
  summary: string;
  /** What exactly will happen: chat, text, files, message ids. */
  details?: unknown;
  /** `agent`, an external MCP client name, or `cli`. */
  client?: string | null;
  created?: number | null;
  expires_at?: number | null;
}

function num(v: unknown): number {
  const n = Number(v);
  return Number.isFinite(n) ? n : 0;
}

function normTranscript(r: Record<string, unknown>): Transcript {
  // Segments come in milliseconds (`start_ms`); seconds are accepted too.
  const segs = listOf<Record<string, unknown>>(r?.segments as Record<string, unknown>[] | undefined, "segments").map((s) => ({
    start: s.start_ms != null ? num(s.start_ms) / 1000 : num(s.start),
    end: s.end_ms != null ? num(s.end_ms) / 1000 : num(s.end),
    text: String(s.text ?? "").trim(),
  }));
  const text = typeof r?.text === "string" ? (r.text as string) : segs.map((s) => s.text).join(" ");
  return {
    text: text.trim(),
    segments: segs.filter((s) => s.text),
    lang: ((r?.lang ?? r?.language) as string | null | undefined) || null,
    source: (r?.source ?? r?.engine ?? null) as string | null,
    cached: Boolean(r?.cached),
  };
}

function normApproval(r: Record<string, unknown>): ApprovalRequest {
  return {
    id: String(r.id ?? ""),
    tool: String(r.tool ?? r.tool_name ?? r.toolName ?? ""),
    summary: String(r.summary ?? ""),
    details: r.details ?? r.detail ?? r.args ?? null,
    client: (r.client ?? null) as string | null,
    created: (r.created ?? r.created_at ?? null) as number | null,
    expires_at: (r.expires_at ?? r.expiresAt ?? null) as number | null,
  };
}

function normDedupe(r: Record<string, unknown>): DedupeStats {
  return {
    enabled: r?.enabled !== false,
    files: num(r?.files),
    bytes: num(r?.bytes),
    avoided: num(r?.avoided),
    avoided_bytes: num(r?.avoided_bytes),
  };
}

export const tg3 = {
  scanCancel: (scanId: string) => call<unknown>("telegram_scan_cancel", { scanId }),
  realtimeGet: () => call<RealtimeState>("telegram_realtime_get"),
  realtimeSet: (enabled: boolean) => call<RealtimeState>("telegram_realtime_set", { enabled }),
  dedupeStats: async () => normDedupe(await call<Record<string, unknown>>("telegram_dedupe_stats")),
  dedupeSet: async (enabled: boolean) => normDedupe(await call<Record<string, unknown>>("telegram_dedupe_set", { enabled })),
  dedupeClear: async () => normDedupe(await call<Record<string, unknown>>("telegram_dedupe_clear")),
  searchMedia: (c: ChatRef, q: MediaSearch) =>
    call<TelegramMediaItem[]>("telegram_search_media", {
      ...chatArgs(c),
      query: q.query ?? "",
      mediaType: q.mediaType ?? undefined,
      fromDate: q.fromDate,
      toDate: q.toDate,
      minSize: q.minSize,
      maxSize: q.maxSize,
      limit: q.limit ?? 100,
    }),
  transcribe: async (c: ChatRef, messageId: number, lang?: string, force = false) =>
    normTranscript(
      await call<Record<string, unknown>>("telegram_transcribe", { ...chatArgs(c), messageId, lang: lang || undefined, force: force || undefined }),
    ),
  /** Cached transcript or null; never starts work. */
  transcriptGet: async (c: ChatRef, messageId: number): Promise<Transcript | null> => {
    const r = await call<Record<string, unknown> | null>("telegram_transcript_get", { ...chatArgs(c), messageId });
    return r ? normTranscript(r) : null;
  },
  /** Telegram's own transcription (Premium). `pending` means ask again in a few seconds. */
  transcribeNative: async (c: ChatRef, messageId: number): Promise<Transcript & { pending: boolean }> => {
    const r = await call<Record<string, unknown>>("telegram_voice_transcribe_native", { ...chatArgs(c), messageId });
    return { ...normTranscript(r), source: "telegram", pending: Boolean(r?.pending) };
  },
  addToWatch: (a: { paths?: string[]; folder?: string }) =>
    call<{ added?: number; skipped?: number; errors?: { path: string; error: string }[] }>("telegram_add_to_watch", a),
  approvalList: async (): Promise<ApprovalRequest[]> => {
    const r = await call<Record<string, unknown>[] | { items: Record<string, unknown>[] }>("telegram_approval_list");
    return listOf<Record<string, unknown>>(r, "items").map(normApproval);
  },
  approvalAnswer: (id: string, approve: boolean) => call<unknown>("telegram_approval_answer", { id, approve }),
};

/** Files added by `telegram_add_to_watch`, or null when the answer has no count. */
export function addedCount(r: { added?: number } | null | undefined): number | null {
  return r && typeof r.added === "number" ? r.added : null;
}

export function newScanId(): string {
  return `scan-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

export const PLAYABLE = new Set(["video", "gif", "audio", "voice", "round_video"]);

export function isPlayable(item: { media_type: string; file_name?: string }): boolean {
  if (PLAYABLE.has(item.media_type)) return true;
  return /\.(mp4|m4v|mov|webm|mkv|mp3|m4a|aac|ogg|oga|opus|flac|wav)$/i.test(item.file_name ?? "");
}

export function isAudio(item: { media_type: string; file_name?: string }): boolean {
  return item.media_type === "audio" || item.media_type === "voice" || /\.(mp3|m4a|aac|ogg|oga|opus|flac|wav)$/i.test(item.file_name ?? "");
}

/** Private link to a message; works for members of groups and channels. */
export function messageLink(chat: ChatRef, messageId: number): string | null {
  if (chat.chat_type === "private") return null;
  const bare = String(Math.abs(chat.id)).replace(/^100(?=\d{9,})/, "");
  return `https://t.me/c/${bare}/${messageId}`;
}

export function formatDuration(sec: number): string {
  if (!Number.isFinite(sec) || sec < 0) sec = 0;
  const s = Math.floor(sec % 60);
  const m = Math.floor((sec / 60) % 60);
  const h = Math.floor(sec / 3600);
  const pad = (n: number) => String(n).padStart(2, "0");
  return h ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/** "mp4, .MKV mov" → ["mp4", "mkv", "mov"]. */
export function parseExts(text: string): string[] {
  return text
    .split(/[\s,;]+/)
    .map((s) => s.trim().replace(/^\./, "").toLowerCase())
    .filter(Boolean);
}

export const TEMPLATE_VARS = ["chat", "chat_id", "msg_id", "date", "time", "file_name", "ext", "type", "caption", "album"] as const;

function sanitize(s: string): string {
  return s.replace(/[\\/:*?"<>|\u0000-\u001f]+/g, "_").replace(/\s+/g, " ").trim();
}

/** Local mirror of the backend naming template, for the preview only. */
export function renderTemplate(
  template: string,
  chat: ChatRef,
  item: { message_id: number; file_name: string; media_type: string; date: number; caption?: string; grouped_id?: number },
): string {
  const d = new Date((item.date || Date.now() / 1000) * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  const dot = item.file_name.lastIndexOf(".");
  const vars: Record<string, string> = {
    chat: sanitize(chat.title),
    chat_id: String(chat.id),
    msg_id: String(item.message_id),
    date: `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`,
    time: `${pad(d.getHours())}-${pad(d.getMinutes())}-${pad(d.getSeconds())}`,
    file_name: sanitize(item.file_name),
    ext: dot > 0 ? item.file_name.slice(dot + 1) : "",
    type: item.media_type,
    caption: sanitize((item.caption ?? "").slice(0, 60)),
    album: item.grouped_id ? String(item.grouped_id) : "",
  };
  return template
    .replace(/\{([a-z_]+)\}/g, (m, k: string) => (k in vars ? vars[k] : m))
    .split("/")
    .map((p) => p.trim())
    .filter(Boolean)
    .join("/");
}

export function splitLinks(text: string): string[] {
  return text
    .split(/[\s,]+/)
    .map((s) => s.trim())
    .filter((s) => /t\.me\/|telegram\.me\/|^tg:\/\//i.test(s));
}

export function listOf<T>(v: T[] | Record<string, unknown> | null | undefined, key: string): T[] {
  if (Array.isArray(v)) return v;
  const inner = v && (v as Record<string, unknown>)[key];
  return Array.isArray(inner) ? (inner as T[]) : [];
}

// ---- formatting ----

export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.min(units.length - 1, Math.floor(Math.log(n) / Math.log(1024)));
  const v = n / 1024 ** i;
  return `${v >= 100 || i === 0 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`;
}

export function formatDate(unix: number): string {
  if (!unix) return "";
  return new Date(unix * 1000).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });
}

export function kindLabel(kind: RouteKind): string {
  switch (kind) {
    case "direct":
      return "Direct";
    case "socks5":
      return "SOCKS5";
    case "http":
      return "HTTP";
    case "mt_proxy":
      return "MTProto";
    case "shadowsocks":
      return "Shadowsocks";
    case "trojan":
      return "Trojan";
  }
}

/** i18n key suffix under `telegram.net.source` plus the value to interpolate. */
export function sourceInfo(s: RouteSource): { key: string; value: string } {
  switch (s.type) {
    case "local_app":
      return { key: "local_app", value: s.app };
    case "clash":
      return { key: "local_app", value: "Clash" };
    case "env":
      return { key: "env", value: s.var };
    case "subscription":
      return { key: "subscription", value: hostOf(s.url) };
    case "public_list":
      return s.list === RESCUE_LIST ? { key: "rescue", value: "" } : { key: "public_list", value: s.list };
    default:
      return { key: s.type, value: "" };
  }
}

function hostOf(url: string): string {
  try {
    return new URL(url).host || url;
  } catch {
    return url;
  }
}

