import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Progress } from "$lib/washmybelly/api";

export function onProgress(task: string, fn: (p: Progress) => void): Promise<UnlistenFn> {
  return listen<Progress>("washmybelly-progress", (e) => {
    if (e.payload.task === task) fn(e.payload);
  });
}
