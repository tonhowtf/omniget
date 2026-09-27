import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Progress } from "$lib/washmybelly/api";

/// Listen to `washmybelly-progress` for one task (`junk_scan`, `execute`...).
export function onProgress(task: string, fn: (p: Progress) => void): Promise<UnlistenFn> {
  return listen<Progress>("washmybelly-progress", (e) => {
    if (e.payload.task === task) fn(e.payload);
  });
}
