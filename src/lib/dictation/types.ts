export type Mode = "hold" | "toggle";
export type Output = "paste" | "copy";
export type Phase = "idle" | "listening" | "transcribing" | "done" | "error";
export type Source = "page" | "shortcut";
export type Outcome = "pasted" | "copied" | "inserted";
export type Platform = "macos" | "windows" | "linux";

export type Replacement = { find: string; replace: string };

export type Settings = {
  enabled: boolean;
  shortcut: string;
  mode: Mode;
  device: string | null;
  language: string;
  model: string;
  output: Output;
  restore_clipboard: boolean;
  capitalize: boolean;
  remove_fillers: boolean;
  replacements: Replacement[];
  max_seconds: number;
  save_history: boolean;
};

export type Entry = {
  id: string;
  text: string;
  raw: string;
  created_at: number;
  duration_ms: number;
  language: string;
  model: string;
  outcome: Outcome | string;
};

export type ErrorInfo = { code: string; detail: string | null };

export type StateEvent = {
  phase: Phase;
  session: number;
  source: Source;
  started_at: number | null;
  max_ms: number;
  entry: Entry | null;
  outcome: Outcome | null;
  error: ErrorInfo | null;
  notice: string | null;
};

export type ShortcutStatus = {
  enabled: boolean;
  shortcut: string;
  registered: boolean;
  paused: boolean;
  error: string | null;
};

export type ModelStatus = { id: string; label: string; bytes: number; downloaded: boolean; default: boolean };

export type Status = {
  settings: Settings;
  state: StateEvent;
  platform: Platform;
  ffmpeg: boolean;
  paste_supported: boolean;
  runtime_installed: boolean;
  runtime_downloadable: boolean;
  model_ready: boolean;
  models: ModelStatus[];
  installing: boolean;
  shortcut: ShortcutStatus;
  default_shortcut: string;
};

export type Device = { id: string; name: string; index: number | null };

export type ShortcutCheck = { normalized: string | null; error: string | null };

export type LevelEvent = { session: number; level: number };

export type ModelProgress = { model: string; stage: "model" | "runtime"; fraction: number };
