// Reader preferences: per device, in the webview (like the music player's
// EQ). A book or series can override the comic layout; that part is saved
// with the book in read.db (`prefs` / `reader`).

import type { ReaderPrefs } from "./foliate";
import { DEFAULT_PREFS } from "./prefs-defaults";

export { DEFAULT_PREFS };

const KEY = "omniget.read.prefs.v1";

function load(): ReaderPrefs {
  try {
    return { ...DEFAULT_PREFS, ...JSON.parse(localStorage.getItem(KEY) ?? "{}") };
  } catch {
    return { ...DEFAULT_PREFS };
  }
}

export const prefs = $state<ReaderPrefs>(load());

export function savePrefs() {
  try {
    localStorage.setItem(KEY, JSON.stringify($state.snapshot(prefs)));
  } catch {
    /* private mode */
  }
}

export function resetPrefs() {
  Object.assign(prefs, DEFAULT_PREFS);
  savePrefs();
}

// Named presets ("Night manga", "Study PDF"...), like Moku's reader presets.
const PKEY = "omniget.read.presets.v1";

export type Preset = { name: string; prefs: Partial<ReaderPrefs> };

export function presets(): Preset[] {
  try {
    return JSON.parse(localStorage.getItem(PKEY) ?? "[]");
  } catch {
    return [];
  }
}

export function savePreset(name: string) {
  const list = presets().filter((p) => p.name !== name);
  list.push({ name, prefs: $state.snapshot(prefs) });
  try {
    localStorage.setItem(PKEY, JSON.stringify(list));
  } catch {
    /* ignore */
  }
}

export function deletePreset(name: string) {
  try {
    localStorage.setItem(PKEY, JSON.stringify(presets().filter((p) => p.name !== name)));
  } catch {
    /* ignore */
  }
}

export function applyPreset(p: Preset) {
  Object.assign(prefs, p.prefs);
  savePrefs();
}
