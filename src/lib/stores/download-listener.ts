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
};

const seenCourseIds = new Set<number>();
const seenUdemyCourseIds = new Set<number>();

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
    } else {
      let msg = tr("toast.download_error", { name: d.course_name });
      if (d.error) msg += ` — ${d.error}`;
      showToast("error", msg);
    }
  });

  const unlistenUdemyProgress = await listen<UdemyProgressPayload>("udemy-download-progress", (event) => {
    const d = event.payload;

    if (!seenUdemyCourseIds.has(d.course_id)) {
      seenUdemyCourseIds.add(d.course_id);
      const tr = get(t);
      showToast("info", tr("toast.download_started", { name: d.course_name }));
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
    } else {
      let msg = tr("toast.download_error", { name: d.course_name });
      if (d.error) msg += ` — ${d.error}`;
      showToast("error", msg);
    }
  });

  const unlistenQueueState = await listen<QueueItemInfo[]>(
    "queue-state-update",
    (event) => {
      throttledSyncQueueState(event.payload);
    },
  );

  const unlistenQueueItemProgress = await listen<QueueItemProgressPayload>(
    "queue-item-progress",
    (event) => {
      const d = event.payload;
      upsertGenericProgress(
        d.id,
        d.title,
        d.platform,
        d.percent,
        d.speed_bytes_per_sec,
        d.downloaded_bytes,
        d.total_bytes,
        d.phase,
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
      } else {
        const errorMsg = d.error ?? d.result?.error ?? tr("common.unknown_error");
        markFileError(d.id, errorMsg);
        showToast("error", `${tr("convert.toast_error")} — ${errorMsg}`);
      }
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
    unlistenMediaPreview();
    if (throttleTimer !== null) {
      clearTimeout(throttleTimer);
      throttleTimer = null;
    }
  };
}
