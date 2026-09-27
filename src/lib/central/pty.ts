// Typed bridge to the embedded terminal (src-tauri/src/pty). Sessions live in
// the app process; a view attaches (history + cursor), then follows the live
// `pty://data` stream, acking each chunk once xterm has written it.

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type PtyForeground = { running: boolean; label: string; pid: number | null };

export type PtySessionInfo = {
  id: string;
  title: string;
  cwd: string | null;
  command: string[];
  pid: number | null;
  cols: number;
  rows: number;
  alive: boolean;
  exit_code: number | null;
  exit_signal: string | null;
  created_at: number;
  foreground: PtyForeground;
};

export type PtyOpenRequest = {
  id?: string;
  cwd?: string | null;
  /** argv; empty opens the default shell. */
  command?: string[];
  env?: Record<string, string>;
  cols?: number;
  rows?: number;
  title?: string;
  /** Typed after start, not submitted. */
  initial_input?: string;
  history_cap?: number;
};

export type PtyAttached = {
  info: PtySessionInfo | null;
  /** base64 of the sanitized history. */
  history: string;
  generation: number;
  /** Absolute offset the history ends at. */
  offset: number;
  alive: boolean;
};

export type PtyDataEvent = { id: string; generation: number; seq: number; start: number; data: string };
export type PtyExitEvent = { id: string; code: number | null; signal: string | null };
export type PtyActivityEvent = { id: string } & PtyForeground;

/** What "add selection to chat" hands the composer. */
export type TerminalSelection = { text: string; lines: [number, number]; terminal_id: string };

export const ptyOpen = (req: PtyOpenRequest) => invoke<PtySessionInfo>("pty_open", { req });
export const ptyWrite = (id: string, data: string) => invoke<void>("pty_write", { id, data });
export const ptyResize = (id: string, cols: number, rows: number) => invoke<void>("pty_resize", { id, cols, rows });
export const ptyClose = (id: string, deleteHistory = false) => invoke<void>("pty_close", { id, deleteHistory });
export const ptyList = () => invoke<PtySessionInfo[]>("pty_list");
export const ptyAttach = (id: string) => invoke<PtyAttached>("pty_attach", { id });
export const ptyDetach = (id: string) => invoke<void>("pty_detach", { id });
export const ptyAck = (id: string, seq: number) => invoke<void>("pty_ack", { id, seq });
export const ptyClear = (id: string) => invoke<number>("pty_clear", { id });
export const ptyForeground = (id: string) => invoke<PtyForeground>("pty_foreground", { id });

export function decodeBase64(s: string): Uint8Array {
  const bin = atob(s);
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

export function errorCode(e: unknown): string {
  const s = String(e ?? "");
  const i = s.indexOf(":");
  return i > 0 ? s.slice(0, i) : s;
}

// One listener per event for the whole app, dispatched by terminal id.
export type PtyHandlers = {
  data?: (e: PtyDataEvent) => void;
  exit?: (e: PtyExitEvent) => void;
  activity?: (e: PtyActivityEvent) => void;
  stall?: () => void;
};

const subscribers = new Map<string, Set<PtyHandlers>>();
let hub: Promise<UnlistenFn[]> | null = null;

function dispatch<K extends keyof PtyHandlers>(id: string, key: K, arg: Parameters<NonNullable<PtyHandlers[K]>>[0]) {
  const set = subscribers.get(id);
  if (!set) return;
  for (const h of set) (h[key] as ((a: unknown) => void) | undefined)?.(arg);
}

function ensureHub(): Promise<UnlistenFn[]> {
  if (!hub) {
    hub = Promise.all([
      listen<PtyDataEvent>("pty://data", (e) => dispatch(e.payload.id, "data", e.payload)),
      listen<PtyExitEvent>("pty://exit", (e) => dispatch(e.payload.id, "exit", e.payload)),
      listen<PtyActivityEvent>("pty://activity", (e) => dispatch(e.payload.id, "activity", e.payload)),
      listen<{ id: string }>("pty://stall", (e) => dispatch(e.payload.id, "stall", undefined as never)),
    ]);
  }
  return hub;
}

/** Subscribes to one terminal's events. Resolves once the listeners are live,
 *  so an attach issued afterwards cannot miss a chunk. */
export async function subscribePty(id: string, handlers: PtyHandlers): Promise<() => void> {
  let set = subscribers.get(id);
  if (!set) {
    set = new Set();
    subscribers.set(id, set);
  }
  set.add(handlers);
  await ensureHub();
  return () => {
    const s = subscribers.get(id);
    if (!s) return;
    s.delete(handlers);
    if (s.size === 0) subscribers.delete(id);
  };
}

export function newTerminalId(): string {
  const r = typeof crypto !== "undefined" && "randomUUID" in crypto ? crypto.randomUUID() : `${Date.now()}-${Math.random().toString(36).slice(2)}`;
  return `term-${r}`;
}
