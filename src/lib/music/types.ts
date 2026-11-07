export type Track = {
  id: number;
  path: string;
  folder: string;
  title: string;
  artist: string;
  album: string;
  album_artist: string;
  genre: string;
  composer: string;
  year: number | null;
  track_no: number | null;
  disc_no: number | null;
  duration_ms: number;
  bitrate: number | null;
  sample_rate: number | null;
  bit_depth: number | null;
  codec: string;
  cover: string | null;
  has_lyrics: boolean;
  rg_track_gain: number | null;
  rg_track_peak: number | null;
  rg_album_gain: number | null;
  rg_album_peak: number | null;
  loudness_lufs: number | null;
  bpm: number | null;
  musical_key: string;
  camelot: string;
  cutoff_hz: number | null;
  intro_s: number | null;
  outro_s: number | null;
  end_s: number | null;
  analyzed: boolean;
  play_count: number;
  skip_count: number;
  last_played: number | null;
  rating: number;
  liked: boolean;
  added_at: number;
  size: number;
  source_url: string;
};

export type MusicStatus = {
  port: number;
  token: string;
  roots: string[];
  track_count: number;
  scanning: boolean;
  ffmpeg: boolean;
  download_dir: string;
  analysis: [number, number, boolean];
};

export type Album = {
  key: string;
  title: string;
  artist: string;
  year: number | null;
  cover: string | null;
  tracks: Track[];
  duration_ms: number;
  added_at: number;
  codec: string;
};

export type Artist = {
  name: string;
  tracks: Track[];
  albums: Album[];
  cover: string | null;
  plays: number;
};

type LrcWord = { time_ms: number; text: string };
export type LrcLine = { time_ms: number; text: string; words?: LrcWord[] };
type Lyrics = {
  lines: LrcLine[];
  plain: string | null;
  title: string | null;
  artist: string | null;
  album: string | null;
  offset_ms: number;
};
export type LyricsPayload = { source: "sidecar" | "tag" | "subtitles" | "cache" | "none"; raw: string; lyrics: Lyrics };

export type LrclibRecord = {
  id: number;
  trackName: string;
  artistName: string;
  albumName: string | null;
  duration: number | null;
  instrumental: boolean;
  plainLyrics: string | null;
  syncedLyrics: string | null;
};

export type SmartRule = { field: string; op: string; value: unknown; value2?: unknown };
export type SmartQuery = {
  rules: SmartRule[];
  match_all: boolean;
  order: string | null;
  descending: boolean;
  limit: number | null;
};

export type Playlist = {
  id: number;
  name: string;
  smart: SmartQuery | null;
  count: number;
  duration_ms: number;
  covers: string[];
  updated_at: number;
};

type TrackMeta = {
  title: string;
  artist: string;
  album: string;
  album_artist: string;
  genre: string;
  composer: string;
  comment: string;
  year: number | null;
  track: number | null;
  track_total: number | null;
  disc: number | null;
  disc_total: number | null;
  compilation: boolean;
  bpm: number | null;
  musical_key: string;
  rg_track_gain: number | null;
  rg_track_peak: number | null;
  rg_album_gain: number | null;
  rg_album_peak: number | null;
  lyrics: string;
  mb_recording_id: string;
  mb_release_id: string;
  duration_ms: number;
  bitrate: number | null;
  sample_rate: number | null;
  bit_depth: number | null;
  channels: number | null;
  codec: string;
  has_cover: boolean;
};

export type TagDetail = { meta: TrackMeta; path: string; size: number; cover: string | null };

export type TagPatch = Partial<Record<
  | "title" | "artist" | "album" | "album_artist" | "genre" | "composer" | "comment"
  | "year" | "track" | "track_total" | "disc" | "disc_total" | "bpm" | "musical_key" | "lyrics",
  string
>>;

export type CoverInput =
  | { kind: "keep" }
  | { kind: "remove" }
  | { kind: "file"; path: string }
  | { kind: "url"; url: string };

export type MbCandidate = {
  recording_id: string;
  release_id: string;
  title: string;
  artist: string;
  album: string;
  album_artist: string;
  year: number | null;
  track: number | null;
  track_total: number | null;
  length_ms: number | null;
  score: number;
  release_type: string;
  cover_url: string;
};

export type Preset =
  | "mp3_v0" | "mp3_320" | "mp3_192" | "aac256" | "opus160" | "opus96" | "vorbis" | "flac" | "alac" | "wav";

export type ConvertOptions = {
  sample_rate?: number | null;
  gain_db?: number | null;
  trim_silence?: boolean;
  mono?: boolean;
  start_s?: number | null;
  end_s?: number | null;
  fade_in_s?: number | null;
  fade_out_s?: number | null;
};

export type ConvertJob = {
  id: number;
  track_id: number;
  title: string;
  output: string;
  state: "queued" | "running" | "done" | "failed" | "cancelled";
  progress: number;
  error: string | null;
};

export type RbStation = {
  stationuuid: string;
  name: string;
  url: string;
  url_resolved: string;
  homepage: string;
  favicon: string;
  tags: string;
  country: string;
  countrycode: string;
  language: string;
  codec: string;
  bitrate: number;
  hls: number;
  votes: number;
  clickcount: number;
  lastcheckok: number;
};

export type Station = {
  id: string;
  name: string;
  url: string;
  homepage: string;
  favicon: string;
  tags: string;
  country: string;
  codec: string;
  bitrate: number;
  hls: boolean;
  last_played?: number | null;
  play_count?: number;
};

export type Stats = {
  tracks: number;
  albums: number;
  artists: number;
  total_ms: number;
  total_bytes: number;
  listened_ms_30d: number;
  plays_30d: number;
  per_day: [number, number][];
  per_hour: number[];
  top_tracks: [number, number][];
  top_artists: [string, number][];
  top_genres: [string, number][];
  formats: [string, number][];
};

export type HistoryEntry = { track_id: number; played_at: number; ms_played: number; completed: boolean };

export type RenamePlan = { id: number; from: string; to: string; conflict: boolean };

export type Spectrogram = { cols: number; rows: number; data: string; nyquist: number; cutoff_hz: number | null };
