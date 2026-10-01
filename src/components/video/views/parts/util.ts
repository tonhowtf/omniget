import { listen } from "@tauri-apps/api/event";
import { ensureServer, serverUrl } from "$lib/video/api";
import type { Progress } from "$lib/video/types";

export async function ready(): Promise<void> {
  await ensureServer().catch(() => null);
}

export function img(p: string | null | undefined): string | null {
  return serverUrl(p);
}

export function fmtDur(ms: number | null | undefined): string {
  if (!ms || ms < 0) return "";
  const s = Math.round(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (h) return `${h}h ${String(m).padStart(2, "0")}m`;
  if (m) return `${m}m`;
  return `${s}s`;
}

export function fmtClock(ms: number | null | undefined): string {
  const s = Math.max(0, Math.floor((ms ?? 0) / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${sec}` : `${m}:${sec}`;
}

export function fmtHour(ts: number): string {
  return new Date(ts).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

export function fmtDay(ts: number): string {
  const d = new Date(ts);
  const today = new Date();
  const y = new Date(today);
  y.setDate(today.getDate() - 1);
  if (d.toDateString() === today.toDateString()) return "today";
  if (d.toDateString() === y.toDateString()) return "yesterday";
  return d.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short", year: d.getFullYear() === today.getFullYear() ? undefined : "numeric" });
}

export function ratio(p: Progress | null | undefined): number | null {
  if (!p || p.watched || !p.duration_ms || p.position_ms <= 0) return null;
  return Math.min(1, p.position_ms / p.duration_ms);
}

export function hue(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0;
  return h % 360;
}

export function stripHtml(s: string | null | undefined): string {
  if (!s) return "";
  return s
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<[^>]+>/g, "")
    .replace(/&amp;/g, "&")
    .replace(/&quot;/g, '"')
    .replace(/&#0?39;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .trim();
}

export function lsGet<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw ? (JSON.parse(raw) as T) : fallback;
  } catch {
    return fallback;
  }
}

export function lsSet(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
  }
}

export function onLibraryChanged(cb: () => void): () => void {
  let off: (() => void) | null = null;
  let dead = false;
  void listen("video-library-changed", () => cb())
    .then((u) => {
      if (dead) u();
      else off = u;
    })
    .catch(() => {});
  return () => {
    dead = true;
    off?.();
  };
}

export function errText(e: unknown): string {
  if (e instanceof Error) return e.message;
  return typeof e === "string" ? e : JSON.stringify(e);
}

export function baseName(p: string): string {
  return p.replace(/[\\/]+$/, "").split(/[\\/]/).pop() || p;
}
