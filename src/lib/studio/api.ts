import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AsrProgress,
  AsrStatus,
  Probe,
  RenderOutcome,
  RenderProgress,
  RenderRequest,
  Tools,
  TranscribeProgress,
  Transcript,
} from "./types";

export const RENDER_EVENT = "studio-render-progress";
export const TRANSCRIBE_EVENT = "studio-transcribe-progress";
export const ASR_EVENT = "studio-asr-progress";

export function tools(): Promise<Tools> {
  return invoke<Tools>("studio_tools");
}

export function probe(path: string): Promise<Probe> {
  return invoke<Probe>("studio_probe", { path });
}

export function asrStatus(): Promise<AsrStatus> {
  return invoke<AsrStatus>("studio_asr_status");
}

export function asrInstall(model: string | null): Promise<AsrStatus> {
  return invoke<AsrStatus>("studio_asr_install", { model });
}

export function transcribe(jobId: string, path: string, lang: string | null, model: string | null, durationMs: number): Promise<Transcript> {
  return invoke<Transcript>("studio_transcribe", { jobId, path, lang, model, durationMs });
}

export function render(jobId: string, request: RenderRequest): Promise<RenderOutcome> {
  return invoke<RenderOutcome>("studio_render", { jobId, request });
}

export function cancel(jobId: string): Promise<void> {
  return invoke<void>("studio_cancel", { jobId });
}

export function saveText(path: string, contents: string): Promise<void> {
  return invoke<void>("studio_save_text", { path, contents });
}

export function readText(path: string): Promise<string> {
  return invoke<string>("studio_read_text", { path });
}

export function reveal(path: string): Promise<void> {
  return invoke<void>("reveal_file", { path });
}

export function openFile(path: string): Promise<void> {
  return invoke<void>("open_path_default", { path });
}

export function mediaSrc(path: string): string {
  return convertFileSrc(path);
}

export function onRender(handler: (e: RenderProgress) => void): Promise<UnlistenFn> {
  return listen<RenderProgress>(RENDER_EVENT, (e) => handler(e.payload));
}

export function onTranscribe(handler: (e: TranscribeProgress) => void): Promise<UnlistenFn> {
  return listen<TranscribeProgress>(TRANSCRIBE_EVENT, (e) => handler(e.payload));
}

export function onAsr(handler: (e: AsrProgress) => void): Promise<UnlistenFn> {
  return listen<AsrProgress>(ASR_EVENT, (e) => handler(e.payload));
}

export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) return String((error as { message: unknown }).message);
  return String(error);
}

export function newJobId(prefix: string): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : `${prefix}-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB"];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 10 || i === 0 ? Math.round(v) : v.toFixed(1)} ${units[i]}`;
}
