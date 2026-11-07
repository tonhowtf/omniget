export type Platform = "macos" | "windows" | "linux" | "other";

export type SourceKind = "screen" | "window" | "audio";

export type Source = { id: string; name: string; kind: SourceKind };

export type Sources = {
  platform: Platform;
  ffmpeg: boolean;
  screens: Source[];
  audio: Source[];
  error: string | null;
};

export type Region = { x: number; y: number; w: number; h: number };

export type Container = "mp4" | "mov" | "mkv";

export type Quality = "high" | "balanced" | "small";

export type RecordOptions = {
  screen: string;
  region: Region | null;
  fps: number;
  mic: string | null;
  cursor: boolean;
  container: Container;
  quality: Quality;
  out_dir: string | null;
};

export type RecordingInfo = { id: string; path: string; started_at: number };

export type RecordOutcome = {
  path: string;
  ok: boolean;
  duration: number;
  size_bytes: number;
  forced: boolean;
  stopped_by_user: boolean;
  error: string | null;
};

export type RecordingEvent = { id: string; state: "done" | "failed"; outcome: RecordOutcome };

export type VideoStream = { codec: string; width: number; height: number; fps: number | null };

export type AudioStream = { index: number; codec: string; channels: number | null };

export type ProbeResult = {
  path: string;
  name: string;
  size_bytes: number;
  duration: number;
  format_name: string;
  video: VideoStream | null;
  audio: AudioStream[];
  keyframes: number[];
};

export type Span = { start: number; end: number };

export type ExportMode = "merge" | "separate";

export type ExportRequest = {
  path: string;
  segments: Span[];
  mode: ExportMode;
  precise: boolean;
  strip_audio: boolean;
  out_dir: string | null;
};

export type ExportResult = { outputs: string[]; size_bytes: number };

export type ExportEvent = {
  job_id: string;
  percent: number;
  step: number;
  steps: number;
  state: "running" | "done" | "failed" | "cancelled";
  error?: string;
};

export type EntryKind = "recording" | "export" | "snapshot";

export type HistoryEntry = {
  id: string;
  kind: EntryKind;
  path: string;
  created_at: number;
  duration: number | null;
  size_bytes: number;
  source: string | null;
  exists: boolean;
};
