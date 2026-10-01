// Thin wrappers over the OmniTube Tauri commands (`commands::tube::*`).
// Argument names are camelCase here; Tauri maps them to the snake_case
// parameters on the Rust side.
import { invoke } from "@tauri-apps/api/core";
import type {
  Account,
  AlbumPage,
  ArtistPage,
  AutoRule,
  ChannelPage,
  CommentsPage,
  DeArrow,
  HistoryEntry,
  ImportReport,
  Item,
  LocalSub,
  Lyrics,
  Page,
  PlaylistPage,
  QueuePage,
  Ryd,
  SbSegment,
  StorageStats,
  StreamSet,
  TubeStatus,
  WatchInfo,
} from "./types";

export type SearchFilter = "all" | "video" | "short" | "channel" | "playlist" | "live" | "movie";
export type MusicSearchFilter = "all" | "songs" | "videos" | "albums" | "artists" | "playlists" | "community_playlists" | "podcasts";
export type ChannelTabName = "home" | "videos" | "shorts" | "streams" | "playlists" | "podcasts" | "releases";
export type ChannelSort = "latest" | "popular" | "oldest";
export type MusicLibraryKind = "playlists" | "songs" | "albums" | "artists" | "liked" | "history" | "subscriptions";
/** Which parser a continuation token belongs to. */
export type ContinuationKind =
  | "home"
  | "search"
  | "feed"
  | "channel"
  | "playlist"
  | "related"
  | "history"
  | "music_home"
  | "music_browse"
  | "music_search"
  | "music_playlist"
  | "music_library";

type TubeAction =
  | "like"
  | "dislike"
  | "remove_like"
  | "subscribe"
  | "unsubscribe"
  | "watch_later_add"
  | "watch_later_remove"
  | "playlist_add"
  | "playlist_remove"
  | "not_interested"
  | "dont_recommend_channel"
  | "music_like"
  | "music_dislike"
  | "music_remove_like"
  | "library_add"
  | "library_remove";

type ActionTarget = {
  video_id?: string | null;
  channel_id?: string | null;
  playlist_id?: string | null;
  set_video_id?: string | null;
  feedback_token?: string | null;
  /** Without an account, local "watch later" and "subscribe" keep these for display. */
  title?: string | null;
  channel_name?: string | null;
  avatar?: string | null;
};

type DownloadRequest = {
  video_id: string;
  title: string;
  /** null = best by the user's quality setting */
  video_itag?: number | null;
  audio_itag?: number | null;
  audio_only: boolean;
  /** Height cap when no itag is given (null = best). */
  height?: number | null;
  /** Overrides settings.tube.sb_download for this download. */
  sponsorblock?: "off" | "mark" | "remove" | null;
  /** null = settings.tube.download_folder or the app default. */
  folder?: string | null;
  /** For the SponsorBlock channel whitelist. */
  channel_id?: string | null;
  music: boolean;
};

// ---- status & account ----
export const status = () => invoke<TubeStatus>("tube_status");
export const login = () => invoke<Account>("tube_login");
export const logout = () => invoke<void>("tube_logout");
export const account = () => invoke<Account>("tube_account");

// ---- YouTube (video face) ----
export const home = (chipToken?: string | null) => invoke<Page>("tube_home", { chipToken: chipToken ?? null });
export const search = (query: string, filter: SearchFilter = "all") => invoke<Page>("tube_search", { query, filter });
export const suggest = (query: string, music = false) => invoke<string[]>("tube_suggest", { query, music });
export const continuation = (kind: ContinuationKind, token: string) => invoke<Page>("tube_continuation", { kind, token });
export const watch = (videoId: string, playlistId?: string | null, index?: number | null) =>
  invoke<WatchInfo>("tube_watch", { videoId, playlistId: playlistId ?? null, index: index ?? null });
export const comments = (token: string) => invoke<CommentsPage>("tube_comments", { token });
export const channel = (id: string, tab: ChannelTabName = "home", sort: ChannelSort | null = null) =>
  invoke<ChannelPage>("tube_channel", { id, tab, sort });
export const playlist = (id: string) => invoke<PlaylistPage>("tube_playlist", { id });
/** Subscriptions feed: the account's when logged in, else built from local subs. */
export const subscriptionsFeed = (refresh = false) => invoke<Page>("tube_subscriptions_feed", { refresh });
/** "You" page: history row, watch later, playlists, liked (account); local lists otherwise. */
export const library = () => invoke<Page>("tube_library");
export const history = () => invoke<Page>("tube_history");
export const shorts = () => invoke<Page>("tube_shorts");
export const action = (action: TubeAction, target: ActionTarget) => invoke<void>("tube_action", { action, target });
/** Playlists of the account the video could be saved to (id, title, contains). */
export const savePlaylists = (videoId: string) =>
  invoke<{ id: string; title: string; contains: boolean }[]>("tube_save_playlists", { videoId });
export const createPlaylist = (title: string, videoIds: string[], music = false) =>
  invoke<string>("tube_playlist_create", { title, videoIds, music });

// ---- streams & player ----
/** Resolve playable streams (innertube ladder, yt-dlp fallback) and open a proxy session. */
export const streams = (videoId: string, music = false) => invoke<StreamSet>("tube_streams", { videoId, music });
/** Force the yt-dlp path (after the innertube streams failed in the player). */
export const streamsFallback = (videoId: string, music = false) => invoke<StreamSet>("tube_streams_fallback", { videoId, music });
/** Progress report: local history, resume, and (if enabled) the account history. */
export const progress = (entry: HistoryEntry, sendAccount: boolean) => invoke<void>("tube_progress", { entry, sendAccount });
export const resumeFor = (videoId: string) => invoke<number | null>("tube_resume_for", { videoId });

// ---- extras ----
export const sponsorblock = (videoId: string) => invoke<SbSegment[]>("tube_sponsorblock", { videoId });
export const dearrow = (videoIds: string[]) => invoke<Record<string, DeArrow>>("tube_dearrow", { videoIds });
export const ryd = (videoId: string) => invoke<Ryd | null>("tube_ryd", { videoId });

// ---- download ----
export const download = (req: DownloadRequest) => invoke<string>("tube_download", { req });

// ---- YouTube Music ----
export const musicHome = (params?: string | null) => invoke<Page>("tube_music_home", { params: params ?? null });
export const musicExplore = () => invoke<Page>("tube_music_explore");
export const musicBrowse = (browseId: string, params?: string | null) =>
  invoke<Page>("tube_music_browse", { browseId, params: params ?? null });
export const musicSearch = (query: string, filter: MusicSearchFilter = "all") => invoke<Page>("tube_music_search", { query, filter });
export const musicAlbum = (browseId: string) => invoke<AlbumPage>("tube_music_album", { browseId });
export const musicArtist = (browseId: string) => invoke<ArtistPage>("tube_music_artist", { browseId });
export const musicPlaylist = (id: string) => invoke<PlaylistPage>("tube_music_playlist", { id });
/** Up-next queue for a song; `radio` starts an endless radio (`RDAMVM<id>`). */
export const musicNext = (videoId: string, playlistId?: string | null, radio = false, continuationToken?: string | null) =>
  invoke<QueuePage>("tube_music_next", { videoId, playlistId: playlistId ?? null, radio, continuation: continuationToken ?? null });
export const musicLyrics = (videoId: string, title: string, artist: string, durationS: number) =>
  invoke<Lyrics | null>("tube_music_lyrics", { videoId, title, artist, durationS });
export const musicRelated = (browseId: string) => invoke<Page>("tube_music_related", { browseId });
export const musicLibrary = (kind: MusicLibraryKind) => invoke<Page>("tube_music_library", { kind });

// ---- local (no account) ----
export const subsList = () => invoke<LocalSub[]>("tube_subs_list");
export const subsAdd = (channelId: string, name: string, avatar?: string | null) =>
  invoke<LocalSub>("tube_subs_add", { channelId, name, avatar: avatar ?? null });
export const subsRemove = (channelId: string) => invoke<void>("tube_subs_remove", { channelId });
export const subsUpdate = (channelId: string, notify: boolean | null, autoDownload: boolean | null) =>
  invoke<void>("tube_subs_update", { channelId, notify, autoDownload });
export const localHistory = (limit = 200) => invoke<HistoryEntry[]>("tube_local_history", { limit });
export const clearLocalHistory = () => invoke<void>("tube_local_history_clear");
export const removeLocalHistory = (videoId: string) => invoke<void>("tube_local_history_remove", { videoId });
export const importFile = (path: string) => invoke<ImportReport>("tube_import", { path });
export const exportSubs = (path: string, format: "opml" | "newpipe" = "opml") => invoke<void>("tube_export_subs", { path, format });

// ---- auto-download ----
export const rulesList = () => invoke<AutoRule[]>("tube_rules_list");
export const rulesSave = (rule: AutoRule) => invoke<AutoRule>("tube_rules_save", { rule });
export const rulesRemove = (id: string) => invoke<void>("tube_rules_remove", { id });
/** What the rule would queue right now (same code as the poller). */
export const rulesPreview = (rule: AutoRule) => invoke<Item[]>("tube_rules_preview", { rule });
export const storageStats = () => invoke<StorageStats>("tube_storage_stats");
export const autoCheckNow = () => invoke<number>("tube_auto_check_now");
