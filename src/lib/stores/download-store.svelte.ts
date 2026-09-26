export type DownloadStatus = "queued" | "downloading" | "paused" | "complete" | "error" | "seeding";

export type QueueKind =
  | "video"
  | "audio"
  | "image"
  | "pdf"
  | "webpage"
  | "generic";

type BaseItem = {
  id: number;
  name: string;
  /** `null` = total unknown: show bytes and an indeterminate bar, never a number. */
  percent: number | null;
  status: DownloadStatus;
  error?: string;
  startedAt: number;
  lastUpdateAt: number;
  queueKind?: QueueKind;
};

/** Stream (formato) que o yt-dlp está baixando; vem de `%(info.*)s` no template de progresso. */
export type StreamInfo = {
  format_id: string;
  height?: number;
  width?: number;
  fps?: number;
  vcodec?: string;
  acodec?: string;
  ext?: string;
  filesize?: number;
  format_note?: string;
};

/** Último comando yt-dlp rodado para o item, já redigido no backend. */
export type CommandRecord = {
  program: string;
  args: string[];
  display: string;
  attempt: number;
  max_attempts: number;
  player_client?: string;
  connections: number;
  engine: string;
  overridden: boolean;
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
  author?: string | null;
  durationSeconds?: number | null;
  stream?: StreamInfo | null;
  streamsDone?: StreamInfo[];
  plannedFormats?: string[] | null;
  fragmentIndex?: number | null;
  fragmentCount?: number | null;
  startedAtMs?: number | null;
  command?: CommandRecord | null;
};

export type GenericProgressExtra = {
  stream?: StreamInfo | null;
  fragmentIndex?: number | null;
  fragmentCount?: number | null;
  plannedFormats?: string[] | null;
};

export type DownloadItem = GenericDownloadItem;

export type SpeedPoint = { t: number; bps: number };

const SPEED_SMOOTHING = 0.3;
const SPEED_HISTORY_MAX = 60;
const AGGREGATE_SAMPLE_MS = 900;

let downloads = $state(new Map<number, DownloadItem>());
const speedHistory = new Map<number, SpeedPoint[]>();
const suppressedGenericIds = new Set<number>();
let flushScheduled = false;

let aggregateSpeedHistory = $state<SpeedPoint[]>([]);
let lastAggregateSampleAt = 0;
let aggregateBatch = $state(new Map<number, DownloadItem>());
let aggregateBatchId = 0;
let batchWasPending = false;
let batchCancelled = false;
let acknowledgedFailures = $state(new Set<number>());

function isPending(item: DownloadItem): boolean {
  return item.status === "queued" || item.status === "downloading" || item.status === "paused";
}

function updateAggregateBatch() {
  const pending = [...downloads.values()].some(isPending);
  let next = new Map(aggregateBatch);
  if (pending && !batchWasPending) {
    next = new Map();
    aggregateBatchId++;
    batchCancelled = false;
  }
  for (const [id, previous] of next) {
    if (!downloads.has(id) && previous.status !== "complete" && previous.status !== "seeding") {
      next.delete(id);
      batchCancelled = true;
    }
  }
  for (const [id, item] of downloads) {
    if (isPending(item) || next.has(id)) next.set(id, { ...item });
    if (item.status !== "error") acknowledgedFailures.delete(id);
  }
  for (const id of acknowledgedFailures) {
    if (!downloads.has(id)) acknowledgedFailures.delete(id);
  }
  batchWasPending = pending;
  aggregateBatch = next;
}

export function dismissAggregateFailures() {
  acknowledgedFailures = new Set(
    [...downloads.values()].filter(item => item.status === "error").map(item => item.id),
  );
}

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

function sampleAggregateSpeed(now: number) {
  let bps = 0;
  let anyActive = false;
  let anyPending = false;
  for (const item of downloads.values()) {
    if (item.status === "downloading") {
      bps += finiteBytes(item.speed);
      anyActive = true;
    } else if (item.status === "queued" || item.status === "paused") {
      anyPending = true;
    }
  }

  if (!anyActive && !anyPending) {
    if (aggregateSpeedHistory.length > 0) aggregateSpeedHistory = [];
    lastAggregateSampleAt = 0;
    return;
  }

  if (now - lastAggregateSampleAt < AGGREGATE_SAMPLE_MS) return;
  if (!anyActive && aggregateSpeedHistory.length === 0) return;

  lastAggregateSampleAt = now;
  const next = aggregateSpeedHistory.length >= SPEED_HISTORY_MAX
    ? aggregateSpeedHistory.slice(aggregateSpeedHistory.length - SPEED_HISTORY_MAX + 1)
    : aggregateSpeedHistory.slice();
  next.push({ t: now, bps });
  aggregateSpeedHistory = next;
}

export function getAggregateSpeedHistory(): SpeedPoint[] {
  return aggregateSpeedHistory;
}

function scheduleFlush() {
  updateAggregateBatch();
  if (flushScheduled) return;
  flushScheduled = true;
  requestAnimationFrame(() => {
    flushScheduled = false;
    sampleAggregateSpeed(Date.now());
    downloads = new Map(downloads);
  });
}

function flushNow() {
  updateAggregateBatch();
  flushScheduled = false;
  sampleAggregateSpeed(Date.now());
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

export type DownloadAggregate = {
  batchId: number;
  outcome: "idle" | "working" | "complete" | "stopped";
  activeCount: number;
  queuedCount: number;
  pausedCount: number;
  failedCount: number;
  speedBps: number;
  downloadedBytes: number;
  totalBytes: number | null;
  percent: number | null;
  etaSeconds: number | null;
};

function finiteBytes(value: number | null | undefined): number {
  return value != null && Number.isFinite(value) ? Math.max(0, value) : 0;
}

export function getAggregate(): DownloadAggregate {
  let activeCount = 0, queuedCount = 0, pausedCount = 0, failedCount = 0;
  let speedBps = 0;
  for (const item of downloads.values()) {
    if (item.status === "queued") queuedCount++;
    if (item.status === "paused") pausedCount++;
    if (item.status === "downloading") {
      activeCount++;
      speedBps += finiteBytes(item.speed);
    }
  }

  let downloadedBytes = 0, knownTotal = 0, remainingBytes = 0;
  let reportedPercentTotal = 0;
  let allTotalsKnown = aggregateBatch.size > 0;
  let allPercentsKnown = aggregateBatch.size > 0;
  let remainingTotalsKnown = true, everyRemainingHasEta = true;
  let allSucceeded = aggregateBatch.size > 0 && !batchCancelled;
  let hasFailed = false;
  let maxItemEta = 0;
  for (const item of aggregateBatch.values()) {
    if (item.status === "error" && !acknowledgedFailures.has(item.id)) failedCount++;
    const finished = item.status === "complete" || item.status === "seeding";
    allSucceeded &&= finished;
    hasFailed ||= item.status === "error";
    const reportedPercent = finished
      ? 100
      : knownPercent(item.percent);
    if (reportedPercent === null) allPercentsKnown = false;
    else reportedPercentTotal += reportedPercent;
    const bytes = finiteBytes(item.downloadedBytes);
    const total = finiteBytes(item.totalBytes) > 0
      ? finiteBytes(item.totalBytes)
      : finished && bytes > 0 ? bytes : null;
    downloadedBytes += finished && total !== null ? total : total !== null ? Math.min(bytes, total) : bytes;
    if (total === null) allTotalsKnown = false;
    else knownTotal += total;
    if (!isPending(item)) continue;
    if (total === null) remainingTotalsKnown = false;
    else remainingBytes += Math.max(0, total - bytes);
    const reportedEta = item.etaSeconds;
    const estimate = reportedEta != null && Number.isFinite(reportedEta) && reportedEta > 0
      ? reportedEta
      : total !== null && finiteBytes(item.speed) > 0 ? Math.max(0, total - bytes) / item.speed : null;
    if (item.status !== "downloading" || finiteBytes(item.speed) === 0 || estimate === null) everyRemainingHasEta = false;
    else maxItemEta = Math.max(maxItemEta, estimate);
  }

  const totalBytes = allTotalsKnown && knownTotal > 0 ? knownTotal : null;
  // Some engines (notably segmented yt-dlp downloads) know logical progress
  // before they know the final combined byte total. The item card already uses
  // that reported percentage, so the global bar must use it as its fallback
  // instead of switching to an unrelated indeterminate animation.
  const percent = totalBytes !== null
    ? Math.min(100, downloadedBytes / totalBytes * 100)
    : allPercentsKnown ? reportedPercentTotal / aggregateBatch.size : null;
  let etaSeconds: number | null = null;
  if (speedBps > 0 && activeCount > 0 && pausedCount === 0 && queuedCount === 0 && !hasFailed && everyRemainingHasEta) {
    const eta = remainingTotalsKnown ? remainingBytes / speedBps : maxItemEta;
    if (Number.isFinite(eta) && eta > 0) etaSeconds = eta;
  }
  const pending = activeCount + queuedCount + pausedCount > 0;
  return {
    batchId: aggregateBatchId,
    outcome: pending ? "working" : allSucceeded ? "complete" : aggregateBatch.size || batchCancelled ? "stopped" : "idle",
    activeCount, queuedCount, pausedCount, failedCount,
    speedBps, downloadedBytes, totalBytes, percent, etaSeconds,
  };
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
  percent: number | null;
  speed_bytes_per_sec: number;
  downloaded_bytes: number;
  total_bytes: number | null;
  file_path: string | null;
  file_size_bytes: number | null;
  file_count: number | null;
  thumbnail_url: string | null;
  kind?: QueueKind;
  eta_seconds?: number | null;
  quality?: string | null;
  download_mode?: string | null;
  author?: string | null;
  duration_seconds?: number | null;
  phase?: string | null;
  stream?: StreamInfo | null;
  streams_done?: StreamInfo[];
  planned_formats?: string[] | null;
  fragment_index?: number | null;
  fragment_count?: number | null;
  started_at_ms?: number | null;
  command?: CommandRecord | null;
};

/** Backend percent, or `null` when the engine does not know the total. */
export function knownPercent(value: number | null | undefined): number | null {
  return value != null && Number.isFinite(value) ? Math.min(100, Math.max(0, value)) : null;
}

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
      suppressedGenericIds.add(id);
    }
  }

  for (const qi of items) {
    suppressedGenericIds.delete(qi.id);
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
      percent: knownPercent(qi.percent),
      speed: effectiveSpeed,
      downloadedBytes: qi.downloaded_bytes,
      totalBytes: qi.total_bytes,
      phase: qi.phase ?? (existing?.kind === "generic" ? existing.phase : undefined) ?? "queued",
      etaSeconds: qi.eta_seconds ?? null,
      status: dlStatus,
      error: extractError(qi.status),
      startedAt: existing?.startedAt ?? now,
      lastUpdateAt: now,
      filePath: qi.file_path ?? undefined,
      fileCount: qi.file_count ?? undefined,
      thumbnail_url: qi.thumbnail_url,
      queueKind: qi.kind,
      quality: qi.quality ?? null,
      downloadMode: qi.download_mode ?? null,
      author: qi.author ?? null,
      durationSeconds: qi.duration_seconds ?? null,
      stream: qi.stream ?? (existing?.kind === "generic" ? existing.stream : null) ?? null,
      streamsDone: qi.streams_done ?? (existing?.kind === "generic" ? existing.streamsDone : undefined) ?? [],
      plannedFormats: qi.planned_formats ?? (existing?.kind === "generic" ? existing.plannedFormats : null) ?? null,
      fragmentIndex: qi.fragment_index ?? null,
      fragmentCount: qi.fragment_count ?? null,
      startedAtMs: qi.started_at_ms ?? null,
      command: qi.command ?? null,
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
  const item = downloads.get(id);
  if (item) {
    downloads.delete(id);
    clearSpeedHistory(id);
    if (item.kind === "generic") {
      suppressedGenericIds.add(id);
    }
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
  percent: number | null,
  speedBytesPerSec: number,
  downloadedBytes: number,
  totalBytes: number | null,
  phase: string,
  etaSeconds?: number | null,
  extra?: GenericProgressExtra,
) {
  const now = Date.now();
  if (suppressedGenericIds.has(id)) return;
  const existing = downloads.get(id);
  const prev = existing?.kind === "generic" ? existing : undefined;

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

  // Evento de progresso é um patch por cima do item, não uma substituição:
  // thumbnail, tipo, comando e afins só chegam pelo `queue-state-update`.
  const nextStream = extra?.stream ?? prev?.stream ?? null;
  let streamsDone = prev?.streamsDone ?? [];
  if (nextStream && prev?.stream && prev.stream.format_id !== nextStream.format_id) {
    if (!streamsDone.some((s) => s.format_id === prev.stream!.format_id)) {
      streamsDone = [...streamsDone, prev.stream];
    }
  }
  downloads.set(id, {
    ...(prev ?? {}),
    kind: "generic",
    id,
    name: title || prev?.name || "",
    platform: platform || prev?.platform || "",
    percent: knownPercent(percent),
    speed: effectiveSpeed,
    downloadedBytes,
    totalBytes: totalBytes ?? prev?.totalBytes ?? null,
    phase,
    etaSeconds: etaSeconds ?? null,
    status: resolvedStatus,
    startedAt: existing?.startedAt ?? now,
    lastUpdateAt: now,
    quality: prev?.quality,
    downloadMode: prev?.downloadMode,
    stream: nextStream,
    streamsDone,
    fragmentIndex: extra?.fragmentIndex ?? prev?.fragmentIndex ?? null,
    fragmentCount: extra?.fragmentCount ?? prev?.fragmentCount ?? null,
    plannedFormats: extra?.plannedFormats ?? prev?.plannedFormats ?? null,
  });

  if (resolvedStatus === "downloading") {
    pushSpeedPoint(id, effectiveSpeed);
  }

  scheduleFlush();
}

export { formatBytes, formatSpeed, formatEta } from "../download-format";
