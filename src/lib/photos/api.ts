import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  DupGroup,
  Edits,
  ExportOptions,
  ExportOutcome,
  ExportProgress,
  FileReport,
  Folders,
  IndexProgress,
  Photo,
  Preview,
  RemoveMode,
} from "./types";

export const INDEX_EVENT = "photos-index-progress";
export const EXPORT_EVENT = "photos-export-progress";

export function folders(): Promise<Folders> {
  return invoke<Folders>("photos_folders");
}

export function setFolders(folders: string[]): Promise<Folders> {
  return invoke<Folders>("photos_set_folders", { folders });
}

export function indexStart(): Promise<IndexProgress> {
  return invoke<IndexProgress>("photos_index_start");
}

export function indexStatus(): Promise<IndexProgress> {
  return invoke<IndexProgress>("photos_index_status");
}

export function indexCancel(): Promise<void> {
  return invoke<void>("photos_index_cancel");
}

export function list(): Promise<Photo[]> {
  return invoke<Photo[]>("photos_list");
}

export function duplicates(maxDistance: number): Promise<DupGroup[]> {
  return invoke<DupGroup[]>("photos_duplicates", { maxDistance });
}

export function remove(paths: string[], mode: RemoveMode): Promise<FileReport> {
  return invoke<FileReport>("photos_remove", { paths, mode });
}

export function move(paths: string[], dest: string): Promise<FileReport> {
  return invoke<FileReport>("photos_move", { paths, dest });
}

export function editsGet(path: string): Promise<Edits | null> {
  return invoke<Edits | null>("photos_edits_get", { path });
}

export function editsSet(path: string, edits: Edits | null): Promise<void> {
  return invoke<void>("photos_edits_set", { path, edits });
}

export function preview(path: string, edits: Edits): Promise<Preview> {
  return invoke<Preview>("photos_preview", { path, edits });
}

export function release(): Promise<void> {
  return invoke<void>("photos_release");
}

export function exportPhotos(batchId: string, paths: string[], options: ExportOptions): Promise<ExportOutcome[]> {
  return invoke<ExportOutcome[]>("photos_export", { batchId, paths, options });
}

export function exportCancel(batchId: string): Promise<void> {
  return invoke<void>("photos_export_cancel", { batchId });
}

export function reveal(path: string): Promise<void> {
  return invoke<void>("reveal_file", { path });
}

export function thumbSrc(thumb: string): string {
  return thumb.startsWith("data:") ? thumb : convertFileSrc(thumb);
}

export function onIndex(handler: (p: IndexProgress) => void): Promise<UnlistenFn> {
  return listen<IndexProgress>(INDEX_EVENT, (e) => handler(e.payload));
}

export function onExport(handler: (p: ExportProgress) => void): Promise<UnlistenFn> {
  return listen<ExportProgress>(EXPORT_EVENT, (e) => handler(e.payload));
}

export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) return String((error as { message: unknown }).message);
  return String(error);
}
