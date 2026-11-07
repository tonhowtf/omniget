import { invoke } from "@tauri-apps/api/core";
import type { CollectionsResult, ExportFile, IconsResult, Library, SavedFile, SearchResult } from "./types";

export { errorOf } from "./errors";

export function collections(refresh = false): Promise<CollectionsResult> {
  return invoke<CollectionsResult>("icons_collections", { refresh });
}

export function search(query: string, prefixes: string[], start = 0): Promise<SearchResult> {
  return invoke<SearchResult>("icons_search", { query, prefixes, start });
}

export function getIcons(prefix: string, names: string[]): Promise<IconsResult> {
  return invoke<IconsResult>("icons_get", { prefix, names });
}

export function library(): Promise<Library> {
  return invoke<Library>("icons_library");
}

export function saveLibrary(library: Library): Promise<Library> {
  return invoke<Library>("icons_library_save", { library });
}

export function cacheInfo(): Promise<{ bytes: number }> {
  return invoke<{ bytes: number }>("icons_cache_info");
}

export function clearCache(): Promise<void> {
  return invoke<void>("icons_cache_clear");
}

export function saveSvg(fileName: string, text: string): Promise<SavedFile | null> {
  return invoke<SavedFile | null>("icons_save_file", { kind: "svg", fileName, text });
}

export function savePng(fileName: string, base64: string): Promise<SavedFile | null> {
  return invoke<SavedFile | null>("icons_save_file", { kind: "png", fileName, base64 });
}

export function exportCollection(mode: "folder" | "sprite", name: string, files: ExportFile[], sprite?: string): Promise<SavedFile | null> {
  return invoke<SavedFile | null>("icons_export", { mode, name, files, sprite: sprite ?? null });
}

export function reveal(path: string): Promise<void> {
  return invoke<void>("reveal_file", { path });
}
