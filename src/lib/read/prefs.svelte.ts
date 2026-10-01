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
  }
}

export function resetPrefs() {
  Object.assign(prefs, DEFAULT_PREFS);
  savePrefs();
}

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
  }
}

export function deletePreset(name: string) {
  try {
    localStorage.setItem(PKEY, JSON.stringify(presets().filter((p) => p.name !== name)));
  } catch {
  }
}

export function applyPreset(p: Preset) {
  Object.assign(prefs, p.prefs);
  savePrefs();
}
