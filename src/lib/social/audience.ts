export type ViewId =
  | "not_following_back"
  | "fans"
  | "mutuals"
  | "unfollowers"
  | "new_followers"
  | "followers"
  | "following"
  | "inactive"
  | "kept"
  | "hidden"
  | "stargazers"
  | "new_stargazers"
  | "unstarred";

export const STAR_VIEWS: ViewId[] = ["stargazers", "new_stargazers", "unstarred"];

export const LIST_VIEWS: ViewId[] = [
  "not_following_back",
  "unfollowers",
  "new_followers",
  "fans",
  "mutuals",
  "followers",
  "following",
  "inactive",
  "kept",
  "hidden",
];

export const SORTS = [
  "newest",
  "oldest",
  "followers",
  "followers_asc",
  "following",
  "following_asc",
  "posts",
  "ratio",
  "ratio_asc",
  "ratio_balanced",
  "handle",
  "name",
  "first_seen",
] as const;

export type Person = {
  id: string;
  handle: string;
  name: string;
  avatar?: string;
  verified: boolean;
  private: boolean;
  followers?: number;
  following?: number;
  posts?: number;
  bio: string;
  first_seen?: number;
  followed_at?: number;
  url: string;
  ratio?: number;
  follows_me?: boolean;
  followed_by_me?: boolean;
  kept: boolean;
  hidden: boolean;
  event?: "gained" | "lost";
  between?: [number, number];
  status?: "unfollowed" | "unstarred" | "gone";
  checked?: boolean;
  current?: boolean;
  activity?: "inactive" | "no_posts";
  last_post?: number;
};

export type SyncedAt = { followers: number | null; following: number | null };

export type ListSide = { fetched: number; expected: number | null; source: "site" | "export"; at: number | null };

export type Completeness = { followers: ListSide | null; following: ListSide | null };

export type Gap = { kind: "followers" | "following"; fetched: number; expected: number };

export function gaps(c: Completeness | null | undefined): Gap[] {
  const out: Gap[] = [];
  for (const kind of ["following", "followers"] as const) {
    const side = c?.[kind];
    if (side && side.source === "site" && side.expected != null && side.fetched < side.expected) {
      out.push({ kind, fetched: side.fetched, expected: side.expected });
    }
  }
  return out;
}

export type ImportResult = {
  account: string;
  files: { file: string; kind: "followers" | "following"; accounts: number }[];
  imported: Record<"followers" | "following", { accounts: number; with_date: number } | null>;
  sources: Record<"followers" | "following", "site" | "export" | null>;
  completeness: Completeness;
  warning: string | null;
  not_following_back: number | null;
};

export type ListResult = {
  view: ViewId;
  total: number;
  returned: number;
  offset: number;
  truncated: boolean;
  unavailable_fields: string[];
  items: Person[];
  empty_reason?: "not_synced" | "history_too_short" | "no_matches";
  unknown?: number;
  checked_now?: number;
  check_stopped?: string;
  account: string;
  synced_at: SyncedAt;
  completeness?: Completeness;
  warning?: string;
};

export type ListState = {
  kind: "followers" | "following" | "stargazers";
  state: "complete" | "short" | "pending" | "partial" | "abandoned";
  fetched: number;
  expected: number | null;
  started: number;
  finished: number | null;
};

export type Summary = {
  account: string;
  counts: Record<string, number>;
  history: { snapshots: number; started: number | null; too_short: boolean };
  lists: ListState[];
  synced_at: SyncedAt;
  completeness?: Completeness;
  warning?: string;
};

export type SyncResult = {
  account: string;
  complete: boolean;
  stopped: "done" | "time" | "page_cap" | "rate_limited" | "short" | "fresh" | "sweep_wait";
  pages: number;
  fetched: number;
  total_expected: number | null;
  lists: ListState[];
  changes: { gained: number; lost: number } | null;
  first_snapshot: boolean;
  continue_with: { tool: string; args: Record<string, unknown> } | null;
  retry_after_seconds: number | null;
};

export type TrendDay = {
  day: string;
  followers: number | null;
  following: number | null;
  ratio: number | null;
  gained: number;
  lost: number;
};

export type Trend = {
  days: TrendDay[];
  retention: {
    buckets: { bucket: string; count: number }[];
    since_before_history: number;
    history_started: number | null;
    history_days: number | null;
  };
  too_short: boolean;
};

export type Tri = "" | "yes" | "no";

export type Filters = {
  q: string;
  sort: string;
  verified: Tri;
  private: Tri;
  has_avatar: Tri;
  has_bio: Tri;
  has_link: Tri;
  min_followers: string;
  max_followers: string;
  min_ratio: string;
  max_ratio: string;
};

export const EMPTY_FILTERS: Filters = {
  q: "",
  sort: "newest",
  verified: "",
  private: "",
  has_avatar: "",
  has_bio: "",
  has_link: "",
  min_followers: "",
  max_followers: "",
  min_ratio: "",
  max_ratio: "",
};

const FLAGS = ["verified", "private", "has_avatar", "has_bio", "has_link"] as const;
const NUMBERS = ["min_followers", "max_followers", "min_ratio", "max_ratio"] as const;

export function filterArgs(f: Filters): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  if (f.q.trim()) out.q = f.q.trim();
  if (f.sort && f.sort !== "newest") out.sort = f.sort;
  for (const k of FLAGS) if (f[k]) out[k] = f[k] === "yes";
  for (const k of NUMBERS) {
    const n = Number(f[k].replace(",", "."));
    if (f[k].trim() && Number.isFinite(n) && n >= 0) out[k] = k.endsWith("ratio") ? n : Math.round(n);
  }
  return out;
}

export function activeFilters(f: Filters): number {
  const { q: _q, ...rest } = filterArgs(f);
  return Object.keys(rest).length;
}

export function repoName(raw: string): string | null {
  const t = raw.trim().replace(/^https?:\/\/(www\.)?github\.com\//i, "").replace(/\.git$/i, "");
  const m = t.match(/^([A-Za-z0-9-]+)\/([A-Za-z0-9._-]+)(?:[/?#].*)?$/);
  return m ? `${m[1]}/${m[2]}`.toLowerCase() : null;
}

type StarCounts = { stargazers?: number; new_stargazers?: number; unstarred?: number };

export function fromRepoSummary(s: Summary): Summary {
  const at = s.synced_at as unknown as { stargazers?: number | null };
  const c = s.counts as StarCounts;
  return { ...s, counts: { ...s.counts, followers: c.stargazers ?? 0 }, synced_at: { followers: at.stargazers ?? null, following: null } };
}

export function fromRepoTrend(t: Trend): Trend {
  return { ...t, days: t.days.map((d) => ({ ...d, followers: (d as unknown as { stargazers?: number | null }).stargazers ?? null, following: null, ratio: null })) };
}

export function lastSync(s: SyncedAt | undefined): number | null {
  if (!s) return null;
  const times = [s.followers, s.following].filter((x): x is number => x != null);
  return times.length ? Math.min(...times) : null;
}

export function pendingSync(lists: ListState[]): boolean {
  const open = lists.filter((l) => l.state === "partial" || l.state === "pending");
  return open.length > 0 && lists.some((l) => l.state !== "pending");
}

export function progress(lists: ListState[]): { done: number; total: number | null } {
  const done = lists.reduce((n, l) => n + l.fetched, 0);
  const known = lists.length > 0 && lists.every((l) => l.expected != null);
  return { done, total: known ? lists.reduce((n, l) => n + (l.expected ?? 0), 0) : null };
}

export function linePath(values: (number | null)[], width: number, height: number, pad = 4): string {
  const known = values.filter((v): v is number => v != null);
  if (!known.length) return "";
  const min = Math.min(...known);
  const span = Math.max(...known) - min || 1;
  const step = values.length > 1 ? (width - pad * 2) / (values.length - 1) : 0;
  let path = "";
  let pen = false;
  values.forEach((v, i) => {
    if (v == null) {
      pen = false;
      return;
    }
    const x = values.length > 1 ? pad + i * step : width / 2;
    const y = height - pad - ((v - min) / span) * (height - pad * 2);
    path += `${pen ? "L" : "M"}${x.toFixed(1)} ${y.toFixed(1)}`;
    pen = true;
  });
  return path;
}

export type Bar = { x: number; width: number; up: number; down: number };

export function changeBars(days: TrendDay[], width: number, half: number, pad = 4): Bar[] {
  const peak = Math.max(1, ...days.map((d) => Math.max(d.gained, d.lost)));
  const slot = (width - pad * 2) / Math.max(days.length, 1);
  const w = Math.max(2, Math.min(18, slot * 0.6));
  return days.map((d, i) => ({
    x: pad + slot * i + (slot - w) / 2,
    width: w,
    up: (d.gained / peak) * half,
    down: (d.lost / peak) * half,
  }));
}

export function errorCode(e: unknown): string {
  const text = typeof e === "string" ? e : e && typeof e === "object" && "message" in e ? String((e as { message: unknown }).message) : String(e);
  return text.split(":")[0].trim();
}
