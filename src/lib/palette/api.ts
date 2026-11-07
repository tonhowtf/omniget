import { invoke } from "@tauri-apps/api/core";

export type LoadOutcome = { data: unknown | null; moved_aside: string | null };
export type Swatch = { hex: string; l: number; c: number; h: number; share: number };
export type Extraction = { colors: Swatch[]; width: number; height: number; preview: string };

export function loadProjects(): Promise<LoadOutcome> {
  return invoke<LoadOutcome>("palette_projects_load");
}

export function saveProjects(data: unknown): Promise<void> {
  return invoke<void>("palette_projects_save", { data });
}

export function extract(path: string, count: number): Promise<Extraction> {
  return invoke<Extraction>("palette_extract", { path, count });
}

export function saveFile(contents: string, fileName: string): Promise<string | null> {
  return invoke<string | null>("palette_save_file", { contents, fileName });
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

export const IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff", "tga", "ico"];

export function isImagePath(path: string): boolean {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return IMAGE_EXTENSIONS.includes(ext);
}
