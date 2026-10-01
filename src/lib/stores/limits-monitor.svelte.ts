import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
export type Edge = 'top' | 'right' | 'bottom' | 'left';
export type StripSize = 's' | 'm' | 'l';
export type Prefs = { enabled: boolean; edge: Edge; along: Record<string, number>; providers: { id: string; enabled: boolean; muted: boolean }[]; notify_thresholds: boolean; thresholds: number[]; notify_reset: boolean; notify_pace: boolean; size: StripSize; show_percent: boolean; show_pace: boolean; contrast: boolean };
type AccountTag = { label: string | null; email: string | null };
export type ProviderInfo = { id: string; label: string; local: boolean; beta: boolean; detected: boolean; account?: AccountTag | null };
type Described = { prefs: Prefs; open: boolean; providers: ProviderInfo[] };
type MonitorRing = { id: string; label: string; account?: AccountTag | null; status: string; read_at: number | null; reading: { windows: { id: string; label: string; used: number | null; resets_at: number | null }[] } | null };
let prefs = $state<Prefs | null>(null);
let providers = $state<ProviderInfo[]>([]);
let rings = $state<MonitorRing[]>([]);
let busy = $state(false);
let error = $state<string | null>(null);
let consumers = 0;
let generation = 0;
let off: (() => void)[] = [];
function take(value: Described) { prefs = value.prefs; providers = value.providers; }
export function getLimitsMonitor() { return { prefs, providers, rings, busy, error }; }
export function acquireLimitsMonitor() {
  consumers++;
  if (consumers === 1) {
    const token = ++generation;
    error = null;
    void (async () => {
      try {
        for (const [name, handler] of [
          ['limits://prefs', (payload: Prefs) => { prefs = payload; }],
          ['limits://state', (payload: { rings: MonitorRing[] }) => { rings = payload.rings; }],
        ] as const) {
          const stop = await listen<any>(name, e => { if (token === generation) (handler as (p: any) => void)(e.payload); });
          if (token !== generation) { stop(); return; }
          off.push(stop);
        }
        const described = await invoke<Described>('limits_strip_get_prefs');
        if (token !== generation) return;
        take(described);
        const snapshot = await invoke<{ rings: MonitorRing[] }>('limits_strip_state');
        if (token === generation) rings = snapshot.rings;
      } catch (e) { if (token === generation) error = String(e); }
    })();
  }
  let released = false;
  return () => {
    if (released) return;
    released = true;
    if (--consumers === 0) { generation++; off.forEach(stop => stop()); off = []; }
  };
}
export async function saveLimitsPrefs(next: Prefs) {
  if (busy) return false;
  busy = true; error = null;
  const before = prefs;
  try { take(await invoke<Described>('limits_strip_set_prefs', { prefs: next })); return true; }
  catch (e) { prefs = before; error = String(e); return false; }
  finally { busy = false; }
}
