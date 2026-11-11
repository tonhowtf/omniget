import type { Platform } from "./api";
import type { Metric } from "./insights";

export const QUICK_FILTERS = ["likely_inactive", "no_avatar_no_bio", "follows_many_few_followers", "verified", "private"] as const;
export type NamedFilter = { name: string; kind: "quick" | "saved"; about?: string; filter: Record<string, unknown>; created?: number };
export type FilterList = { total: number; saved: number; max_saved: number; items: NamedFilter[] };

export type Counted = { word?: string; hashtag?: string; location?: string; accounts: number };
export type CountedList = { total: number; returned: number; truncated: boolean; items: Counted[] };
export type Interests = {
  followers: number;
  with_bio: number;
  with_bio_share: number | null;
  thin: boolean;
  thin_reason?: string;
  words: CountedList;
  hashtags: CountedList;
  name_words: CountedList;
  locations: CountedList & { available: boolean; with_location: number; reason?: string };
  with_link: { accounts: number; share_of_bios: number | null };
  verified: { accounts: number; share: number | null };
  empty_reason?: string;
};

export type Recommendation = { id: string; message: string; numbers: Record<string, number | string | null>; act_with: { tool: string; args: Record<string, unknown>; note?: string } };
export type Recommendations = { total: number; items: Recommendation[]; not_triggered: { id: string; reason: string }[]; empty_reason?: string };

export type DigestPeople = { total: number; returned: number; truncated: boolean; items: { handle: string; name: string; url: string }[] };
export type Digest = {
  platform: Platform;
  since: number;
  until: number;
  first: boolean;
  marked: boolean;
  quiet: boolean;
  followers: { available: boolean; current?: number | null; delta?: number; gained?: DigestPeople; lost?: DigestPeople; reason?: string };
  monitors: { available: boolean; unread?: number; saved?: number; monitors?: { id: number; name: string; unread: number }[]; reason?: string };
  top_post: { available: boolean; posts_in_period?: number; post?: { id: string; url: string; text: string; engagement: number; created_at: number | null } | null; reason?: string };
  scheduled: { available: boolean; due_next_24h?: number; items?: { id: number; at: string | null; text: string | null }[]; reason?: string };
  recommendations: Recommendations;
};

export type Diff = { a: number | null; b: number | null; diff: number | null; pct: number | null };
export type PeriodSide = { from: number; to: number; days: number; posts: number; engagement: Metric; metrics: Record<string, Metric> };
export type Periods = {
  a: PeriodSide;
  b: PeriodSide;
  difference: Record<string, Diff>;
  coverage: { sample: number; covers_a: boolean; covers_b: boolean; note?: string };
  empty_reason?: string;
};
export const PERIOD_ROWS = ["posts", "engagement_total", "engagement_mean", "engagement_median", "likes_mean", "replies_mean", "reposts_mean", "saves_mean", "views_mean"] as const;

export const SERIES_KEYS = ["likes", "replies", "reposts", "saves", "views"] as const;
export type SeriesKey = (typeof SERIES_KEYS)[number];
export type SeriesPoint = { at: number } & Record<SeriesKey, number | null>;
export type Series = { post_id: string; total: number; points: SeriesPoint[]; change: Record<SeriesKey, number | null>; first_at: number | null; last_at: number | null; empty_reason?: string };
export type Tracked = { total: number; items: { post_id: string; points: number; first_at: number; last_at: number }[] };

export type Engaged = { id?: string; handle: string; name: string; avatar?: string | null; verified?: boolean; follows_me?: boolean; post?: { text: string; url: string } };
export type EngagementPage = { available: boolean; count: number; items: Engaged[]; next_cursor: string | null; reason?: string; empty_reason?: string };
export const ENGAGEMENT_KINDS = ["reposts", "quotes", "replies", "likes"] as const;
export type EngagementKind = (typeof ENGAGEMENT_KINDS)[number];

export type Placed = { handle: string; rank: string };
export type DrawResult = {
  seed: string;
  method: string;
  source: string;
  pool: number;
  pool_truncated: boolean;
  checked: number;
  filled: boolean;
  winners: Placed[];
  runner_ups: Placed[];
  disqualified: { handle: string; reason: string }[];
  notes?: string[];
  csv_path?: string;
};

export type NotifyState = { surge: boolean; unfollowers: boolean; monitors: number[] };
export type ReportResult = { path: string; markdown_path: string; sections: Record<string, boolean> };

export function hasExtras(p: Platform): boolean {
  return p === "x" || p === "instagram" || p === "tiktok";
}

export function weighted<T extends { accounts: number }>(items: T[], min = 0.85, max = 1.9): (T & { size: number })[] {
  const top = Math.max(1, ...items.map((i) => i.accounts));
  const low = Math.min(top, ...items.map((i) => i.accounts));
  const span = Math.sqrt(top) - Math.sqrt(low);
  return items.map((i) => ({ ...i, size: span > 0 ? Math.round((min + ((Math.sqrt(i.accounts) - Math.sqrt(low)) / span) * (max - min)) * 100) / 100 : 1 }));
}

export function isoDay(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

export function lastDays(now: Date, days: number): { from: string; to: string } {
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate() - (days - 1));
  return { from: isoDay(start), to: isoDay(now) };
}

export function periodArgs(form: { from: string; to: string; auto: boolean; compareFrom: string; compareTo: string }): Record<string, unknown> | null {
  const day = /^\d{4}-\d{2}-\d{2}$/;
  if (!day.test(form.from) || !day.test(form.to) || form.from > form.to) return null;
  if (form.auto) return { from: form.from, to: form.to };
  if (!day.test(form.compareFrom) || !day.test(form.compareTo) || form.compareFrom > form.compareTo) return null;
  if (form.compareFrom <= form.to && form.from <= form.compareTo) return null;
  return { from: form.from, to: form.to, compare_from: form.compareFrom, compare_to: form.compareTo };
}

export function signed(n: number | null | undefined, digits = 2): string {
  if (n == null) return "–";
  const r = Math.round(n * 10 ** digits) / 10 ** digits;
  return r > 0 ? `+${r}` : String(r);
}

export function seriesPath(points: SeriesPoint[], key: SeriesKey, width: number, height: number): string {
  const known = points.filter((p) => p[key] != null);
  if (known.length < 2) return "";
  const t0 = known[0].at;
  const span = Math.max(1, known[known.length - 1].at - t0);
  const values = known.map((p) => p[key] as number);
  const low = Math.min(...values);
  const range = Math.max(1, Math.max(...values) - low);
  return known
    .map((p, i) => `${i ? "L" : "M"}${(((p.at - t0) / span) * width).toFixed(1)} ${(height - (((p[key] as number) - low) / range) * height).toFixed(1)}`)
    .join(" ");
}

export function savable(args: Record<string, unknown>): Record<string, unknown> | null {
  const out = Object.fromEntries(Object.entries(args).filter(([, v]) => v !== "" && v != null));
  return Object.keys(out).length ? out : null;
}

export function drawArgs(form: { post: string; source: EngagementKind; winners: number; runnerUps: number; mustFollow: boolean; mustRepost: boolean; exclude: string; seed: string }, extra: Record<string, unknown> = {}): Record<string, unknown> {
  const args: Record<string, unknown> = {
    platform: "x",
    post: form.post,
    source: form.source,
    winners: Math.min(100, Math.max(1, Math.round(form.winners) || 1)),
    runner_ups: Math.min(100, Math.max(0, Math.round(form.runnerUps) || 0)),
    ...extra,
  };
  if (form.mustFollow) args.must_follow = true;
  if (form.mustRepost) args.must_have_reposted = true;
  const exclude = form.exclude
    .split(/[\s,]+/)
    .map((h) => h.trim().replace(/^@/, ""))
    .filter(Boolean);
  if (exclude.length) args.exclude = exclude;
  if (form.seed.trim()) args.seed = form.seed.trim();
  return args;
}

export function recParams(r: Recommendation, weekday: (index: number) => string): Record<string, string | number> {
  const out: Record<string, string | number> = {};
  for (const [k, v] of Object.entries(r.numbers)) if (v != null) out[k] = v;
  if (typeof r.numbers.weekday === "number") out.day = weekday(r.numbers.weekday);
  if (typeof r.numbers.hour === "number") out.hour = String(r.numbers.hour).padStart(2, "0");
  return out;
}
