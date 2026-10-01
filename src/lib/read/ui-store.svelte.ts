import { invoke } from "@tauri-apps/api/core";
import { get } from "svelte/store";
import { t } from "$lib/i18n";
import { showToast } from "$lib/stores/toast-store.svelte";
import type { MenuItem } from "$components/music/ContextMenu.svelte";
import type { Book, Chapter, Hit, Series } from "./types";

export type View =
  | { kind: "home" }
  | { kind: "library"; shelf?: number; filter?: string }
  | { kind: "book"; id: number }
  | { kind: "manga" }
  | { kind: "series"; id: number }
  | { kind: "updates" }
  | { kind: "browse"; source?: string; q?: string }
  | { kind: "hit"; hit: Hit }
  | { kind: "annotations" }
  | { kind: "downloads" }
  | { kind: "stats" }
  | { kind: "tools"; tab?: string; ids?: number[] }
  | { kind: "settings"; tab?: string };

export type ReaderTarget =
  | { kind: "book"; id: number; at?: string }
  | { kind: "chapter"; chapterId: number; seriesId: number };

export const ui = $state({
  view: { kind: "home" } as View,
  stack: [] as View[],
  reader: null as ReaderTarget | null,
  menu: null as { x: number; y: number; items: MenuItem[] } | null,
  confirm: null as { title: string; body: string; danger?: boolean; ok: string; run: () => void } | null,
  shelfPicker: null as number[] | null,
  selection: [] as number[],
  search: "",
});

export const tr = (k: string, vars?: Record<string, unknown>) => get(t)(k, vars) as string;

export function go(v: View) {
  const cur = ui.view;
  if (JSON.stringify(cur) === JSON.stringify(v)) return;
  ui.stack = [...ui.stack.slice(-30), cur];
  ui.view = v;
  ui.selection = [];
}

export function back() {
  const prev = ui.stack[ui.stack.length - 1];
  if (!prev) return;
  ui.stack = ui.stack.slice(0, -1);
  ui.view = prev;
}

export function openMenu(e: MouseEvent, items: MenuItem[]) {
  e.preventDefault();
  e.stopPropagation();
  ui.menu = { x: e.clientX, y: e.clientY, items };
}

export function openBook(b: Pick<Book, "id">, at?: string) {
  ui.reader = { kind: "book", id: b.id, at };
}

export async function openChapter(ch: Pick<Chapter, "id" | "series_id">) {
  ui.reader = { kind: "chapter", chapterId: ch.id, seriesId: ch.series_id };
}

export function confirm(title: string, body: string, ok: string, run: () => void, danger = true) {
  ui.confirm = { title, body, ok, run, danger };
}

export async function toggleFavorite(b: Book) {
  try {
    await invoke("read_update_book", { id: b.id, patch: { favorite: !b.favorite } });
  } catch (e) {
    showToast("error", String(e));
  }
}

export async function setStatus(ids: number[], status: string) {
  try {
    await invoke("read_bulk_update", { ids, status });
  } catch (e) {
    showToast("error", String(e));
  }
}

export async function followSeries(s: Series, follow: boolean) {
  await invoke("read_series_follow", { id: s.id, follow });
  showToast("success", follow ? tr("read.series.followed") : tr("read.series.unfollowed"));
}

export function fmtDuration(ms: number) {
  const m = Math.round(ms / 60000);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  return `${h} h ${m % 60 ? `${m % 60} min` : ""}`.trim();
}

export function fmtBytes(n: number) {
  if (n < 1024) return `${n} B`;
  if (n < 1048576) return `${(n / 1024).toFixed(0)} KB`;
  if (n < 1073741824) return `${(n / 1048576).toFixed(1)} MB`;
  return `${(n / 1073741824).toFixed(2)} GB`;
}

export function fmtDate(ms: number | null | undefined) {
  if (!ms) return "";
  return new Date(ms).toLocaleDateString();
}

export function relTime(ms: number | null | undefined) {
  if (!ms) return "";
  const d = Date.now() - ms;
  const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  if (d < 3_600_000) return rtf.format(-Math.max(1, Math.round(d / 60_000)), "minute");
  if (d < 86_400_000) return rtf.format(-Math.round(d / 3_600_000), "hour");
  if (d < 30 * 86_400_000) return rtf.format(-Math.round(d / 86_400_000), "day");
  if (d < 365 * 86_400_000) return rtf.format(-Math.round(d / (30 * 86_400_000)), "month");
  return rtf.format(-Math.round(d / (365 * 86_400_000)), "year");
}

export function chapterLabel(c: Pick<Chapter, "number" | "volume" | "title">) {
  const parts: string[] = [];
  if (c.volume != null) parts.push(`Vol. ${c.volume}`);
  if (c.number != null) parts.push(`${tr("read.series.ch")} ${c.number}`);
  const title = c.title && !/^#?\d+$/.test(c.title.trim()) ? c.title : "";
  return [parts.join(" "), title].filter(Boolean).join(" · ") || c.title;
}
