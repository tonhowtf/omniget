import { invoke } from "@tauri-apps/api/core";

export type DcUser = { id: string; username: string; global_name?: string | null; avatar?: string | null; bot?: boolean };
export type DcAttachment = { id: string; filename: string; url: string; proxy_url?: string | null; size: number; content_type?: string | null; width?: number | null; height?: number | null };
export type DcEmbedMedia = { url: string; proxy_url?: string | null; width?: number | null; height?: number | null };
export type DcEmbed = {
  type?: string | null;
  title?: string | null;
  description?: string | null;
  url?: string | null;
  color?: number | null;
  author?: { name: string } | null;
  image?: DcEmbedMedia | null;
  thumbnail?: DcEmbedMedia | null;
  video?: DcEmbedMedia | null;
};
export type DcMessage = {
  id: string;
  channel_id: string;
  guild_id?: string | null;
  author: DcUser;
  content: string;
  timestamp: string;
  edited_timestamp?: string | null;
  attachments: DcAttachment[];
  embeds: DcEmbed[];
  mentions: DcUser[];
  mention_roles: string[];
  mention_everyone: boolean;
  type: number;
  sticker_items: { id: string; name: string }[];
  referenced_message?: DcMessage | null;
};
export type DcChannel = {
  id: string;
  guild_id?: string | null;
  kind: number;
  name: string;
  position: number;
  parent_id?: string | null;
  last_message_id?: string | null;
  topic?: string | null;
  can_send: boolean;
  icon?: string | null;
  recipients: DcUser[];
};
export type DcGuild = { id: string; name: string; icon?: string | null; channels: DcChannel[] };
export type DcReadState = { last_message_id?: string | null; mention_count: number };
export type DcSnapshot = { me: DcUser; guilds: DcGuild[]; dms: DcChannel[]; read_states: Record<string, DcReadState> };
export type DcAccount = { id: string; username: string; global_name?: string | null; avatar?: string | null };
export type DcConnection = { state: "offline" | "connecting" | "ready" | "reconnecting" | "failed"; reason?: string; retry_in_ms?: number; resumed?: boolean };
export type DcStatus = { accounts: DcAccount[]; active: string | null; connected_account: string | null; connection: DcConnection };
export type DcSaved = { folder: string; paths: string[]; failed: number; skipped: number; cancelled: boolean };
export type DcExported = { folder: string; files: string[]; messages: number; cancelled: boolean };
export type MediaKind = "image" | "video" | "audio" | "file";
export type MediaFilter = { kinds: MediaKind[]; after_ms?: number | null; before_ms?: number | null; include_embeds: boolean };
export type ExportFormat = "html" | "json" | "txt";
export type ExportOptions = { formats: ExportFormat[]; after_ms?: number | null; before_ms?: number | null; include_media: boolean };
export type QrEvent =
  | { stage: "code"; svg: string; url: string }
  | { stage: "scanned"; user: { id: string; discriminator: string; username: string; avatar?: string | null } }
  | { stage: "done"; account: DcAccount }
  | { stage: "cancelled" }
  | { stage: "expired" }
  | { stage: "error"; error: string };
export type DownloadProgress = { job_id: string; phase: "scan" | "download" | "media" | "done" | "cancelled" | "error"; scanned?: number; found?: number; done?: number; skipped?: number; failed?: number; bytes?: number; error?: string };
export type MessageCreateEvent = { message: DcMessage; guild_id?: string | null };
export type MessageUpdateEvent = { message: DcMessage; partial: boolean };
export type MessageDeleteEvent = { id: string; channel_id: string };
export type MessageDeleteBulkEvent = { ids: string[]; channel_id: string };
export type ReadEvent = { channel_id: string; message_id: string; mention_count?: number | null };

export const status = () => invoke<DcStatus>("discord_status");
export const connect = () => invoke<DcStatus>("discord_connect");
export const disconnect = () => invoke<DcStatus>("discord_disconnect");
export const loginToken = (token: string) => invoke<DcAccount>("discord_login_token", { token });
export const qrStart = () => invoke<void>("discord_qr_start");
export const qrCancel = () => invoke<void>("discord_qr_cancel");
export const browserLogin = () => invoke<void>("discord_browser_login");
export const logout = (accountId: string) => invoke<DcStatus>("discord_logout", { accountId });
export const switchAccount = (accountId: string) => invoke<DcStatus>("discord_switch_account", { accountId });
export const snapshot = () => invoke<DcSnapshot | null>("discord_snapshot");
export const messages = (channelId: string, before?: string | null, limit = 50) =>
  invoke<DcMessage[]>("discord_messages", { channelId, before: before ?? null, limit });
export const send = (channelId: string, content: string, replyTo?: string | null) =>
  invoke<DcMessage>("discord_send", { channelId, content, replyTo: replyTo ?? null });
export const edit = (channelId: string, messageId: string, content: string) => invoke<DcMessage>("discord_edit", { channelId, messageId, content });
export const remove = (channelId: string, messageId: string) => invoke<void>("discord_delete", { channelId, messageId });
export const ack = (channelId: string, messageId: string) => invoke<void>("discord_ack", { channelId, messageId });
export const downloadMessage = (channelId: string, messageId: string, dir?: string | null) =>
  invoke<DcSaved>("discord_download_message", { channelId, messageId, dir: dir || null });
export const downloadChannel = (jobId: string, channelId: string, dir: string | null, filter: MediaFilter) =>
  invoke<DcSaved>("discord_download_channel", { jobId, channelId, dir: dir || null, filter });
export const exportChannel = (jobId: string, channelId: string, dir: string | null, options: ExportOptions) =>
  invoke<DcExported>("discord_export", { jobId, channelId, dir: dir || null, options });
export const cancelJob = (jobId: string) => invoke<void>("discord_cancel", { jobId });
export const reveal = (path: string) => invoke<void>("reveal_file", { path });

export function errText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e && typeof e === "object" && "message" in e) return String((e as { message: unknown }).message);
  return String(e);
}

export function errKey(e: unknown): string {
  const raw = errText(e);
  if (raw.startsWith("rate_limited")) return "rate_limited";
  if (raw.startsWith("network")) return "network";
  if (raw.startsWith("forbidden")) return "forbidden";
  if (raw.startsWith("not_found")) return "not_found";
  if (raw.startsWith("http_")) return "http";
  if (raw.startsWith("refused_")) return "refused";
  if (raw.startsWith("remote auth")) return "remote_auth";
  const known = ["unauthorized", "token_format", "bot_token", "captcha_required", "not_connected", "no_token", "no_media", "too_long", "empty", "unknown_account", "bad_id", "unexpected_response"];
  return known.includes(raw) ? raw : "generic";
}

export function rateLimitSeconds(e: unknown): number {
  const m = /^rate_limited:(\d+)/.exec(errText(e));
  return m ? Number(m[1]) : 0;
}
