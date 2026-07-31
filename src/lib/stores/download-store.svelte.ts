export type DownloadStatus = "queued" | "downloading" | "paused" | "complete" | "error" | "seeding";

export type QueueKind =
  | "video"
  | "audio"
  | "image"
  | "pdf"
  | "book"
  | "webpage"
  | "telegram_media"
  | "course_lesson"
  | "generic";

type BaseItem = {
  id: number;
  name: string;
  percent: number;
  status: DownloadStatus;
  error?: string;
  startedAt: number;
  lastUpdateAt: number;
  queueKind?: QueueKind;
  external?: boolean;
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
  etaSeconds?: number | null;
  filePath?: string;
  fileCount?: number;
  thumbnail_url?: string | null;
  quality?: string | null;
  downloadMode?: string | null;
};

export type DownloadItem = CourseDownloadItem | GenericDownloadItem;

export type SpeedPoint = { t: number; bps: number };

const SPEED_SMOOTHING = 0.3;
const SPEED_HISTORY_MAX = 60;

let downloads = $state(new Map<number, DownloadItem>());
const speedHistory = new Map<number, SpeedPoint[]>();
let flushScheduled = false;

function pushSpeedPoint(id: number, bps: number) {
  let arr = speedHistory.get(id);
  if (!arr) {
    arr = [];
    speedHistory.set(id, arr);
  }
  arr.push({ t: Date.now(), bps });
  if (arr.length > SPEED_HISTORY_MAX) {
    arr.splice(0, arr.length - SPEED_HISTORY_MAX);
  }
}

export function getSpeedHistory(id: number): SpeedPoint[] {
  return speedHistory.get(id) ?? [];
}

function clearSpeedHistory(id: number) {
  speedHistory.delete(id);
}

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

export type DownloadCounts = {
  active: number;
  queued: number;
  badge: number;
  paused: number;
  finished: number;
};

export function getCounts(): DownloadCounts {
  let active = 0, queued = 0, paused = 0, finished = 0;
  for (const item of downloads.values()) {
    switch (item.status) {
      case "downloading":
      case "seeding": active++; break;
      case "queued": queued++; break;
      case "paused": paused++; break;
      case "complete":
      case "error": finished++; break;
    }
  }
  return { active, queued, badge: active + queued, paused, finished };
}

export function getActiveCount(): number {
  return getCounts().active;
}

export function getQueuedCount(): number {
  return getCounts().queued;
}

export function getBadgeCount(): number {
  return getCounts().badge;
}

export function getPausedCount(): number {
  return getCounts().paused;
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
  pushSpeedPoint(courseId, speed);
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
      clearSpeedHistory(id);
      flushNow();
      break;
    }
  }
}

export function clearFinished() {
  let changed = false;
  for (const [id, item] of downloads) {
    if (item.status === "complete") {
      downloads.delete(id);
      clearSpeedHistory(id);
      changed = true;
    }
  }
  if (changed) {
    flushNow();
  }
}

export function getFinishedCount(): number {
  let n = 0;
  for (const item of downloads.values()) {
    if (item.status === "complete") n++;
  }
  return n;
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
  kind?: QueueKind;
  external?: boolean;
  eta_seconds?: number | null;
  quality?: string | null;
  download_mode?: string | null;
};

function queueStatusToDownloadStatus(status: { type: string; data?: unknown }): DownloadStatus {
  switch (status.type) {
    case "Queued": return "queued";
    case "Active": return "downloading";
    case "Paused": return "paused";
    case "Seeding": return "seeding";
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
      clearSpeedHistory(id);
    }
  }

  for (const qi of items) {
    const existing = downloads.get(qi.id);
    const dlStatus = queueStatusToDownloadStatus(qi.status);

    let speed = qi.speed_bytes_per_sec;
    if (existing && existing.kind === "generic" && existing.speed > 0 && speed > 0) {
      speed = existing.speed * (1 - SPEED_SMOOTHING) + qi.speed_bytes_per_sec * SPEED_SMOOTHING;
    }

    const effectiveSpeed = (dlStatus === "downloading" || dlStatus === "seeding") ? speed : 0;

    downloads.set(qi.id, {
      kind: "generic",
      id: qi.id,
      name: qi.title,
      platform: qi.platform,
      percent: Math.max(0, qi.percent),
      speed: effectiveSpeed,
      downloadedBytes: qi.downloaded_bytes,
      totalBytes: qi.total_bytes,
      phase: (existing?.kind === "generic" ? existing.phase : undefined) ?? "queued",
      etaSeconds: qi.eta_seconds ?? null,
      status: dlStatus,
      error: extractError(qi.status),
      startedAt: existing?.startedAt ?? now,
      lastUpdateAt: now,
      filePath: qi.file_path ?? undefined,
      fileCount: qi.file_count ?? undefined,
      thumbnail_url: qi.thumbnail_url,
      queueKind: qi.kind,
      external: qi.external,
      quality: qi.quality ?? null,
      downloadMode: qi.download_mode ?? null,
    });

    if (dlStatus === "downloading" || dlStatus === "seeding") {
      pushSpeedPoint(qi.id, effectiveSpeed);
    } else if (dlStatus === "complete" || dlStatus === "error") {
      clearSpeedHistory(qi.id);
    }
  }

  flushNow();
}

export function removeDownload(id: number) {
  if (downloads.has(id)) {
    downloads.delete(id);
    clearSpeedHistory(id);
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
  clearSpeedHistory(id);
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
  etaSeconds?: number | null,
) {
  const now = Date.now();
  const existing = downloads.get(id);

  let speed = speedBytesPerSec;
  if (existing && existing.kind === "generic" && existing.speed > 0 && speedBytesPerSec > 0) {
    speed = existing.speed * (1 - SPEED_SMOOTHING) + speedBytesPerSec * SPEED_SMOOTHING;
  }

  // Preserve non-downloading statuses (paused, seeding, complete, error)
  // to avoid race conditions with queue-state-update events
  const keepStatus = existing?.kind === "generic"
    && (existing.status === "paused" || existing.status === "seeding" || existing.status === "complete" || existing.status === "error");
  const resolvedStatus: DownloadStatus = keepStatus ? existing!.status : "downloading";

  const effectiveSpeed = resolvedStatus === "downloading" ? speed : 0;

  downloads.set(id, {
    kind: "generic",
    id,
    name: title,
    platform,
    percent: Math.max(0, percent),
    speed: effectiveSpeed,
    downloadedBytes,
    totalBytes,
    phase,
    etaSeconds: etaSeconds ?? null,
    status: resolvedStatus,
    startedAt: existing?.startedAt ?? now,
    lastUpdateAt: now,
    quality: existing?.kind === "generic" ? existing.quality : undefined,
    downloadMode: existing?.kind === "generic" ? existing.downloadMode : undefined,
  });

  if (resolvedStatus === "downloading") {
    pushSpeedPoint(id, effectiveSpeed);
  }

  scheduleFlush();
}

export { formatBytes, formatSpeed, formatEta } from "../download-format";

