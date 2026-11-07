import { invoke } from "@tauri-apps/api/core";
import type { BoardEntry, BoardFile, BoardLibrary, Deleted, ExportKind, SavedInfo } from "./types";

export function library(): Promise<BoardLibrary> {
  return invoke<BoardLibrary>("board_library");
}

export function chooseFolder(title: string): Promise<BoardLibrary | null> {
  return invoke<BoardLibrary | null>("board_choose_folder", { title });
}

export function resetFolder(): Promise<BoardLibrary> {
  return invoke<BoardLibrary>("board_reset_folder");
}

export function create(fallback: string, name?: string): Promise<BoardEntry> {
  return invoke<BoardEntry>("board_create", { name: name ?? null, fallback });
}

export function read(path: string): Promise<BoardFile> {
  return invoke<BoardFile>("board_read", { path });
}

export function openFile(title: string): Promise<BoardFile | null> {
  return invoke<BoardFile | null>("board_open_file", { title });
}

export function save(path: string, contents: string, thumbnail: string | null): Promise<SavedInfo> {
  return invoke<SavedInfo>("board_save", { path, contents, thumbnail });
}

export function rename(path: string, newName: string): Promise<BoardEntry> {
  return invoke<BoardEntry>("board_rename", { path, newName });
}

export function duplicate(path: string, suffix: string): Promise<BoardEntry> {
  return invoke<BoardEntry>("board_duplicate", { path, suffix });
}

export function remove(path: string): Promise<Deleted> {
  return invoke<Deleted>("board_delete", { path });
}

export function restore(restorePath: string): Promise<BoardEntry> {
  return invoke<BoardEntry>("board_restore", { restorePath });
}

export function importFiles(title: string): Promise<BoardEntry[]> {
  return invoke<BoardEntry[]>("board_import", { title });
}

export function thumbnail(path: string): Promise<string | null> {
  return invoke<string | null>("board_thumbnail", { path });
}

export function exportFile(fileName: string, kind: ExportKind, data: string, title: string): Promise<string | null> {
  return invoke<string | null>("board_export", { fileName, kind, data, title });
}

export function copyPng(data: string): Promise<void> {
  return invoke<void>("board_copy_png", { data });
}

export function reveal(path: string): Promise<void> {
  return invoke<void>("reveal_file", { path });
}

export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) return String((error as { message: unknown }).message);
  return String(error);
}

const CODES = [
  "BOARD_NAME_EMPTY",
  "BOARD_NAME_INVALID",
  "BOARD_NAME_TOO_LONG",
  "BOARD_EXISTS",
  "BOARD_NOT_FOUND",
  "BOARD_OUTSIDE",
  "BOARD_NOT_SCENE",
  "BOARD_TOO_LARGE",
  "BOARD_NO_FOLDER",
  "BOARD_IO",
  "BOARD_TRASH",
  "BOARD_BAD_DATA",
  "BOARD_CLIPBOARD",
] as const;

export type ErrorCode = (typeof CODES)[number] | "unknown";

export function errorCode(error: unknown): { code: ErrorCode; detail: string } {
  const text = errorText(error);
  const code = CODES.find((c) => text === c || text.startsWith(`${c}:`));
  if (!code) return { code: "unknown", detail: text };
  return { code, detail: text.slice(code.length).replace(/^:\s*/, "") };
}

export function errorKey(code: ErrorCode): string {
  return code === "unknown" ? "board.err.unknown" : `board.err.${code.slice("BOARD_".length).toLowerCase()}`;
}

export function blobToBase64(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const url = String(reader.result ?? "");
      resolve(url.slice(url.indexOf(",") + 1));
    };
    reader.onerror = () => reject(reader.error ?? new Error("read failed"));
    reader.readAsDataURL(blob);
  });
}
