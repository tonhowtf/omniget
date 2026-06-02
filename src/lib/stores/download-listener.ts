import { listen } from "@tauri-apps/api/event";
import { get } from "svelte/store";
import { t } from "$lib/i18n";
import {
  upsertProgress,
  markComplete,
  syncQueueState,
  upsertGenericProgress,
  getDownloads,
} from "./download-store.svelte";
import { showToast } from "./toast-store.svelte";
import {
  updateFileProgress,
  markFileComplete,
  markFileError,
} from "./convert-store.svelte";
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

type ProgressPayload = {
  course_id: number;
  course_name: string;
  percent: number;
  current_module: string;
  current_page: string;
  downloaded_bytes: number;
  total_pages: number;
  completed_pages: number;
  total_modules: number;
  current_module_index: number;
};

type CompletePayload = {
  course_name: string;
  success: boolean;
  error: string | null;
};

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

export type BatchFileStatusPayload = {
  batch_id: number;
  message_id: number;
  status: "waiting" | "downloading" | "done" | "error" | "skipped";
  percent: number;
  error: string | null;
};

type BatchFileStatusCallback = (payload: BatchFileStatusPayload) => void;
let batchFileStatusCallback: BatchFileStatusCallback | null = null;

export function onBatchFileStatus(cb: BatchFileStatusCallback | null) {
  batchFileStatusCallback = cb;
}

type QueueItemProgressPayload = {
  id: number;
  title: string;
  platform: string;
  percent: number;
  speed_bytes_per_sec: number;
  downloaded_bytes: number;
  total_bytes: number | null;
  phase: string;
  eta_seconds?: number | null;
};

type ConvertProgressPayload = {
  id: number;
  percent: number;
};

type ConvertCompletePayload = {
  id: number;
  success: boolean;
  result: {
    output_path: string;
    file_size_bytes: number;
    duration_seconds: number;
    error: string | null;
  } | null;
  error: string | null;
};

type UdemyProgressPayload = {
  course_id: number;
  course_name: string;
  percent: number;
  current_chapter: string;
  current_lecture: string;
  downloaded_bytes: number;
  total_lectures: number;
  completed_lectures: number;
};

type UdemyCompletePayload = {
  course_name: string;
  success: boolean;
  error: string | null;
  drm_skipped: number;
};

const seenCourseIds = new Set<number>();
const seenUdemyCourseIds = new Set<number>();
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
  const unlistenProgress = await listen<ProgressPayload>("download-progress", (event) => {
    const d = event.payload;

    if (!seenCourseIds.has(d.course_id)) {
      seenCourseIds.add(d.course_id);
      const tr = get(t);
      showToast("info", tr("toast.download_started", { name: d.course_name }));
      addLog("info", "download", `Course download started: ${d.course_name}`);
    }

    upsertProgress(
      d.course_id,
      d.course_name,
      d.percent,
      d.current_module,
      d.current_page,
      d.downloaded_bytes,
      d.total_pages,
      d.completed_pages,
      d.total_modules,
      d.current_module_index,
    );
  });

  const unlistenComplete = await listen<CompletePayload>("download-complete", (event) => {
    const d = event.payload;
    markComplete(d.course_name, d.success, d.error ?? undefined);

    const tr = get(t);
    if (d.success) {
      showToast("success", tr("toast.download_complete", { name: d.course_name }));
      void notifyComplete(d.course_name);
      addLog("info", "download", `Course download complete: ${d.course_name}`);
      recordDownloadComplete(0);
      void rpcSyncIdleStats();
    } else {
      let msg = tr("toast.download_error", { name: d.course_name });
      if (d.error) msg += ` — ${d.error}`;
      showToast("error", msg);
      addLog("error", "download", `Course download failed: ${d.course_name}`, d.error ?? undefined);
    }
  });

  const unlistenUdemyProgress = await listen<UdemyProgressPayload>("udemy-download-progress", (event) => {
    const d = event.payload;

    if (!seenUdemyCourseIds.has(d.course_id)) {
      seenUdemyCourseIds.add(d.course_id);
      const tr = get(t);
      showToast("info", tr("toast.download_started", { name: d.course_name }));
      addLog("info", "download", `Udemy download started: ${d.course_name}`);
    }

    upsertProgress(
      d.course_id,
      d.course_name,
      d.percent,
      d.current_chapter,
      d.current_lecture,
      d.downloaded_bytes,
      d.total_lectures,
      d.completed_lectures,
      0,
      0,
    );
  });

  const unlistenUdemyComplete = await listen<UdemyCompletePayload>("udemy-download-complete", (event) => {
    const d = event.payload;
    markComplete(d.course_name, d.success, d.error ?? undefined);
    seenUdemyCourseIds.delete([...seenUdemyCourseIds].find(id => {
      const item = getDownloads().get(id);
      return item?.name === d.course_name;
    }) ?? -1);

    const tr = get(t);
    if (d.success) {
      showToast("success", tr("toast.download_complete", { name: d.course_name }));
      void notifyComplete(d.course_name);
      addLog("info", "download", `Udemy download complete: ${d.course_name}`);
      recordDownloadComplete(0);
      void rpcSyncIdleStats();
      if (d.drm_skipped > 0) {
        showToast("info", tr("toast.drm_skipped", { count: String(d.drm_skipped) }));
        addLog("warn", "download", `${d.drm_skipped} DRM-protected video(s) skipped`, d.course_name);
      }
    } else {
      let msg = tr("toast.download_error", { name: d.course_name });
      if (d.error) msg += ` — ${d.error}`;
      showToast("error", msg);
      addLog("error", "download", `Udemy download failed: ${d.course_name}`, d.error ?? undefined);
    }
  });

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
      );
    },
  );

  const unlistenBatchFileStatus = await listen<BatchFileStatusPayload>(
    "telegram-batch-file-status",
    (event) => {
      if (batchFileStatusCallback) {
        batchFileStatusCallback(event.payload);
      }
    },
  );

  const unlistenConvertProgress = await listen<ConvertProgressPayload>(
    "convert-progress",
    (event) => {
      updateFileProgress(event.payload.id, event.payload.percent);
    },
  );

  const unlistenConvertComplete = await listen<ConvertCompletePayload>(
    "convert-complete",
    (event) => {
      const d = event.payload;
      const tr = get(t);
      if (d.success && d.result) {
        markFileComplete(d.id, d.result.output_path, d.result.file_size_bytes);
        showToast("success", tr("convert.toast_complete"));
        addLog("info", "convert", `Conversion complete: ${d.result.output_path}`);
      } else {
        const errorMsg = d.error ?? d.result?.error ?? tr("common.unknown_error");
        markFileError(d.id, errorMsg);
        showToast("error", `${tr("convert.toast_error")} — ${errorMsg}`);
        addLog("error", "convert", `Conversion failed`, errorMsg);
      }
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
    unlistenProgress();
    unlistenComplete();
    unlistenUdemyProgress();
    unlistenUdemyComplete();
    unlistenQueueState();
    unlistenQueueItemProgress();
    unlistenBatchFileStatus();
    unlistenConvertProgress();
    unlistenConvertComplete();
    unlistenFileCopied();
    unlistenMediaPreview();
    clearInterval(cookieCheckInterval);
    if (throttleTimer !== null) {
      clearTimeout(throttleTimer);
      throttleTimer = null;
    }
  };
}
