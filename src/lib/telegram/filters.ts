// Form state for the media filters shared by auto-save rules and the
// whole-chat download, and its conversion to the backend `MediaFilter`.

import { parseExts, type MediaFilter } from "./api";

export interface FilterForm {
  types: string[];
  minMb: string;
  maxMb: string;
  keyword: string;
  extIn: string;
  extOut: string;
}

export function emptyFilter(types: string[] = ["video"]): FilterForm {
  return { types: [...types], minMb: "", maxMb: "", keyword: "", extIn: "", extOut: "" };
}

const MB = 1024 * 1024;

function mb(v: string): number | undefined {
  const n = Number(String(v).replace(",", "."));
  return Number.isFinite(n) && n > 0 ? Math.round(n * MB) : undefined;
}

export function toMb(bytes: number | null | undefined): string {
  return bytes && bytes > 0 ? String(Math.round((bytes / MB) * 10) / 10) : "";
}

export function filterFromForm(f: FilterForm): MediaFilter {
  const out: MediaFilter = {};
  if (f.types.length) out.mediaTypes = [...f.types];
  const min = mb(f.minMb);
  const max = mb(f.maxMb);
  if (min) out.minSize = min;
  if (max) out.maxSize = max;
  if (f.keyword.trim()) out.keyword = f.keyword.trim();
  const inc = parseExts(f.extIn);
  const exc = parseExts(f.extOut);
  if (inc.length) out.extInclude = inc;
  if (exc.length) out.extExclude = exc;
  return out;
}

/** "2026-09-29" in local time → unix seconds; `end` moves to 23:59:59. */
export function dateToUnix(v: string, end = false): number | undefined {
  if (!v) return undefined;
  const [y, m, d] = v.split("-").map(Number);
  if (!y || !m || !d) return undefined;
  const dt = end ? new Date(y, m - 1, d, 23, 59, 59) : new Date(y, m - 1, d, 0, 0, 0);
  return Math.floor(dt.getTime() / 1000);
}

export function positiveInt(v: string | number): number | undefined {
  const n = Math.floor(Number(v));
  return Number.isFinite(n) && n > 0 ? n : undefined;
}
