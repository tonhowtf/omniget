export type DownloadStatus = "queued" | "downloading" | "paused" | "complete" | "error";

type BaseItem = {
  id: number;
  name: string;
  percent: number;
  status: DownloadStatus;
  error?: string;
  startedAt: number;
  lastUpdateAt: number;
};

export type CourseDownloadItem = BaseItem & {
  kind: "course";
  currentModule: string;
  currentPage: string;
  bytesDownloaded: number;
  speed: number;
  totalPages: number;
  completedPages: number;
  totalModules: number;
  currentModuleIndex: number;
};

export type GenericDownloadItem = BaseItem & {
  kind: "generic";
  platform: string;
  speed: number;
  downloadedBytes: number;
  totalBytes: number | null;
  phase: string;
  filePath?: string;
  fileCount?: number;
  thumbnail_url?: string | null;
};

export type DownloadItem = CourseDownloadItem | GenericDownloadItem;

const SPEED_SMOOTHING = 0.3;

let downloads = $state(new Map<number, DownloadItem>());
let flushScheduled = false;

function scheduleFlush() {
  if (flushScheduled) return;
  flushScheduled = true;
  requestAnimationFrame(() => {
    flushScheduled = false;
    downloads = new Map(downloads);
  });
}

function flushNow() {
  flushScheduled = false;
  downloads = new Map(downloads);
}

export function getDownloads(): Map<number, DownloadItem> {
  return downloads;
}

export function getActiveCount(): number {
  let count = 0;
  for (const item of downloads.values()) {
    if (item.status === "downloading") count++;
  }
  return count;
}

export function getQueuedCount(): number {
  let count = 0;
  for (const item of downloads.values()) {
    if (item.status === "queued") count++;
  }
  return count;
}

export function getBadgeCount(): number {
  let count = 0;
  for (const item of downloads.values()) {
    if (item.status === "downloading" || item.status === "queued") count++;
  }
  return count;
}

export function getPausedCount(): number {
  let count = 0;
  for (const item of downloads.values()) {
    if (item.status === "paused") count++;
  }
  return count;
}

export function upsertProgress(
  courseId: number,
  courseName: string,
  percent: number,
  currentModule: string,
  currentPage: string,
  downloadedBytes: number,
  totalPages: number,
  completedPages: number,
  totalModules: number,
  currentModuleIndex: number,
) {
  const now = Date.now();
  const existing = downloads.get(courseId);

  let speed = 0;
  if (existing && existing.kind === "course" && existing.bytesDownloaded > 0 && downloadedBytes > existing.bytesDownloaded) {
    const dt = (now - existing.lastUpdateAt) / 1000;
    if (dt > 0.1) {
      const instantSpeed = (downloadedBytes - existing.bytesDownloaded) / dt;
      speed = existing.speed > 0
        ? existing.speed * (1 - SPEED_SMOOTHING) + instantSpeed * SPEED_SMOOTHING
        : instantSpeed;
    } else {
      speed = existing.speed;
    }
  }

  downloads.set(courseId, {
    kind: "course",
    id: courseId,
    name: courseName,
    percent: Math.max(0, percent),
    currentModule,
    currentPage,
    status: "downloading",
    startedAt: existing?.startedAt ?? now,
    bytesDownloaded: downloadedBytes,
    lastUpdateAt: now,
    speed,
    totalPages,
    completedPages,
    totalModules,
    currentModuleIndex,
  });
  scheduleFlush();
}

export function markComplete(courseName: string, success: boolean, error?: string) {
  for (const [id, item] of downloads) {
    if (item.name === courseName) {
      const base = {
        ...item,
        percent: success ? 100 : item.percent,
        status: (success ? "complete" : "error") as DownloadStatus,
        error,
        lastUpdateAt: Date.now(),
      };
      if (item.kind === "course") {
        downloads.set(id, { ...base, kind: "course", speed: 0 } as CourseDownloadItem);
      } else {
        downloads.set(id, base as GenericDownloadItem);
      }
      flushNow();
      break;
    }
  }
}

export function clearFinished() {
  let changed = false;
  for (const [id, item] of downloads) {
    if (item.status === "complete" || item.status === "error") {
      downloads.delete(id);
      changed = true;
    }
  }
  if (changed) {
    flushNow();
  }
}

export function getFinishedCount(): number {
  let count = 0;
  for (const item of downloads.values()) {
    if (item.status === "complete" || item.status === "error") count++;
  }
  return count;
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
};

function queueStatusToDownloadStatus(status: { type: string; data?: unknown }): DownloadStatus {
  switch (status.type) {
    case "Queued": return "queued";
    case "Active": return "downloading";
    case "Paused": return "paused";
    case "Complete": return "complete";
    case "Error": return "error";
    default: return "queued";
  }
}

function extractError(status: { type: string; data?: unknown }): string | undefined {
  if (status.type === "Error" && status.data && typeof status.data === "object" && "message" in (status.data as Record<string, unknown>)) {
    return (status.data as { message: string }).message;
  }
  if (status.type === "Error" && typeof status.data === "string") {
    return status.data;
  }
  return undefined;
}

export function syncQueueState(items: QueueItemInfo[]) {
  const now = Date.now();
  const queueIds = new Set(items.map(i => i.id));

  for (const [id, item] of downloads) {
    if (item.kind === "generic" && !queueIds.has(id)) {
      downloads.delete(id);
    }
  }

  for (const qi of items) {
    const existing = downloads.get(qi.id);
    const dlStatus = queueStatusToDownloadStatus(qi.status);

    let speed = qi.speed_bytes_per_sec;
    if (existing && existing.kind === "generic" && existing.speed > 0 && speed > 0) {
      speed = existing.speed * (1 - SPEED_SMOOTHING) + qi.speed_bytes_per_sec * SPEED_SMOOTHING;
    }

    downloads.set(qi.id, {
      kind: "generic",
      id: qi.id,
      name: qi.title,
      platform: qi.platform,
      percent: Math.max(0, qi.percent),
      speed: dlStatus === "downloading" ? speed : 0,
      downloadedBytes: qi.downloaded_bytes,
      totalBytes: qi.total_bytes,
      phase: (existing?.kind === "generic" ? existing.phase : undefined) ?? "queued",
      status: dlStatus,
      error: extractError(qi.status),
      startedAt: existing?.startedAt ?? now,
      lastUpdateAt: now,
      filePath: qi.file_path ?? undefined,
      fileCount: qi.file_count ?? undefined,
      thumbnail_url: qi.thumbnail_url,
    });
  }

  flushNow();
}

export function removeDownload(id: number) {
  if (downloads.has(id)) {
    downloads.delete(id);
    flushNow();
  }
}

export function markGenericComplete(id: number, success: boolean, error?: string, filePath?: string, fileCount?: number, totalBytes?: number | null) {
  const item = downloads.get(id);
  if (!item || item.kind !== "generic") return;

  downloads.set(id, {
    ...item,
    percent: success ? 100 : item.percent,
    status: (success ? "complete" : "error") as DownloadStatus,
    error,
    filePath,
    fileCount,
    totalBytes: totalBytes ?? item.totalBytes,
    speed: 0,
    lastUpdateAt: Date.now(),
  });
  flushNow();
}

export function upsertGenericProgress(
  id: number,
  title: string,
  platform: string,
  percent: number,
  speedBytesPerSec: number,
  downloadedBytes: number,
  totalBytes: number | null,
  phase: string,
) {
  const now = Date.now();
  const existing = downloads.get(id);

  let speed = speedBytesPerSec;
  if (existing && existing.kind === "generic" && existing.speed > 0 && speedBytesPerSec > 0) {
    speed = existing.speed * (1 - SPEED_SMOOTHING) + speedBytesPerSec * SPEED_SMOOTHING;
  }

  downloads.set(id, {
    kind: "generic",
    id,
    name: title,
    platform,
    percent: Math.max(0, percent),
    speed,
    downloadedBytes,
    totalBytes,
    phase,
    status: "downloading",
    startedAt: existing?.startedAt ?? now,
    lastUpdateAt: now,
  });
  scheduleFlush();
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

export function formatSpeed(bytesPerSec: number): string {
  if (bytesPerSec <= 0) return "0 KB/s";
  if (bytesPerSec < 1024 * 1024) return `${(bytesPerSec / 1024).toFixed(0)} KB/s`;
  return `${(bytesPerSec / (1024 * 1024)).toFixed(1)} MB/s`;
}

