/**
 * Utilidades compartilhadas pelos componentes da seção Tools: diálogos de
 * arquivo, progresso (`tool-progress`), formatação e erros. Cada tool é um
 * componente pequeno; o que se repete mora aqui.
 */
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ToolProgress = {
  id: string;
  stage: string;
  done: number;
  total: number | null;
  message: string | null;
};

export function errText(e: unknown): string {
  if (typeof e === "string") return e;
  const m = (e as { message?: unknown })?.message;
  return typeof m === "string" ? m : String(e);
}

type Filter = { name: string; extensions: string[] };

export const FILTERS = {
  media: [{ name: "Media", extensions: ["mp4", "mkv", "webm", "mov", "avi", "mp3", "m4a", "wav", "flac", "ogg", "opus", "aac"] }],
  video: [{ name: "Video", extensions: ["mp4", "mkv", "webm", "mov", "avi"] }],
  images: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "gif"] }],
  subtitles: [{ name: "Subtitles", extensions: ["srt", "vtt", "ass"] }],
  audio: [{ name: "Audio", extensions: ["mp3", "m4a", "wav"] }],
} satisfies Record<string, Filter[]>;

export async function pickFile(filters?: Filter[]): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ multiple: false, directory: false, filters });
  return typeof r === "string" ? r : null;
}

export async function pickFiles(filters?: Filter[]): Promise<string[]> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ multiple: true, directory: false, filters });
  if (Array.isArray(r)) return r;
  return typeof r === "string" ? [r] : [];
}

export async function pickDir(): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ multiple: false, directory: true });
  return typeof r === "string" ? r : null;
}

export async function saveAs(defaultPath?: string, filters?: Filter[]): Promise<string | null> {
  const { save } = await import("@tauri-apps/plugin-dialog");
  const r = await save({ defaultPath, filters });
  return typeof r === "string" ? r : null;
}

export async function reveal(path: string): Promise<void> {
  const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
  await revealItemInDir(path);
}

export async function openPath(path: string): Promise<void> {
  const { openPath: op } = await import("@tauri-apps/plugin-opener");
  await op(path);
}

export async function openUrl(url: string): Promise<void> {
  const { openUrl: ou } = await import("@tauri-apps/plugin-opener");
  await ou(url);
}

export function onToolProgress(cb: (p: ToolProgress) => void): Promise<UnlistenFn> {
  return listen<ToolProgress>("tool-progress", (e) => cb(e.payload));
}

export function pct(p: ToolProgress | null | undefined): number | null {
  if (!p || !p.total) return null;
  return Math.max(0, Math.min(100, Math.round((p.done / p.total) * 100)));
}

export function fmtBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = n;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v < 10 && i > 0 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

export function fmtMs(ms: number): string {
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")}` : `${m}:${String(sec).padStart(2, "0")}`;
}

export function fmtSecs(secs: number): string {
  return fmtMs(secs * 1000);
}

export function fmtUsd(v: number): string {
  if (v === 0) return "$0";
  if (v < 0.01) return `$${v.toFixed(4)}`;
  return `$${v.toFixed(2)}`;
}

export function baseName(path: string): string {
  return path.split(/[\\/]/).pop() ?? path;
}

export function dirName(path: string): string {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return i > 0 ? path.slice(0, i) : path;
}
