// The music library as the webview sees it: every track row, the local
// server's address, and live scan/analysis progress. One instance for the
// whole app so the mini player and /music share it.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { MusicStatus, Track } from "./types";

type ScanProgress = { done: number; total: number; current: string };
type AnalysisProgress = { done: number; total: number; track_id: number; error: string | null };

const state = $state({
  tracks: [] as Track[],
  status: null as MusicStatus | null,
  loaded: false,
  loading: false,
  error: "",
  scan: null as ScanProgress | null,
  analysis: null as AnalysisProgress | null,
  /** bumped on every change so derived views recompute cheaply */
  version: 0,
});

let byId = new Map<number, Track>();
let listening: UnlistenFn[] = [];
let pendingRefresh = new Set<number>();
let refreshTimer: ReturnType<typeof setTimeout> | null = null;

export function library() {
  return state;
}

export function trackById(id: number): Track | undefined {
  return byId.get(id);
}

function reindex() {
  byId = new Map(state.tracks.map((t) => [t.id, t]));
  state.version++;
}

export async function ensureStatus(): Promise<MusicStatus> {
  if (state.status) return state.status;
  state.status = await invoke<MusicStatus>("music_status");
  return state.status;
}

export async function refreshStatus(): Promise<MusicStatus> {
  state.status = await invoke<MusicStatus>("music_status");
  return state.status;
}

export async function loadLibrary(force = false) {
  if (state.loading || (state.loaded && !force)) return;
  state.loading = true;
  state.error = "";
  try {
    await ensureStatus();
    state.tracks = await invoke<Track[]>("music_tracks");
    reindex();
    state.loaded = true;
    await startListening();
  } catch (e) {
    state.error = String(e);
  } finally {
    state.loading = false;
  }
}

export function upsertTracks(rows: Track[]) {
  if (!rows.length) return;
  const incoming = new Map(rows.map((r) => [r.id, r]));
  const next = state.tracks.map((t) => incoming.get(t.id) ?? t);
  for (const r of rows) if (!byId.has(r.id)) next.push(r);
  state.tracks = next;
  reindex();
}

export function removeTracksLocal(ids: number[]) {
  const drop = new Set(ids);
  state.tracks = state.tracks.filter((t) => !drop.has(t.id));
  reindex();
}

export function patchTracksLocal(ids: number[], patch: Partial<Track>) {
  const set = new Set(ids);
  state.tracks = state.tracks.map((t) => (set.has(t.id) ? { ...t, ...patch } : t));
  reindex();
}

/** Re-reads a few rows from Rust (after analysis or tag edits), batched. */
function refreshTracks(ids: number[]) {
  for (const id of ids) pendingRefresh.add(id);
  if (refreshTimer) return;
  refreshTimer = setTimeout(async () => {
    refreshTimer = null;
    const batch = [...pendingRefresh];
    pendingRefresh = new Set();
    try {
      upsertTracks(await invoke<Track[]>("music_tracks_by_ids", { ids: batch }));
    } catch {
      /* the next full load will catch up */
    }
  }, 400);
}

async function startListening() {
  if (listening.length) return;
  listening = await Promise.all([
    listen<ScanProgress>("music-scan-progress", (e) => {
      state.scan = e.payload;
    }),
    listen("music-scan-done", async () => {
      state.scan = null;
      try {
        state.tracks = await invoke<Track[]>("music_tracks");
        reindex();
        await refreshStatus();
      } catch {
        /* ignore */
      }
    }),
    listen<AnalysisProgress>("music-analysis-progress", (e) => {
      state.analysis = e.payload;
      refreshTracks([e.payload.track_id]);
    }),
    listen("music-analysis-done", async () => {
      state.analysis = null;
      // album gains were written for the whole batch
      try {
        state.tracks = await invoke<Track[]>("music_tracks");
        reindex();
      } catch {
        /* ignore */
      }
    }),
  ]);
}

// ---- local server URLs ----

function base(): string {
  const s = state.status;
  return s ? `http://127.0.0.1:${s.port}` : "";
}

function key(): string {
  return state.status ? `k=${encodeURIComponent(state.status.token)}` : "";
}

export function trackUrl(id: number): string {
  return `${base()}/t/${id}?${key()}`;
}

export function transcodeUrl(id: number, startS = 0): string {
  return `${base()}/x/${id}?${key()}${startS > 0 ? `&ss=${startS.toFixed(2)}` : ""}`;
}

export function coverUrl(hash: string | null | undefined, thumb = true): string {
  if (!hash || !state.status) return "";
  return `${base()}/c/${hash}?${key()}${thumb ? "&t=1" : ""}`;
}

export function radioUrl(url: string, session: string): string {
  return `${base()}/r?${key()}&s=${encodeURIComponent(session)}&u=${encodeURIComponent(url)}`;
}

export function proxyUrl(url: string): string {
  return `${base()}/p?${key()}&u=${encodeURIComponent(url)}`;
}

/** Asks for a folder and adds it to the library (the scan starts in Rust). */
export async function addLibraryFolder(): Promise<boolean> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open({ directory: true, multiple: true });
  const paths = Array.isArray(picked) ? picked : picked ? [picked] : [];
  if (!paths.length) return false;
  for (const p of paths) {
    await invoke<string[]>("music_roots_add", { path: p });
  }
  await refreshStatus();
  if (!state.loaded) await loadLibrary(true);
  return true;
}

export async function rescan(full = false) {
  await invoke("music_scan", { full }).catch(() => {});
}
