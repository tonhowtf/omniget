import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Device, Entry, LevelEvent, ModelProgress, Settings, ShortcutCheck, ShortcutStatus, StateEvent, Status } from "./types";

export const STATE_EVENT = "dictation-state";
export const LEVEL_EVENT = "dictation-level";
export const MODEL_EVENT = "dictation-model-progress";

export const status = () => invoke<Status>("dictation_status");
export const saveSettings = (settings: Settings) => invoke<Status>("dictation_save_settings", { settings });
export const checkShortcut = (shortcut: string) => invoke<ShortcutCheck>("dictation_check_shortcut", { shortcut });
export const pauseShortcut = (paused: boolean) => invoke<ShortcutStatus>("dictation_pause_shortcut", { paused });
export const devices = () => invoke<Device[]>("dictation_devices");
export const start = () => invoke<StateEvent>("dictation_start");
export const stop = () => invoke<void>("dictation_stop");
export const cancel = () => invoke<void>("dictation_cancel");
export const history = () => invoke<Entry[]>("dictation_history");
export const deleteEntry = (id: string) => invoke<Entry[]>("dictation_history_delete", { id });
export const clearHistory = () => invoke<void>("dictation_history_clear");
export const copy = (text: string) => invoke<void>("dictation_copy", { text });
export const preview = (text: string, settings: Settings, fallbackLanguage: string | null) =>
  invoke<string>("dictation_preview", { text, settings, fallbackLanguage });
export const openPrivacy = (pane: string) => invoke<void>("dictation_open_privacy", { pane });
export const installModel = (model: string | null) => invoke<Status>("dictation_install_model", { model });

export const onState = (fn: (e: StateEvent) => void): Promise<UnlistenFn> => listen<StateEvent>(STATE_EVENT, (e) => fn(e.payload));
export const onLevel = (fn: (e: LevelEvent) => void): Promise<UnlistenFn> => listen<LevelEvent>(LEVEL_EVENT, (e) => fn(e.payload));
export const onModel = (fn: (e: ModelProgress) => void): Promise<UnlistenFn> => listen<ModelProgress>(MODEL_EVENT, (e) => fn(e.payload));

export function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object" && "message" in error) return String((error as { message: unknown }).message);
  return String(error);
}
