import type { Crop, DupGroup, Edits, Photo } from "./types";

export type GroupBy = "month" | "day" | "folder";

export type Section = { key: string; title: string; photos: Photo[] };

export type Handle = "move" | "nw" | "ne" | "sw" | "se";

export const EMPTY_EDITS: Edits = { exposure: 0, contrast: 0, saturation: 0, temperature: 0, rotate: 0, crop: null };

export const TONE_KEYS = ["exposure", "contrast", "saturation", "temperature"] as const;

export type ToneKey = (typeof TONE_KEYS)[number];

export const ASPECTS: { id: string; ratio: number | null }[] = [
  { id: "free", ratio: null },
  { id: "original", ratio: 0 },
  { id: "1:1", ratio: 1 },
  { id: "4:3", ratio: 4 / 3 },
  { id: "3:2", ratio: 3 / 2 },
  { id: "16:9", ratio: 16 / 9 },
];

export const MIN_CROP = 0.05;

export const SIMILARITY = { close: 6, similar: 12 } as const;

export function photoTime(p: Photo): number {
  return p.taken_at ?? p.modified_at;
}

function wallDate(seconds: number): Date {
  return new Date(seconds * 1000);
}

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

export function dateKey(seconds: number, by: "month" | "day"): string {
  const d = wallDate(seconds);
  const month = `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}`;
  return by === "month" ? month : `${month}-${pad(d.getUTCDate())}`;
}

export function dateTitle(seconds: number, by: "month" | "day", locale: string): string {
  const options: Intl.DateTimeFormatOptions =
    by === "month" ? { timeZone: "UTC", month: "long", year: "numeric" } : { timeZone: "UTC", day: "numeric", month: "long", year: "numeric" };
  const text = new Intl.DateTimeFormat(locale, options).format(wallDate(seconds));
  return text.charAt(0).toLocaleUpperCase(locale) + text.slice(1);
}

function trimSlash(p: string): string {
  return p.length > 1 ? p.replace(/[\\/]+$/, "") : p;
}

export function baseName(p: string): string {
  const parts = trimSlash(p).split(/[\\/]/);
  return parts[parts.length - 1] || p;
}

export function folderLabel(folder: string, roots: string[]): string {
  const f = trimSlash(folder);
  const root = roots
    .map(trimSlash)
    .filter((r) => f === r || f.startsWith(r + "/") || f.startsWith(r + "\\"))
    .sort((a, b) => b.length - a.length)[0];
  if (!root) return f;
  const rest = f.slice(root.length).replace(/^[\\/]+/, "");
  return rest ? `${baseName(root)} / ${rest.split(/[\\/]/).join(" / ")}` : baseName(root);
}

export function groupPhotos(photos: Photo[], by: GroupBy, locale: string, roots: string[] = []): Section[] {
  const sections = new Map<string, Section>();
  for (const p of photos) {
    const key = by === "folder" ? p.folder : dateKey(photoTime(p), by);
    let s = sections.get(key);
    if (!s) {
      s = { key, title: by === "folder" ? folderLabel(p.folder, roots) : dateTitle(photoTime(p), by, locale), photos: [] };
      sections.set(key, s);
    }
    s.photos.push(p);
  }
  const list = [...sections.values()];
  for (const s of list) s.photos.sort((a, b) => photoTime(b) - photoTime(a) || a.path.localeCompare(b.path));
  if (by === "folder") list.sort((a, b) => a.title.localeCompare(b.title, locale, { numeric: true }));
  else list.sort((a, b) => b.key.localeCompare(a.key));
  return list;
}

export function filterPhotos(photos: Photo[], query: string): Photo[] {
  const q = query.trim().toLocaleLowerCase();
  if (!q) return photos;
  return photos.filter((p) => p.name.toLocaleLowerCase().includes(q) || p.folder.toLocaleLowerCase().includes(q));
}

export function isIdentity(e: Edits): boolean {
  return TONE_KEYS.every((k) => e[k] === 0) && e.rotate % 360 === 0 && e.crop === null;
}

export function sameEdits(a: Edits, b: Edits): boolean {
  return (
    TONE_KEYS.every((k) => a[k] === b[k]) &&
    a.rotate === b.rotate &&
    (a.crop === b.crop || (!!a.crop && !!b.crop && a.crop.x === b.crop.x && a.crop.y === b.crop.y && a.crop.w === b.crop.w && a.crop.h === b.crop.h))
  );
}

export function rotateCrop(c: Crop, clockwise: boolean): Crop {
  return clockwise ? { x: 1 - (c.y + c.h), y: c.x, w: c.h, h: c.w } : { x: c.y, y: 1 - (c.x + c.w), w: c.h, h: c.w };
}

export function rotate(e: Edits, clockwise: boolean): Edits {
  const turn = clockwise ? 90 : 270;
  return { ...e, rotate: (e.rotate + turn) % 360, crop: e.crop ? rotateCrop(e.crop, clockwise) : null };
}

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

export function fitAspect(c: Crop, ratio: number, imageAspect: number): Crop {
  const target = ratio / imageAspect;
  let w = c.w;
  let h = w / target;
  if (h > c.h) {
    h = c.h;
    w = h * target;
  }
  return { x: c.x + (c.w - w) / 2, y: c.y + (c.h - h) / 2, w, h };
}

export function resizeCrop(start: Crop, handle: Handle, dx: number, dy: number, ratio: number | null, imageAspect: number): Crop {
  if (handle === "move") {
    return { ...start, x: clamp(start.x + dx, 0, 1 - start.w), y: clamp(start.y + dy, 0, 1 - start.h) };
  }
  const west = handle === "nw" || handle === "sw";
  const north = handle === "nw" || handle === "ne";
  const ax = west ? start.x + start.w : start.x;
  const ay = north ? start.y + start.h : start.y;
  const maxW = west ? ax : 1 - ax;
  const maxH = north ? ay : 1 - ay;
  let w = clamp(west ? start.w - dx : start.w + dx, MIN_CROP, maxW);
  let h = clamp(north ? start.h - dy : start.h + dy, MIN_CROP, maxH);
  if (ratio) {
    const target = ratio / imageAspect;
    h = w / target;
    if (h > maxH) {
      h = maxH;
      w = h * target;
    }
    if (h < MIN_CROP) {
      h = MIN_CROP;
      w = Math.min(maxW, h * target);
    }
  }
  return { x: west ? ax - w : ax, y: north ? ay - h : ay, w, h };
}

export function cleanCrop(c: Crop | null): Crop | null {
  if (!c) return null;
  if (c.x <= 0.001 && c.y <= 0.001 && c.w >= 0.999 && c.h >= 0.999) return null;
  const r = (v: number) => Math.round(v * 10000) / 10000;
  return { x: r(c.x), y: r(c.y), w: r(c.w), h: r(c.h) };
}

export function defaultRemovals(groups: DupGroup[]): Set<string> {
  const out = new Set<string>();
  for (const g of groups) for (const p of g.paths) if (p !== g.keep) out.add(p);
  return out;
}

export function savedBytes(groups: DupGroup[], chosen: Set<string>, sizes: Map<string, number>): number {
  let total = 0;
  for (const g of groups) for (const p of g.paths) if (chosen.has(p)) total += sizes.get(p) ?? 0;
  return total;
}

export function keepsOne(group: DupGroup, chosen: Set<string>): boolean {
  return group.paths.some((p) => !chosen.has(p));
}

export function rangeSelect(order: string[], anchor: string | null, target: string): string[] {
  const end = order.indexOf(target);
  const start = anchor === null ? -1 : order.indexOf(anchor);
  if (end < 0) return [];
  if (start < 0) return [target];
  const [a, b] = start < end ? [start, end] : [end, start];
  return order.slice(a, b + 1);
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}
