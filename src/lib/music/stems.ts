import { invoke } from "@tauri-apps/api/core";

export type StemFormat = "flac" | "mp3" | "wav";

export type StemJobState = "queued" | "preparing" | "running" | "done" | "failed" | "cancelled";

export type StemOut = { stem: string; path: string; track_id: number | null };

export type StemJob = {
  id: number;
  track_id: number | null;
  source: string;
  title: string;
  outputs: StemOut[];
  state: StemJobState;
  progress: number;
  error: string | null;
};

export type StemSet = {
  source: string;
  source_id: number | null;
  vocals: string;
  vocals_id: number | null;
  instrumental: string;
  instrumental_id: number | null;
  model: string;
  created_at: number;
};

export type StemModel = {
  id: string;
  label: string;
  bytes: number;
  license: string;
  source: string;
  stems: string[];
  downloaded: boolean;
  default: boolean;
};

export type StemsStatus = {
  runtime_installed: boolean;
  runtime_can_download: boolean;
  models: StemModel[];
};

export type ModelProgress = { id: string; stage: string; done: number; total: number | null };

export type StemsRequest = {
  track_ids: number[];
  paths: string[];
  format: StemFormat;
  out_dir: string;
  labels: { vocals: string; instrumental: string };
};

export const STEM_FORMATS: StemFormat[] = ["flac", "mp3", "wav"];

export const PROGRESS_EVENT = "music-stems-progress";
export const MODEL_EVENT = "music-stems-model-progress";

export const MEDIA_EXTENSIONS = ["mp3", "flac", "wav", "m4a", "aac", "ogg", "opus", "wma", "aiff", "aif", "alac", "mp4", "mkv", "webm", "mov", "avi"];

export const stemsApi = {
  status: () => invoke<StemsStatus>("music_stems_status"),
  installModel: () => invoke<StemsStatus>("music_stems_install_model", { model: null }),
  removeModel: () => invoke<StemsStatus>("music_stems_remove_model", { model: null }),
  start: (request: StemsRequest) => invoke<StemJob[]>("music_stems_start", { request }),
  cancel: (id: number | null) => invoke<void>("music_stems_cancel", { id }),
  jobs: () => invoke<StemJob[]>("music_stems_jobs"),
  clear: () => invoke<void>("music_stems_clear"),
  sets: () => invoke<StemSet[]>("music_stems_sets"),
};

export function isActive(j: StemJob): boolean {
  return j.state === "queued" || j.state === "preparing" || j.state === "running";
}

export function mergeJob(jobs: StemJob[], j: StemJob): StemJob[] {
  const i = jobs.findIndex((x) => x.id === j.id);
  if (i < 0) return [...jobs, j];
  const next = jobs.slice();
  next[i] = j;
  return next;
}

export type StemRole = "source" | "instrumental" | "vocals";

export function findSet(sets: StemSet[], trackId: number | null | undefined): { set: StemSet; role: StemRole } | null {
  if (trackId == null) return null;
  for (const set of sets) {
    if (set.instrumental_id === trackId) return { set, role: "instrumental" };
    if (set.vocals_id === trackId) return { set, role: "vocals" };
  }
  for (const set of sets) {
    if (set.source_id === trackId && set.instrumental_id != null) return { set, role: "source" };
  }
  return null;
}

export function companionVolume(mix: number, volume: number, muted: boolean): number {
  if (muted) return 0;
  const m = Math.min(1, Math.max(0, mix));
  return Math.min(1, m * volume * volume);
}

export type MediaClock = { time: number; paused: boolean; rate: number };

export type SyncPlan = { seek: number | null; play: boolean | null; rate: number | null };

export const DRIFT_LIMIT_S = 0.12;

export function syncPlan(main: MediaClock, companion: MediaClock, limit = DRIFT_LIMIT_S): SyncPlan {
  const play = main.paused === companion.paused ? null : !main.paused;
  const drift = Math.abs(main.time - companion.time);
  const seek = drift > limit || (main.paused && drift > 0.02) ? main.time : null;
  const rate = Math.abs(main.rate - companion.rate) > 1e-3 ? main.rate : null;
  return { seek, play, rate };
}

export function swapInQueue(queue: number[], index: number, id: number): number[] {
  return queue.map((q, i) => (i === index ? id : q));
}

export function fileName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

const TOOL_TABS = ["analyze", "convert", "stems", "rename", "lyrics", "duplicates", "quality"];

export function toolFromParams(params: Pick<URLSearchParams, "get">): string | null {
  const tool = params.get("tool");
  return tool && TOOL_TABS.includes(tool) ? tool : null;
}
