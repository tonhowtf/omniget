import type { Span } from "./types";

export type Segment = Span & { id: string };

export const MIN_SEGMENT = 0.05;

export function clamp(t: number, duration: number): number {
  if (!Number.isFinite(t)) return 0;
  return Math.min(Math.max(t, 0), Math.max(duration, 0));
}

export function length(seg: Span): number {
  return Math.max(0, seg.end - seg.start);
}

export function totalLength(segs: Span[]): number {
  return segs.reduce((sum, s) => sum + length(s), 0);
}

export function isValid(seg: Span): boolean {
  return Number.isFinite(seg.start) && Number.isFinite(seg.end) && seg.end - seg.start >= MIN_SEGMENT;
}

export function nearestKeyframe(t: number, keyframes: number[]): number {
  if (keyframes.length === 0) return t;
  let lo = 0;
  let hi = keyframes.length - 1;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (keyframes[mid] < t) lo = mid + 1;
    else hi = mid;
  }
  const after = keyframes[lo];
  const before = lo > 0 ? keyframes[lo - 1] : after;
  return Math.abs(t - before) <= Math.abs(after - t) ? before : after;
}

export function keyframeAtOrBefore(t: number, keyframes: number[]): number | null {
  let found: number | null = null;
  for (const k of keyframes) {
    if (k <= t + 1e-6) found = k;
    else break;
  }
  return found;
}

export function stepKeyframe(t: number, keyframes: number[], dir: 1 | -1): number | null {
  if (dir > 0) return keyframes.find((k) => k > t + 1e-3) ?? null;
  for (let i = keyframes.length - 1; i >= 0; i--) if (keyframes[i] < t - 1e-3) return keyframes[i];
  return null;
}

export function snap(t: number, keyframes: number[], enabled: boolean): number {
  return enabled ? nearestKeyframe(t, keyframes) : t;
}

export type Marks = { segments: Segment[]; pendingIn: number | null };

export function markIn(marks: Marks, t: number): Marks {
  return { segments: marks.segments, pendingIn: t };
}

export function markOut(marks: Marks, t: number, makeId: () => string): { marks: Marks; added: Segment | null; error: "before_in" | "too_short" | null } {
  const lastEnd = marks.segments.length ? Math.max(...marks.segments.map((s) => s.end)) : 0;
  const start = marks.pendingIn ?? (lastEnd < t ? lastEnd : 0);
  if (t <= start) return { marks, added: null, error: marks.pendingIn !== null ? "before_in" : "too_short" };
  const seg = { id: makeId(), start, end: t };
  if (!isValid(seg)) return { marks, added: null, error: "too_short" };
  return { marks: { segments: [...marks.segments, seg], pendingIn: null }, added: seg, error: null };
}

export function updateSegment(segs: Segment[], id: string, patch: Partial<Span>, duration: number): Segment[] {
  return segs.map((s) => {
    if (s.id !== id) return s;
    const start = clamp(patch.start ?? s.start, duration);
    const end = clamp(patch.end ?? s.end, duration);
    return end - start >= MIN_SEGMENT ? { ...s, start, end } : s;
  });
}

export function move<T>(list: T[], from: number, to: number): T[] {
  if (from === to || from < 0 || from >= list.length || to < 0 || to >= list.length) return list;
  const out = list.slice();
  const [item] = out.splice(from, 1);
  out.splice(to, 0, item);
  return out;
}

export function remove<T extends { id: string }>(list: T[], id: string): T[] {
  return list.filter((s) => s.id !== id);
}

export function hasOverlaps(segs: Span[]): boolean {
  const sorted = segs.slice().sort((a, b) => a.start - b.start);
  for (let i = 1; i < sorted.length; i++) if (sorted[i].start < sorted[i - 1].end - 1e-6) return true;
  return false;
}

export function mergeOverlaps(segs: Segment[]): Segment[] {
  const sorted = segs.slice().sort((a, b) => a.start - b.start);
  const out: Segment[] = [];
  for (const seg of sorted) {
    const last = out[out.length - 1];
    if (last && seg.start <= last.end + 1e-6) {
      out[out.length - 1] = { ...last, end: Math.max(last.end, seg.end) };
    } else {
      out.push({ ...seg });
    }
  }
  return out;
}

export function segmentAt(segs: Span[], t: number): number {
  return segs.findIndex((s) => t >= s.start && t < s.end);
}

export function toRequestSpans(segs: Span[], duration: number): Span[] {
  return segs
    .map((s) => ({ start: clamp(s.start, duration), end: clamp(s.end, duration) }))
    .filter(isValid);
}
