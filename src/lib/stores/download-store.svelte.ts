export type DownloadStatus = "downloading" | "complete" | "error";

export type DownloadItem = {
  courseId: number;
  courseName: string;
  percent: number;
  currentModule: string;
  currentPage: string;
  status: DownloadStatus;
  error?: string;
  startedAt: number;
  bytesDownloaded: number;
  lastUpdateAt: number;
  speed: number;
  totalPages: number;
  completedPages: number;
  totalModules: number;
  currentModuleIndex: number;
};

const SPEED_SMOOTHING = 0.3;

let downloads = $state(new Map<number, DownloadItem>());

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
  if (existing && existing.bytesDownloaded > 0 && downloadedBytes > existing.bytesDownloaded) {
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
    courseId,
    courseName,
    percent,
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
  downloads = new Map(downloads);
}

export function markComplete(courseName: string, success: boolean, error?: string) {
  for (const [id, item] of downloads) {
    if (item.courseName === courseName) {
      downloads.set(id, {
        ...item,
        percent: success ? 100 : item.percent,
        status: success ? "complete" : "error",
        error,
        lastUpdateAt: Date.now(),
        speed: 0,
      });
      downloads = new Map(downloads);
      break;
    }
  }
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

export type I18nValue = { key: string; params?: Record<string, number> };

export function getEtaI18n(item: DownloadItem): I18nValue {
  if (item.percent <= 0 || item.speed <= 0) return { key: "downloads.eta_calculating" };
  const elapsed = (item.lastUpdateAt - item.startedAt) / 1000;
  if (elapsed < 2) return { key: "downloads.eta_calculating" };
  const remaining = elapsed * (100 - item.percent) / item.percent;
  if (!isFinite(remaining) || remaining < 0) return { key: "downloads.eta_calculating" };
  if (remaining < 60) return { key: "downloads.eta_seconds", params: { n: Math.ceil(remaining) } };
  if (remaining < 3600) return { key: "downloads.eta_minutes", params: { n: Math.ceil(remaining / 60) } };
  const hours = Math.floor(remaining / 3600);
  const mins = Math.ceil((remaining % 3600) / 60);
  return { key: "downloads.eta_hours", params: { h: hours, m: mins } };
}
