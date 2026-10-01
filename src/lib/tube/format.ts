import type { Thumb } from "./types";

let LOCALE = "en";

export function setFormatLocale(l: string | null | undefined) {
  if (l) LOCALE = l;
}

export function fmtDuration(s: number | null | undefined): string {
  if (s == null || !Number.isFinite(s) || s < 0) return "0:00";
  const t = Math.floor(s);
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const sec = t % 60;
  const ss = String(sec).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

const compactCache = new Map<string, Intl.NumberFormat>();

export function fmtCount(n: number | null | undefined, locale = LOCALE): string {
  if (n == null || !Number.isFinite(n)) return "";
  let f = compactCache.get(locale);
  if (!f) {
    try {
      f = new Intl.NumberFormat(locale, { notation: "compact", maximumFractionDigits: 1 });
    } catch {
      f = new Intl.NumberFormat("en", { notation: "compact", maximumFractionDigits: 1 });
    }
    compactCache.set(locale, f);
  }
  const abs = Math.abs(n);
  let v = n;
  if (abs >= 1e9) v = Math.trunc(n / 1e8) * 1e8;
  else if (abs >= 1e6) v = Math.trunc(n / 1e5) * 1e5;
  else if (abs >= 1e3) v = Math.trunc(n / 1e2) * 1e2;
  return f.format(v);
}

function https(url: string): string {
  if (url.startsWith("//")) return "https:" + url;
  if (url.startsWith("http://")) return "https://" + url.slice(7);
  return url;
}

export function thumbUrl(thumbs: Thumb[] | null | undefined, width: number, dpr = typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1): string {
  if (!thumbs || thumbs.length === 0) return "";
  const need = width * Math.min(dpr, 2);
  let best: Thumb | null = null;
  let largest: Thumb = thumbs[0];
  for (const t of thumbs) {
    if ((t.width || 0) > (largest.width || 0)) largest = t;
    if ((t.width || 0) >= need && (!best || t.width < best.width)) best = t;
  }
  return https((best ?? largest).url);
}

export function videoThumb(id: string, q: "default" | "mqdefault" | "hqdefault" | "sddefault" | "maxresdefault" = "mqdefault"): string {
  return `https://i.ytimg.com/vi/${id}/${q}.jpg`;
}

export function bestAvatar(src: string | Thumb[] | null | undefined, size = 88, dpr = typeof window !== "undefined" ? window.devicePixelRatio || 1 : 1): string {
  if (!src) return "";
  const px = Math.round(size * Math.min(dpr, 2));
  const url = typeof src === "string" ? https(src) : thumbUrl(src, size, dpr);
  if (!url) return "";
  if (/(ggpht\.com|googleusercontent\.com)/.test(url)) {
    if (/=s\d+/.test(url)) return url.replace(/=s\d+/, `=s${px}`);
    if (/=w\d+-h\d+/.test(url)) return url.replace(/=w\d+-h\d+/, `=w${px}-h${px}`);
  }
  return url;
}

const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["year", 365 * 24 * 3600e3],
  ["month", 30 * 24 * 3600e3],
  ["week", 7 * 24 * 3600e3],
  ["day", 24 * 3600e3],
  ["hour", 3600e3],
  ["minute", 60e3],
  ["second", 1e3],
];

export function timeAgo(ms: number | null | undefined, now = Date.now(), locale = LOCALE): string {
  if (ms == null || !Number.isFinite(ms)) return "";
  const diff = ms - now;
  const abs = Math.abs(diff);
  let rtf: Intl.RelativeTimeFormat;
  try {
    rtf = new Intl.RelativeTimeFormat(locale, { numeric: "always" });
  } catch {
    rtf = new Intl.RelativeTimeFormat("en", { numeric: "always" });
  }
  for (const [unit, size] of UNITS) {
    if (abs >= size || unit === "second") {
      const v = Math.trunc(diff / size);
      return rtf.format(v === 0 ? -0 : v, unit);
    }
  }
  return "";
}

export function fmtBytes(b: number | null | undefined): string {
  if (b == null || !Number.isFinite(b)) return "";
  const u = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let v = b;
  while (v >= 1024 && i < u.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 10 || i === 0 ? Math.round(v) : v.toFixed(1)} ${u[i]}`;
}
