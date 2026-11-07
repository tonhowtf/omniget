import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { ConvertOptions, ConvertOutcome, FormatInfo, InspectResult, ProgressEvent, RunItem, ToolStatus } from "./types";

export const PROGRESS_EVENT = "converter-progress";

export function tools(): Promise<ToolStatus> {
  return invoke<ToolStatus>("converter_tools");
}

export function formats(): Promise<FormatInfo[]> {
  return invoke<FormatInfo[]>("converter_formats");
}

export function inspect(paths: string[]): Promise<InspectResult[]> {
  return invoke<InspectResult[]>("converter_inspect", { paths });
}

export function run(batchId: string, items: RunItem[], outDir: string | null, options: ConvertOptions): Promise<ConvertOutcome[]> {
  return invoke<ConvertOutcome[]>("converter_run", { batchId, items, outDir, options });
}

export function cancel(batchId: string): Promise<void> {
  return invoke<void>("converter_cancel", { batchId });
}

export function reveal(path: string): Promise<void> {
  return invoke<void>("reveal_file", { path });
}

export function onProgress(handler: (event: ProgressEvent) => void): Promise<UnlistenFn> {
  return listen<ProgressEvent>(PROGRESS_EVENT, (event) => handler(event.payload));
}

export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) return String((error as { message: unknown }).message);
  return String(error);
}
