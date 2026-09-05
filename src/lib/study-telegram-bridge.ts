import { invoke } from "@tauri-apps/api/core";
import { pluginInvoke } from "$lib/plugin-invoke";

export type TelegramChatType = "private" | "group" | "channel";

export type TelegramChat = {
  id: number;
  title: string;
  chat_type: TelegramChatType;
  peer_hash?: number;
  last_message?: string;
  last_message_date?: number;
  unread_count?: number;
  is_muted?: boolean;
  is_pinned?: boolean;
  is_verified?: boolean;
  is_online?: boolean;
};

export type TelegramMediaType =
  | "photo"
  | "video"
  | "document"
  | "audio"
  | "webpage"
  | "gif"
  | "round_video"
  | "round_voice"
  | "photo_video"
  | "chat_photos";

export type TelegramWebpageInfo = {
  url: string;
  site_name?: string;
  title?: string;
  description?: string;
  embed_url?: string;
  embed_type?: string;
  duration_sec?: number;
};

export type TelegramMediaItem = {
  message_id: number;
  file_name: string;
  file_size: number;
  media_type: string;
  date: number;
  webpage?: TelegramWebpageInfo;
  caption?: string;
  grouped_id?: number;
};

export type TelegramQrStart = { svg: string; expires: number };

export type TelegramSelf = {
  user_id: number;
  username: string | null;
  first_name: string;
  last_name: string | null;
  phone: string | null;
};

export function telegramGetSelf(): Promise<TelegramSelf> {
  return pluginInvoke<TelegramSelf>("telegram", "telegram_get_self");
}

export type TelegramStreamInfo = {
  port: number;
  token: string;
  base_url: string;
};

let cachedStreamInfo: TelegramStreamInfo | null = null;

export async function telegramStreamInfo(): Promise<TelegramStreamInfo> {
  if (cachedStreamInfo) return cachedStreamInfo;
  cachedStreamInfo = await pluginInvoke<TelegramStreamInfo>(
    "telegram",
    "telegram_stream_info",
  );
  return cachedStreamInfo;
}

export async function telegramStreamUrl(args: {
  chatId: number;
  chatType: TelegramChatType;
  messageId: number;
}): Promise<string> {
  const info = await telegramStreamInfo();
  return `${info.base_url}/stream/${args.chatType}/${args.chatId}/${args.messageId}?token=${info.token}`;
}

export function telegramDeleteMessages(args: {
  chatId: number;
  chatType: TelegramChatType;
  messageIds: number[];
  revoke?: boolean;
}): Promise<{ deleted: number }> {
  return pluginInvoke<{ deleted: number }>(
    "telegram",
    "telegram_delete_messages",
    args,
  );
}

export type TelegramForumTopic = {
  id: number;
  title: string;
  icon_color: number;
  icon_emoji_id?: number | null;
  is_closed?: boolean;
  is_hidden?: boolean;
};

export type TelegramForumTopicsResult = {
  is_forum: boolean;
  topics: TelegramForumTopic[];
};

export function telegramListForumTopics(args: {
  chatId: number;
  chatType: TelegramChatType;
  query?: string;
}): Promise<TelegramForumTopicsResult> {
  return pluginInvoke<TelegramForumTopicsResult>(
    "telegram",
    "telegram_list_forum_topics",
    args,
  );
}

/** Parsed shape of the plugin's forward-error string once JSON.parse'd -
 * see `sanitize_forward_error` in the plugin's `lib.rs`. Forward errors that
 * aren't from the plugin's own classifier (e.g. a Tauri transport failure)
 * won't parse as this and should fall back to a generic message. */
export type TelegramForwardErrorInfo = {
  code:
    | "flood_wait"
    | "topic_closed"
    | "worker_busy"
    | "write_forbidden"
    | "topic_or_message_gone"
    | "network"
    | "unknown";
  retry_after_secs?: number | null;
  message: string;
};

export function parseTelegramForwardError(
  raw: unknown,
): TelegramForwardErrorInfo | null {
  if (typeof raw !== "string") return null;
  try {
    const parsed = JSON.parse(raw);
    if (
      parsed &&
      typeof parsed === "object" &&
      typeof parsed.code === "string" &&
      typeof parsed.message === "string"
    ) {
      return parsed as TelegramForwardErrorInfo;
    }
  } catch {
    /* not JSON - a plain string error (missing args, "Not authenticated", etc.) */
  }
  return null;
}

/** Emitted by the plugin between forward batches (only when the selection
 * needed more than one `messages.forwardMessages` call). Listen for
 * `"telegram-forward-progress"` via `@tauri-apps/api/event` while a forward
 * is in flight. */
export type TelegramForwardProgress = {
  forwarded: number;
  total: number;
  batch: number;
  batches_total: number;
  retry_after_secs?: number;
  retry_attempt?: number;
};

export function telegramForwardMessages(args: {
  fromChatId: number;
  fromChatType: TelegramChatType;
  toChatId: number;
  toChatType: TelegramChatType;
  messageIds: number[];
  dropAuthor?: boolean;
  dropCaptions?: boolean;
  /** Forum topic to forward into. Omit for ordinary chats, private chats,
   * plain channels, and Saved Messages - passing nothing reproduces the
   * exact old (pre-forum-support) behavior. */
  topicId?: number;
}): Promise<{ ok: true; forwarded: number; skipped: number; total: number }> {
  return pluginInvoke<{ ok: true; forwarded: number; skipped: number; total: number }>(
    "telegram",
    "telegram_forward_messages",
    args,
  );
}

export function telegramEditCaption(args: {
  chatId: number;
  chatType: TelegramChatType;
  messageId: number;
  caption: string;
}): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>(
    "telegram",
    "telegram_edit_caption",
    args,
  );
}

export function telegramSearchGlobal(args: {
  query: string;
  limit?: number;
}): Promise<TelegramMediaItem[]> {
  return pluginInvoke<TelegramMediaItem[]>(
    "telegram",
    "telegram_search_global",
    args,
  );
}

export type TelegramGlobalSearchHit = TelegramMediaItem & {
  chat_id: number;
  chat_type: TelegramChatType;
  chat_title: string;
};

export function telegramSearchGlobalHits(args: {
  query: string;
  limit?: number;
}): Promise<TelegramGlobalSearchHit[]> {
  return pluginInvoke<TelegramGlobalSearchHit[]>(
    "telegram",
    "telegram_search_global_hits",
    args,
  );
}

export function telegramUploadMedia(args: {
  chatId: number;
  chatType: TelegramChatType;
  filePath: string;
  caption?: string;
}): Promise<{ id: number; file_name: string }> {
  return pluginInvoke<{ id: number; file_name: string }>(
    "telegram",
    "telegram_upload_media",
    args,
  );
}

export function studyUploadsCreate(args: {
  chatId: number;
  chatType: TelegramChatType;
  sourcePath: string;
  fileSize?: number;
}): Promise<{ id: number }> {
  return pluginInvoke<{ id: number }>(
    "study",
    "study:library:telegram:uploads:create",
    args,
  );
}

export async function enqueueTelegramUpload(args: {
  chat: TelegramChat;
  filePath: string;
  fileSize: number;
  caption?: string;
}): Promise<{ studyId: number; telegramId: number }> {
  const created = await studyUploadsCreate({
    chatId: args.chat.id,
    chatType: args.chat.chat_type,
    sourcePath: args.filePath,
    fileSize: args.fileSize,
  });
  const tg = await telegramUploadMedia({
    chatId: args.chat.id,
    chatType: args.chat.chat_type,
    filePath: args.filePath,
    caption: args.caption,
  });
  await studyDownloadsUpdate({
    id: created.id,
    status: "downloading",
  });
  return { studyId: created.id, telegramId: tg.id };
}

export function detectEmbedPlatform(
  url: string,
  siteName?: string | null,
): "youtube" | "vimeo" | "twitch" | "loom" | null {
  const u = url.toLowerCase();
  const sn = (siteName ?? "").toLowerCase();
  if (sn.includes("youtube") || /youtube\.com|youtu\.be/.test(u)) return "youtube";
  if (sn.includes("vimeo") || /vimeo\.com/.test(u)) return "vimeo";
  if (sn.includes("twitch") || /twitch\.tv/.test(u)) return "twitch";
  if (sn.includes("loom") || /loom\.com/.test(u)) return "loom";
  return null;
}

export function buildEmbedUrl(
  platform: "youtube" | "vimeo" | "twitch" | "loom",
  url: string,
): string | null {
  if (platform === "youtube") {
    const m = url.match(/(?:v=|youtu\.be\/|\/embed\/)([\w-]{11})/);
    return m ? `https://www.youtube.com/embed/${m[1]}` : null;
  }
  if (platform === "vimeo") {
    const m = url.match(/vimeo\.com\/(\d+)/);
    return m ? `https://player.vimeo.com/video/${m[1]}` : null;
  }
  if (platform === "loom") {
    const m = url.match(/loom\.com\/share\/([\w-]+)/);
    return m ? `https://www.loom.com/embed/${m[1]}` : null;
  }
  return null;
}

export type TelegramSessionState =
  | { kind: "authenticated"; phone: string }
  | { kind: "unauthenticated" }
  | { kind: "plugin_missing" };

type PluginInfo = { id: string; enabled: boolean; loaded: boolean };

export async function telegramSessionState(): Promise<TelegramSessionState> {
  let pluginPresent = false;
  try {
    const plugins = await invoke<PluginInfo[]>("list_plugins");
    pluginPresent = plugins.some(
      (p) => p.id === "telegram" && p.enabled && p.loaded,
    );
  } catch {
    pluginPresent = true;
  }
  if (!pluginPresent) return { kind: "plugin_missing" };
  try {
    const phone = await pluginInvoke<string>("telegram", "telegram_check_session");
    return { kind: "authenticated", phone };
  } catch {
    return { kind: "unauthenticated" };
  }
}

export function telegramQrStart(): Promise<TelegramQrStart> {
  return pluginInvoke<TelegramQrStart>("telegram", "telegram_qr_start");
}

export function telegramQrPoll(): Promise<string> {
  return pluginInvoke<string>("telegram", "telegram_qr_poll");
}

export function telegramSendCode(phone: string): Promise<null> {
  return pluginInvoke<null>("telegram", "telegram_send_code", { phone });
}

export function telegramVerifyCode(code: string): Promise<string> {
  return pluginInvoke<string>("telegram", "telegram_verify_code", { code });
}

export function telegramVerify2fa(password: string): Promise<string> {
  return pluginInvoke<string>("telegram", "telegram_verify_2fa", { password });
}

export function telegramLogout(): Promise<null> {
  return pluginInvoke<null>("telegram", "telegram_logout");
}

export type ChatPageOffset = {
  date: number;
  id: number;
  peer_id: number;
  peer_type: string;
  peer_hash: number;
};

export type ChatPage = {
  chats: TelegramChat[];
  next_offset: ChatPageOffset | null;
};

export function telegramListChatsPage(args: {
  offsetDate?: number;
  offsetId?: number;
  offsetPeer?: { peer_id: number; peer_type: string; peer_hash: number };
  limit?: number;
}): Promise<ChatPage> {
  return pluginInvoke<ChatPage>("telegram", "telegram_list_chats_page", args);
}

let chatsCache: { items: TelegramChat[]; ts: number } | null = null;
const CHATS_CACHE_TTL_MS = 5 * 60 * 1000;

export async function telegramListChats(force = false): Promise<TelegramChat[]> {
  if (!force && chatsCache && Date.now() - chatsCache.ts < CHATS_CACHE_TTL_MS) {
    return chatsCache.items;
  }
  const items = await pluginInvoke<TelegramChat[]>(
    "telegram",
    "telegram_list_chats",
  );
  chatsCache = { items, ts: Date.now() };
  return items;
}

export const telegramGetChats = telegramListChats;

export function invalidateChatsCache() {
  chatsCache = null;
}

export type StudyCachedChat = {
  id: number;
  chat_type: TelegramChatType;
  title: string;
  photo_b64: string | null;
  sort_order: number;
  fetched_at: number;
  peer_hash: number;
  last_message?: string | null;
  last_message_date?: number | null;
  unread_count?: number;
  is_muted?: boolean;
  is_pinned?: boolean;
  is_verified?: boolean;
};

export function telegramRestorePeerHashes(items: [number, number][]): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>("telegram", "telegram_restore_peer_hashes", {
    items,
  });
}

export function telegramCreateFolder(name: string): Promise<TelegramChat> {
  return pluginInvoke<TelegramChat>("telegram", "telegram_create_folder", { name });
}

export function telegramDeleteChannel(chatId: number): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>("telegram", "telegram_delete_channel", { chatId });
}

export function telegramRenameChannel(args: {
  chatId: number;
  title: string;
}): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>("telegram", "telegram_rename_channel", args);
}

export function telegramLeaveChannel(args: {
  chatId: number;
  chatType: TelegramChatType;
}): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>("telegram", "telegram_leave_channel", args);
}

export function telegramDeleteHistory(args: {
  chatId: number;
  chatType: TelegramChatType;
  justClear?: boolean;
  revoke?: boolean;
}): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>("telegram", "telegram_delete_history", {
    chatId: args.chatId,
    chatType: args.chatType,
    justClear: args.justClear ?? false,
    revoke: args.revoke ?? false,
  });
}

export type TelegramParticipantRole =
  | "creator"
  | "admin"
  | "member"
  | "banned"
  | "left"
  | "self";

export type TelegramParticipant = {
  user_id: number;
  first_name: string;
  last_name: string;
  username: string | null;
  is_bot: boolean;
  role: TelegramParticipantRole;
  joined_at: number | null;
};

export type TelegramParticipantsPage = {
  count: number;
  users: TelegramParticipant[];
};

export type TelegramParticipantFilter =
  | "recent"
  | "admins"
  | "bots"
  | "banned"
  | "restricted"
  | "search";

export function telegramListParticipants(args: {
  chatId: number;
  filter?: TelegramParticipantFilter;
  offset?: number;
  limit?: number;
  search?: string;
}): Promise<TelegramParticipantsPage> {
  return pluginInvoke<TelegramParticipantsPage>(
    "telegram",
    "telegram_list_participants",
    {
      chatId: args.chatId,
      filter: args.filter ?? "recent",
      offset: args.offset ?? 0,
      limit: args.limit ?? 50,
      search: args.search,
    },
  );
}

export function telegramSetBlocked(args: {
  chatId: number;
  chatType: TelegramChatType;
  blocked: boolean;
}): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>("telegram", "telegram_set_blocked", args);
}

export function telegramReportPeer(args: {
  chatId: number;
  chatType: TelegramChatType;
  messageIds?: number[];
  option?: number[];
  message?: string;
}): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>("telegram", "telegram_report_peer", {
    chatId: args.chatId,
    chatType: args.chatType,
    messageIds: args.messageIds ?? [],
    option: args.option ?? [],
    message: args.message ?? "",
  });
}

export type TelegramPerfBucket = {
  min_mb: number;
  max_mb: number;
  threads: number;
};

export type TelegramPerfSettings = {
  max_threads: number;
  max_part_size_kb: number;
  buckets: TelegramPerfBucket[];
};

export function telegramPerfGet(): Promise<TelegramPerfSettings> {
  return pluginInvoke<TelegramPerfSettings>("telegram", "telegram_perf_get");
}

export function telegramPerfSet(args: { maxThreads: number }): Promise<{ max_threads: number }> {
  return pluginInvoke<{ max_threads: number }>("telegram", "telegram_perf_set", args);
}

export type TelegramBandwidthStats = {
  used_today_bytes: number;
  total_used_bytes: number;
  quota_bytes: number;
  date: string;
  percentage: number;
};

export function telegramBandwidthStats(): Promise<TelegramBandwidthStats> {
  return pluginInvoke<TelegramBandwidthStats>("telegram", "telegram_bandwidth_stats");
}

export function telegramBandwidthSetQuota(args: { gb: number }): Promise<{ quota_bytes: number; used_today_bytes: number }> {
  return pluginInvoke<{ quota_bytes: number; used_today_bytes: number }>(
    "telegram",
    "telegram_bandwidth_set_quota",
    args,
  );
}

export function telegramBandwidthReset(): Promise<{ used_today_bytes: number; date: string }> {
  return pluginInvoke<{ used_today_bytes: number; date: string }>(
    "telegram",
    "telegram_bandwidth_reset",
  );
}

export type TelegramCloneOptions = {
  delay_ms: number;
  batch_size: number;
  limit?: number | null;
  drop_author: boolean;
  drop_captions: boolean;
};

export type TelegramCloneSession = {
  id: string;
  source_chat_id: number;
  source_chat_type: TelegramChatType;
  source_title: string;
  dest_chat_id: number;
  dest_chat_type: TelegramChatType;
  dest_title: string;
  last_message_id: number;
  total_collected: number;
  cloned_count: number;
  failed_count: number;
  status: "running" | "paused" | "completed" | "error" | "cancelled";
  options: TelegramCloneOptions;
  created_at: number;
  updated_at: number;
  error?: string;
};

export type TelegramCloneStartResult = {
  session_id: string;
  dest_chat_id: number;
  dest_chat_type: TelegramChatType;
  dest_title: string;
};

export type TelegramCloneProgressEvent = {
  session_id: string;
  stage: "fetching" | "cloning" | "completed" | "paused" | "error";
  total: number;
  current: number;
  failed: number;
  last_message_id: number;
  error?: string;
};

export function telegramCloneStart(args: {
  sourceId: number;
  sourceType: TelegramChatType;
  sourceTitle: string;
  destId?: number;
  destType?: TelegramChatType;
  destTitle?: string;
  resumeId?: string;
  options?: Partial<TelegramCloneOptions>;
}): Promise<TelegramCloneStartResult> {
  return pluginInvoke<TelegramCloneStartResult>("telegram", "telegram_clone_start", args);
}

export function telegramCloneStatus(args: { sessionId: string }): Promise<TelegramCloneSession> {
  return pluginInvoke<TelegramCloneSession>("telegram", "telegram_clone_status", args);
}

export function telegramCloneList(): Promise<TelegramCloneSession[]> {
  return pluginInvoke<TelegramCloneSession[]>("telegram", "telegram_clone_list");
}

export function telegramClonePause(args: { sessionId: string }): Promise<{ cancelled: boolean }> {
  return pluginInvoke<{ cancelled: boolean }>("telegram", "telegram_clone_pause", args);
}

export function telegramCloneCancel(args: { sessionId: string }): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>("telegram", "telegram_clone_cancel", args);
}

export function telegramCloneDelete(args: { sessionId: string }): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>("telegram", "telegram_clone_delete", args);
}

export type TelegramAccountProfile = {
  id: string;
  label: string;
  phone_redacted?: string;
  user_id?: number;
  created_at: number;
  updated_at: number;
};

export function telegramAccountsList(): Promise<TelegramAccountProfile[]> {
  return pluginInvoke<TelegramAccountProfile[]>("telegram", "telegram_accounts_list");
}

export function telegramAccountsSaveCurrent(args: {
  label?: string;
  phone?: string;
  userId?: number;
}): Promise<TelegramAccountProfile> {
  return pluginInvoke<TelegramAccountProfile>("telegram", "telegram_accounts_save_current", args);
}

export function telegramAccountsPrepareAdd(args: {
  label: string;
  phone?: string;
  userId?: number;
}): Promise<TelegramAccountProfile> {
  return pluginInvoke<TelegramAccountProfile>("telegram", "telegram_accounts_prepare_add", args);
}

export function telegramAccountsRestore(args: { id: string }): Promise<{ ok: true; needs_restart: boolean }> {
  return pluginInvoke<{ ok: true; needs_restart: boolean }>(
    "telegram",
    "telegram_accounts_restore",
    args,
  );
}

export function telegramAccountsRemove(args: { id: string }): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>("telegram", "telegram_accounts_remove", args);
}

export function telegramAccountsRename(args: { id: string; label: string }): Promise<TelegramAccountProfile> {
  return pluginInvoke<TelegramAccountProfile>("telegram", "telegram_accounts_rename", args);
}

export function telegramAccountsBackupNow(): Promise<{ path: string; name: string }> {
  return pluginInvoke<{ path: string; name: string }>("telegram", "telegram_accounts_backup_now");
}

export function telegramAccountsListBackups(): Promise<Array<{ name: string; modified_at: number }>> {
  return pluginInvoke<Array<{ name: string; modified_at: number }>>(
    "telegram",
    "telegram_accounts_list_backups",
  );
}

export type TelegramSyncState = {
  enabled: boolean;
  interval_min: number;
  last_success_at: number;
  last_duration_ms: number;
  last_updated_count: number;
  is_syncing: boolean;
};

export function telegramSyncState(): Promise<TelegramSyncState> {
  return pluginInvoke<TelegramSyncState>("telegram", "telegram_sync_state");
}

export function telegramSyncNow(): Promise<{ updated: number }> {
  return pluginInvoke<{ updated: number }>("telegram", "telegram_sync_now");
}

export function telegramSyncSettingsSet(args: { enabled?: boolean; intervalMin?: number }): Promise<TelegramSyncState> {
  return pluginInvoke<TelegramSyncState>("telegram", "telegram_sync_settings_set", args);
}

export function isOmnigetFolder(chat: TelegramChat): boolean {
  if (chat.chat_type !== "channel") return false;
  return /\[og\]\s*$/i.test(chat.title);
}

export function studyChatsCacheGet(): Promise<{ chats: StudyCachedChat[] }> {
  return pluginInvoke<{ chats: StudyCachedChat[] }>(
    "study",
    "study:library:telegram:chats:cache_get",
  );
}

export function studyChatsCacheUpsert(args: {
  chats: TelegramChat[];
  startingSortOrder?: number;
  replace?: boolean;
}): Promise<{ ok: true; count: number }> {
  return pluginInvoke<{ ok: true; count: number }>(
    "study",
    "study:library:telegram:chats:cache_upsert",
    args,
  );
}

export function studyChatsCacheSetPhoto(args: {
  chatId: number;
  photoB64: string | null;
}): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>(
    "study",
    "study:library:telegram:chats:cache_set_photo",
    args,
  );
}

export function studyChatsCacheClear(): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>(
    "study",
    "study:library:telegram:chats:cache_clear",
  );
}

export type PlaybackState = {
  position_seconds: number;
  duration_seconds: number;
  finished: boolean;
  last_played_at: number;
  title: string | null;
};

export type PlaybackStateRow = {
  message_id: number;
  position_seconds: number;
  duration_seconds: number;
  finished: boolean;
  last_played_at: number;
};

export async function studyPlaybackGet(args: {
  chatId: number;
  messageId: number;
}): Promise<PlaybackState | null> {
  const r = await pluginInvoke<PlaybackState | null>(
    "study",
    "study:library:telegram:playback:get",
    args,
  );
  return r;
}

export function studyPlaybackSave(args: {
  chatId: number;
  messageId: number;
  positionSeconds: number;
  durationSeconds?: number;
  finished?: boolean;
  title?: string;
}): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>(
    "study",
    "study:library:telegram:playback:save",
    args,
  );
}

export function studyPlaybackForChat(chatId: number): Promise<{ items: PlaybackStateRow[] }> {
  return pluginInvoke<{ items: PlaybackStateRow[] }>(
    "study",
    "study:library:telegram:playback:for_chat",
    { chatId },
  );
}

export function studyMediaThumbsForChat(args: {
  chatId: number;
  messageIds?: number[];
}): Promise<{ thumbs: Record<string, string> }> {
  return pluginInvoke<{ thumbs: Record<string, string> }>(
    "study",
    "study:library:telegram:media_thumbs:for_chat",
    args,
  );
}

export function studyMediaThumbSet(args: {
  chatId: number;
  messageId: number;
  thumbB64: string;
}): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>(
    "study",
    "study:library:telegram:media_thumbs:set",
    args,
  );
}

const TG_PEER_COLORS: Array<[string, string]> = [
// Paleta oficial de avatares do Telegram (dado da plataforma, não é cor de UI do app).
  ["#ff845e", "#d45246"],
  ["#febb5b", "#f68136"],
  ["#5caffa", "#408acf"],
  ["#9ad164", "#46ba43"],
  ["#5bcbe3", "#359ad4"],
  ["#e5ca77", "#cdb464"],
  ["#ff8aac", "#d95574"],
  ["#b694f9", "#6c61df"],
];

const TG_PEER_INDEX_MAP = [0, 7, 4, 1, 6, 3, 5];

function peerIndex(seed: number | string): number {
  let n: number;
  if (typeof seed === "number") {
    n = Math.abs(seed);
  } else {
    let h = 0;
    for (let i = 0; i < seed.length; i++) {
      h = (h << 5) - h + seed.charCodeAt(i);
      h |= 0;
    }
    n = Math.abs(h);
  }
  return TG_PEER_INDEX_MAP[n % 7];
}

export function avatarColor(seed: number | string): string {
  return TG_PEER_COLORS[peerIndex(seed)][0];
}

export function avatarGradient(seed: number | string): [string, string] {
  return TG_PEER_COLORS[peerIndex(seed)];
}

export function telegramListMedia(args: {
  chatId: number;
  chatType: TelegramChatType;
  mediaType?: TelegramMediaType;
  offset?: number;
  limit?: number;
}): Promise<TelegramMediaItem[]> {
  return pluginInvoke<TelegramMediaItem[]>("telegram", "telegram_list_media", args);
}

export type TelegramDiagListMedia = {
  chat_id: number;
  chat_type: string;
  access_hash_cached: number;
  access_hash_resolved: number;
  auto_resolve_attempted: boolean;
  auto_resolve_outcome: string;
  get_history_count: number;
  get_history_with_media: number;
  get_history_error: string | null;
  search_photo_count: number;
  search_video_count: number;
  search_document_count: number;
  search_audio_count: number;
  search_errors: string[];
  final_count: number;
  elapsed_ms: number;
};

export function telegramDiagListMedia(args: {
  chatId: number;
  chatType: TelegramChatType;
}): Promise<TelegramDiagListMedia> {
  return pluginInvoke<TelegramDiagListMedia>("telegram", "telegram_diag_list_media", args);
}

export function telegramSearchMedia(args: {
  chatId: number;
  chatType: TelegramChatType;
  query: string;
  mediaType?: TelegramMediaType;
  limit?: number;
}): Promise<TelegramMediaItem[]> {
  return pluginInvoke<TelegramMediaItem[]>("telegram", "telegram_search_media", args);
}

export function telegramExpandAlbum(args: {
  chatId: number;
  chatType: TelegramChatType;
  messageId: number;
}): Promise<TelegramMediaItem[]> {
  return pluginInvoke<TelegramMediaItem[]>("telegram", "telegram_expand_album", args);
}

export function telegramDownloadMedia(args: {
  chatId: number;
  chatType: TelegramChatType;
  messageId: number;
  fileName: string;
  outputDir: string;
}): Promise<{ id: number; file_name: string }> {
  return pluginInvoke<{ id: number; file_name: string }>(
    "telegram",
    "telegram_download_media",
    args,
  );
}

export function telegramDownloadBatch(args: {
  chatId: number;
  chatType: TelegramChatType;
  chatTitle: string;
  items: { message_id: number; file_name: string; file_size: number }[];
  outputDir: string;
  resume?: boolean;
}): Promise<number> {
  return pluginInvoke<number>("telegram", "telegram_download_batch", args);
}

export function telegramCancelBatch(batchId: number): Promise<null> {
  return pluginInvoke<null>("telegram", "telegram_cancel_batch", { batchId });
}

export function telegramGetThumbnail(args: {
  chatId: number;
  chatType: TelegramChatType;
  messageId: number;
}): Promise<string> {
  return pluginInvoke<string>("telegram", "telegram_get_thumbnail", args);
}

export function telegramGetChatPhoto(args: {
  chatId: number;
  chatType: TelegramChatType;
}): Promise<string> {
  return pluginInvoke<string>("telegram", "telegram_get_chat_photo", args);
}

export function telegramClearThumbnailCache(): Promise<null> {
  return pluginInvoke<null>("telegram", "telegram_clear_thumbnail_cache");
}

export type DownloadStatus =
  | "queued"
  | "downloading"
  | "done"
  | "error"
  | "cancelled";

export type StudyDownloadRow = {
  id: number;
  chat_id: number;
  chat_type: TelegramChatType;
  message_id: number;
  file_name: string;
  expected_size: number;
  status: DownloadStatus;
  progress_pct: number;
  error_msg: string | null;
  output_path: string | null;
  queued_at: number;
  started_at: number | null;
  finished_at: number | null;
  direction: "download" | "upload";
};

const liveDownloads = new Map<number, number>();

export function bindLiveDownload(telegramId: number, studyId: number) {
  liveDownloads.set(telegramId, studyId);
}

export function studyIdForTelegramId(telegramId: number): number | undefined {
  return liveDownloads.get(telegramId);
}

export function clearLiveDownload(telegramId: number) {
  liveDownloads.delete(telegramId);
}

export function studyDownloadsCreate(args: {
  chatId: number;
  chatType: TelegramChatType;
  messageId: number;
  fileName: string;
  expectedSize?: number;
}): Promise<{ id: number }> {
  return pluginInvoke<{ id: number }>(
    "study",
    "study:library:telegram:downloads:create",
    args,
  );
}

export function studyDownloadsUpdate(args: {
  id: number;
  status?: DownloadStatus;
  progressPct?: number;
  errorMsg?: string;
  outputPath?: string;
}): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>(
    "study",
    "study:library:telegram:downloads:update",
    args,
  );
}

export function studyDownloadsList(
  status?: DownloadStatus,
): Promise<{ downloads: StudyDownloadRow[] }> {
  return pluginInvoke<{ downloads: StudyDownloadRow[] }>(
    "study",
    "study:library:telegram:downloads:list",
    status ? { status } : {},
  );
}

export function studyDownloadsRemove(id: number): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>(
    "study",
    "study:library:telegram:downloads:remove",
    { id },
  );
}

export function studyDownloadsClearFinished(): Promise<{ removed: number }> {
  return pluginInvoke<{ removed: number }>(
    "study",
    "study:library:telegram:downloads:clear_finished",
  );
}

export function studyDefaultOutputDir(): Promise<{ path: string }> {
  return pluginInvoke<{ path: string }>(
    "study",
    "study:library:telegram:default_output_dir",
  );
}

export type LibraryTelegramSettings = {
  allow_remote_delete: boolean;
  auto_resume_downloads: boolean;
  thumb_cache_limit_mb?: number;
  bulk_confirm_threshold?: number;
  resume_batch?: boolean;
  takeout_for_protected?: boolean;
  default_output_dir?: string | null;
};

export function telegramTakeoutStart(): Promise<{ takeout_id: number }> {
  return pluginInvoke<{ takeout_id: number }>("telegram", "telegram_takeout_start");
}

export function telegramTakeoutFinish(args: { success: boolean }): Promise<null> {
  return pluginInvoke<null>("telegram", "telegram_takeout_finish", args);
}

export type TelegramFullChannelInfo = {
  chat_id: number;
  title: string;
  about: string;
  username?: string;
  participants_count?: number;
};

export function telegramFullChannelInfo(args: {
  chatId: number;
  chatType: TelegramChatType;
}): Promise<TelegramFullChannelInfo> {
  return pluginInvoke<TelegramFullChannelInfo>("telegram", "telegram_full_channel_info", args);
}

export function telegramCancelOp(args: { opId: string }): Promise<{ cancelled: boolean }> {
  return pluginInvoke<{ cancelled: boolean }>("telegram", "telegram_cancel_op", args);
}

export type FolderAutoscanResult = {
  detected: Array<{ chat_id: number; chat_type: string; title: string; matched_via: string }>;
  needs_about_check: Array<{ chat_id: number; chat_type: string; title: string }>;
  note: string;
};

export function studyTgFolderAutoscan(): Promise<FolderAutoscanResult> {
  return pluginInvoke<FolderAutoscanResult>("study", "study:library:tg:folder_autoscan");
}

export function telegramSetMute(args: {
  chatId: number;
  chatType: TelegramChatType;
  muteUntil: number;
}): Promise<null> {
  return pluginInvoke<null>("telegram", "telegram_set_mute", args);
}

export function telegramTogglePin(args: {
  chatId: number;
  chatType: TelegramChatType;
  pinned: boolean;
}): Promise<null> {
  return pluginInvoke<null>("telegram", "telegram_toggle_pin", args);
}

export function telegramSetArchived(args: {
  chatId: number;
  chatType: TelegramChatType;
  archived: boolean;
}): Promise<null> {
  return pluginInvoke<null>("telegram", "telegram_set_archived", args);
}

export function telegramReorderPinned(args: {
  items: Array<{ chat_id: number; chat_type: TelegramChatType }>;
}): Promise<null> {
  return pluginInvoke<null>("telegram", "telegram_reorder_pinned", args);
}

export function studyLibraryTelegramSettingsGet(): Promise<LibraryTelegramSettings> {
  return pluginInvoke<LibraryTelegramSettings>(
    "study",
    "study:library:telegram:settings:get",
  );
}

export function studyLibraryTelegramSettingsSave(
  patch: Partial<LibraryTelegramSettings>,
): Promise<LibraryTelegramSettings> {
  return pluginInvoke<LibraryTelegramSettings>(
    "study",
    "study:library:telegram:settings:save",
    { patch },
  );
}

export function studyReadRegisterFile(filePath: string): Promise<{
  id: number;
  format: string;
}> {
  return pluginInvoke<{ id: number; format: string }>(
    "study",
    "study:read:scanner:register_file",
    { filePath },
  );
}

export type StudyBookmarkRow = {
  id: number;
  chat_id: number;
  chat_type: TelegramChatType;
  message_id: number;
  title: string;
  file_size: number;
  mime_type: string | null;
  thumb_b64: string | null;
  note: string | null;
  added_at: number;
  tags: string[];
};

export type TagSummary = { tag: string; count: number };

export function studyBookmarksToggle(args: {
  chatId: number;
  chatType: TelegramChatType;
  messageId: number;
  title: string;
  fileSize?: number;
  mimeType?: string;
  thumbB64?: string;
}): Promise<{ bookmarked: boolean; id: number | null }> {
  return pluginInvoke<{ bookmarked: boolean; id: number | null }>(
    "study",
    "study:library:telegram:bookmarks:toggle",
    args,
  );
}

export function studyBookmarksList(): Promise<{ bookmarks: StudyBookmarkRow[] }> {
  return pluginInvoke<{ bookmarks: StudyBookmarkRow[] }>(
    "study",
    "study:library:telegram:bookmarks:list",
  );
}

export function studyBookmarksSetNote(
  id: number,
  note: string | null,
): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>(
    "study",
    "study:library:telegram:bookmarks:set_note",
    { id, note },
  );
}

export function studyTagsAdd(args: {
  chatId: number;
  messageId: number;
  tag: string;
}): Promise<{ ok: true; tag: string }> {
  return pluginInvoke<{ ok: true; tag: string }>(
    "study",
    "study:library:telegram:tags:add",
    args,
  );
}

export function studyTagsRemove(args: {
  chatId: number;
  messageId: number;
  tag: string;
}): Promise<{ ok: true }> {
  return pluginInvoke<{ ok: true }>(
    "study",
    "study:library:telegram:tags:remove",
    args,
  );
}

export function studyTagsListForMessage(args: {
  chatId: number;
  messageId: number;
}): Promise<{ tags: string[] }> {
  return pluginInvoke<{ tags: string[] }>(
    "study",
    "study:library:telegram:tags:list_for_message",
    args,
  );
}

export function studyTagsAll(): Promise<{ tags: TagSummary[] }> {
  return pluginInvoke<{ tags: TagSummary[] }>(
    "study",
    "study:library:telegram:tags:all",
  );
}

export type SmartFolderId =
  | "pdfs"
  | "videos"
  | "audios"
  | "images"
  | "untagged"
  | "yt-courses";

export function categorizeMedia(
  mimeType: string | null | undefined,
  fileName: string,
): "video" | "audio" | "image" | "pdf" | "archive" | "other" {
  const ext = fileName.split(".").pop()?.toLowerCase() ?? "";
  const mt = (mimeType ?? "").toLowerCase();
  if (
    mt === "video" ||
    ["mp4", "mkv", "webm", "avi", "mov", "m4v", "ts"].includes(ext)
  )
    return "video";
  if (
    mt === "audio" ||
    ["mp3", "flac", "ogg", "wav", "m4a", "opus", "aac"].includes(ext)
  )
    return "audio";
  if (
    mt === "photo" ||
    ["jpg", "jpeg", "png", "webp", "gif", "bmp", "heic"].includes(ext)
  )
    return "image";
  if (ext === "pdf") return "pdf";
  if (["zip", "rar", "7z", "tar", "gz", "bz2", "xz"].includes(ext))
    return "archive";
  return "other";
}

export function bookmarkMatchesSmartFolder(
  bookmark: StudyBookmarkRow,
  folderId: SmartFolderId,
): boolean {
  const cat = categorizeMedia(bookmark.mime_type, bookmark.title);
  switch (folderId) {
    case "pdfs":
      return cat === "pdf";
    case "videos":
      return cat === "video";
    case "audios":
      return cat === "audio";
    case "images":
      return cat === "image";
    case "untagged":
      return bookmark.tags.length === 0;
    case "yt-courses":
      return (
        (bookmark.mime_type ?? "").toLowerCase() === "webpage" &&
        bookmark.title.toLowerCase().includes("youtube")
      );
    default:
      return false;
  }
}

export async function enqueueTelegramDownload(args: {
  chatId: number;
  chatType: TelegramChatType;
  messageId: number;
  fileName: string;
  fileSize: number;
  outputDir: string;
}): Promise<{ studyId: number; telegramId: number }> {
  const created = await studyDownloadsCreate({
    chatId: args.chatId,
    chatType: args.chatType,
    messageId: args.messageId,
    fileName: args.fileName,
    expectedSize: args.fileSize,
  });
  const tg = await telegramDownloadMedia({
    chatId: args.chatId,
    chatType: args.chatType,
    messageId: args.messageId,
    fileName: args.fileName,
    outputDir: args.outputDir,
  });
  bindLiveDownload(tg.id, created.id);
  await studyDownloadsUpdate({
    id: created.id,
    status: "downloading",
  });
  return { studyId: created.id, telegramId: tg.id };
}

const liveBatches = new Map<number, Map<number, number>>();

export function studyIdForBatchItem(
  batchId: number,
  messageId: number,
): number | undefined {
  return liveBatches.get(batchId)?.get(messageId);
}

export function clearBatch(batchId: number) {
  liveBatches.delete(batchId);
}

export async function enqueueTelegramDownloadBatch(args: {
  chat: TelegramChat;
  items: { message_id: number; file_name: string; file_size: number }[];
  outputDir: string;
}): Promise<{ batchId: number; studyIds: number[] }> {
  const messageToStudy = new Map<number, number>();
  const studyIds: number[] = [];
  for (const it of args.items) {
    const created = await studyDownloadsCreate({
      chatId: args.chat.id,
      chatType: args.chat.chat_type,
      messageId: it.message_id,
      fileName: it.file_name,
      expectedSize: it.file_size,
    });
    messageToStudy.set(it.message_id, created.id);
    studyIds.push(created.id);
  }
  const batchId = await telegramDownloadBatch({
    chatId: args.chat.id,
    chatType: args.chat.chat_type,
    chatTitle: args.chat.title,
    items: args.items,
    outputDir: args.outputDir,
  });
  liveBatches.set(batchId, messageToStudy);
  for (const id of studyIds) {
    await studyDownloadsUpdate({ id, status: "downloading" });
  }
  return { batchId, studyIds };
}

export const TELEGRAM_FREE_LIMIT_BYTES = 2 * 1024 * 1024 * 1024; // 2.0 GB
export const TELEGRAM_PREMIUM_LIMIT_BYTES = 4 * 1024 * 1024 * 1024; // 4.0 GB

export type TelegramUploadTier = "free" | "premium";
export type TelegramSendAsMode = "video" | "document" | "audio";

export type TelegramUploadArgs = {
  filePath: string;
  destChatId: string | number;
  customFileName?: string;
  customThumbnailPath?: string;
  caption?: string;
  botToken?: string;
  tier?: TelegramUploadTier;
  sendAs?: TelegramSendAsMode;
};

export function calculateTelegramChunks(
  fileSize: number,
  tier: TelegramUploadTier = "free",
): { requiresSplit: boolean; limitBytes: number; partCount: number; partSize: number } {
  const limitBytes = tier === "premium" ? TELEGRAM_PREMIUM_LIMIT_BYTES : TELEGRAM_FREE_LIMIT_BYTES;
  if (fileSize <= limitBytes) {
    return { requiresSplit: false, limitBytes, partCount: 1, partSize: fileSize };
  }
  const partCount = Math.ceil(fileSize / limitBytes);
  const partSize = Math.ceil(fileSize / partCount);
  return { requiresSplit: true, limitBytes, partCount, partSize };
}

export function translateTelegramError(errorString: string, tr?: (key: string) => string): string {
  const lower = errorString.toLowerCase();
  const t = tr ?? ((k) => k);

  if (lower.includes("blocked by the user") || lower.includes("can't initiate conversation") || lower.includes("bot was blocked")) {
    return t("telegram.err_bot_not_started");
  }
  if (lower.includes("not a member") || lower.includes("not an admin") || lower.includes("kicked") || lower.includes("rights to send") || lower.includes("not enough rights")) {
    return t("telegram.err_bot_not_member");
  }
  if (lower.includes("chat not found") || lower.includes("chat_invalid")) {
    return t("telegram.err_chat_not_found");
  }
  if (lower.includes("unauthorized") || lower.includes("invalid token")) {
    return t("telegram.err_unauthorized");
  }
  if (lower.includes("file_too_large") || lower.includes("entity too large") || lower.includes("file is too big")) {
    return t("telegram.err_bot_file_too_large");
  }
  if (lower.includes("too many requests") || lower.includes("retry after")) {
    return t("telegram.err_rate_limit");
  }
  return errorString;
}

export async function telegramUploadFile(args: TelegramUploadArgs): Promise<{ success: boolean; messageId?: number }> {
  const sendAs = args.sendAs ?? "video";
  if (args.botToken) {
    return pluginInvoke<{ success: boolean; messageId?: number }>("telegram", "telegram_bot_upload_file", {
      botToken: args.botToken,
      chatId: String(args.destChatId),
      filePath: args.filePath,
      customFileName: args.customFileName ?? null,
      customThumbnailPath: args.customThumbnailPath ?? null,
      caption: args.caption ?? null,
      sendAs,
    }).catch(async () => {
      return invoke<{ success: boolean; messageId?: number }>("telegram_bot_upload_file", {
        botToken: args.botToken,
        chatId: String(args.destChatId),
        filePath: args.filePath,
        customFileName: args.customFileName ?? null,
        customThumbnailPath: args.customThumbnailPath ?? null,
        caption: args.caption ?? null,
        sendAs,
      });
    });
  }
  return pluginInvoke<{ success: boolean; messageId?: number }>("telegram", "telegram_user_upload_file", {
    chatId: args.destChatId,
    filePath: args.filePath,
    customFileName: args.customFileName ?? null,
    customThumbnailPath: args.customThumbnailPath ?? null,
    caption: args.caption ?? null,
    sendAs,
  }).catch(async () => {
    return invoke<{ success: boolean; messageId?: number }>("telegram_user_upload_file", {
      chatId: args.destChatId,
      filePath: args.filePath,
      customFileName: args.customFileName ?? null,
      customThumbnailPath: args.customThumbnailPath ?? null,
      caption: args.caption ?? null,
      sendAs,
    });
  });
}
