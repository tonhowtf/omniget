export type ServerInfo = { port: number; token: string };

export type RootKind = "library" | "course";
export type Root = { path: string; kind: RootKind; added_at: number; online: boolean };

type MediaKind = "movie" | "episode" | "lesson" | "video";

type Chapter = { start_ms: number; end_ms: number; title: string; kind?: SkipKind | null };
type SkipKind = "intro" | "outro" | "recap" | "sponsor" | "preview";

export type Progress = {
  key: string;
  position_ms: number;
  duration_ms: number;
  watched: boolean;
  play_count: number;
  last_played: number | null;
  seen_ratio: number;
};

export type MediaItem = {
  id: number;
  key: string;
  path: string;
  root: string;
  folder: string;
  kind: MediaKind;
  title: string;
  show: string | null;
  season: number | null;
  episode: number | null;
  episode_end: number | null;
  year: number | null;
  lesson_no: number | null;
  module: string | null;
  course_id: number | null;
  duration_ms: number;
  width: number | null;
  height: number | null;
  vcodec: string;
  acodec: string;
  container: string;
  size: number;
  has_subs: boolean;
  thumb: string | null;
  added_at: number;
  offline: boolean;
  progress: Progress | null;
};

export type ItemsQuery = {
  kind?: MediaKind | null;
  root?: string | null;
  folder?: string | null;
  search?: string | null;
  limit?: number | null;
  offset?: number | null;
};

export type ContinueEntry = {
  key: string;
  title: string;
  subtitle: string;
  thumb: string | null;
  progress: Progress;
  kind: "resume" | "next";
  source: SourceRef;
};

export type SourceRef =
  | { kind: "local"; id: number }
  | { kind: "url"; url: string }
  | { kind: "jellyfin"; source: number; item: string }
  | { kind: "iptv"; channel: number }
  | { kind: "addon"; addon: number; type: string; id: string; stream_index?: number | null };

export type CourseSummary = {
  id: number;
  title: string;
  root_path: string;
  lessons: number;
  watched: number;
  duration_ms: number;
  watched_ms: number;
  last_played: number | null;
  next_lesson: number | null;
  thumb: string | null;
};

export type Attachment = { path: string; name: string; kind: "pdf" | "code" | "subtitle" | "archive" | "image" | "doc" | "other" };

export type Lesson = MediaItem & { number: string; attachments: Attachment[] };

type CourseModule = { index: number; title: string; lessons: Lesson[]; duration_ms: number; watched: number };

export type CourseDetail = { course: CourseSummary; modules: CourseModule[]; loose_attachments: Attachment[] };

export type ShowSummary = { name: string; seasons: number; episodes: number; watched: number; thumb: string | null; year: number | null };
export type ShowDetail = { name: string; seasons: { season: number; episodes: MediaItem[] }[]; next: number | null };

export type Home = {
  continue: ContinueEntry[];
  recent: MediaItem[];
  courses: CourseSummary[];
  counts: { movies: number; shows: number; courses: number; videos: number };
};

export type PlayMode = "direct" | "remux" | "transcode_audio" | "transcode" | "hls" | "remote";

type SubtitleTrack = {
  id: string;
  label: string;
  lang: string;
  url: string;
  default: boolean;
  forced: boolean;
  source: "embedded" | "external" | "online" | "generated" | "remote";
};

type AudioTrack = { index: number; label: string; lang: string; codec: string; default: boolean };

export type PlaybackInfo = {
  key: string;
  title: string;
  subtitle: string;
  source: SourceRef;
  url: string;
  mode: PlayMode;
  seek_reloads: boolean;
  offset_ms: number;
  reason: string;
  duration_ms: number;
  start_ms: number;
  is_live: boolean;
  subtitles: SubtitleTrack[];
  audio_tracks: AudioTrack[];
  chapters: Chapter[];
  info: { container: string; vcodec: string; acodec: string; width: number | null; height: number | null; bitrate: number | null };
  prefs: PlayPrefs;
  next: SourceRef | null;
  prev: SourceRef | null;
  course_id: number | null;
};

export type PlayPrefs = {
  speed: number;
  subtitle_lang: string | null;
  subtitle_id: string | null;
  audio_index: number | null;
  subtitle_delay_ms: number;
  subtitle_size: number;
  subtitle_bg: boolean;
  volume: number;
  skip_silence: "off" | "skip" | "speed";
  auto_pause: boolean;
};

export type ProgressReport = { key: string; position_ms: number; duration_ms: number; playing: boolean; watched_from_ms?: number | null };

export type Note = {
  id: number;
  key: string;
  t_ms: number;
  t_end_ms: number | null;
  text: string;
  image: string | null;
  created_at: number;
  updated_at: number;
};

export type Card = {
  id: number;
  key: string;
  front: string;
  back: string;
  t_ms: number | null;
  image: string | null;
  due: number;
  interval_days: number;
  ease: number;
  reps: number;
  created_at: number;
};

export type Cue = { start_ms: number; end_ms: number; text: string };

export type TranscriptHit = { key: string; title: string; media_id: number | null; start_ms: number; text: string };

export type HistoryEntry = { key: string; title: string; subtitle: string; at: number; position_ms: number; duration_ms: number; source: SourceRef };

export type ScanProgress = { done: number; total: number; current: string; finished: boolean };

export type SourceKind = "jellyfin" | "iptv" | "addon";
type SourceStatus = "ok" | "slow" | "down" | "disabled";
export type SourceInfo = {
  id: number;
  kind: SourceKind;
  name: string;
  url: string;
  enabled: boolean;
  status: SourceStatus;
  last_error: string | null;
  updated_at: number;
  extra: Record<string, unknown>;
};

export type JfView = { id: string; name: string; collection_type: string | null; image: string | null };
export type JfItem = {
  id: string;
  name: string;
  type: string;
  year: number | null;
  overview: string | null;
  image: string | null;
  backdrop: string | null;
  series_name: string | null;
  season: number | null;
  episode: number | null;
  runtime_ms: number | null;
  position_ms: number | null;
  played: boolean;
  child_count: number | null;
};

export type IptvChannel = {
  id: number;
  source: number;
  name: string;
  group: string;
  logo: string | null;
  tvg_id: string | null;
  favorite: boolean;
  now: EpgProgramme | null;
  next: EpgProgramme | null;
};
export type EpgProgramme = { channel: number; start: number; stop: number; title: string; desc: string | null; category: string | null };

export type AddonInfo = { id: number; manifest_url: string; name: string; version: string; description: string; logo: string | null; types: string[]; catalogs: AddonCatalog[]; resources: string[]; enabled: boolean };
export type AddonCatalog = { addon: number; type: string; id: string; name: string; extra: { name: string; required: boolean; options: string[] }[] };
export type AddonMeta = {
  addon: number;
  id: string;
  type: string;
  name: string;
  poster: string | null;
  background: string | null;
  logo: string | null;
  description: string | null;
  release_info: string | null;
  genres: string[];
  videos: { id: string; title: string; season: number | null; episode: number | null; released: string | null; thumbnail: string | null }[];
};
export type AddonStream = { addon: number; addon_name: string; index: number; name: string; title: string; kind: "url" | "youtube" | "external"; url: string; playable: boolean };

export type AniMedia = {
  id: number;
  id_mal: number | null;
  title: string;
  title_native: string | null;
  cover: string | null;
  banner: string | null;
  episodes: number | null;
  status: string | null;
  season_year: number | null;
  format: string | null;
  genres: string[];
  description: string | null;
  next_airing: { episode: number; airing_at: number } | null;
  list: { status: string; progress: number; score: number | null } | null;
};
export type AniViewer = { id: number; name: string; avatar: string | null };

export type CastDevice = { id: string; name: string; model: string; host: string; port: number; kind: "chromecast" };
export type CastStatus = {
  device: string | null;
  state: "idle" | "connecting" | "buffering" | "playing" | "paused" | "error";
  position_ms: number;
  duration_ms: number;
  volume: number;
  muted: boolean;
  title: string;
  error: string | null;
};
