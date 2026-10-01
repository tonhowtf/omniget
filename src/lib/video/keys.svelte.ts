// Remappable player shortcuts. A binding is the KeyboardEvent.key value, so
// "N" means Shift+n. Overrides persist per device in localStorage.

export type KeyAction =
  | "toggle"
  | "toggle_alt"
  | "back10"
  | "fwd10"
  | "back5"
  | "fwd5"
  | "vol_up"
  | "vol_down"
  | "mute"
  | "fullscreen"
  | "pip"
  | "frame_back"
  | "frame_fwd"
  | "speed_down"
  | "speed_up"
  | "speed_reset"
  | "sub_cycle"
  | "sub_delay_minus"
  | "sub_delay_plus"
  | "sub_next_now"
  | "ab_loop"
  | "line_prev"
  | "line_next"
  | "line_repeat"
  | "auto_pause"
  | "skip_silence"
  | "note"
  | "card"
  | "panel"
  | "info"
  | "next"
  | "prev"
  | "shortcuts";

const DEFAULT_KEYS: Record<KeyAction, string> = {
  toggle: " ",
  toggle_alt: "k",
  back10: "j",
  fwd10: "l",
  back5: "ArrowLeft",
  fwd5: "ArrowRight",
  vol_up: "ArrowUp",
  vol_down: "ArrowDown",
  mute: "m",
  fullscreen: "f",
  pip: "p",
  frame_back: ",",
  frame_fwd: ".",
  speed_down: "[",
  speed_up: "]",
  speed_reset: "=",
  sub_cycle: "c",
  sub_delay_minus: "g",
  sub_delay_plus: "h",
  sub_next_now: "H",
  ab_loop: "b",
  line_prev: "a",
  line_next: "d",
  line_repeat: "r",
  auto_pause: "u",
  skip_silence: "s",
  note: "n",
  card: "C",
  panel: "t",
  info: "i",
  next: "N",
  prev: "P",
  shortcuts: "?",
};

/** Groups for the shortcut sheet, in display order. */
export const KEY_GROUPS: { id: string; actions: KeyAction[] }[] = [
  { id: "playback", actions: ["toggle", "toggle_alt", "back10", "fwd10", "back5", "fwd5", "frame_back", "frame_fwd", "next", "prev"] },
  { id: "sound_view", actions: ["vol_up", "vol_down", "mute", "fullscreen", "pip", "panel", "info"] },
  { id: "speed", actions: ["speed_down", "speed_up", "speed_reset", "skip_silence"] },
  { id: "subtitles", actions: ["sub_cycle", "sub_delay_minus", "sub_delay_plus", "sub_next_now"] },
  { id: "study", actions: ["ab_loop", "line_prev", "line_next", "line_repeat", "auto_pause", "note", "card", "shortcuts"] },
];

const KEYS_KEY = "omniget.video.keys.v1";

function load(): Record<KeyAction, string> {
  try {
    const raw = localStorage.getItem(KEYS_KEY);
    return raw ? { ...DEFAULT_KEYS, ...JSON.parse(raw) } : { ...DEFAULT_KEYS };
  } catch {
    return { ...DEFAULT_KEYS };
  }
}

export const keymap = $state<Record<KeyAction, string>>(load());

function save() {
  try {
    localStorage.setItem(KEYS_KEY, JSON.stringify($state.snapshot(keymap)));
  } catch {
    /* storage unavailable: bindings last for this session */
  }
}

/** Binds `key` to `action`, unbinding whatever action used it before. */
export function rebind(action: KeyAction, key: string) {
  for (const a of Object.keys(keymap) as KeyAction[]) {
    if (a !== action && keymap[a] === key) keymap[a] = "";
  }
  keymap[action] = key;
  save();
}

export function resetKeys() {
  Object.assign(keymap, DEFAULT_KEYS);
  save();
}

export function actionFor(key: string): KeyAction | null {
  for (const a of Object.keys(keymap) as KeyAction[]) {
    if (keymap[a] === key) return a;
  }
  // digits are fixed: 0-9 jump to 0-90 %
  return null;
}

/** Human label for a binding ("Space", "←"). */
export function keyLabel(key: string): string {
  const names: Record<string, string> = {
    " ": "Space",
    ArrowLeft: "←",
    ArrowRight: "→",
    ArrowUp: "↑",
    ArrowDown: "↓",
    Escape: "Esc",
    Enter: "↵",
    Tab: "⇥",
  };
  if (names[key]) return names[key];
  if (key.length === 1 && key !== key.toLowerCase()) return `⇧ ${key.toLowerCase()}`;
  return key.length === 1 ? key.toUpperCase() : key;
}
