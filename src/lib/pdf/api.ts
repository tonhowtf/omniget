import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { CompressOutcome, ImageLayout, OutputPlan, PdfInfo, Preset, ProgressEvent, SavedFile } from "./types";

export const PROGRESS_EVENT = "pdf-progress";

export function inspect(paths: string[]): Promise<PdfInfo[]> {
  return invoke<PdfInfo[]>("pdf_inspect", { paths });
}

export async function bytes(path: string): Promise<Uint8Array> {
  const data = await invoke<ArrayBuffer | number[]>("pdf_bytes", { path });
  return data instanceof ArrayBuffer ? new Uint8Array(data) : Uint8Array.from(data);
}

export function save(sources: string[], outputs: OutputPlan[], overwrite: boolean): Promise<SavedFile[]> {
  return invoke<SavedFile[]>("pdf_save", { sources, outputs, overwrite });
}

export function compress(jobId: string, path: string, preset: Preset, outPath: string): Promise<CompressOutcome> {
  return invoke<CompressOutcome>("pdf_compress", { jobId, path, preset, outPath });
}

export function expandImages(paths: string[]): Promise<string[]> {
  return invoke<string[]>("pdf_expand_images", { paths });
}

export async function imageThumb(path: string): Promise<Blob> {
  const data = await invoke<ArrayBuffer | number[]>("pdf_image_thumb", { path });
  return new Blob([data instanceof ArrayBuffer ? data : Uint8Array.from(data)], { type: "image/jpeg" });
}

export function fromImages(jobId: string, paths: string[], layout: ImageLayout, outPath: string): Promise<SavedFile> {
  return invoke<SavedFile>("pdf_from_images", { jobId, paths, layout, outPath });
}

export function cancel(jobId: string): Promise<void> {
  return invoke<void>("pdf_cancel", { jobId });
}

export function reveal(path: string): Promise<void> {
  return invoke<void>("reveal_file", { path });
}

export function openFile(path: string): Promise<void> {
  return invoke<void>("open_path_default", { path });
}

export function onProgress(handler: (event: ProgressEvent) => void): Promise<UnlistenFn> {
  return listen<ProgressEvent>(PROGRESS_EVENT, (event) => handler(event.payload));
}

export function failureCode(error: unknown): string {
  if (error && typeof error === "object" && "code" in error) return String((error as { code: unknown }).code);
  return "io";
}

export function newJobId(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : `pdf-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}
