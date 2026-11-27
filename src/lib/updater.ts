import { check, type DownloadEvent } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { storeChangelogForUpdate } from "$lib/stores/changelog-store.svelte";
import { invoke } from "@tauri-apps/api/core";

export interface InstallInfo {
  kind: string;
  self_update: boolean;
  download_url: string;
}

export interface UpdateInfo {
  available: boolean;
  version?: string;
  body?: string;
  selfUpdate?: boolean;
  installKind?: string;
  downloadUrl?: string;
}

const CHECK_TIMEOUT_MS = 30_000;

async function proxyUrl(): Promise<string | undefined> {
  const url = await invoke<string | null>("get_updater_proxy_url");
  return url ?? undefined;
}

export async function getInstallInfo(): Promise<InstallInfo> {
  try {
    return await invoke<InstallInfo>("get_install_info");
  } catch {
    return {
      kind: "unknown",
      self_update: true,
      download_url: "https://getomniget.com/download",
    };
  }
}

function withTimeout<T>(promise: Promise<T>, ms: number, label: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(label)), ms);
    promise.then(
      (value) => { clearTimeout(timer); resolve(value); },
      (err) => { clearTimeout(timer); reject(err); },
    );
  });
}

export async function checkForUpdate(): Promise<UpdateInfo> {
  try {
    const [info, update] = await Promise.all([
      getInstallInfo(),
      check({ proxy: await proxyUrl() }),
    ]);
    if (update) {
      return {
        available: true,
        version: update.version,
        body: update.body ?? undefined,
        selfUpdate: info.self_update,
        installKind: info.kind,
        downloadUrl: info.download_url,
      };
    }
    return {
      available: false,
      selfUpdate: info.self_update,
      installKind: info.kind,
      downloadUrl: info.download_url,
    };
  } catch {
    return { available: false };
  }
}

export async function installUpdate(onProgress?: (percent: number | null) => void): Promise<void> {
  const info = await getInstallInfo();
  if (!info.self_update) {
    throw new Error("This installation must be updated from the download page.");
  }
  const update = await withTimeout(
    check({ proxy: await proxyUrl() }),
    CHECK_TIMEOUT_MS,
    "Timed out checking for the update.",
  );
  if (!update) {
    throw new Error("The update is no longer available from the update server.");
  }
  if (update.body && update.version) {
    storeChangelogForUpdate(update.body, update.version);
  }
  let downloaded = 0;
  let total: number | null = null;
  await update.downloadAndInstall((event: DownloadEvent) => {
    if (event.event === "Started") {
      total = event.data.contentLength ?? null;
      downloaded = 0;
      onProgress?.(total && total > 0 ? 0 : null);
    } else if (event.event === "Progress") {
      downloaded += event.data.chunkLength;
      if (total && total > 0) {
        onProgress?.(Math.min(100, Math.round((downloaded / total) * 100)));
      }
    } else if (event.event === "Finished") {
      onProgress?.(100);
    }
  });
  await relaunch();
}
