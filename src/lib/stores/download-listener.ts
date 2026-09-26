import { listen } from "@tauri-apps/api/event";
import { get } from "svelte/store";
import { t } from "$lib/i18n";
import {
  syncQueueState,
  upsertGenericProgress,
} from "./download-store.svelte";
import { showToast } from "./toast-store.svelte";
import { setMediaPreview } from "./media-preview-store.svelte";
import { addLog } from "./debug-store.svelte";
import { recordDownloadComplete } from "./download-stats.svelte";
import { rpcSyncIdleStats } from "$lib/rpc";

// Best-effort OS notification so completions reach the user when the window is
// in the background (parity with the in-app toast and the channel feature).
async function notifyComplete(title: string) {
  try {
    const n = await import("@tauri-apps/plugin-notification");
    let granted = await n.isPermissionGranted();
    if (!granted) {
      granted = (await n.requestPermission()) === "granted";
    }
    if (granted) {
      const tr = get(t);
      n.sendNotification({
        title: tr("toast.download_complete", { name: title }) as string,
      });
    }
  } catch {
    // notifications are optional; never block completion handling
  }
}

type QueueItemInfo = {
  id: number;
  url: string;
  platform: string;
  title: string;
  status: { type: string; data?: unknown };
  percent: number;
  speed_bytes_per_sec: number;
  downloaded_bytes: number;
  total_bytes: number | null;
  file_path: string | null;
  file_size_bytes: number | null;
  file_count: number | null;
  thumbnail_url: string | null;
  eta_seconds?: number | null;
};

type QueueItemProgressPayload = {
  id: number;
  title: string;
  platform: string;
  /** `null` when the engine does not know the total. */
  percent: number | null;
  speed_bytes_per_sec: number;
  downloaded_bytes: number;
  total_bytes: number | null;
  phase: string;
  eta_seconds?: number | null;
  stream?: import("./download-store.svelte").StreamInfo | null;
  fragment_index?: number | null;
  fragment_count?: number | null;
  planned_formats?: string[] | null;
};

const loggedQueueTerminal = new Set<number>();
const queueToastEligibleIds = new Set<number>();
const seenGenericIds = new Set<number>();
let queueStateInitialized = false;

let throttleTimer: ReturnType<typeof setTimeout> | null = null;
let pendingPayload: QueueItemInfo[] | null = null;

function throttledSyncQueueState(payload: QueueItemInfo[]) {
  pendingPayload = payload;
  if (throttleTimer !== null) return;

  syncQueueState(payload);
  pendingPayload = null;

  throttleTimer = setTimeout(() => {
    throttleTimer = null;
    if (pendingPayload !== null) {
      syncQueueState(pendingPayload);
      pendingPayload = null;
    }
  }, 250);
}

export async function initDownloadListener(): Promise<() => void> {
  const unlistenQueueState = await listen<QueueItemInfo[]>(
    "queue-state-update",
    (event) => {
      const payload = event.payload;
      if (!queueStateInitialized) {
        for (const item of payload) {
          if (item.status.type === "Complete" || item.status.type === "Error") {
            loggedQueueTerminal.add(item.id);
          } else {
            queueToastEligibleIds.add(item.id);
          }
        }
        queueStateInitialized = true;
        throttledSyncQueueState(payload);
        return;
      }

      for (const item of payload) {
        if (item.status.type !== "Complete" && item.status.type !== "Error") {
          queueToastEligibleIds.add(item.id);
          continue;
        }
        if (loggedQueueTerminal.has(item.id)) continue;
        if (!queueToastEligibleIds.has(item.id)) {
          loggedQueueTerminal.add(item.id);
          continue;
        }
        if (item.status.type === "Error") {
          loggedQueueTerminal.add(item.id);
          queueToastEligibleIds.delete(item.id);
          const errMsg = typeof item.status.data === "string"
            ? item.status.data
            : (item.status.data as { message?: string } | undefined)?.message;
          addLog("error", "download", `Download error: ${item.title}`, errMsg ?? undefined);
        } else if (item.status.type === "Complete") {
          loggedQueueTerminal.add(item.id);
          queueToastEligibleIds.delete(item.id);
          addLog("info", "download", `Download complete: ${item.title}`, item.file_path ?? undefined);
          const tr = get(t);
          showToast("success", tr("toast.generic_download_complete", { name: item.title }));
          void notifyComplete(item.title);
          recordDownloadComplete(item.file_size_bytes ?? 0);
          void rpcSyncIdleStats();
        }
      }
      throttledSyncQueueState(payload);
    },
  );

  const unlistenQueueItemProgress = await listen<QueueItemProgressPayload>(
    "queue-item-progress",
    (event) => {
      const d = event.payload;
      if (!seenGenericIds.has(d.id)) {
        seenGenericIds.add(d.id);
        addLog("info", "download", `Download started: ${d.title}`, `Platform: ${d.platform}`);
      }
      queueToastEligibleIds.add(d.id);
      upsertGenericProgress(
        d.id,
        d.title,
        d.platform,
        d.percent,
        d.speed_bytes_per_sec,
        d.downloaded_bytes,
        d.total_bytes,
        d.phase,
        d.eta_seconds ?? null,
        {
          stream: d.stream ?? null,
          fragmentIndex: d.fragment_index ?? null,
          fragmentCount: d.fragment_count ?? null,
          plannedFormats: d.planned_formats ?? null,
        },
      );
    },
  );

  const unlistenFileCopied = await listen<{ path: string }>(
    "file-copied-to-clipboard",
    () => {
      const tr = get(t);
      showToast("success", tr("toast.file_copied_to_clipboard"));
    },
  );

  const unlistenMediaPreview = await listen<{
    url: string;
    title: string;
    author: string;
    thumbnail_url: string | null;
    duration_seconds: number | null;
  }>("media-info-preview", (event) => {
    setMediaPreview(event.payload);
  });

  let cookieErrorShown = false;
  const cookieCheckInterval = setInterval(async () => {
    if (cookieErrorShown) return;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const hasError = await invoke<boolean>("check_cookie_error");
      if (hasError && !cookieErrorShown) {
        cookieErrorShown = true;
        const tr = get(t);
        showToast("error", tr("common.cookie_error_message"), 15000);
        addLog("error", "system", "Cookie access failed - Chrome/Edge cookies are not accessible");
      }
    } catch {}
  }, 5000);

  return () => {
    unlistenQueueState();
    unlistenQueueItemProgress();
    unlistenFileCopied();
    unlistenMediaPreview();
    clearInterval(cookieCheckInterval);
    if (throttleTimer !== null) {
      clearTimeout(throttleTimer);
      throttleTimer = null;
    }
  };
}
