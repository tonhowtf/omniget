/// Pure logic behind the washmybelly UI kit: grouping and selection for
/// `SelectList`, the virtual window, plan and receipt summaries. No Svelte
/// and no Tauri here, so it is all covered by `ui.test.ts`.

import type { Risk } from "./api";
import { dayBucket, type DayBucket } from "./format";

export type SelectItem = {
  id: string;
  label: string;
  detail?: string | null;
  bytes?: number;
  risk?: Risk;
  group?: string;
  disabled?: boolean;
  /// Optional hint under the label, drawn instead of `detail` when present.
  hint?: string | null;
  /// Detail is a path: shortened in the middle instead of at the end.
  detailIsPath?: boolean;
};

export type ItemGroup<T extends SelectItem = SelectItem> = {
  key: string;
  items: T[];
  count: number;
  bytes: number;
};

export const RISK_ORDER: readonly Risk[] = ["read", "reversible", "destructive", "extreme"];

/// Groups in first-appearance order (the scan already sorts them the way the
/// module wants). Items without a group land in `""`.
export function groupItems<T extends SelectItem>(items: readonly T[]): ItemGroup<T>[] {
  const map = new Map<string, ItemGroup<T>>();
  for (const it of items) {
    const key = it.group ?? "";
    let g = map.get(key);
    if (!g) {
      g = { key, items: [], count: 0, bytes: 0 };
      map.set(key, g);
    }
    g.items.push(it);
    g.count++;
    g.bytes += it.bytes ?? 0;
  }
  return [...map.values()];
}

/// Count and bytes of the selected, enabled items.
export function sumSelected(items: readonly SelectItem[], selected: ReadonlySet<string>): { count: number; bytes: number } {
  let count = 0;
  let bytes = 0;
  for (const it of items) {
    if (it.disabled || !selected.has(it.id)) continue;
    count++;
    bytes += it.bytes ?? 0;
  }
  return { count, bytes };
}

export type TriState = "none" | "some" | "all";

/// Checkbox state of a set of items (disabled ones do not count).
export function selectionState(items: readonly SelectItem[], selected: ReadonlySet<string>): TriState {
  let on = 0;
  let total = 0;
  for (const it of items) {
    if (it.disabled) continue;
    total++;
    if (selected.has(it.id)) on++;
  }
  if (total === 0 || on === 0) return "none";
  return on === total ? "all" : "some";
}

/// New selection with `items` turned on or off. Other ids are kept.
export function setMany(selected: ReadonlySet<string>, items: readonly SelectItem[], on: boolean): Set<string> {
  const next = new Set(selected);
  for (const it of items) {
    if (it.disabled) continue;
    if (on) next.add(it.id);
    else next.delete(it.id);
  }
  return next;
}

export function toggleOne(selected: ReadonlySet<string>, id: string): Set<string> {
  const next = new Set(selected);
  if (next.has(id)) next.delete(id);
  else next.add(id);
  return next;
}

/// Drops ids that are no longer in the list (after a rescan).
export function pruneSelection(selected: ReadonlySet<string>, items: readonly SelectItem[]): Set<string> {
  const ids = new Set(items.map((i) => i.id));
  return new Set([...selected].filter((id) => ids.has(id)));
}

// ── Virtual list ─────────────────────────────────────────────────────

export type Row<T extends SelectItem = SelectItem> =
  | { kind: "header"; key: string; group: ItemGroup<T>; collapsed: boolean }
  | { kind: "item"; key: string; item: T; group: string };

/// Flattens groups into rows. A group header is emitted only when there is
/// more than one group or the single group has a name.
export function flattenRows<T extends SelectItem>(groups: readonly ItemGroup<T>[], collapsed: ReadonlySet<string> = new Set()): Row<T>[] {
  const withHeaders = groups.length > 1 || (groups.length === 1 && groups[0].key !== "");
  const rows: Row<T>[] = [];
  for (const g of groups) {
    const isCollapsed = collapsed.has(g.key);
    if (withHeaders) rows.push({ kind: "header", key: `h:${g.key}`, group: g, collapsed: isCollapsed });
    if (isCollapsed) continue;
    for (const it of g.items) rows.push({ kind: "item", key: it.id, item: it, group: g.key });
  }
  return rows;
}

/// Prefix sums of row heights: `offsets[i]` is the top of row i and
/// `offsets[rows.length]` the total height.
export function rowOffsets(heights: readonly number[]): number[] {
  const out = new Array<number>(heights.length + 1);
  out[0] = 0;
  for (let i = 0; i < heights.length; i++) out[i + 1] = out[i] + heights[i];
  return out;
}

/// First row whose bottom is below `y` (binary search over offsets).
function rowAt(offsets: readonly number[], y: number): number {
  let lo = 0;
  let hi = offsets.length - 2;
  if (hi < 0) return 0;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (offsets[mid + 1] <= y) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/// Rows to render for a viewport: `[start, end)` plus the padding above and
/// below. `overscan` rows on each side keep fast scrolling from flashing.
export function visibleRange(
  offsets: readonly number[],
  scrollTop: number,
  viewport: number,
  overscan = 8,
): { start: number; end: number; padTop: number; padBottom: number } {
  const n = offsets.length - 1;
  if (n <= 0) return { start: 0, end: 0, padTop: 0, padBottom: 0 };
  const first = rowAt(offsets, Math.max(0, scrollTop));
  const last = rowAt(offsets, Math.max(0, scrollTop + viewport - 1));
  const start = Math.max(0, first - overscan);
  const end = Math.min(n, last + 1 + overscan);
  return { start, end, padTop: offsets[start], padBottom: offsets[n] - offsets[end] };
}

// ── Plans ────────────────────────────────────────────────────────────

type Buckets = Record<string, { count: number; bytes: number }>;

/// `by_risk` in risk order, highest last, skipping empty buckets.
export function riskRows(byRisk: Buckets | null | undefined): { risk: Risk; count: number; bytes: number }[] {
  if (!byRisk) return [];
  return RISK_ORDER.filter((r) => (byRisk[r]?.count ?? 0) > 0).map((r) => ({ risk: r, ...byRisk[r] }));
}

/// `by_group` biggest first (then by name for a stable order).
export function groupRows(byGroup: Buckets | null | undefined): { group: string; count: number; bytes: number }[] {
  if (!byGroup) return [];
  return Object.entries(byGroup)
    .map(([group, v]) => ({ group, count: v.count, bytes: v.bytes }))
    .sort((a, b) => b.bytes - a.bytes || b.count - a.count || a.group.localeCompare(b.group));
}

export function hasRisk(byRisk: Buckets | null | undefined, risk: Risk): boolean {
  return (byRisk?.[risk]?.count ?? 0) > 0;
}

/// Label key of the one primary button, by plan kind.
export function confirmKey(kind: string): string {
  switch (kind) {
    case "cleanup":
    case "dev_artifacts":
    case "duplicates":
    case "empty_folders":
    case "large_files":
      return "washmybelly.plan.confirm";
    case "uninstall":
      return "washmybelly.plan.confirm_uninstall";
    case "startup":
    case "privacy":
    case "tweaks":
    case "hosts":
      return "washmybelly.plan.confirm_apply";
    case "packages":
      return "washmybelly.plan.confirm_packages";
    case "maintenance":
      return "washmybelly.plan.confirm_run";
    default:
      return "washmybelly.plan.confirm_generic";
  }
}

/// Title key of the review sheet, by plan kind.
export function reviewTitleKey(kind: string): string {
  switch (kind) {
    case "cleanup":
    case "dev_artifacts":
    case "duplicates":
    case "empty_folders":
    case "large_files":
      return "washmybelly.plan.review_title";
    default:
      return "washmybelly.plan.review_title_changes";
  }
}

/// Does this kind free space (so "N freed" makes sense), or change settings?
export function freesSpace(kind: string): boolean {
  return ["cleanup", "dev_artifacts", "duplicates", "empty_folders", "large_files", "uninstall", "packages", "maintenance"].includes(kind);
}

// ── Execution progress ───────────────────────────────────────────────

export type StepState = "pending" | "active" | "done" | "failed";
export type ExecStep = { id: "prepare" | "run" | "admin" | "done"; state: StepState };

/// Steps shown while a plan runs, from the last `washmybelly-progress`
/// stage (`run`, `admin`, `done`). The admin step exists only when needed.
export function execSteps(stage: string | null, needsAdmin: boolean, failed = false): ExecStep[] {
  const ids: ExecStep["id"][] = needsAdmin ? ["prepare", "run", "admin", "done"] : ["prepare", "run", "done"];
  const at = stage === null ? 0 : Math.max(0, ids.indexOf(stage as ExecStep["id"]));
  const finished = stage === "done";
  return ids.map((id, i) => {
    let state: StepState = "pending";
    if (finished || i < at) state = "done";
    else if (i === at) state = failed ? "failed" : "active";
    return { id, state };
  });
}

/// 0..1 progress for the bar, never going backwards past done.
export function fraction(done: number, total: number): number {
  if (!total || total <= 0) return 0;
  return Math.max(0, Math.min(1, done / total));
}

// ── Receipts ─────────────────────────────────────────────────────────

export type ReceiptLike = { receipt_id: string; created: number };

/// Receipts (newest first) under day headings.
export function groupByDay<T extends ReceiptLike>(list: readonly T[], now: Date = new Date()): { key: string; bucket: DayBucket; ts: number; items: T[] }[] {
  const out: { key: string; bucket: DayBucket; ts: number; items: T[] }[] = [];
  const sorted = [...list].sort((a, b) => b.created - a.created);
  for (const r of sorted) {
    const { bucket, key } = dayBucket(r.created, now);
    let g = out[out.length - 1];
    if (!g || g.key !== key) {
      g = { key, bucket, ts: r.created, items: [] };
      out.push(g);
    }
    g.items.push(r);
  }
  return out;
}

/// Items of a receipt split by status, failed first (what needs attention).
export function splitByStatus<T extends { status: "done" | "skipped" | "failed" }>(items: readonly T[] | null | undefined): { failed: T[]; skipped: T[]; done: T[] } {
  const out = { failed: [] as T[], skipped: [] as T[], done: [] as T[] };
  for (const it of items ?? []) out[it.status].push(it);
  return out;
}

/// Label key for a plan kind in the history list.
export function kindKey(kind: string): string {
  return `washmybelly.receipts.kind.${kind}`;
}

// ── Charts ───────────────────────────────────────────────────────────

/// "Nice" upper bound for an axis: 1, 2, 2.5 or 5 × 10^n at or above `v`.
export function niceMax(v: number): number {
  if (!Number.isFinite(v) || v <= 0) return 1;
  const exp = Math.floor(Math.log10(v));
  const base = 10 ** exp;
  for (const m of [1, 2, 2.5, 5, 10]) {
    if (m * base >= v) return m * base;
  }
  return 10 * base;
}

/// SVG path of a series over a `width × height` box, `max` at the top. The
/// last `capacity` points fill the width, so a short series grows from the
/// right like Activity Monitor.
export function linePath(values: readonly number[], width: number, height: number, max: number, capacity = values.length): string {
  if (values.length === 0) return "";
  const cap = Math.max(2, capacity);
  const step = width / (cap - 1);
  const x0 = width - step * (values.length - 1);
  const y = (v: number) => height - (Math.max(0, Math.min(v, max)) / (max || 1)) * height;
  return values.map((v, i) => `${i ? "L" : "M"}${(x0 + i * step).toFixed(1)},${y(v).toFixed(1)}`).join(" ");
}

/// Same line closed down to the baseline, for the soft area under it.
export function areaPath(values: readonly number[], width: number, height: number, max: number, capacity = values.length): string {
  const line = linePath(values, width, height, max, capacity);
  if (!line) return "";
  const cap = Math.max(2, capacity);
  const step = width / (cap - 1);
  const x0 = width - step * (values.length - 1);
  const xEnd = x0 + step * (values.length - 1);
  return `${line} L${xEnd.toFixed(1)},${height} L${x0.toFixed(1)},${height} Z`;
}

/// Health score colour token: green, yellow, orange, red.
export function healthTone(score: number): "good" | "fair" | "poor" | "bad" {
  if (score >= 80) return "good";
  if (score >= 60) return "fair";
  if (score >= 40) return "poor";
  return "bad";
}

/// Module whose tile a plan kind borrows in the history list.
export function kindModule(kind: string): string {
  switch (kind) {
    case "cleanup":
      return "clean";
    case "dev_artifacts":
    case "duplicates":
    case "empty_folders":
    case "large_files":
      return "disk";
    case "hosts":
      return "privacy";
    case "uninstall":
    case "startup":
    case "privacy":
    case "tweaks":
    case "packages":
    case "maintenance":
      return kind;
    default:
      return "receipts";
  }
}
