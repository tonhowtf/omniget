// Shared connection state for the `/telegram` screen. One listener on
// `telegram-net-status` keeps every panel (status strip, wizard, Connection
// tab) looking at the same snapshot.

import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { net, errText, type NetStatus, type RouteView } from "./api";

export const netState = $state<{
  status: NetStatus | null;
  scanning: boolean;
  error: string | null;
}>({ status: null, scanning: false, error: null });

let unlisten: UnlistenFn | null = null;
let refs = 0;

export async function attachNet(): Promise<void> {
  refs += 1;
  if (refs > 1) return;
  try {
    unlisten = await listen<NetStatus>("telegram-net-status", (ev) => {
      netState.status = ev.payload;
    });
  } catch {
    /* outside Tauri */
  }
  try {
    netState.status = await net.status();
  } catch (e) {
    netState.error = errText(e);
  }
}

export function detachNet(): void {
  refs = Math.max(0, refs - 1);
  if (refs === 0) {
    unlisten?.();
    unlisten = null;
  }
}

/** Runs a full scan. Concurrent callers share the running one. */
let running: Promise<NetStatus | null> | null = null;
export function scanNet(): Promise<NetStatus | null> {
  if (running) return running;
  netState.scanning = true;
  netState.error = null;
  running = net
    .scan()
    .then((s) => {
      netState.status = s;
      return s;
    })
    .catch((e) => {
      netState.error = errText(e);
      return null;
    })
    .finally(() => {
      netState.scanning = false;
      running = null;
    });
  return running;
}

export async function refreshNet(): Promise<void> {
  try {
    netState.status = await net.status();
  } catch (e) {
    netState.error = errText(e);
  }
}

export function isScanning(): boolean {
  return netState.scanning || netState.status?.phase === "scanning";
}

export function workingRoutes(s: NetStatus | null): RouteView[] {
  return (s?.routes ?? []).filter((r) => r.health.ok === true);
}

export function hasWorkingRoute(s: NetStatus | null): boolean {
  return workingRoutes(s).length > 0;
}

/** Clock drift beyond this breaks MTProto's message ids. */
export const CLOCK_WARN_SECS = 30;
