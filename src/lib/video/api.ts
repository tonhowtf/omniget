// Typed wrappers for every `video_*` Tauri command. Argument names follow the
// Rust parameters in camelCase (Tauri converts them).

import { invoke } from "@tauri-apps/api/core";
import type {
  AddonInfo,
  AddonMeta,
  AddonStream,
  AniMedia,
  AniViewer,
  Card,
  CastDevice,
  CastStatus,
  CourseDetail,
  CourseSummary,
  Cue,
  EpgProgramme,
  HistoryEntry,
  Home,
  IptvChannel,
  ItemsQuery,
  JfItem,
  JfView,
  MediaItem,
  Note,
  PlaybackInfo,
  PlayMode,
  PlayPrefs,
  Progress,
  ProgressReport,
  Root,
  RootKind,
  ServerInfo,
  ShowDetail,
  ShowSummary,
  
  SourceInfo,
  SourceRef,
  TranscriptHit,
} from "./types";

export type VideoStatus = {
  server: ServerInfo;
  roots: Root[];
  counts: { movies: number; shows: number; courses: number; videos: number };
  scanning: boolean;
  ffmpeg: boolean;
};

let serverCache: ServerInfo | null = null;

export async function status(): Promise<VideoStatus> {
  const s = await invoke<VideoStatus>("video_status");
  serverCache = s.server;
  return s;
}

/** Absolute URL for a server path returned by the backend ("/thumb/12"). */
export function serverUrl(path: string | null | undefined): string | null {
  if (!path) return null;
  if (/^https?:/i.test(path)) return path;
  if (!serverCache) return null;
  const sep = path.includes("?") ? "&" : "?";
  return `http://127.0.0.1:${serverCache.port}${path}${sep}k=${serverCache.token}`;
}

export async function ensureServer(): Promise<ServerInfo> {
  if (serverCache) return serverCache;
  return (await status()).server;
}

// ---- library ----
export const rootsAdd = (path: string, kind: RootKind) => invoke<void>("video_roots_add", { path, kind });
export const rootsRemove = (path: string) => invoke<void>("video_roots_remove", { path });
export const scan = () => invoke<void>("video_scan");
export const items = (query: ItemsQuery) => invoke<MediaItem[]>("video_items", { query });
export const item = (id: number) => invoke<MediaItem>("video_item", { id });
export const home = () => invoke<Home>("video_home");
export const shows = () => invoke<ShowSummary[]>("video_shows");
export const show = (name: string) => invoke<ShowDetail>("video_show", { name });
export const courses = () => invoke<CourseSummary[]>("video_courses");
export const course = (id: number) => invoke<CourseDetail>("video_course", { id });
export const courseImport = (path: string) => invoke<CourseSummary>("video_course_import", { path });
export const courseRemove = (id: number) => invoke<void>("video_course_remove", { id });
export const reveal = (mediaId: number) => invoke<void>("video_reveal", { mediaId });

// ---- playback ----
export const play = (source: SourceRef) => invoke<PlaybackInfo>("video_play", { source });
export const playFallback = (source: SourceRef, failedMode: PlayMode) =>
  invoke<PlaybackInfo>("video_play_fallback", { source, failedMode });
export const progress = (report: ProgressReport) => invoke<Progress>("video_progress", { report });
export const setWatched = (keys: string[], watched: boolean) => invoke<void>("video_set_watched", { keys, watched });
export const continueRemove = (key: string) => invoke<void>("video_continue_remove", { key });
export const history = (limit = 200) => invoke<HistoryEntry[]>("video_history", { limit });
export const historyClear = () => invoke<void>("video_history_clear");
export const prefsSet = (scope: string, prefs: Partial<PlayPrefs>) => invoke<void>("video_prefs_set", { scope, prefs });
export const linkResolve = (url: string) => invoke<PlaybackInfo>("video_link_resolve", { url });
export const openExternal = (source: SourceRef, player: "mpv" | "iina" | "vlc" | null = null) =>
  invoke<void>("video_open_external", { source, player });

// ---- study ----
export const notes = (key: string) => invoke<Note[]>("video_notes", { key });
export const noteAdd = (key: string, tMs: number, text: string, opts: { tEndMs?: number | null; capture?: boolean; mediaId?: number | null } = {}) =>
  invoke<Note>("video_note_add", { key, tMs, tEndMs: opts.tEndMs ?? null, text, capture: opts.capture ?? false, mediaId: opts.mediaId ?? null });
export const noteUpdate = (id: number, text: string) => invoke<void>("video_note_update", { id, text });
export const noteDelete = (id: number) => invoke<void>("video_note_delete", { id });
export const notesExport = (opts: { key?: string | null; courseId?: number | null }) =>
  invoke<string>("video_notes_export", { key: opts.key ?? null, courseId: opts.courseId ?? null });
export const cards = (key: string | null = null) => invoke<Card[]>("video_cards", { key });
export const cardAdd = (key: string, front: string, back: string, tMs: number | null = null, image: string | null = null) =>
  invoke<Card>("video_card_add", { key, front, back, tMs, image });
export const cardReview = (id: number, grade: 0 | 1 | 2 | 3) => invoke<Card>("video_card_review", { id, grade });
export const cardsDue = () => invoke<Card[]>("video_cards_due");
export const cardDelete = (id: number) => invoke<void>("video_card_delete", { id });
export const ankiExport = (ids: number[]) => invoke<{ added: number }>("video_anki_export", { ids });
export const transcript = (key: string, mediaId: number | null = null) => invoke<Cue[]>("video_transcript", { key, mediaId });
export const transcribe = (key: string, mediaId: number | null = null) => invoke<Cue[]>("video_transcribe", { key, mediaId });
export const transcriptSearch = (query: string, courseId: number | null = null) =>
  invoke<TranscriptHit[]>("video_transcript_search", { query, courseId });
/** Silent ranges, in seconds. */
export const silences = (mediaId: number, minS = 0.6, noiseDb = -35) =>
  invoke<[number, number][]>("video_silences", { mediaId, minS, noiseDb });
export const sourceForKey = (key: string) => invoke<SourceRef>("video_source_for_key", { key });
export const openPath = (path: string) => invoke<PlaybackInfo>("video_open_path", { path });
export const ask = (key: string, question: string, mode: "ask" | "summary" | "quiz" | "chapters" = "ask") =>
  invoke<string>("video_ask", { key, question, mode });

// ---- sources ----
export const sources = () => invoke<SourceInfo[]>("video_sources");
export const sourceRemove = (id: number) => invoke<void>("video_source_remove", { id });
export const sourceToggle = (id: number, enabled: boolean) => invoke<void>("video_source_toggle", { id, enabled });

// Jellyfin / Emby
export const jfConnect = (server: string, username: string, password: string) =>
  invoke<SourceInfo>("video_jf_connect", { server, username, password });
export const jfQuickConnectStart = (server: string) => invoke<{ code: string; secret: string }>("video_jf_quickconnect_start", { server });
export const jfQuickConnectPoll = (server: string, secret: string) =>
  invoke<SourceInfo | null>("video_jf_quickconnect_poll", { server, secret });
export const jfViews = (source: number) => invoke<JfView[]>("video_jf_views", { source });
export const jfItems = (source: number, opts: { parentId?: string | null; search?: string | null; types?: string[] | null; start?: number; limit?: number } = {}) =>
  invoke<{ items: JfItem[]; total: number }>("video_jf_items", {
    source,
    parentId: opts.parentId ?? null,
    search: opts.search ?? null,
    types: opts.types ?? null,
    start: opts.start ?? 0,
    limit: opts.limit ?? 60,
  });
export const jfResume = (source: number) => invoke<JfItem[]>("video_jf_resume", { source });
export const jfNextUp = (source: number) => invoke<JfItem[]>("video_jf_nextup", { source });
export const jfItem = (source: number, id: string) => invoke<JfItem>("video_jf_item", { source, id });
export const jfChildren = (source: number, id: string) => invoke<JfItem[]>("video_jf_children", { source, id });
export const jfPlay = (source: number, itemId: string) => invoke<PlaybackInfo>("video_jf_play", { source, itemId });
export const jfReport = (source: number, itemId: string, event: "start" | "progress" | "stop", positionMs: number, paused: boolean) =>
  invoke<void>("video_jf_report", { source, itemId, event, positionMs, paused });

// IPTV
export const iptvAdd = (name: string, url: string, epgUrl: string | null = null) => invoke<SourceInfo>("video_iptv_add", { name, url, epgUrl });
export const iptvAddXtream = (name: string, server: string, username: string, password: string) =>
  invoke<SourceInfo>("video_iptv_add_xtream", { name, server, username, password });
export const iptvRefresh = (source: number) => invoke<void>("video_iptv_refresh", { source });
export const iptvGroups = (source: number | null = null) => invoke<{ name: string; count: number }[]>("video_iptv_groups", { source });
export const iptvChannels = (opts: { source?: number | null; group?: string | null; search?: string | null; favorites?: boolean } = {}) =>
  invoke<IptvChannel[]>("video_iptv_channels", {
    source: opts.source ?? null,
    group: opts.group ?? null,
    search: opts.search ?? null,
    favorites: opts.favorites ?? false,
  });
export const iptvEpg = (channelIds: number[], fromMs: number, toMs: number) => invoke<EpgProgramme[]>("video_iptv_epg", { channelIds, fromMs, toMs });
export const iptvFavorite = (channelId: number, fav: boolean) => invoke<void>("video_iptv_favorite", { channelId, fav });
export const iptvPlay = (channelId: number) => invoke<PlaybackInfo>("video_iptv_play", { channelId });

// Add-ons (open Stremio protocol; http and YouTube streams only)
export const addonInstall = (manifestUrl: string) => invoke<AddonInfo>("video_addon_install", { manifestUrl });
export const addons = () => invoke<AddonInfo[]>("video_addons");
export const addonRemove = (id: number) => invoke<void>("video_addon_remove", { id });
export const addonCatalog = (addon: number, type: string, id: string, extra: [string, string][] = [], skip = 0) =>
  invoke<AddonMeta[]>("video_addon_catalog", { addon, type, id, extra, skip });
export const addonMeta = (type: string, id: string) => invoke<AddonMeta>("video_addon_meta", { type, id });
export const addonStreams = (type: string, id: string) => invoke<AddonStream[]>("video_addon_streams", { type, id });
export const addonPlay = (stream: AddonStream, type: string, id: string, title: string) =>
  invoke<PlaybackInfo>("video_addon_play", { stream, type, id, title });

// AniList + AniSkip
export const anilistSetToken = (token: string) => invoke<void>("video_anilist_set_token", { token });
export const anilistViewer = () => invoke<AniViewer | null>("video_anilist_viewer");
export const anilistSearch = (q: string) => invoke<AniMedia[]>("video_anilist_search", { q });
export const anilistMedia = (id: number) => invoke<AniMedia>("video_anilist_media", { id });
export const anilistList = (status: string | null = null) => invoke<AniMedia[]>("video_anilist_list", { status });
export const anilistProgress = (mediaId: number, episode: number) => invoke<AniMedia>("video_anilist_progress", { mediaId, episode });

// Cast
export const castDiscover = () => invoke<CastDevice[]>("video_cast_discover");
export const castStart = (deviceId: string, source: SourceRef, startMs: number) =>
  invoke<CastStatus>("video_cast_start", { deviceId, source, startMs });
export const castControl = (action: "play" | "pause" | "seek" | "volume" | "mute", value: number | null = null) =>
  invoke<void>("video_cast_control", { action, value });
export const castStop = () => invoke<void>("video_cast_stop");
export const castStatus = () => invoke<CastStatus>("video_cast_status");
