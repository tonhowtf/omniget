import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Book, QueueItem, ReadStatusInfo, Series, Shelf } from "./types";

type ScanProgress = { done: number; total: number; current: string };

const state = $state({
  books: [] as Book[],
  shelves: [] as Shelf[],
  series: [] as Series[],
  status: null as ReadStatusInfo | null,
  loaded: false,
  loading: false,
  error: "",
  scan: null as ScanProgress | null,
  queue: [] as QueueItem[],
  updating: null as { done: number; total: number; title: string } | null,
  version: 0,
});

let byId = new Map<number, Book>();
let listening: UnlistenFn[] = [];
let refreshTimer: ReturnType<typeof setTimeout> | null = null;

export function library() {
  return state;
}

export function bookById(id: number): Book | undefined {
  return byId.get(id);
}

function reindex() {
  byId = new Map(state.books.map((b) => [b.id, b]));
  state.version++;
}

export async function ensureStatus(): Promise<ReadStatusInfo> {
  if (state.status) return state.status;
  state.status = await invoke<ReadStatusInfo>("read_status");
  return state.status;
}

export async function refreshStatus(): Promise<ReadStatusInfo> {
  state.status = await invoke<ReadStatusInfo>("read_status");
  return state.status;
}

export async function loadLibrary(force = false) {
  if (state.loading || (state.loaded && !force)) return;
  state.loading = true;
  state.error = "";
  try {
    await ensureStatus();
    const [books, shelves, series] = await Promise.all([
      invoke<Book[]>("read_books"),
      invoke<Shelf[]>("read_shelves"),
      invoke<Series[]>("read_series_list", { onlyLibrary: true }),
    ]);
    state.books = books;
    state.shelves = shelves;
    state.series = series;
    reindex();
    state.loaded = true;
  } catch (e) {
    state.error = String(e);
  } finally {
    state.loading = false;
  }
  startListening();
}

export function scheduleRefresh(ms = 250) {
  if (refreshTimer) clearTimeout(refreshTimer);
  refreshTimer = setTimeout(() => {
    refreshTimer = null;
    void loadLibrary(true);
  }, ms);
}

export async function refreshSeries() {
  try {
    state.series = await invoke<Series[]>("read_series_list", { onlyLibrary: true });
    state.version++;
  } catch {
  }
}

export async function refreshShelves() {
  try {
    state.shelves = await invoke<Shelf[]>("read_shelves");
    state.version++;
  } catch {
  }
}

export function patchBookLocal(b: Book) {
  const i = state.books.findIndex((x) => x.id === b.id);
  if (i >= 0) state.books[i] = b;
  else state.books = [...state.books, b];
  reindex();
}

export function upsertBooks(rows: Book[]) {
  for (const b of rows) patchBookLocal(b);
}

export function removeBooksLocal(ids: number[]) {
  const s = new Set(ids);
  state.books = state.books.filter((b) => !s.has(b.id));
  reindex();
}

function startListening() {
  if (listening.length) return;
  void Promise.all([
    listen<ScanProgress>("read-scan-progress", (e) => (state.scan = e.payload)),
    listen("read-scan-done", () => {
      state.scan = null;
      scheduleRefresh(50);
    }),
    listen("read-library-changed", () => scheduleRefresh()),
    listen("read-series-changed", () => void refreshSeries()),
    listen<QueueItem>("read-chapter-download", (e) => {
      const i = state.queue.findIndex((q) => q.chapter_id === e.payload.chapter_id);
      if (i >= 0) state.queue[i] = e.payload;
      else state.queue = [e.payload, ...state.queue];
    }),
    listen<{ done: number; total: number; title: string }>("read-updates-progress", (e) => (state.updating = e.payload)),
    listen("read-updates-done", () => {
      state.updating = null;
      void refreshSeries();
    }),
  ]).then((u) => (listening = u));
}

function base() {
  const s = state.status;
  return s ? `http://127.0.0.1:${s.port}` : "";
}

export function bookUrl(id: number) {
  const s = state.status;
  return s ? `${base()}/b/${id}?k=${s.token}` : "";
}

export function coverUrl(hash: string | null | undefined, thumb = true) {
  const s = state.status;
  if (!hash || !s) return "";
  return `${base()}/c/${hash}?k=${s.token}${thumb ? "&t=1" : ""}`;
}

export function pageUrl(bookId: number, n: number) {
  const s = state.status;
  return s ? `${base()}/pg/${bookId}/${n}?k=${s.token}` : "";
}

export function proxiedImage(url: string | null | undefined, referer = "") {
  const s = state.status;
  if (!url || !s) return url ?? "";
  if (!referer) return url;
  const h = encodeURIComponent(JSON.stringify([["Referer", referer]]));
  return `${base()}/img?k=${s.token}&u=${encodeURIComponent(url)}&h=${h}`;
}

export function seriesCover(s: Pick<Series, "cover" | "cover_url" | "source" | "url">) {
  if (s.cover) return coverUrl(s.cover);
  if (!s.cover_url) return "";
  return s.source.startsWith("decl:") ? proxiedImage(s.cover_url, s.url) : s.cover_url;
}

export async function addLibraryFolder(path: string) {
  const roots = await invoke<string[]>("read_roots_add", { path });
  if (state.status) state.status.roots = roots;
}
