import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  ExportEvent,
  ExportRequest,
  ExportResult,
  HistoryEntry,
  ProbeResult,
  RecordOptions,
  RecordOutcome,
  RecordingEvent,
  RecordingInfo,
  Sources,
} from "./types";

export const RECORDING_EVENT = "capture-recording";
export const EXPORT_EVENT = "capture-export-progress";

export const sources = () => invoke<Sources>("capture_sources");
export const defaultDir = () => invoke<string>("capture_default_dir");
export const status = () => invoke<RecordingInfo | null>("capture_status");
export const start = (options: RecordOptions) => invoke<RecordingInfo>("capture_start", { options });
export const stop = () => invoke<RecordOutcome>("capture_stop");
export const probe = (path: string) => invoke<ProbeResult>("capture_probe", { path });
export const exportCut = (jobId: string, request: ExportRequest) => invoke<ExportResult>("capture_export", { jobId, request });
export const cancel = (jobId: string) => invoke<void>("capture_cancel", { jobId });
export const snapshot = (path: string, at: number, outDir: string | null) => invoke<string>("capture_snapshot", { path, at, outDir });
export const history = () => invoke<HistoryEntry[]>("capture_history");
export const historyRemove = (id: string) => invoke<HistoryEntry[]>("capture_history_remove", { id });
export const openPrivacy = (kind: "screen" | "mic") => invoke<void>("capture_open_privacy", { kind });
export const reveal = (path: string) => invoke<void>("reveal_file", { path });

export function onRecording(handler: (e: RecordingEvent) => void): Promise<UnlistenFn> {
  return listen<RecordingEvent>(RECORDING_EVENT, (event) => handler(event.payload));
}

export function onExport(handler: (e: ExportEvent) => void): Promise<UnlistenFn> {
  return listen<ExportEvent>(EXPORT_EVENT, (event) => handler(event.payload));
}

export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) return String((error as { message: unknown }).message);
  return String(error);
}

export function newId(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : `cap-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}
