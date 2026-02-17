import { listen } from "@tauri-apps/api/event";
import { get } from "svelte/store";
import { t } from "$lib/i18n";
import {
  upsertProgress,
  markComplete,
  upsertGenericProgress,
  markGenericComplete,
} from "./download-store.svelte";
import { showToast } from "./toast-store.svelte";

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

type GenericProgressPayload = {
  id: number;
  title: string;
  platform: string;
  percent: number;
  speed_bytes_per_sec: number;
  downloaded_bytes: number;
  total_bytes: number | null;
  eta_seconds: number | null;
};

type GenericCompletePayload = {
  id: number;
  title: string;
  platform: string;
  success: boolean;
  error: string | null;
  file_path: string | null;
  file_size_bytes: number | null;
  file_count: number | null;
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

const seenCourseIds = new Set<number>();
const seenGenericIds = new Set<number>();

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

  const unlistenGenericProgress = await listen<GenericProgressPayload>(
    "generic-download-progress",
    (event) => {
      const d = event.payload;

      if (!seenGenericIds.has(d.id)) {
        seenGenericIds.add(d.id);
        const tr = get(t);
        showToast("info", tr("toast.download_started", { name: d.title }));
      }

      upsertGenericProgress(d.id, d.title, d.platform, d.percent, d.speed_bytes_per_sec, d.downloaded_bytes, d.total_bytes, d.eta_seconds);
    },
  );

  const unlistenGenericComplete = await listen<GenericCompletePayload>(
    "generic-download-complete",
    (event) => {
      const d = event.payload;
      markGenericComplete(d.id, d.success, d.error ?? undefined, d.file_path ?? undefined, d.file_count ?? undefined, d.file_size_bytes);

      const tr = get(t);
      if (d.success) {
        showToast("success", tr("toast.download_complete", { name: d.title }));
      } else {
        let msg = tr("toast.download_error", { name: d.title });
        if (d.error) msg += ` — ${d.error}`;
        showToast("error", msg);
      }
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

  return () => {
    unlistenProgress();
    unlistenComplete();
    unlistenGenericProgress();
    unlistenGenericComplete();
    unlistenBatchFileStatus();
  };
}
