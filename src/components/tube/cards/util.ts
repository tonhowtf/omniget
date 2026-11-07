import { go, type Route } from "$lib/tube/ui.svelte";
import { openVideo } from "$lib/tube/video.svelte";
import { showToast } from "$lib/stores/toast-store.svelte";
import type { Item, Nav, StreamSet, TextRun } from "$lib/tube/types";
import * as api from "$lib/tube/api";

export { errText } from "$lib/tube/format";

export function watchUrl(id: string, t?: number | null): string {
  return `https://www.youtube.com/watch?v=${id}${t && t > 0 ? `&t=${Math.floor(t)}s` : ""}`;
}

export function itemUrl(item: Item): string {
  switch (item.kind) {
    case "video":
      return watchUrl(item.id);
    case "short":
      return `https://www.youtube.com/shorts/${item.id}`;
    case "playlist":
      return `https://www.youtube.com/playlist?list=${item.id}`;
    case "channel":
      return `https://www.youtube.com/channel/${item.id}`;
    default:
      return "";
  }
}

export function watchHref(id: string, list?: string | null): string {
  return `/youtube?v=${encodeURIComponent(id)}${list ? `&list=${encodeURIComponent(list)}` : ""}`;
}

export function channelHref(id: string): string {
  return `/youtube?c=${encodeURIComponent(id)}`;
}

export function playlistHref(id: string): string {
  return `/youtube?list=${encodeURIComponent(id)}`;
}

export function openWatch(id: string, opts: { list?: string | null; index?: number | null; t?: number | null } = {}) {
  try {
    openVideo(id, opts);
  } catch {
    go({ name: "watch", id, list: opts.list ?? null, index: opts.index ?? null, t: opts.t ?? null });
  }
}

export function openChannel(id: string) {
  go({ name: "channel", id });
}

export function openPlaylist(id: string) {
  go({ name: "playlist", id });
}

export function routeForNav(nav: Nav | null | undefined): Route | null {
  if (!nav) return null;
  const b = nav.browse_id;
  if (b === "FEhistory") return { name: "history" };
  if (b === "FEsubscriptions" || b === "FEchannels") return { name: "subscriptions" };
  if (b === "FEshorts") return { name: "shorts" };
  if (b === "FElibrary" || b === "FEyou") return { name: "library" };
  if (b.startsWith("VL")) return { name: "playlist", id: b.slice(2) };
  if (b === "WL" || b === "LL" || b.startsWith("PL")) return { name: "playlist", id: b };
  if (b.startsWith("UC") || b.startsWith("@")) return { name: "channel", id: b };
  return null;
}

export async function followRun(run: TextRun, onSeek?: (t: number) => void, currentVideo?: string): Promise<void> {
  if (run.t != null && (!run.video_id || run.video_id === currentVideo) && onSeek) {
    onSeek(run.t);
    return;
  }
  if (run.video_id) {
    openWatch(run.video_id, { t: run.t ?? null });
    return;
  }
  if (run.channel_id) {
    openChannel(run.channel_id);
    return;
  }
  if (run.url) {
    const { open } = await import("@tauri-apps/plugin-shell");
    await open(run.url);
  }
}

export async function openExternal(url: string) {
  const { open } = await import("@tauri-apps/plugin-shell");
  await open(url);
}

export async function copyText(text: string, okMsg: string) {
  try {
    const { writeText } = await import("@tauri-apps/plugin-clipboard-manager");
    await writeText(text);
  } catch {
    await navigator.clipboard?.writeText(text);
  }
  showToast("success", okMsg);
}

const streamCache = new Map<string, Promise<StreamSet>>();

export function streamsFor(id: string): Promise<StreamSet> {
  const hit = streamCache.get(id);
  if (hit) return hit;
  const p = api.streams(id).then(
    (s) => {
      const ttl = Math.max(60_000, s.expires_ms - Date.now() - 5 * 60_000);
      setTimeout(() => streamCache.delete(id), ttl);
      return s;
    },
    (e) => {
      streamCache.delete(id);
      throw e;
    },
  );
  streamCache.set(id, p);
  return p;
}

export function streamUrl(set: StreamSet, itag: number): string {
  return `${set.base}/${itag}?k=${encodeURIComponent(set.token)}`;
}

export function dayBucket(ms: number, now = Date.now()): "today" | "yesterday" | "week" | "month" | "older" {
  const d = new Date(ms);
  const start = new Date(now);
  start.setHours(0, 0, 0, 0);
  const day = 24 * 3600e3;
  if (d.getTime() >= start.getTime()) return "today";
  if (d.getTime() >= start.getTime() - day) return "yesterday";
  if (d.getTime() >= start.getTime() - 6 * day) return "week";
  if (d.getTime() >= start.getTime() - 30 * day) return "month";
  return "older";
}

export function lsGet<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    return raw == null ? fallback : (JSON.parse(raw) as T);
  } catch {
    return fallback;
  }
}

export function lsSet(key: string, value: unknown) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
  }
}
