import type { AsrStatus, Transcript } from "./types";

export type PendingTranscribe = { job: string; path: string; promise: Promise<Transcript> };
export type PendingDownload = { model: string; promise: Promise<AsrStatus> };

let transcribing: PendingTranscribe | null = null;
let downloading: PendingDownload | null = null;

export function trackTranscribe(job: string, path: string, promise: Promise<Transcript>): PendingTranscribe {
  const entry = { job, path, promise };
  transcribing = entry;
  const clear = () => {
    if (transcribing === entry) transcribing = null;
  };
  promise.then(clear, clear);
  return entry;
}

export function trackDownload(model: string, promise: Promise<AsrStatus>): PendingDownload {
  const entry = { model, promise };
  downloading = entry;
  const clear = () => {
    if (downloading === entry) downloading = null;
  };
  promise.then(clear, clear);
  return entry;
}

export function pendingTranscribe(): PendingTranscribe | null {
  return transcribing;
}

export function pendingDownload(): PendingDownload | null {
  return downloading;
}

export function milestone(percent: number, step = 25): number {
  if (!Number.isFinite(percent) || percent <= 0) return 0;
  return Math.min(100, Math.floor(percent / step) * step);
}
