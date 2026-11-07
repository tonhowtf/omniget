import type { PageSpec } from "./types";

export type PageItem =
  | { key: string; kind: "page"; path: string; page: number; rotate: number }
  | { key: string; kind: "blank"; width: number; height: number };

export type Size = { width: number; height: number };

export type Range = [number, number];

export type RangeResult = { ok: true; ranges: Range[] } | { ok: false; error: "empty" | "syntax" | "bounds" | "order" };

export const A4: Size = { width: 595.28, height: 841.89 };

let seq = 0;

export function newKey(): string {
  seq += 1;
  return `p${seq}`;
}

export function pagesOf(path: string, count: number): PageItem[] {
  return Array.from({ length: count }, (_, page) => ({ key: newKey(), kind: "page" as const, path, page, rotate: 0 }));
}

export function rotateItems(items: PageItem[], keys: Set<string>, delta: number): PageItem[] {
  return items.map((item) => {
    if (!keys.has(item.key)) return item;
    if (item.kind === "page") return { ...item, rotate: (((item.rotate + delta) % 360) + 360) % 360 };
    return Math.abs(delta) % 180 === 90 ? { ...item, width: item.height, height: item.width } : item;
  });
}

export function removeItems(items: PageItem[], keys: Set<string>): PageItem[] {
  return items.filter((item) => !keys.has(item.key));
}

export function moveItems(items: PageItem[], keys: Set<string>, to: number): PageItem[] {
  const moving = items.filter((item) => keys.has(item.key));
  if (moving.length === 0) return items;
  const target = Math.max(0, Math.min(to, items.length));
  const before = items.slice(0, target).filter((item) => keys.has(item.key)).length;
  const rest = items.filter((item) => !keys.has(item.key));
  const at = target - before;
  return [...rest.slice(0, at), ...moving, ...rest.slice(at)];
}

export function insertBlank(items: PageItem[], at: number, size: Size): PageItem[] {
  const blank: PageItem = { key: newKey(), kind: "blank", width: size.width, height: size.height };
  const index = Math.max(0, Math.min(at, items.length));
  return [...items.slice(0, index), blank, ...items.slice(index)];
}

export function selectRange(items: PageItem[], anchor: string, key: string): Set<string> {
  const a = items.findIndex((item) => item.key === anchor);
  const b = items.findIndex((item) => item.key === key);
  if (a < 0 || b < 0) return new Set([key]);
  const [from, to] = a <= b ? [a, b] : [b, a];
  return new Set(items.slice(from, to + 1).map((item) => item.key));
}

export function parseRanges(text: string, max: number): RangeResult {
  const parts = text
    .split(/[,;]/)
    .map((p) => p.replace(/\s+/g, ""))
    .filter(Boolean);
  if (parts.length === 0) return { ok: false, error: "empty" };
  const ranges: Range[] = [];
  for (const part of parts) {
    const m = /^(\d*)(?:([-–])(\d*))?$/.exec(part);
    if (!m || (m[1] === "" && (m[2] === undefined || m[3] === ""))) return { ok: false, error: "syntax" };
    const start = m[1] === "" ? 1 : Number(m[1]);
    const end = m[2] === undefined ? start : m[3] === "" ? max : Number(m[3]);
    if (start < 1 || end < 1 || start > max || end > max) return { ok: false, error: "bounds" };
    if (start > end) return { ok: false, error: "order" };
    ranges.push([start, end]);
  }
  return { ok: true, ranges };
}

export function everyN(total: number, n: number): Range[] {
  const size = Math.max(1, Math.floor(n));
  const out: Range[] = [];
  for (let start = 1; start <= total; start += size) out.push([start, Math.min(total, start + size - 1)]);
  return out;
}

export function sourcesOf(items: PageItem[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const item of items) {
    if (item.kind === "page" && !seen.has(item.path)) {
      seen.add(item.path);
      out.push(item.path);
    }
  }
  return out;
}

export function toSpecs(items: PageItem[], sources: string[]): PageSpec[] {
  return items.map((item) => {
    if (item.kind === "blank") return { kind: "blank", width: item.width, height: item.height };
    const source = sources.indexOf(item.path);
    if (source < 0) throw new Error(`unknown source ${item.path}`);
    return { kind: "page", source, page: item.page, rotate: item.rotate };
  });
}

export function sliceRange(items: PageItem[], [start, end]: Range): PageItem[] {
  return items.slice(start - 1, end);
}

export function rangeLabel([start, end]: Range): string {
  return start === end ? String(start) : `${start}-${end}`;
}

export function isUntouched(items: PageItem[], path: string, count: number): boolean {
  return (
    items.length === count &&
    items.every((item, i) => item.kind === "page" && item.path === path && item.page === i && item.rotate === 0)
  );
}

export function baseName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

export function stem(path: string): string {
  const name = baseName(path);
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
}

export function parentDir(path: string): string {
  const idx = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return idx > 0 ? path.slice(0, idx) : path;
}

export function joinPath(dir: string, name: string): string {
  const sep = dir.includes("\\") && !dir.includes("/") ? "\\" : "/";
  return dir.endsWith(sep) ? `${dir}${name}` : `${dir}${sep}${name}`;
}

export function safeFileName(name: string): string {
  const cleaned = name
    .replace(/[\\/:*?"<>|\u0000-\u001f]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/[. ]+$/, "");
  return cleaned || "document";
}

export function pdfName(name: string): string {
  const safe = safeFileName(name);
  return /\.pdf$/i.test(safe) ? safe : `${safe}.pdf`;
}

function normPath(p: string): string {
  return p.normalize("NFC").replace(/\\/g, "/").replace(/\/+$/, "");
}

export function samePath(a: string, b: string): boolean {
  return normPath(a) === normPath(b);
}

export function maybeSamePath(a: string, b: string): boolean {
  return normPath(a).toLowerCase() === normPath(b).toLowerCase();
}

export function plural(tr: (key: string, vars?: Record<string, unknown>) => unknown, key: string, count: number, vars: Record<string, unknown> = {}): string {
  return String(tr(count === 1 ? `${key}_one` : key, { count, ...vars }));
}
