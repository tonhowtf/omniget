import type { Platform, Post } from "./api";

export type Report = "summary" | "posts" | "times" | "media" | "hashtags" | "words" | "trend" | "outliers" | "compare" | "repos";
export const REPORTS: Report[] = ["summary", "posts", "times", "media", "hashtags", "words", "trend", "outliers", "compare"];
export const SAMPLES = [20, 50, 100, 200];
export const REPO_SORTS = ["stars", "forks", "open_issues", "pushed"] as const;
export type RepoRow = { repo: string; url: string; description: string; stars: number | null; forks: number | null; open_issues: number | null; pushed: number | null; language: string | null; fork: boolean; archived: boolean };
export type RepoTotals = { repos: number; stars: number; forks: number; open_issues: number; forked_from_others: number; archived: number };

export function reportsFor(p: Platform): Report[] {
  return p === "github" ? ["summary", "posts", "times", "trend", "repos", "compare"] : REPORTS;
}

export function hasReplies(p: Platform): boolean {
  return p === "x" || p === "reddit";
}

export type Metric = { available: boolean; posts?: number; total?: number; mean?: number; median?: number };
export type Unavailable = { metric: string; reason: string };
export type Summary = {
  sample: number;
  analyzed: number;
  from: number | null;
  to: number | null;
  types: Record<string, number>;
  metrics: Record<string, Metric>;
  engagement: Metric;
  confidence: "low" | "medium" | "high";
  unavailable: Unavailable[];
};
export type PostRow = {
  id: string;
  url: string;
  created_at: number | null;
  type: string;
  media: string;
  text: string;
  likes: number | null;
  replies: number | null;
  reposts: number | null;
  saves: number | null;
  views: number | null;
  engagement: number;
  lift: number | null;
  outlier: "mild" | "strong" | "huge" | null;
};
export type Listing<T> = { total: number; returned: number; offset: number; truncated: boolean; items: T[]; unit?: string; timezone?: string };
export type Cell = { weekday: number; hour: number; posts: number; engagement: number; mean: number; thin: boolean };
export type Times = { cells: Cell[]; best: Cell | null; thin_below: number; undated: number; timezone: string };
export type MediaRow = { media: string; posts: number; engagement_mean: number | null; engagement_median: number | null; views_mean: number | null };
export type HashtagRow = { hashtag: string; uses: number; engagement_mean: number | null; engagement_median: number | null };
export type WordRow = { word: string; uses: number; posts: number };
export type TrendRow = { start: string; posts: number; engagement: number; mean: number | null };
export type Outliers = { metric: string; median: number | null; sample: number; confidence: string; min_recommended: number; total: number; items: PostRow[]; reason?: string };
export type Compare = { sample_each: number; confidence: string; accounts: { handle: string; summary: Summary }[] };
export type Envelope<T> = {
  platform: Platform;
  handle?: string;
  report: Report;
  sample_requested?: number;
  sample?: number;
  fetched_at?: number;
  cached?: boolean;
  result: T;
  empty_reason?: string;
  partial_reason?: string;
};

export type Surge = { surge: boolean; today: number; trailing_average: number | null; days_used: number; multiple: number; min_today: number; reason?: string };
export type MonitorOptions = {
  phrase?: string;
  any?: string[];
  exclude?: string[];
  hashtags?: string[];
  from?: string[];
  to?: string[];
  mentions?: string[];
  lang?: string;
  min_likes?: number;
  min_reposts?: number;
  exclude_replies?: boolean;
  exclude_reposts?: boolean;
  sort?: string;
  time?: string;
  subreddit?: string;
};

export const REDDIT_SORTS = ["new", "relevance", "hot", "top", "comments"] as const;
export const REDDIT_TIMES = ["hour", "day", "week", "month", "year", "all"] as const;

export function redditOptions(form: { sort: string; time: string; subreddit: string }): MonitorOptions {
  const out: MonitorOptions = {};
  if (form.sort && form.sort !== "new") out.sort = form.sort;
  if (form.time) out.time = form.time;
  if (form.subreddit.trim()) out.subreddit = form.subreddit.trim();
  return out;
}

export function monitorQueryKey(p: Platform, kind: string): string {
  if (p === "x") return "query_x";
  if (p === "instagram") return kind === "profile" ? "query_profile" : "query_hashtag";
  if (p === "reddit" && kind === "subreddit") return "query_subreddit";
  if (p === "github") return kind === "repositories" ? "query_gh_repositories" : kind === "issues" ? "query_gh_issues" : "query_repo";
  return "query_tiktok";
}
export type Monitor = {
  id: number;
  platform: Platform;
  name: string;
  kind: string;
  query: string;
  input: { query?: string; options?: MonitorOptions };
  builtin: boolean;
  created: number;
  last_checked: number | null;
  unread: number;
  total_stored: number;
};
export type Hit = { post: Post; seen_at: number; read: boolean };
export type MonitorList = { monitors: Monitor[]; unread: number; kinds: string[]; mentions_inbox: { available: boolean; reason?: string } };
export type MonitorDetail = {
  monitor: Monitor;
  hits: { total: number; returned: number; truncated: boolean; items: Hit[]; empty_reason?: string };
  days: { day: string; count: number }[];
  surge: Surge;
};
export type CheckResult = {
  checked: number;
  new: number;
  surging: number[];
  monitors: { id: number; name: string; new: number; surge: Surge; partial_reason?: string }[];
  failed?: { id: number; name: string; error: string }[];
  partial_reason?: string;
};

export type ToolError = { code: string; message: string; next_step: string | null; retryable: boolean };

export function toolError(e: unknown): ToolError {
  const text = typeof e === "string" ? e : e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : String(e);
  const i = text.indexOf(":");
  if (i <= 0 || !/^[A-Z_0-9]+$/.test(text.slice(0, i))) return { code: "UNKNOWN", message: text, next_step: null, retryable: false };
  const code = text.slice(0, i);
  const rest = text.slice(i + 1).trim();
  if (rest.startsWith("{")) {
    try {
      const body = JSON.parse(rest) as { message?: unknown; next_step?: unknown; retryable?: unknown };
      return {
        code,
        message: typeof body.message === "string" ? body.message : rest,
        next_step: typeof body.next_step === "string" ? body.next_step : null,
        retryable: body.retryable === true,
      };
    } catch {
      return { code, message: rest, next_step: null, retryable: false };
    }
  }
  return { code, message: rest, next_step: null, retryable: code === "RATE_LIMITED" || code === "NETWORK" };
}

export function whoKey(p: Platform): "handle" | "username" {
  return p === "x" ? "handle" : "username";
}

export function cleanHandle(raw: string): string {
  const t = raw.trim().replace(/^@+/, "");
  const link = t.match(/^https?:\/\/[^/]+\/@?([^/?#]+)/);
  return (link ? link[1] : t).trim();
}

export function splitList(raw: string): string[] {
  return raw
    .split(/[,\n]/)
    .map((x) => x.trim())
    .filter(Boolean);
}

export function heatGrid(cells: Cell[]): { max: number; rows: (Cell | null)[][] } {
  const rows: (Cell | null)[][] = Array.from({ length: 7 }, () => Array.from({ length: 24 }, () => null));
  let max = 0;
  for (const c of cells) {
    if (c.weekday < 0 || c.weekday > 6 || c.hour < 0 || c.hour > 23) continue;
    rows[c.weekday][c.hour] = c;
    if (c.mean > max) max = c.mean;
  }
  return { max, rows };
}

export function heatLevel(mean: number, max: number): number {
  if (max <= 0 || mean <= 0) return 0;
  return Math.min(4, Math.max(1, Math.ceil((mean / max) * 4)));
}

export function sparkPoints(values: number[], width: number, height: number): string {
  if (!values.length) return "";
  const max = Math.max(...values, 1);
  const step = values.length > 1 ? width / (values.length - 1) : 0;
  return values
    .map((v, i) => {
      const x = values.length > 1 ? i * step : width / 2;
      const y = height - (v / max) * height;
      return `${Math.round(x * 10) / 10},${Math.round(y * 10) / 10}`;
    })
    .join(" ");
}

export function sortRows<T extends Record<string, unknown>>(rows: T[], key: string, ascending: boolean): T[] {
  return [...rows].sort((a, b) => {
    const x = a[key];
    const y = b[key];
    if (x == null && y == null) return 0;
    if (x == null) return 1;
    if (y == null) return -1;
    const order = typeof x === "number" && typeof y === "number" ? x - y : String(x).localeCompare(String(y));
    return ascending ? order : -order;
  });
}

export function weekdayName(index: number, locale?: string): string {
  const monday = new Date(Date.UTC(2024, 0, 1 + index));
  try {
    return new Intl.DateTimeFormat(locale, { weekday: "short", timeZone: "UTC" }).format(monday);
  } catch {
    return String(index);
  }
}

export function buildOptions(form: {
  phrase: string;
  any: string;
  exclude: string;
  hashtags: string;
  from: string;
  to: string;
  mentions: string;
  lang: string;
  minLikes: string;
  minReposts: string;
  excludeReplies: boolean;
  excludeReposts: boolean;
}): MonitorOptions {
  const out: MonitorOptions = {};
  if (form.phrase.trim()) out.phrase = form.phrase.trim();
  for (const key of ["any", "exclude", "hashtags", "from", "to", "mentions"] as const) {
    const list = splitList(form[key]);
    if (list.length) out[key] = list;
  }
  if (form.lang.trim()) out.lang = form.lang.trim();
  const likes = Number.parseInt(form.minLikes, 10);
  if (likes > 0) out.min_likes = likes;
  const reposts = Number.parseInt(form.minReposts, 10);
  if (reposts > 0) out.min_reposts = reposts;
  if (form.excludeReplies) out.exclude_replies = true;
  if (form.excludeReposts) out.exclude_reposts = true;
  return out;
}
