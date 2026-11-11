type Args = Record<string, unknown>;

const NOW = Math.floor(Date.now() / 1000);
const DAY = 86400;
const PLATFORM: Record<string, string> = { x: "x", ig: "instagram", tt: "tiktok", rd: "reddit", pin: "pinterest", gh: "github" };
const KINDS: Record<string, string[]> = { instagram: ["hashtag", "profile"], reddit: ["search", "subreddit"], github: ["repositories", "issues", "releases", "repo_issues"] };

function repos(a: Args) {
  const names = ["loop", "fetchers", "notes.md", "dotfiles", "tiny-queue", "paper-ui", "old-site"];
  const key = String(a.sort ?? "stars");
  const dir = a.order === "asc" ? 1 : -1;
  const items = names
    .map((n, i) => ({ repo: `preview.me/${n}`, url: `https://github.com/preview.me/${n}`, description: TEXTS[i % TEXTS.length], stars: 900 - i * 140 + (i % 2) * 60, forks: 40 - i * 5, open_issues: (i * 7) % 23, pushed: NOW - i * 9 * DAY, language: ["Rust", "TypeScript", "Go"][i % 3], fork: i === 5, archived: i === 6 }))
    .sort((x, y) => (Number(x[key as "stars"]) - Number(y[key as "stars"])) * dir);
  const sum = (k: "stars" | "forks" | "open_issues") => items.reduce((n, r) => n + r[k], 0);
  return { total: items.length, returned: items.length, offset: 0, truncated: false, items, totals: { repos: items.length, stars: sum("stars"), forks: sum("forks"), open_issues: sum("open_issues"), forked_from_others: 1, archived: 1 }, sort: key, order: dir === 1 ? "asc" : "desc" };
}
const TEXTS = [
  "Shipped the new export flow today #buildinpublic #tauri",
  "Thread: what I learned moving our docs to plain markdown",
  "Morning light at the studio #design",
  "Tried the 30-day sketch challenge. Day 12 and still going #design",
  "New track is out. Headphones on #music",
  "Small tools beat big platforms. Change my mind.",
];

function row(platform: string, n: number) {
  const likes = n % 9 === 0 ? 1400 + n * 11 : 60 + ((n * 37) % 180);
  const replies = 3 + (n % 12);
  const reposts = platform === "instagram" ? null : 4 + (n % 20);
  const saves = platform === "instagram" ? null : 2 + (n % 7);
  const views = platform === "instagram" && n % 3 ? null : likes * 22;
  const engagement = likes + replies + (reposts ?? 0) + (saves ?? 0);
  const lift = Math.round((engagement / 190) * 100) / 100;
  return {
    id: String(1900000000 + n),
    url: platform === "x" ? `https://x.com/preview.me/status/${1900000000 + n}` : platform === "instagram" ? `https://www.instagram.com/p/C${n}/` : `https://www.tiktok.com/@preview.me/video/${1900000000 + n}`,
    created_at: NOW - n * 9 * 3600,
    type: platform === "x" ? (n % 5 === 0 ? "reply" : "post") : platform === "instagram" ? (n % 3 ? "feed" : "reel") : "video",
    media: platform === "tiktok" ? "video" : ["text", "image", "video", "carousel"][n % 4],
    text: TEXTS[n % TEXTS.length],
    likes,
    replies,
    reposts,
    saves,
    views,
    engagement,
    lift,
    outlier: lift >= 5 ? "huge" : lift >= 2 ? "strong" : lift >= 1.5 ? "mild" : null,
  };
}

function metric(values: (number | null)[]) {
  const v = values.filter((x): x is number => x != null).sort((a, b) => a - b);
  if (!v.length) return { available: false };
  const total = v.reduce((a, b) => a + b, 0);
  return { available: true, posts: v.length, total, mean: Math.round((total / v.length) * 100) / 100, median: v[Math.floor(v.length / 2)] };
}

function summary(platform: string, rows: ReturnType<typeof row>[]) {
  const types: Record<string, number> = {};
  for (const r of rows) types[r.type] = (types[r.type] ?? 0) + 1;
  const hidden = ["reach", "demographics", "profile_visits", "impressions", ...(platform === "x" ? ["quotes"] : platform === "instagram" ? ["saves", "shares", "likes_when_hidden"] : [])];
  return {
    sample: rows.length,
    analyzed: rows.length,
    from: rows[rows.length - 1]?.created_at ?? null,
    to: rows[0]?.created_at ?? null,
    types,
    metrics: {
      likes: metric(rows.map((r) => r.likes)),
      replies: metric(rows.map((r) => r.replies)),
      reposts: metric(rows.map((r) => r.reposts)),
      saves: metric(rows.map((r) => r.saves)),
      views: metric(rows.map((r) => r.views)),
    },
    engagement: metric(rows.map((r) => r.engagement)),
    confidence: rows.length < 10 ? "low" : rows.length < 20 ? "medium" : "high",
    unavailable: hidden.map((m) => ({ metric: m, reason: "Not sent to a web session (preview data)." })),
  };
}

function listing<T>(items: T[], a: Args) {
  const offset = Number(a.offset ?? 0) || 0;
  const limit = Number(a.limit ?? 50) || 50;
  const page = items.slice(offset, offset + limit);
  return { total: items.length, returned: page.length, offset, truncated: offset + page.length < items.length, items: page };
}

function analytics(platform: string, a: Args) {
  const report = String(a.report ?? "summary");
  const handle = String(a.handle ?? a.username ?? "preview.me");
  const sample = Number(a.sample ?? 50) || 50;
  const rows = Array.from({ length: sample }, (_, i) => row(platform, i));
  const envelope = (result: unknown) => ({ platform, handle, report, sample_requested: sample, sample: rows.length, fetched_at: Date.now() - 120000, cached: !a.refresh, result });
  switch (report) {
    case "repos":
      return envelope(repos(a));
    case "posts": {
      const key = String(a.sort ?? "created_at") as keyof ReturnType<typeof row>;
      const dir = a.order === "asc" ? 1 : -1;
      return envelope(listing([...rows].sort((x, y) => (Number(x[key] ?? -1) - Number(y[key] ?? -1)) * dir), a));
    }
    case "times": {
      const cells = [];
      for (let d = 0; d < 7; d++) for (const h of [8, 12, 18, 21]) {
        const posts = ((d * 3 + h) % 5) + 1;
        const mean = 80 + ((d * 53 + h * 17) % 400);
        cells.push({ weekday: d, hour: h, posts, engagement: mean * posts, mean, thin: posts < 3 });
      }
      const best = [...cells].filter((c) => !c.thin).sort((x, y) => y.mean - x.mean)[0] ?? null;
      return envelope({ cells, best, thin_below: 3, undated: 0, total: cells.length, timezone: "-03:00" });
    }
    case "media":
      return envelope(listing(["image", "video", "text", "carousel"].map((m, i) => ({ media: m, posts: 18 - i * 4, engagement_mean: 320 - i * 60, engagement_median: 210 - i * 40, views_mean: m === "text" ? null : 9000 - i * 1500 })), a));
    case "hashtags":
      return envelope(listing(["design", "buildinpublic", "tauri", "music"].map((h, i) => ({ hashtag: h, uses: 14 - i * 3, engagement_mean: 260 + i * 45, engagement_median: 200 + i * 30 })), a));
    case "words":
      return { ...envelope(listing(["export", "markdown", "studio", "sketch", "tools", "track"].map((w, i) => ({ word: w, uses: 12 - i, posts: 9 - i })), a)), note: "" };
    case "trend": {
      const weekly = a.unit === "week";
      const items = Array.from({ length: weekly ? 6 : 14 }, (_, i) => {
        const posts = 1 + (i % 4);
        const engagement = posts * (150 + ((i * 71) % 300));
        return { start: new Date((NOW - ((weekly ? 5 : 13) - i) * (weekly ? 7 : 1) * DAY) * 1000).toISOString().slice(0, 10), posts, engagement, mean: Math.round(engagement / posts) };
      });
      return envelope({ ...listing(items, a), unit: weekly ? "week" : "day", timezone: "-03:00" });
    }
    case "outliers": {
      const items = rows.filter((r) => r.outlier).sort((x, y) => y.lift - x.lift);
      return envelope({ metric: platform === "tiktok" ? "views" : "engagement", median: 190, sample: rows.length, confidence: "high", min_recommended: 20, total: items.length, items });
    }
    case "compare": {
      const handles = (a.handles as string[] | undefined) ?? [handle];
      return {
        platform,
        report,
        result: { sample_each: 40, confidence: "high", total: handles.length, accounts: handles.map((h, i) => ({ handle: h, summary: summary(platform, Array.from({ length: 40 }, (_, n) => row(platform, n + i * 7))) })) },
      };
    }
    default:
      return envelope(summary(platform, rows));
  }
}

type PreviewMonitor = { id: number; platform: string; name: string; kind: string; query: string; input: Args; builtin: boolean; created: number; last_checked: number | null; unread: number; total_stored: number };

const monitors: PreviewMonitor[] = [
  { id: 1, platform: "x", name: "Mentions", kind: "mentions", query: "", input: {}, builtin: true, created: Date.now() - 20 * DAY * 1000, last_checked: Date.now() - 3600000, unread: 2, total_stored: 6 },
  { id: 2, platform: "x", name: "Open source downloaders", kind: "search", query: '"open source" (downloader OR yt-dlp) lang:en -filter:replies', input: { query: "", options: { phrase: "open source", any: ["downloader", "yt-dlp"], lang: "en", exclude_replies: true } }, builtin: false, created: Date.now() - 12 * DAY * 1000, last_checked: Date.now() - 7200000, unread: 4, total_stored: 31 },
  { id: 3, platform: "instagram", name: "#tauri", kind: "hashtag", query: "#tauri", input: { query: "tauri", options: {} }, builtin: false, created: Date.now() - 5 * DAY * 1000, last_checked: null, unread: 0, total_stored: 0 },
];
const readHits = new Set<string>();
let nextId = 10;

function hit(platform: string, m: PreviewMonitor, n: number) {
  const id = `${m.id}${1700 + n}`;
  const handle = ["marina.dev", "kenji", "lucas_r", "studio.norte"][n % 4];
  return {
    post: {
      id,
      url: platform === "x" ? `https://x.com/${handle}/status/${id}` : platform === "instagram" ? `https://www.instagram.com/p/C${id}/` : `https://www.tiktok.com/@${handle}/video/${id}`,
      author: { id: String(20 + (n % 4)), handle, name: handle, verified: n % 4 === 0 },
      text: m.builtin ? `@preview.me ${TEXTS[n % TEXTS.length]}` : TEXTS[(n + 2) % TEXTS.length],
      created_at: NOW - n * 5400,
      media: [],
      stats: { likes: 12 + n * 3, replies: n % 4, reposts: platform === "x" ? n % 6 : undefined },
      liked: false,
      reposted: false,
    },
    seen_at: Date.now() - n * 600000,
    read: readHits.has(id) || n >= m.unread,
  };
}

function buildQuery(platform: string, a: Args): string {
  const base = String(a.query ?? "").trim();
  const kind = String(a.kind ?? "search");
  if (platform === "instagram") {
    if (!base) throw 'INVALID_ARGUMENTS: {"message":"query is not a hashtag (letters, digits and _ only)","retryable":false,"next_step":"Fix the argument and call the tool again."}';
    return kind === "profile" ? `@${base.replace(/^@/, "")}` : `#${base.replace(/^#/, "")}`;
  }
  const o = (a.options ?? {}) as Record<string, unknown>;
  const group = (list: unknown, prefix = "") => {
    const items = ((list as string[] | undefined) ?? []).map((x) => `${prefix}${x.replace(/^[@#]/, "")}`);
    return items.length > 1 ? `(${items.join(" OR ")})` : (items[0] ?? "");
  };
  const parts = [base, o.phrase ? `"${o.phrase}"` : "", group(o.any), group(o.hashtags, "#"), group(o.from, "from:"), group(o.to, "to:"), group(o.mentions, "@")].filter(Boolean);
  if (!parts.length) throw 'INVALID_ARGUMENTS: {"message":"a monitor needs something to look for: a query, a phrase, any-of words, hashtags or accounts","retryable":false,"next_step":"Fix the argument and call the tool again."}';
  for (const w of (o.exclude as string[] | undefined) ?? []) parts.push(`-${w}`);
  if (o.lang) parts.push(`lang:${o.lang}`);
  if (o.min_likes) parts.push(`min_faves:${o.min_likes}`);
  if (o.min_reposts) parts.push(`min_retweets:${o.min_reposts}`);
  if (o.exclude_replies) parts.push("-filter:replies");
  if (o.exclude_reposts) parts.push("-filter:retweets");
  return parts.join(" ");
}

function surge(m: PreviewMonitor) {
  return { surge: m.id === 2, today: m.id === 2 ? 14 : 1, trailing_average: m.id === 2 ? 2.43 : 0.86, days_used: 7, multiple: 3, min_today: 5 };
}

function monitorTools(name: string, a: Args): unknown {
  const platform = String(a.platform ?? "");
  const mine = monitors.filter((m) => m.platform === platform);
  const find = () => {
    const m = mine.find((x) => x.id === Number(a.id));
    if (!m) throw `NOT_FOUND: {"message":"no monitor has id ${a.id}","retryable":false,"next_step":"Call social_monitors to list the monitors and their ids."}`;
    return m;
  };
  if (name === "social_monitors") {
    if (a.id == null) {
      return { platform, total: mine.length, returned: mine.length, truncated: false, unread: mine.reduce((n, m) => n + m.unread, 0), monitors: mine, mentions_inbox: { available: platform === "x" }, kinds: KINDS[platform] ?? ["search"] };
    }
    const m = find();
    const all = Array.from({ length: Math.min(m.total_stored, 8) }, (_, i) => hit(platform, m, i));
    const items = a.unread_only ? all.filter((h) => !h.read) : all;
    const days = m.total_stored ? Array.from({ length: 10 }, (_, i) => ({ day: new Date((NOW - (9 - i) * DAY) * 1000).toISOString().slice(0, 10), count: i === 9 && m.id === 2 ? 14 : 1 + ((i * 3) % 4) })) : [];
    return { monitor: m, hits: { total: items.length, returned: items.length, offset: 0, truncated: false, unread_only: !!a.unread_only, items }, days, surge: surge(m) };
  }
  if (name === "social_monitor_check") {
    const targets = a.id == null ? mine : [find()];
    const done = targets.map((m) => {
      const fresh = m.id === 3 ? 5 : 1;
      m.last_checked = Date.now();
      m.unread += fresh;
      m.total_stored += fresh;
      return { id: m.id, name: m.name, kind: m.kind, query: m.query, new: fresh, returned: fresh, truncated: false, items: Array.from({ length: fresh }, (_, i) => hit(platform, m, i).post), total_stored: m.total_stored, unread: m.unread, checked_at: m.last_checked, surge: surge(m) };
    });
    return { platform, checked: done.length, new: done.reduce((n, m) => n + m.new, 0), surging: done.filter((m) => m.surge.surge).map((m) => m.id), monitors: done };
  }
  const op = String(a.op ?? "");
  if (op === "preview") return { platform, kind: a.kind ?? "search", query: buildQuery(platform, a), saved: false };
  if (op === "create") {
    const query = buildQuery(platform, a);
    const m: PreviewMonitor = { id: nextId++, platform, name: String(a.name ?? query).slice(0, 60), kind: String(a.kind ?? "search"), query, input: { query: a.query ?? "", options: a.options ?? {} }, builtin: false, created: Date.now(), last_checked: null, unread: 0, total_stored: 0 };
    monitors.push(m);
    return { saved: true, monitor: m, query };
  }
  const m = find();
  if (op === "update") {
    m.query = buildQuery(platform, a);
    m.kind = String(a.kind ?? m.kind);
    m.name = String(a.name ?? m.name);
    m.input = { query: a.query ?? "", options: a.options ?? {} };
    return { saved: true, monitor: m, query: m.query };
  }
  if (op === "remove") {
    if (a.confirm !== true) return { removed: false, dry_run: true, would_remove: { monitor: m, hits: m.total_stored } };
    monitors.splice(monitors.indexOf(m), 1);
    return { removed: true, id: m.id, hits_removed: m.total_stored };
  }
  if (op === "mark_read") {
    const ids = a.post_ids as string[] | undefined;
    const read = a.read !== false;
    if (!ids) m.unread = read ? 0 : m.unread;
    else {
      for (const id of ids) {
        if (read) readHits.add(id);
        else readHits.delete(id);
      }
      m.unread = Math.max(0, m.unread + (read ? -ids.length : 0));
    }
    return { id: m.id, updated: ids?.length ?? 0, read, unread: m.unread };
  }
  throw "INVALID_ARGUMENTS: op must be create, update, remove, mark_read or preview";
}

export function insightsFixture(name: string, a: Args): unknown {
  if (name.startsWith("social_monitor")) return monitorTools(name, a);
  if (name === "social_draw") {
    const handles = ((a.handles as string[] | undefined) ?? ["marina.dev", "kenji", "lucas_r", "studio.norte"]).map((h) => h.toLowerCase());
    const winners = Number(a.winners ?? 1);
    const place = (h: string, i: number) => ({ handle: h, rank: `${i}`.padStart(64, "0") });
    return { platform: a.platform, seed: String(a.seed ?? "preview-seed"), method: "preview", source: "handles", pool: handles.length, pool_truncated: false, checked: handles.length, requested: { winners, runner_ups: Number(a.runner_ups ?? 0) }, filled: handles.length >= winners, winners: handles.slice(0, winners).map(place), runner_ups: handles.slice(winners, winners + Number(a.runner_ups ?? 0)).map(place), disqualified: [] };
  }
  if (name === "x_engagement") {
    return { post: a.post, kind: a.kind, available: true, count: 3, items: ["marina.dev", "kenji", "lucas_r"].map((h, i) => ({ id: String(20 + i), handle: h, name: h, verified: i === 0, follows_me: i !== 1 })), next_cursor: null };
  }
  const platform = PLATFORM[name.split("_")[0]];
  if (!platform) return undefined;
  const verb = name.slice(name.indexOf("_") + 1);
  if (verb === "analytics") return analytics(platform, a);
  if (verb === "analytics_export") return { path: `/Users/preview/Downloads/omniget-${platform}-preview.me-posts-preview.csv`, rows: Number(a.sample ?? 50), platform };
  return undefined;
}
