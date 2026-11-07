import type { Click, CutItem, CutOutcome, Inspected, InstallProgress, Status } from "./types";

export const MODEL_CHOICES = [
  { id: "u2netp", label: "cutout.model_fast", hint: "cutout.model_fast_hint" },
  { id: "silueta", label: "cutout.model_balanced", hint: "cutout.model_balanced_hint" },
  { id: "isnet-general-use", label: "cutout.model_quality", hint: "cutout.model_quality_hint" },
] as const;

export const DEFAULT_MODEL = MODEL_CHOICES[0].id;
export const REFINE_MODEL = "sam21";
export const EDGE_LIMIT = 20;
export const MAX_CLICKS = 24;
export const SWATCHES = ["#ffffff", "#000000", "#f2f2f2", "#1e6fe8", "#34c759", "#ff3b30", "#ffcc00"];

export type RowStatus = "idle" | "queued" | "running" | "done" | "failed" | "stopped";

export type Row = {
  id: string;
  source: Inspected;
  frameMs: number | null;
  clicks: Click[];
  status: RowStatus;
  outcome: CutOutcome | null;
};

export function rowId(path: string, frameMs: number | null): string {
  return frameMs === null ? path : `${path}#${frameMs}`;
}

export function isVideo(row: Row): boolean {
  return row.source.info?.kind === "video";
}

export function durationOf(row: Row): number {
  return row.source.info?.duration_ms ?? 0;
}

export function clampFrame(ms: number, durationMs: number): number {
  if (!Number.isFinite(ms) || ms < 0) return 0;
  const last = Math.max(0, durationMs - 100);
  return Math.round(Math.min(ms, last));
}

export function firstFrame(durationMs: number): number {
  return clampFrame(Math.min(durationMs * 0.1, 5000), durationMs);
}

export function newRows(inspected: Inspected[], taken: Iterable<string>): Row[] {
  const seen = new Set(taken);
  const rows: Row[] = [];
  for (const source of inspected) {
    const frameMs = source.info?.kind === "video" ? firstFrame(source.info.duration_ms) : null;
    const id = rowId(source.path, frameMs);
    if (seen.has(id)) continue;
    seen.add(id);
    rows.push({ id, source, frameMs, clicks: [], status: "idle", outcome: null });
  }
  return rows;
}

export function moveFrame(rows: Row[], row: Row, ms: number): Row | null {
  const frameMs = clampFrame(ms, durationOf(row));
  const id = rowId(row.source.path, frameMs);
  if (rows.some((r) => r !== row && r.id === id)) return null;
  return { ...row, id, frameMs, clicks: [], status: "idle", outcome: null };
}

export function addMoment(rows: Row[], row: Row): Row[] {
  const duration = durationOf(row);
  const step = Math.max(1000, Math.round(duration / 10));
  let ms = clampFrame((row.frameMs ?? 0) + step, duration);
  const taken = new Set(rows.map((r) => r.id));
  while (taken.has(rowId(row.source.path, ms)) && ms < duration - 100) ms = clampFrame(ms + 1000, duration);
  const id = rowId(row.source.path, ms);
  if (taken.has(id)) return rows;
  const at = rows.indexOf(row);
  const copy: Row = { id, source: row.source, frameMs: ms, clicks: [], status: "idle", outcome: null };
  return [...rows.slice(0, at + 1), copy, ...rows.slice(at + 1)];
}

export function pointIn(clientX: number, clientY: number, rect: { left: number; top: number; width: number; height: number }): { x: number; y: number } | null {
  if (rect.width <= 0 || rect.height <= 0) return null;
  const x = (clientX - rect.left) / rect.width;
  const y = (clientY - rect.top) / rect.height;
  if (x < 0 || x > 1 || y < 0 || y > 1) return null;
  return { x, y };
}

export function addClick(clicks: Click[], click: Click): Click[] {
  return [...clicks, click].slice(-MAX_CLICKS);
}

export function undoClick(clicks: Click[]): Click[] {
  return clicks.slice(0, -1);
}

export function installPercent(parts: Record<string, Pick<InstallProgress, "done" | "total">>): number | null {
  let done = 0;
  let total = 0;
  for (const part of Object.values(parts)) {
    if (!part.total) continue;
    done += Math.min(part.done, part.total);
    total += part.total;
  }
  return total > 0 ? Math.floor((done / total) * 100) : null;
}

export function modelReady(status: Status | null, model: string): boolean {
  if (!status?.runtime_installed) return false;
  if (model === REFINE_MODEL) return status.refine.downloaded;
  return status.models.some((m) => m.id === model && m.downloaded);
}

export function modelBytes(status: Status | null, model: string): number {
  if (!status) return 0;
  if (model === REFINE_MODEL) return status.refine.bytes;
  return status.models.find((m) => m.id === model)?.bytes ?? 0;
}

export function toItems(rows: Row[]): CutItem[] {
  return rows.map((r) => ({ path: r.source.path, frame_ms: r.frameMs, clicks: r.clicks }));
}

export function pending(rows: Row[]): Row[] {
  return rows.filter((r) => r.source.info && r.status !== "done");
}

export function tally(rows: Row[]) {
  const counts = { idle: 0, queued: 0, running: 0, done: 0, failed: 0, stopped: 0 };
  for (const row of rows) counts[row.status] += 1;
  return counts;
}

export function formatMoment(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}
