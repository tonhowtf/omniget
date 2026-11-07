import type { RawSegment, Segment, Word } from "./types";

export const MIN_CUE_MS = 200;

let counter = 0;

export function newId(): string {
  counter += 1;
  return `seg-${Date.now().toString(36)}-${counter}`;
}

function pad(n: number, width = 2): string {
  return String(Math.floor(n)).padStart(width, "0");
}

export function formatTimestamp(ms: number, sep: "," | "."): string {
  const total = Math.max(0, Math.round(ms));
  const h = Math.floor(total / 3_600_000);
  const m = Math.floor((total % 3_600_000) / 60_000);
  const s = Math.floor((total % 60_000) / 1000);
  return `${pad(h)}:${pad(m)}:${pad(s)}${sep}${pad(total % 1000, 3)}`;
}

export function formatClock(ms: number): string {
  const total = Math.max(0, Math.round(ms));
  const m = Math.floor(total / 60_000);
  const s = Math.floor((total % 60_000) / 1000);
  const tenths = Math.floor((total % 1000) / 100);
  return `${m}:${pad(s)}.${tenths}`;
}

export function parseTimestamp(raw: string): number | null {
  const m = raw.trim().match(/^(?:(\d+):)?(\d{1,2}):(\d{1,2})(?:[,.](\d{1,3}))?$/);
  if (!m) return null;
  const [, h, mm, ss, frac] = m;
  const ms = frac ? Number(frac.padEnd(3, "0")) : 0;
  return (Number(h ?? 0) * 3600 + Number(mm) * 60 + Number(ss)) * 1000 + ms;
}

export function tokenize(text: string): string[] {
  return text.split(/\s+/).filter(Boolean);
}

export function estimateWords(text: string, start_ms: number, end_ms: number): Word[] {
  const tokens = tokenize(text);
  if (tokens.length === 0) return [];
  const span = Math.max(0, end_ms - start_ms);
  const weights = tokens.map((t) => t.length + 1);
  const total = weights.reduce((a, b) => a + b, 0);
  let acc = 0;
  return tokens.map((t, i) => {
    const s = start_ms + Math.round((span * acc) / total);
    acc += weights[i];
    const e = i === tokens.length - 1 ? end_ms : start_ms + Math.round((span * acc) / total);
    return { text: t, start_ms: s, end_ms: e };
  });
}

export function makeSegment(start_ms: number, end_ms: number, text: string, words?: Word[]): Segment {
  const clean = text.replace(/\s+/g, " ").trim();
  const end = Math.max(end_ms, start_ms + MIN_CUE_MS);
  const usable = words && words.length > 0 && words.map((w) => w.text).join(" ") === tokenize(clean).join(" ");
  return {
    id: newId(),
    start_ms,
    end_ms: end,
    text: clean,
    words: usable ? words : estimateWords(clean, start_ms, end),
    estimated: !usable,
  };
}

export function fromRaw(raw: RawSegment[]): Segment[] {
  return raw
    .filter((r) => r.text.trim().length > 0)
    .map((r) => makeSegment(Math.max(0, r.start_ms), Math.max(0, r.end_ms), r.text, r.words))
    .sort((a, b) => a.start_ms - b.start_ms);
}

export function editText(seg: Segment, text: string): Segment {
  return { ...makeSegment(seg.start_ms, seg.end_ms, text), id: seg.id };
}

export function setTiming(seg: Segment, start_ms: number, end_ms: number): Segment {
  const start = Math.max(0, Math.round(start_ms));
  const end = Math.max(Math.round(end_ms), start + MIN_CUE_MS);
  const oldSpan = Math.max(1, seg.end_ms - seg.start_ms);
  const scale = (end - start) / oldSpan;
  const words = seg.words.map((w) => ({
    text: w.text,
    start_ms: start + Math.round((w.start_ms - seg.start_ms) * scale),
    end_ms: start + Math.round((w.end_ms - seg.start_ms) * scale),
  }));
  return { ...seg, start_ms: start, end_ms: end, words };
}

export function shiftSegment(seg: Segment, delta: number): Segment {
  const d = Math.max(-seg.start_ms, Math.round(delta));
  return {
    ...seg,
    start_ms: seg.start_ms + d,
    end_ms: seg.end_ms + d,
    words: seg.words.map((w) => ({ ...w, start_ms: w.start_ms + d, end_ms: w.end_ms + d })),
  };
}

export function shiftAll(list: Segment[], delta: number): Segment[] {
  const first = list.reduce((min, s) => Math.min(min, s.start_ms), Infinity);
  const d = Number.isFinite(first) ? Math.max(-first, Math.round(delta)) : 0;
  return list.map((s) => shiftSegment(s, d));
}

export function mergeWithNext(list: Segment[], index: number): Segment[] {
  const a = list[index];
  const b = list[index + 1];
  if (!a || !b) return list;
  const merged: Segment = {
    id: a.id,
    start_ms: Math.min(a.start_ms, b.start_ms),
    end_ms: Math.max(a.end_ms, b.end_ms),
    text: `${a.text} ${b.text}`.trim(),
    words: [...a.words, ...b.words],
    estimated: a.estimated || b.estimated,
  };
  return [...list.slice(0, index), merged, ...list.slice(index + 2)];
}

export function wordIndexAt(text: string, caret: number): number {
  let count = 0;
  for (const m of text.matchAll(/\S+/g)) {
    if ((m.index ?? 0) + m[0].length <= caret) count += 1;
  }
  return count;
}

export function splitAt(list: Segment[], index: number, caret?: number): Segment[] {
  const seg = list[index];
  if (!seg || seg.words.length < 2) return list;
  const count = seg.words.length;
  const at = caret === undefined ? Math.ceil(count / 2) : Math.min(count - 1, Math.max(1, wordIndexAt(seg.text, caret)));
  const left = seg.words.slice(0, at);
  const right = seg.words.slice(at);
  const cut = right[0].start_ms;
  const a: Segment = {
    id: seg.id,
    start_ms: seg.start_ms,
    end_ms: Math.max(cut, seg.start_ms + MIN_CUE_MS),
    text: left.map((w) => w.text).join(" "),
    words: left,
    estimated: seg.estimated,
  };
  const b: Segment = {
    id: newId(),
    start_ms: cut,
    end_ms: Math.max(seg.end_ms, cut + MIN_CUE_MS),
    text: right.map((w) => w.text).join(" "),
    words: right,
    estimated: seg.estimated,
  };
  return [...list.slice(0, index), a, b, ...list.slice(index + 1)];
}

export function removeAt(list: Segment[], index: number): Segment[] {
  return list.filter((_, i) => i !== index);
}

export function insertAfter(list: Segment[], index: number, durationMs: number): Segment[] {
  const prev = list[index];
  const next = list[index + 1];
  const start = prev ? prev.end_ms : 0;
  const end = Math.min(next ? next.start_ms : start + 2000, durationMs > 0 ? durationMs : Infinity);
  const seg = makeSegment(start, Math.max(end, start + MIN_CUE_MS), "");
  return [...list.slice(0, index + 1), seg, ...list.slice(index + 1)];
}

function plainText(seg: Segment): string {
  return seg.text.replace(/\r?\n/g, " ").trim();
}

export function toSrt(list: Segment[]): string {
  return list
    .filter((s) => s.text.trim())
    .map((s, i) => `${i + 1}\n${formatTimestamp(s.start_ms, ",")} --> ${formatTimestamp(s.end_ms, ",")}\n${plainText(s)}\n`)
    .join("\n");
}

export function toVtt(list: Segment[]): string {
  const body = list
    .filter((s) => s.text.trim())
    .map((s) => `${formatTimestamp(s.start_ms, ".")} --> ${formatTimestamp(s.end_ms, ".")}\n${plainText(s)}\n`)
    .join("\n");
  return `WEBVTT\n\n${body}`;
}

function stripMarkup(text: string): string {
  return text
    .replace(/<[^>]*>/g, "")
    .replace(/\{[^}]*\}/g, "")
    .replace(/&amp;/g, "&")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&nbsp;/g, " ");
}

export function parseSubtitles(source: string): Segment[] {
  const text = source.replace(/^﻿/, "").replace(/\r\n?/g, "\n");
  const blocks = text.split(/\n{2,}/);
  const raw: RawSegment[] = [];
  for (const block of blocks) {
    const lines = block.split("\n").filter((l) => l.trim() !== "");
    const at = lines.findIndex((l) => l.includes("-->"));
    if (at < 0) continue;
    const [left, rest] = lines[at].split("-->");
    const right = rest.trim().split(/\s+/)[0] ?? "";
    const start = parseTimestamp(left);
    const end = parseTimestamp(right);
    if (start === null || end === null) continue;
    const body = stripMarkup(lines.slice(at + 1).join(" ")).replace(/\s+/g, " ").trim();
    if (!body) continue;
    raw.push({ start_ms: start, end_ms: end, text: body });
  }
  return fromRaw(raw);
}

export function byStart(list: Segment[]): Segment[] {
  return list
    .map((s, i) => ({ s, i }))
    .sort((a, b) => a.s.start_ms - b.s.start_ms || a.i - b.i)
    .map((x) => x.s);
}

export function retime(list: Segment[], index: number, seg: Segment): Segment[] {
  return byStart(list.map((s, i) => (i === index ? seg : s)));
}
