import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { storeChangelogForUpdate } from "$lib/stores/changelog-store.svelte";

export interface UpdateInfo {
  available: boolean;
  version?: string;
  body?: string;
}

export async function checkForUpdate(): Promise<UpdateInfo> {
  try {
    const update = await check();
    if (update) {
      return {
        available: true,
        version: update.version,
        body: update.body ?? undefined,
      };
    }
    return { available: false };
  } catch {
    return { available: false };
  }
}

export async function installUpdate(): Promise<void> {
  const update = await check();
  if (update) {
    if (update.body && update.version) {
      storeChangelogForUpdate(update.body, update.version);
    }
    await update.downloadAndInstall();
    await relaunch();
  }
}
