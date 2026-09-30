// Canonical OmniTube types. Mirror of
// `src-tauri/omniget-core/src/core/tube/model.rs` (same snake_case fields).

export type Thumb = { url: string; width: number; height: number };

export type ChannelRef = {
  id: string;
  name: string;
  handle?: string | null;
  avatar?: string | null;
  verified?: boolean;
  artist?: boolean;
};

export type Badge =
  | "live"
  | "upcoming"
  | "premiere"
  | "members"
  | "new"
  | "hd4k"
  | "cc"
  | "paid"
  | "explicit"
  | "streamed"
  | "auto_dubbed";

export type TextRun = {
  text: string;
  url?: string | null;
  video_id?: string | null;
  t?: number | null;
  channel_id?: string | null;
  bold?: boolean;
};

export type VideoItem = {
  kind: "video";
  id: string;
  title: string;
  channel?: ChannelRef | null;
  thumbs: Thumb[];
  duration_s?: number | null;
  views?: number | null;
  views_text?: string | null;
  published_text?: string | null;
  published_ms?: number | null;
  badges: Badge[];
  watched_pct?: number | null;
  description_snippet?: string | null;
  feedback_token?: string | null;
  set_video_id?: string | null;
};

export type ShortItem = {
  kind: "short";
  id: string;
  title: string;
  thumbs: Thumb[];
  views_text?: string | null;
  channel?: ChannelRef | null;
};

export type ChannelItem = {
  kind: "channel";
  id: string;
  name: string;
  handle?: string | null;
  avatar?: string | null;
  subscribers_text?: string | null;
  video_count_text?: string | null;
  description?: string | null;
  verified?: boolean;
  artist?: boolean;
  subscribed?: boolean | null;
};

export type PlaylistItem = {
  kind: "playlist";
  id: string;
  title: string;
  thumbs: Thumb[];
  channel?: ChannelRef | null;
  count_text?: string | null;
  is_mix: boolean;
  first_video_id?: string | null;
  is_music: boolean;
};

export type AlbumRef = { id: string; name: string };

export type SongItem = {
  kind: "song";
  id: string;
  title: string;
  artists: ChannelRef[];
  album?: AlbumRef | null;
  thumbs: Thumb[];
  duration_s?: number | null;
  explicit: boolean;
  is_video: boolean;
  plays_text?: string | null;
  set_video_id?: string | null;
  liked?: boolean | null;
};

export type AlbumItem = {
  kind: "album";
  id: string;
  playlist_id?: string | null;
  title: string;
  artists: ChannelRef[];
  year?: string | null;
  thumbs: Thumb[];
  album_type?: string | null;
  explicit: boolean;
};

export type ArtistItem = {
  kind: "artist";
  id: string;
  name: string;
  thumbs: Thumb[];
  subscribers_text?: string | null;
};

export type MoodItem = {
  kind: "mood";
  title: string;
  browse_id: string;
  params?: string | null;
  color?: string | null;
};

export type Item = VideoItem | ShortItem | ChannelItem | PlaylistItem | SongItem | AlbumItem | ArtistItem | MoodItem;

export type Nav = { browse_id: string; params?: string | null };

export type ShelfKind = "grid" | "row" | "shorts" | "quick_picks" | "carousel" | "list" | "moods";

export type Shelf = {
  id: string;
  title?: string | null;
  subtitle?: string | null;
  strapline?: string | null;
  thumb?: string | null;
  kind: ShelfKind;
  items: Item[];
  more?: Nav | null;
};

export type Chip = { label: string; token?: string | null; selected: boolean };

export type PageHeader = { title: string; subtitle?: string | null; description?: string | null; thumbs: Thumb[] };

export type Page = {
  header?: PageHeader | null;
  chips: Chip[];
  shelves: Shelf[];
  continuation?: string | null;
  corrected_query?: string | null;
};

export type Chapter = { title: string; start_s: number; thumb?: string | null };
export type Rating = "like" | "dislike";

export type WatchPlaylist = {
  id: string;
  title: string;
  owner?: string | null;
  index: number;
  total?: number | null;
  items: VideoItem[];
  is_mix: boolean;
};

export type WatchInfo = {
  id: string;
  title: string;
  channel?: ChannelItem | null;
  views?: number | null;
  views_text?: string | null;
  published_text?: string | null;
  date_text?: string | null;
  description: TextRun[];
  likes?: number | null;
  likes_text?: string | null;
  rating?: Rating | null;
  chapters: Chapter[];
  related: Item[];
  related_continuation?: string | null;
  comments_token?: string | null;
  comments_count_text?: string | null;
  comments_sorts: CommentSort[];
  playlist?: WatchPlaylist | null;
  is_live: boolean;
  badges: Badge[];
  saved?: boolean | null;
};

export type Comment = {
  id: string;
  author: ChannelRef;
  text: TextRun[];
  published_text?: string | null;
  likes_text?: string | null;
  reply_count: number;
  replies_token?: string | null;
  pinned: boolean;
  hearted: boolean;
  is_owner: boolean;
};

export type CommentSort = { label: string; token: string; selected: boolean };
export type CommentsPage = {
  count_text?: string | null;
  sorts: CommentSort[];
  items: Comment[];
  continuation?: string | null;
};

export type ChannelTabId = "home" | "videos" | "shorts" | "streams" | "playlists" | "podcasts" | "releases" | "community";
export type ChannelTab = { id: ChannelTabId | string; title: string; params?: string | null; selected: boolean };
export type ChannelPage = { channel: ChannelItem; banner: Thumb[]; tabs: ChannelTab[]; page: Page };

export type PlaylistPage = {
  id: string;
  title: string;
  owner?: ChannelRef | null;
  thumbs: Thumb[];
  description?: string | null;
  count_text?: string | null;
  views_text?: string | null;
  updated_text?: string | null;
  items: Item[];
  continuation?: string | null;
  editable: boolean;
  is_music: boolean;
  duration_text?: string | null;
};

export type AlbumPage = {
  id: string;
  playlist_id?: string | null;
  title: string;
  album_type?: string | null;
  year?: string | null;
  artists: ChannelRef[];
  thumbs: Thumb[];
  description?: string | null;
  track_count_text?: string | null;
  duration_text?: string | null;
  tracks: SongItem[];
  explicit: boolean;
  other_versions: AlbumItem[];
};

export type ArtistPage = {
  id: string;
  name: string;
  thumbs: Thumb[];
  description?: string | null;
  subscribers_text?: string | null;
  subscribed?: boolean | null;
  channel_id?: string | null;
  shuffle_playlist_id?: string | null;
  radio_playlist_id?: string | null;
  shelves: Shelf[];
};

export type QueuePage = {
  playlist_id?: string | null;
  tracks: SongItem[];
  continuation?: string | null;
  lyrics_browse_id?: string | null;
  related_browse_id?: string | null;
};

export type LyricLine = { start_ms?: number | null; text: string };
export type Lyrics = { source: "youtube" | "lrclib" | string; synced: boolean; lines: LyricLine[]; footer?: string | null };

export type FormatKind = "video" | "audio" | "muxed";
export type ByteRange = { start: number; end: number };
export type Format = {
  itag: number;
  kind: FormatKind;
  mime: string;
  codecs: string;
  width?: number | null;
  height?: number | null;
  fps?: number | null;
  bitrate: number;
  content_length?: number | null;
  quality_label?: string | null;
  audio_lang?: string | null;
  audio_name?: string | null;
  audio_default: boolean;
  auto_dubbed: boolean;
  hdr: boolean;
  drc: boolean;
  init_range?: ByteRange | null;
  index_range?: ByteRange | null;
};

export type Caption = { lang: string; name: string; auto: boolean; url: string };
export type Storyboard = { url: string; width: number; height: number; count: number; cols: number; rows: number; interval_ms: number };

export type StreamSet = {
  session: string;
  video_id: string;
  duration_s: number;
  client: string;
  formats: Format[];
  loudness_db?: number | null;
  expires_ms: number;
  is_live: boolean;
  hls?: string | null;
  captions: Caption[];
  storyboard?: Storyboard | null;
  base: string;
  token: string;
  title?: string | null;
  author?: string | null;
  thumbs: Thumb[];
};

export type SbCategory =
  | "sponsor"
  | "selfpromo"
  | "interaction"
  | "intro"
  | "outro"
  | "preview"
  | "hook"
  | "music_offtopic"
  | "filler"
  | "poi_highlight"
  | "exclusive_access"
  | "chapter";
export type SbAction = "skip" | "mute" | "full" | "poi" | "chapter";
export type SbSegment = {
  uuid: string;
  category: SbCategory;
  action: SbAction;
  start_s: number;
  end_s: number;
  votes: number;
  description?: string | null;
};

export type DeArrow = { title?: string | null; thumb_time_s?: number | null; thumb_url?: string | null };
export type Ryd = { likes: number; dislikes: number; rating: number; view_count: number };

export type Account = {
  logged_in: boolean;
  name?: string | null;
  handle?: string | null;
  avatar?: string | null;
  email?: string | null;
  channel_id?: string | null;
  premium?: boolean | null;
  slug?: string | null;
};

export type LocalSub = {
  channel_id: string;
  name: string;
  avatar?: string | null;
  added_ms: number;
  last_checked_ms?: number | null;
  notify: boolean;
  auto_download: boolean;
};

export type ImportReport = {
  format: string;
  subs_added: number;
  subs_skipped: number;
  history_added: number;
  playlists_added: number;
  errors: string[];
};

export type HistoryEntry = {
  video_id: string;
  title: string;
  channel?: ChannelRef | null;
  thumb?: string | null;
  duration_s?: number | null;
  position_s: number;
  watched_ms: number;
  kind: "video" | "song";
};

export type AutoRule = {
  id: string;
  name: string;
  enabled: boolean;
  channel_ids: string[];
  include_words: string[];
  exclude_words: string[];
  min_duration_s?: number | null;
  max_duration_s?: number | null;
  max_age_days?: number | null;
  include_shorts: boolean;
  include_live: boolean;
  quality: "best" | "1080" | "720" | "480" | "audio" | string;
  sponsorblock: "off" | "mark" | "remove" | string;
  folder?: string | null;
  max_gb: number;
  keep_days: number;
  keep_per_channel: number;
};

export type StorageStats = { total_bytes: number; files: number; free_bytes?: number | null; cap_bytes?: number | null };

export type TubeStatus = {
  account: Account;
  /// Proxy server base (`http://127.0.0.1:PORT`).
  server: string;
  ytdlp: boolean;
  ffmpeg: boolean;
};

/// Per-category SponsorBlock behavior.
export type SbMode = "skip" | "ask" | "show" | "off";

/// `settings.tube` (AppSettings). Mirror of `TubeSettings` in settings.rs.
export type TubeSettings = {
  enabled: boolean;
  /// "classic" = YouTube's own neutral palette and red progress; "app" = follow the OmniGet theme.
  look: "classic" | "app";
  start_face: "video" | "music";
  start_page: "home" | "subscriptions" | "library";
  use_account: boolean;
  default_quality: "auto" | "2160" | "1440" | "1080" | "720" | "480" | "360";
  prefer_codec: "auto" | "avc1" | "vp9" | "av01";
  audio_quality: "premium" | "high" | "normal" | "low";
  autoplay_next: boolean;
  autoplay_on_open: boolean;
  preview_on_hover: boolean;
  normalize_volume: boolean;
  send_history: boolean;
  endless_radio: boolean;
  sponsorblock: boolean;
  sb_categories: Partial<Record<SbCategory, SbMode>>;
  sb_server: string;
  sb_whitelist: string[];
  sb_download: "off" | "mark" | "remove";
  dearrow: boolean;
  dearrow_thumbs: boolean;
  ryd: boolean;
  hide_shorts: boolean;
  hide_live: boolean;
  hide_upcoming: boolean;
  hide_members: boolean;
  hide_mixes: boolean;
  hide_playables: boolean;
  hide_posts: boolean;
  hide_watched: boolean;
  watched_threshold: number;
  hide_home_feed: boolean;
  hide_related: boolean;
  hide_comments: boolean;
  hide_end_screen: boolean;
  hide_chips: boolean;
  hide_metrics: boolean;
  infinite_scroll: boolean;
  grid_columns: number;
  blocked_channels: { id: string; name: string }[];
  blocked_words: string[];
  whole_word: boolean;
  min_duration_s: number;
  max_duration_s: number;
  lyrics_provider: "auto" | "youtube" | "lrclib" | "off";
  download_folder: string | null;
  storage_cap_gb: number;
};
