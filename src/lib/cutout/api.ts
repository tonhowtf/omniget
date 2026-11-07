import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Background, Click, CutItem, CutOptions, CutOutcome, Edge, Inspected, InstallProgress, Preview, ProgressEvent, Status } from "./types";

export const PROGRESS_EVENT = "cutout-progress";
export const INSTALL_EVENT = "cutout-install-progress";

export function status(): Promise<Status> {
  return invoke<Status>("cutout_status");
}

export function install(model: string): Promise<Status> {
  return invoke<Status>("cutout_install", { model });
}

export function inspect(paths: string[]): Promise<Inspected[]> {
  return invoke<Inspected[]>("cutout_inspect", { paths });
}

export function preview(path: string, frameMs: number | null, model: string, clicks: Click[], edge: Edge, background: Background): Promise<Preview> {
  return invoke<Preview>("cutout_preview", { path, frameMs, model, clicks, edge, background });
}

export function run(batchId: string, items: CutItem[], options: CutOptions): Promise<CutOutcome[]> {
  return invoke<CutOutcome[]>("cutout_run", { batchId, items, options });
}

export function cancel(batchId: string): Promise<void> {
  return invoke<void>("cutout_cancel", { batchId });
}

export function release(): Promise<void> {
  return invoke<void>("cutout_release");
}

export function reveal(path: string): Promise<void> {
  return invoke<void>("reveal_file", { path });
}

export function onProgress(handler: (event: ProgressEvent) => void): Promise<UnlistenFn> {
  return listen<ProgressEvent>(PROGRESS_EVENT, (event) => handler(event.payload));
}

export function onInstall(handler: (event: InstallProgress) => void): Promise<UnlistenFn> {
  return listen<InstallProgress>(INSTALL_EVENT, (event) => handler(event.payload));
}

export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) return String((error as { message: unknown }).message);
  return String(error);
}
