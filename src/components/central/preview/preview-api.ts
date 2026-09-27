// Front glue of the thread Browser tab and SnapShot. Commands live in
// src-tauri/src/commands/central/preview.rs; the preview itself is a native
// webview window anchored over the tab (see src-tauri/src/preview/mod.rs).
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type DevServer = {
  port: number;
  pid: number;
  command: string;
  cwd: string | null;
  url: string;
  inWorkspace: boolean;
  html: boolean;
  status: number | null;
  title: string | null;
};

export type PreviewState = {
  threadId: string;
  label: string;
  url: string;
  title: string;
  loading: boolean;
  visible: boolean;
};

export type Rect = { x: number; y: number; width: number; height: number; scale?: number };

export type WindowInfo = { id: string; app: string; title: string; pid: number | null; x: number; y: number; width: number; height: number };
export type WindowList = { windows: WindowInfo[]; permission: boolean; note: string | null };
export type Captured = { path: string; width: number | null; height: number | null; bytes: number };

export const EVENT_STATE = "preview://state";

export function rectOf(el: HTMLElement): Rect {
  const r = el.getBoundingClientRect();
  return { x: r.left, y: r.top, width: r.width, height: r.height, scale: window.devicePixelRatio || 1 };
}

export const previewApi = {
  discover: (cwd: string | null, all = false) => invoke<DevServer[]>("preview_discover_ports", { cwd, all }),
  open: (threadId: string, url: string | null, rect: Rect | null) => invoke<PreviewState>("preview_open", { threadId, url, rect }),
  setBounds: (threadId: string, rect: Rect) => invoke<void>("preview_set_bounds", { threadId, rect }),
  hide: (threadId: string | null) => invoke<void>("preview_hide", { threadId }),
  close: (threadId: string) => invoke<void>("preview_close", { threadId }),
  navigate: (threadId: string, url: string) => invoke<PreviewState>("preview_navigate", { threadId, url }),
  history: (threadId: string, action: "back" | "forward" | "reload") => invoke<PreviewState>("preview_history", { threadId, action }),
  state: (threadId: string | null) => invoke<PreviewState[]>("preview_state", { threadId }),
  screenshot: (threadId: string) => invoke<Captured>("preview_screenshot", { threadId }),
  listWindows: () => invoke<WindowList>("snapshot_list_windows"),
  capture: (windowId: string) => invoke<Captured>("snapshot_capture", { windowId }),
  requestPermission: () => invoke<boolean>("snapshot_request_permission"),
};

export function onPreviewState(cb: (s: PreviewState) => void): Promise<UnlistenFn> {
  return listen<PreviewState>(EVENT_STATE, (e) => cb(e.payload));
}

/** "CODE: message" → message (the code stays in the title for support). */
export function errText(e: unknown): string {
  const s = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
  const m = /^[A-Z][A-Z0-9_]+:\s*(.*)$/s.exec(s);
  return m ? m[1] : s;
}

/** Last URL typed per thread, so reopening the tab goes back to it. */
const LAST_KEY = "omniget.central.preview.last.v1";
export function lastUrl(threadId: string): string | null {
  try {
    const m = JSON.parse(localStorage.getItem(LAST_KEY) ?? "{}") as Record<string, string>;
    return m[threadId] ?? null;
  } catch {
    return null;
  }
}
export function rememberUrl(threadId: string, url: string) {
  try {
    const m = JSON.parse(localStorage.getItem(LAST_KEY) ?? "{}") as Record<string, string>;
    m[threadId] = url;
    const keys = Object.keys(m);
    if (keys.length > 200) for (const k of keys.slice(0, keys.length - 200)) delete m[k];
    localStorage.setItem(LAST_KEY, JSON.stringify(m));
  } catch {
    /* private mode */
  }
}

/** Threads whose right panel shows the Browser surface (memory only). */
export const browserTabs = new Set<string>();
