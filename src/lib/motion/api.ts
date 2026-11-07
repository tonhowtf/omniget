import { invoke } from "@tauri-apps/api/core";

export type LottieFile = {
  path: string;
  name: string;
  kind: "json" | "dotlottie";
  size_bytes: number;
  animation: string;
  animation_id: string | null;
  animation_count: number;
  manifest: Record<string, unknown> | null;
  embedded_images: number;
};

export type SavedFile = { path: string; size_bytes: number };

export type StageImage = { name: string; mime: string; data_url: string; size_bytes: number };

export type RecentTimeline = { id: string; name: string; saved_at_ms: number; timeline: string };

export function openLottie(path: string): Promise<LottieFile> {
  return invoke<LottieFile>("motion_lottie_open", { path });
}

export function savePng(path: string, dataBase64: string): Promise<SavedFile> {
  return invoke<SavedFile>("motion_png_save", { path, dataBase64 });
}

export function saveTimeline(path: string, json: string): Promise<SavedFile> {
  return invoke<SavedFile>("motion_timeline_save", { path, json });
}

export function loadTimeline(path: string): Promise<string> {
  return invoke<string>("motion_timeline_load", { path });
}

export function openImage(path: string): Promise<StageImage> {
  return invoke<StageImage>("motion_image_open", { path });
}

export function recentList(): Promise<RecentTimeline[]> {
  return invoke<RecentTimeline[]>("motion_recent_list");
}

export function recentSave(id: string | null, name: string, json: string): Promise<RecentTimeline[]> {
  return invoke<RecentTimeline[]>("motion_recent_save", { id, name, json });
}

export function recentRemove(id: string): Promise<RecentTimeline[]> {
  return invoke<RecentTimeline[]>("motion_recent_remove", { id });
}

export function reveal(path: string): Promise<void> {
  return invoke<void>("reveal_file", { path });
}

export async function copyText(text: string): Promise<void> {
  try {
    const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
    await writeText(text);
  } catch {
    await navigator.clipboard.writeText(text);
  }
}

export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) return String((error as { message: unknown }).message);
  return String(error);
}

export function fileName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}
