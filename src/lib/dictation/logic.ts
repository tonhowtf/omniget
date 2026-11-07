import type { Platform, Replacement, StateEvent } from "./types";

export type KeyLike = {
  key: string;
  code: string;
  altKey: boolean;
  ctrlKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
};

const MODIFIER_CODES = new Set([
  "AltLeft",
  "AltRight",
  "ControlLeft",
  "ControlRight",
  "ShiftLeft",
  "ShiftRight",
  "MetaLeft",
  "MetaRight",
  "OSLeft",
  "OSRight",
  "CapsLock",
  "Fn",
]);

const CODE_NAMES: Record<string, string> = {
  Space: "Space",
  Enter: "Enter",
  NumpadEnter: "Enter",
  Tab: "Tab",
  Backspace: "Backspace",
  Delete: "Delete",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  Insert: "Insert",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Comma: "Comma",
  Period: "Period",
  Slash: "Slash",
  Backslash: "Backslash",
  Semicolon: "Semicolon",
  Quote: "Quote",
  BracketLeft: "BracketLeft",
  BracketRight: "BracketRight",
  Backquote: "Backquote",
  Minus: "Minus",
  Equal: "Equal",
  Escape: "Escape",
};

export function keyFromCode(code: string): string | null {
  if (CODE_NAMES[code]) return CODE_NAMES[code];
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1];
  const digit = /^(?:Digit|Numpad)([0-9])$/.exec(code);
  if (digit) return digit[1];
  const fn = /^F([1-9]|1[0-9]|2[0-4])$/.exec(code);
  if (fn) return `F${fn[1]}`;
  return null;
}

export function modifiersOf(e: KeyLike): string[] {
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  return mods;
}

export type Captured = { combo: string | null; held: string[]; cancel: boolean };

export function captureCombo(e: KeyLike): Captured {
  const held = modifiersOf(e);
  if (e.code === "Escape" && held.length === 0) return { combo: null, held, cancel: true };
  if (MODIFIER_CODES.has(e.code)) return { combo: null, held, cancel: false };
  const key = keyFromCode(e.code);
  if (!key) return { combo: null, held, cancel: false };
  return { combo: [...held, key].join("+"), held, cancel: false };
}

const MAC_MODS: Record<string, string> = { Ctrl: "⌃", Alt: "⌥", Shift: "⇧", Super: "⌘" };
const ARROWS: Record<string, string> = { Up: "↑", Down: "↓", Left: "←", Right: "→" };
const PUNCT: Record<string, string> = {
  Comma: ",",
  Period: ".",
  Slash: "/",
  Backslash: "\\",
  Semicolon: ";",
  Quote: "'",
  BracketLeft: "[",
  BracketRight: "]",
  Backquote: "`",
  Minus: "-",
  Equal: "=",
};

export function shortcutCaps(combo: string, platform: Platform): string[] {
  if (!combo.trim()) return [];
  return combo.split("+").map((part) => {
    if (platform === "macos" && MAC_MODS[part]) return MAC_MODS[part];
    if (part === "Super") return platform === "windows" ? "Win" : "Super";
    return ARROWS[part] ?? PUNCT[part] ?? part;
  });
}

export function splitError(raw: string): { code: string; detail: string | null } {
  const m = /^([A-Z][A-Z0-9_]+)(?::\s*([\s\S]*))?$/.exec(raw.trim());
  if (!m) return { code: "UNKNOWN", detail: raw.trim() || null };
  return { code: m[1], detail: m[2]?.trim() || null };
}

const KNOWN_ERRORS = new Set([
  "ASR_UNAVAILABLE",
  "MODEL_MISSING",
  "MODEL_DOWNLOAD_FAILED",
  "FFMPEG_MISSING",
  "NO_MICROPHONE",
  "MIC_FAILED",
  "SILENT",
  "TOO_SHORT",
  "TRANSCRIBE_FAILED",
  "EMPTY_TEXT",
  "CLIPBOARD_FAILED",
  "PASTE_ACCESSIBILITY",
  "PASTE_AUTOMATION",
  "PASTE_NO_TOOL",
  "PASTE_FAILED",
  "BUSY",
  "SAVE_FAILED",
  "NO_DATA_DIR",
]);

const SHORTCUT_ERRORS = new Set(["EMPTY", "NO_KEY", "TOO_MANY_KEYS", "UNKNOWN_KEY", "NEEDS_MODIFIER", "RESERVED", "CONFLICT_APP", "TAKEN"]);

export function errorKey(code: string): string {
  return KNOWN_ERRORS.has(code) ? `dictation.err_${code.toLowerCase()}` : "dictation.err_unknown";
}

export function isShortcutError(code: string): boolean {
  return SHORTCUT_ERRORS.has(code);
}

export function shortcutErrorKey(code: string): string {
  return SHORTCUT_ERRORS.has(code) ? `dictation.sc_${code.toLowerCase()}` : "dictation.err_unknown";
}

export type ErrorAction = "download_model" | "open_accessibility" | "open_automation" | "open_microphone" | "retry" | null;

export function errorAction(code: string, platform: Platform): ErrorAction {
  switch (code) {
    case "MODEL_MISSING":
    case "MODEL_DOWNLOAD_FAILED":
      return "download_model";
    case "PASTE_ACCESSIBILITY":
      return platform === "macos" ? "open_accessibility" : null;
    case "PASTE_AUTOMATION":
      return platform === "macos" ? "open_automation" : null;
    case "SILENT":
    case "MIC_FAILED":
    case "NO_MICROPHONE":
      return platform === "linux" ? "retry" : "open_microphone";
    case "TOO_SHORT":
    case "EMPTY_TEXT":
    case "TRANSCRIBE_FAILED":
      return "retry";
    default:
      return null;
  }
}

export function formatElapsed(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const m = Math.floor(total / 60);
  const s = total % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

export function nearLimit(elapsedMs: number, maxMs: number): boolean {
  return maxMs > 0 && maxMs - elapsedMs <= 10_000;
}

export function duplicateFinds(list: Replacement[]): Set<number> {
  const seen = new Map<string, number>();
  const dups = new Set<number>();
  list.forEach((r, i) => {
    const key = r.find.trim().toLocaleLowerCase();
    if (!key) return;
    const first = seen.get(key);
    if (first === undefined) seen.set(key, i);
    else {
      dups.add(first);
      dups.add(i);
    }
  });
  return dups;
}

export const LANGUAGES = ["auto", "en", "pt", "es", "fr", "de", "it", "nl", "pl", "ru", "uk", "tr", "el", "ar", "fa", "hi", "ja", "ko", "zh", "vi", "id", "sv", "lo"];

export function languageLabel(code: string, uiLocale: string | null | undefined): string {
  try {
    const names = new Intl.DisplayNames([uiLocale || "en"], { type: "language" });
    const label = names.of(code);
    if (label && label !== code) return label.charAt(0).toLocaleUpperCase(uiLocale || "en") + label.slice(1);
  } catch {
    return code;
  }
  return code;
}

export function isBusy(state: StateEvent | null): boolean {
  return state?.phase === "listening" || state?.phase === "transcribing";
}

export function acceptState(current: StateEvent | null, next: StateEvent): boolean {
  if (!current) return true;
  return next.session >= current.session;
}

export function insertAt(value: string, text: string, start: number, end: number): { value: string; caret: number } {
  const s = Math.max(0, Math.min(start, value.length));
  const e = Math.max(s, Math.min(end, value.length));
  const before = value.slice(0, s);
  const after = value.slice(e);
  const pad = before && !/\s$/.test(before) ? " " : "";
  const insert = pad + text;
  return { value: before + insert + after, caret: s + insert.length };
}

export function formatBytes(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
  if (bytes >= 1_000_000) return `${Math.round(bytes / 1_000_000)} MB`;
  if (bytes >= 1_000) return `${Math.round(bytes / 1_000)} KB`;
  return `${bytes} B`;
}
