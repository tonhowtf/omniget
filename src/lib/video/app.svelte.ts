// App-level state of /video: server status, roots, scan progress and a
// library version that views can watch to reload after a scan.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import * as api from "./api";
import type { VideoStatus } from "./api";
import type { PlaybackInfo, RootKind, ScanProgress } from "./types";

const INCOGNITO_KEY = "omniget.video.incognito";

function loadIncognito(): boolean {
  try {
    return localStorage.getItem(INCOGNITO_KEY) === "1";
  } catch {
    return false;
  }
}

export const app = $state<{
  status: VideoStatus | null;
  loaded: boolean;
  error: string | null;
  scan: ScanProgress | null;
  /** bumps on every library change; views read it inside $effect to reload */
  version: number;
  incognito: boolean;
}>({
  status: null,
  loaded: false,
  error: null,
  scan: null,
  version: 0,
  incognito: loadIncognito(),
});

export function setIncognito(on: boolean) {
  app.incognito = on;
  try {
    localStorage.setItem(INCOGNITO_KEY, on ? "1" : "0");
  } catch {
    /* session only */
  }
}

export async function refreshStatus() {
  try {
    app.status = await api.status();
    app.error = null;
    if (app.status.scanning && !app.scan) app.scan = { done: 0, total: 0, current: "", finished: false };
  } catch (e) {
    app.error = String(e);
  } finally {
    app.loaded = true;
  }
}

let unlisten: UnlistenFn[] = [];
let listening = false;

/** Subscribes to backend events once per mount of the app. */
export async function startListening(): Promise<() => void> {
  if (listening) return () => {};
  listening = true;
  try {
    unlisten.push(
      await listen<ScanProgress>("video-scan-progress", (e) => {
        app.scan = e.payload.finished ? null : e.payload;
        if (e.payload.finished) {
          app.version++;
          void refreshStatus();
        }
      }),
    );
    unlisten.push(
      await listen("video-library-changed", () => {
        app.version++;
      }),
    );
  } catch {
    /* outside Tauri */
  }
  return () => {
    for (const u of unlisten) u();
    unlisten = [];
    listening = false;
  };
}

export async function addRoot(path: string, kind: RootKind) {
  await api.rootsAdd(path, kind);
  await refreshStatus();
  await startScan();
}

export async function removeRoot(path: string) {
  await api.rootsRemove(path);
  app.version++;
  await refreshStatus();
}

export async function startScan() {
  app.scan = { done: 0, total: 0, current: "", finished: false };
  try {
    await api.scan();
  } catch (e) {
    app.scan = null;
    throw e;
  }
}

/**
 * Plays a loose file that may not be in the library yet. Needs the backend
 * command `video_open_path` (requested in the W3 handoff); returns null when
 * it doesn't exist so the caller can fall back to adding the folder.
 */
export async function openPathInfo(path: string): Promise<PlaybackInfo | null> {
  try {
    return await invoke<PlaybackInfo>("video_open_path", { path });
  } catch (e) {
    const msg = String(e);
    if (/not found|unknown command|did not find/i.test(msg)) return null;
    throw e;
  }
}
