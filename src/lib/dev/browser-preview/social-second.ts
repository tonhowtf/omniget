import { insightsFixture } from "./social-insights";

type Args = Record<string, unknown>;

const NOW = Math.floor(Date.now() / 1000);
const DAY = 86400;
const PLATFORM: Record<string, string> = { x: "x", ig: "instagram", tt: "tiktok" };
const QUICK: [string, Args][] = [
  ["likely_inactive", { max_posts: 5 }],
  ["no_avatar_no_bio", { has_avatar: false, has_bio: false }],
  ["follows_many_few_followers", { min_following: 1000, max_ratio: 0.1 }],
  ["verified", { verified: true }],
  ["private", { private: true }],
];

const saved: Record<string, Map<string, Args>> = {};
const notify: Record<string, { surge: boolean; unfollowers: boolean; monitors: number[] }> = {};
let digestAt: number | null = null;

function filters(p: string) {
  const own = [...(saved[p] ??= new Map([["Big accounts", { min_followers: 10000 } as Args]])).entries()];
  const items = [...QUICK.map(([name, filter]) => ({ name, kind: "quick", about: "", filter })), ...own.map(([name, filter]) => ({ name, kind: "saved", filter, created: NOW - DAY }))];
  return { view: "filters", total: items.length, returned: items.length, truncated: false, saved: own.length, max_saved: 30, items };
}

function counted(key: string, words: string[]) {
  const items = words.map((w, i) => ({ [key]: w, accounts: Math.max(2, 46 - i * 4 - (i % 3)) }));
  return { total: items.length + 40, returned: items.length, truncated: true, items };
}

function interests(p: string) {
  const bios = p !== "instagram";
  return {
    view: "interests",
    account: "preview.me",
    followers: 1284,
    with_bio: bios ? 912 : 0,
    with_bio_share: bios ? 71 : 0,
    thin: !bios,
    words: bios ? counted("word", ["design", "founder", "developer", "música", "building", "open", "source", "coffee", "writer", "photography", "rust", "indie", "studio", "café", "product"]) : counted("word", []),
    hashtags: bios ? counted("hashtag", ["buildinpublic", "design", "indiedev", "tauri"]) : counted("hashtag", []),
    name_words: counted("word", ["studio", "ana", "lucas", "dev"]),
    locations: p === "x" ? { available: true, with_location: 640, ...counted("location", ["São Paulo, Brasil", "Lisboa", "Berlin", "Tokyo", "Remote", "Buenos Aires"]) } : { available: false, with_location: 0, total: 0, returned: 0, truncated: false, items: [], reason: "No location is stored for these followers." },
    with_link: { accounts: bios ? 301 : 0, share_of_bios: bios ? 33 : null },
    verified: { accounts: 58, share: 4.5 },
    synced_at: { followers: NOW - 2 * 3600 },
  };
}

function recommendations(p: string) {
  const prefix = Object.keys(PLATFORM).find((k) => PLATFORM[k] === p) ?? "x";
  const items = [
    { id: "new_unfollowers", message: "3 accounts stopped following you between the last two syncs.", numbers: { unfollowers: 3, since: NOW - 2 * 3600 }, act_with: { tool: `${prefix}_audience`, args: { view: "unfollowers" } } },
    { id: "unread_hits", message: "7 unread posts in your monitors.", numbers: { unread: 7 }, act_with: { tool: "social_monitors", args: { platform: p } } },
    { id: "best_time", message: "Tuesday around 09:00 is your strongest time.", numbers: { weekday: 1, hour: 9, posts_in_cell: 4, cell_median: 412, overall_median: 190, posts: 50 }, act_with: { tool: p === "x" ? "social_schedule" : `${prefix}_analytics`, args: {} } },
    { id: "media_type", message: "Your image posts do better.", numbers: { media: "image", posts: 12, median: 380, others_median: 170, others_posts: 38, lift: 2.24 }, act_with: { tool: `${prefix}_analytics`, args: { report: "media" } } },
    { id: "not_following_back", message: "12 accounts you follow do not follow back.", numbers: { accounts: 12 }, act_with: { tool: `${prefix}_audience`, args: { view: "not_following_back", kept: false }, note: "One approved call per account." } },
  ];
  return {
    total: items.length,
    returned: items.length,
    truncated: false,
    items,
    not_triggered: [
      { id: "frequency_drop", reason: "24 posts in the last 14 days against 22 before is not a drop" },
      { id: "drafts", reason: "the count is 0" },
      { id: "missed_posts", reason: "the count is 0" },
    ],
  };
}

function periods(a: Args) {
  const side = (posts: number, mean: number) => ({ from: NOW - 14 * DAY, to: NOW - 7 * DAY, days: 7, posts, engagement: { available: true, posts, total: Math.round(posts * mean), mean, median: Math.round(mean * 0.8) }, metrics: {} });
  const diff = (x: number | null, y: number | null) => ({ a: x, b: y, diff: x != null && y != null ? Math.round((x - y) * 100) / 100 : null, pct: x != null && y ? Math.round(((x - y) / y) * 10000) / 100 : null });
  return {
    a: { ...side(11, 240), from: NOW - 7 * DAY, to: NOW },
    b: side(8, 198),
    difference: {
      posts: diff(11, 8),
      engagement_total: diff(2640, 1584),
      engagement_mean: diff(240, 198),
      engagement_median: diff(192, 158),
      likes_mean: diff(201, 170),
      replies_mean: diff(9, 11),
      reposts_mean: diff(18, 12),
      saves_mean: diff(null, null),
      views_mean: diff(5200, 4100),
    },
    coverage: { sample: 200, covers_a: true, covers_b: !a.compare_from },
  };
}

function history(a: Args) {
  if (!a.post) {
    return { total: 2, returned: 2, offset: 0, truncated: false, items: [1900000000, 1900000001].map((id) => ({ post_id: String(id), points: 5, first_at: (NOW - 4 * DAY) * 1000, last_at: NOW * 1000 })) };
  }
  const points = [0, 1, 2, 3, 4].map((i) => ({ at: (NOW - (4 - i) * DAY) * 1000, likes: 40 + i * i * 22, replies: 2 + i * 3, reposts: 4 + i * 5, saves: 1 + i * 2, views: 900 + i * i * 480 }));
  const last = points[points.length - 1];
  return {
    post_id: String(a.post),
    account: "preview.me",
    total: points.length,
    returned: points.length,
    truncated: false,
    first_at: points[0].at,
    last_at: last.at,
    change: { likes: last.likes - points[0].likes, replies: last.replies - points[0].replies, reposts: last.reposts - points[0].reposts, saves: last.saves - points[0].saves, views: last.views - points[0].views },
    points,
  };
}

function digest(p: string, a: Args) {
  const since = digestAt ?? NOW - 7 * DAY;
  const first = digestAt == null;
  const marked = a.mark !== false;
  if (marked) digestAt = NOW;
  const person = (h: string) => ({ handle: h, name: h, url: `https://example.invalid/${h}` });
  return {
    platform: p,
    since,
    until: NOW,
    first,
    marked,
    quiet: false,
    followers: { available: true, current: 1284, delta: 5, gained: { total: 8, returned: 3, truncated: true, items: ["marina.dev", "kenji", "studio.norte"].map(person) }, lost: { total: 3, returned: 3, truncated: false, items: ["lucas_r", "old.account", "bye"].map(person) }, synced_at: NOW - 2 * 3600 },
    monitors: { available: true, unread: 7, saved: 3, monitors: [{ id: 1, name: "Mentions", unread: 4 }, { id: 2, name: "tauri", unread: 3 }] },
    top_post: { available: true, posts_in_period: 6, post: { id: "1900000003", url: "https://example.invalid/post", text: "Small tools beat big platforms. Change my mind.", engagement: 1620, created_at: NOW - 2 * DAY } },
    scheduled: p === "x" ? { available: true, due_next_24h: 2, returned: 2, truncated: false, items: [{ id: 1, at: null, text: "Release notes thread" }, { id: 2, at: null, text: "Friday recap" }] } : { available: false, reason: "Scheduling is not built for this network." },
    recommendations: recommendations(p),
  };
}

export function secondFixture(name: string, a: Args): unknown {
  if (name === "social_digest") return digest(String(a.platform), a);
  if (name === "social_notifications") {
    const s = (notify[String(a.platform)] ??= { surge: false, unfollowers: false, monitors: [] });
    if (a.op === "set") {
      if (typeof a.surge === "boolean") s.surge = a.surge;
      if (typeof a.unfollowers === "boolean") s.unfollowers = a.unfollowers;
      if (typeof a.monitor === "number") s.monitors = a.new_hits ? [...new Set([...s.monitors, a.monitor])] : s.monitors.filter((m) => m !== a.monitor);
    }
    return { platform: a.platform, ...s, channels: { app: true, slack: false, email: false } };
  }
  if (name === "social_draw" && a.export) {
    return { ...(insightsFixture(name, a) as Args), csv_path: `/preview/social/exports/${a.platform}-draw-preview.csv` };
  }
  const p = PLATFORM[name.split("_")[0]];
  if (!p) return undefined;
  const verb = name.slice(name.indexOf("_") + 1);
  if (verb === "report") {
    return { path: `/preview/social/reports/omniget-${p}-preview.me-report.html`, markdown_path: `/preview/social/reports/omniget-${p}-preview.me-report.md`, platform: p, handle: "preview.me", sections: { account: true, followers: true, post_performance: true, best_times: true, top_posts: true, hashtags: true, monitors: true }, powerpoint: false };
  }
  if (verb === "audience" && a.view === "filters") return filters(p);
  if (verb === "audience" && a.view === "interests") return interests(p);
  if (verb === "audience_mark" && a.list === "filters") {
    const own = (saved[p] ??= new Map());
    if (a.remove) own.delete(String(a.name));
    else own.set(String(a.name), (a.filter ?? {}) as Args);
    return { list: "filters", name: a.name, removed: !!a.remove, filter: a.filter, total: own.size, filters: filters(p).items };
  }
  if (verb === "analytics" && a.report === "recommendations") return { platform: p, report: a.report, handle: "preview.me", result: recommendations(p) };
  if (verb === "analytics" && a.report === "compare_periods") return { platform: p, handle: "preview.me", report: a.report, sample_requested: 200, sample: 200, fetched_at: NOW * 1000, cached: false, result: periods(a) };
  if (verb === "analytics" && a.report === "post_history") return { platform: p, report: a.report, result: history(a) };
  return undefined;
}
