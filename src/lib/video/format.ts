// Small pure helpers shared by the player, the study panels and the course views.

import type { Cue } from "./types";

/** "1:02:03" or "12:34" from milliseconds. */
export function fmtTime(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) ms = 0;
  const total = Math.floor(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/** Wall clock "14:05" for a unix ms timestamp, in the user's locale. */
export function fmtClock(at: number): string {
  try {
    return new Date(at).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  } catch {
    return "";
  }
}

/** "1 h 20 min" style duration for course summaries. */
export function fmtDuration(ms: number): { h: number; m: number } {
  const min = Math.round(ms / 60000);
  return { h: Math.floor(min / 60), m: min % 60 };
}

/** Speed label without float noise: 1.25 -> "1.25", 2 -> "2". */
export function fmtSpeed(v: number): string {
  return String(Math.round(v * 100) / 100);
}

function tsToMs(ts: string): number {
  const parts = ts.trim().replace(",", ".").split(":");
  let sec = 0;
  for (const p of parts) sec = sec * 60 + Number(p);
  return Math.round(sec * 1000);
}

/** Minimal WebVTT parser (the backend already converts SRT/ASS to VTT). */
export function parseVtt(text: string): Cue[] {
  const out: Cue[] = [];
  const blocks = text.replace(/^﻿/, "").replace(/\r\n?/g, "\n").split(/\n{2,}/);
  for (const block of blocks) {
    const lines = block.split("\n");
    const at = lines.findIndex((l) => l.includes("-->"));
    if (at < 0) continue;
    const [a, rest] = lines[at].split("-->");
    const b = rest.trim().split(/\s+/)[0];
    const start = tsToMs(a);
    const end = tsToMs(b);
    if (!Number.isFinite(start) || !Number.isFinite(end)) continue;
    const body = lines
      .slice(at + 1)
      .join("\n")
      .replace(/<[^>]+>/g, "")
      .replace(/&amp;/g, "&")
      .replace(/&lt;/g, "<")
      .replace(/&gt;/g, ">")
      .replace(/&nbsp;/g, " ")
      .trim();
    if (body) out.push({ start_ms: start, end_ms: end, text: body });
  }
  out.sort((x, y) => x.start_ms - y.start_ms);
  return out;
}

/** Index of the cue active at `ms` (binary search on sorted cues), or -1. */
export function cueAt(cues: Cue[], ms: number): number {
  let lo = 0;
  let hi = cues.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (cues[mid].start_ms <= ms) {
      found = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  // overlapping cues: walk back to any that still covers ms
  for (let i = found; i >= 0 && i > found - 4; i--) {
    if (cues[i].start_ms <= ms && ms < cues[i].end_ms) return i;
  }
  return -1;
}

/** Index of the last cue that started at or before `ms` (for prev/next line). */
export function cueBefore(cues: Cue[], ms: number): number {
  let lo = 0;
  let hi = cues.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (cues[mid].start_ms <= ms) {
      found = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return found;
}

type AnswerPart = { text: string } | { t_ms: number; label: string };

/** Splits an AI answer into text and clickable "[mm:ss]" / "[h:mm:ss]" citations. */
export function splitCitations(answer: string): AnswerPart[] {
  const parts: AnswerPart[] = [];
  const re = /\[(\d{1,2}:)?(\d{1,2}):(\d{2})\]/g;
  let last = 0;
  let m: RegExpExecArray | null;
  while ((m = re.exec(answer))) {
    if (m.index > last) parts.push({ text: answer.slice(last, m.index) });
    const h = m[1] ? Number(m[1].slice(0, -1)) : 0;
    const t_ms = ((h * 60 + Number(m[2])) * 60 + Number(m[3])) * 1000;
    parts.push({ t_ms, label: m[0].slice(1, -1) });
    last = m.index + m[0].length;
  }
  if (last < answer.length) parts.push({ text: answer.slice(last) });
  return parts;
}

export const VIDEO_EXT = /\.(mp4|m4v|mkv|webm|mov|avi|wmv|flv|ts|m2ts|mts|mpg|mpeg|3gp|ogv)$/i;

export function isLink(s: string): boolean {
  return /^https?:\/\/\S+$/i.test(s.trim());
}

export function baseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

export function parentDir(path: string): string {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return i > 0 ? path.slice(0, i) : path;
}
