// Small formatters for the threads UI: durations, relative times, tokens and
// money. Pure, no i18n (numbers and units are the same in en and pt).

/** 14 s → "14s", 134 s → "2m 14s", 3 780 s → "1h 3m". */
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) ms = 0;
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return s % 60 ? `${m}m ${s % 60}s` : `${m}m`;
  const h = Math.floor(m / 60);
  return m % 60 ? `${h}h ${m % 60}m` : `${h}h`;
}

/** Compact relative age: "now", "5m", "2h", "3d", "4w". */
export function formatAge(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return "";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  const s = Math.max(0, Math.round((now - t) / 1000));
  if (s < 45) return "now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h`;
  const d = Math.round(h / 24);
  if (d < 14) return `${d}d`;
  return `${Math.round(d / 7)}w`;
}

/** Countdown until `iso`, rounded up so it never reads "0m". */
export function formatUntil(iso: string | null | undefined, now = Date.now()): string {
  if (!iso) return "";
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return "";
  const m = Math.max(1, Math.ceil((t - now) / 60000));
  if (m < 60) return `${m}m`;
  const h = Math.ceil(m / 60);
  if (h < 48) return `${h}h`;
  return `${Math.ceil(h / 24)}d`;
}

/** 950 / 1.2k / 84k / 1.2m. */
export function formatTokens(n: number | null | undefined): string {
  const v = Math.max(0, Math.round(n ?? 0));
  if (v < 1000) return String(v);
  if (v < 10_000) return (v / 1000).toFixed(1).replace(/\.0$/, "") + "k";
  if (v < 1_000_000) return Math.round(v / 1000) + "k";
  return (v / 1_000_000).toFixed(1).replace(/\.0$/, "") + "m";
}

export function formatCost(usd: number | null | undefined): string {
  if (usd == null || !Number.isFinite(usd)) return "";
  if (usd === 0) return "$0";
  if (usd < 0.01) return "<$0.01";
  if (usd < 10) return `$${usd.toFixed(2)}`;
  return `$${usd.toFixed(1)}`;
}

/** Time of day for hover timestamps (locale aware). */
export function formatClock(iso: string | null | undefined): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "";
  const today = new Date();
  const sameDay = d.toDateString() === today.toDateString();
  const time = d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
  if (sameDay) return time;
  return `${d.toLocaleDateString(undefined, { month: "numeric", day: "numeric" })} ${time}`;
}

export function formatFull(iso: string | null | undefined): string {
  if (!iso) return "";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? "" : d.toLocaleString();
}

/** Middle-truncates long paths/branches: "src/…/deep/file.ts". */
export function middleTruncate(s: string, max = 36): string {
  if (s.length <= max) return s;
  const keep = max - 1;
  const head = Math.ceil(keep / 2);
  return s.slice(0, head) + "…" + s.slice(s.length - (keep - head));
}

export function basename(p: string): string {
  const parts = p.split(/[\\/]/).filter(Boolean);
  return parts.at(-1) ?? p;
}

export function dirname(p: string): string {
  const i = Math.max(p.lastIndexOf("/"), p.lastIndexOf("\\"));
  return i > 0 ? p.slice(0, i) : "";
}

/** Path relative to `root` when inside it. */
export function relativeTo(p: string, root: string | null | undefined): string {
  if (!root) return p;
  const r = root.replace(/[\\/]+$/, "");
  if (p === r) return ".";
  if (p.startsWith(r + "/") || p.startsWith(r + "\\")) return p.slice(r.length + 1);
  return p;
}
