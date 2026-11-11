type Args = Record<string, unknown>;

const NOW = Math.floor(Date.now() / 1000);
const DAY = 86400;
const NAMES = ["Marina", "Kenji Sato", "Lucas R.", "Studio Norte", "Aiko", "Pedro Lima", "Sofia", "Tomás", "Yara", "Noah", "Elena", "Bruno", "Mei", "Olivia", "Rafa", "Ines", "Dmitri", "Hana", "Caio", "Lea", "Omar", "Nina", "Theo", "Julia"];

type Person = {
  id: string;
  handle: string;
  name: string;
  verified: boolean;
  private: boolean;
  followers?: number;
  following?: number;
  posts?: number;
  bio: string;
  first_seen: number;
};

const synced: Record<string, number> = { x: 2, instagram: 0, tiktok: 2 };
const keep: Record<string, Set<string>> = {};
const hidden: Record<string, Set<string>> = {};

function person(p: string, n: number): Person {
  const name = NAMES[n % NAMES.length];
  const counts = p !== "instagram";
  return {
    id: String(500 + n),
    handle: name.toLowerCase().replace(/[^a-z]+/g, "_").replace(/_$/, "") + (n >= NAMES.length ? n : ""),
    name,
    verified: n % 7 === 0,
    private: n % 9 === 4,
    followers: counts ? 40 + ((n * 977) % 9000) : undefined,
    following: counts ? 30 + ((n * 613) % 1500) : undefined,
    posts: counts ? (n * 31) % 400 : undefined,
    bio: counts ? ["Designer and developer.", "Music, coffee, small tools. ana.dev", "", "Photos from the road."][n % 4] : "",
    first_seen: NOW - (n % 5 === 0 ? 3 : 40) * DAY,
  };
}

function url(p: string, handle: string): string {
  if (p === "x") return `https://x.com/${handle}`;
  return p === "instagram" ? `https://www.instagram.com/${handle}/` : `https://www.tiktok.com/@${handle}`;
}

function lists(p: string) {
  const followers = Array.from({ length: 18 }, (_, i) => person(p, i));
  const following = Array.from({ length: 16 }, (_, i) => person(p, i + 8));
  return { followers, following };
}

function rows(p: string, view: string, a: Args) {
  const { followers, following } = lists(p);
  const fo = new Set(followers.map((x) => x.id));
  const fg = new Set(following.map((x) => x.id));
  const k = (keep[p] ??= new Set(["510"]));
  const h = (hidden[p] ??= new Set());
  const window: [number, number] = [NOW - 3 * DAY, NOW - 2 * 3600];
  const deco = (x: Person, extra: Args = {}) => ({
    ...x,
    avatar: undefined,
    url: url(p, x.handle),
    ratio: x.followers != null && x.following ? Math.round((x.followers / x.following) * 100) / 100 : undefined,
    follows_me: fo.has(x.id),
    followed_by_me: fg.has(x.id),
    kept: k.has(x.id),
    hidden: h.has(x.id),
    ...extra,
  });
  let out: ReturnType<typeof deco>[];
  switch (view) {
    case "followers":
      out = followers.map((x) => deco(x));
      break;
    case "following":
      out = following.map((x) => deco(x));
      break;
    case "not_following_back":
      out = following.filter((x) => !fo.has(x.id)).map((x) => deco(x));
      break;
    case "fans":
      out = followers.filter((x) => !fg.has(x.id)).map((x) => deco(x));
      break;
    case "mutuals":
      out = followers.filter((x) => fg.has(x.id)).map((x) => deco(x));
      break;
    case "unfollowers":
      out = [30, 31, 32].map((n, i) => deco(person(p, n), { event: "lost", between: window, status: i === 1 ? "gone" : "unfollowed", checked: true, current: false }));
      break;
    case "new_followers":
      out = followers.slice(0, 4).map((x) => deco(x, { event: "gained", between: window, current: true }));
      break;
    case "inactive":
      out = following.slice(10, 13).map((x, i) => deco(x, i === 2 ? { activity: "no_posts" } : { activity: "inactive", last_post: NOW - (120 + i * 200) * DAY }));
      break;
    case "kept":
      out = [...following, ...followers].filter((x, i, all) => k.has(x.id) && all.findIndex((y) => y.id === x.id) === i).map((x) => deco(x));
      break;
    case "hidden":
      out = [...following, ...followers].filter((x, i, all) => h.has(x.id) && all.findIndex((y) => y.id === x.id) === i).map((x) => deco(x));
      break;
    default:
      out = [];
  }
  if (view !== "hidden") out = out.filter((x) => !x.hidden);
  const q = String(a.q ?? "").toLowerCase();
  if (q) out = out.filter((x) => `${x.handle} ${x.name} ${x.bio}`.toLowerCase().includes(q));
  if (a.verified != null) out = out.filter((x) => x.verified === a.verified);
  if (a.private != null) out = out.filter((x) => x.private === a.private);
  if (a.min_followers != null) out = out.filter((x) => (x.followers ?? -1) >= Number(a.min_followers));
  if (a.sort === "followers") out.sort((x, y) => (y.followers ?? -1) - (x.followers ?? -1));
  if (a.sort === "handle") out.sort((x, y) => x.handle.localeCompare(y.handle));
  return out;
}

const VIEWS = ["followers", "following", "not_following_back", "fans", "mutuals", "new_followers", "unfollowers", "inactive", "kept", "hidden"];

function listStates(p: string) {
  const state = synced[p] ? "complete" : "pending";
  const at = synced[p] ? NOW - 2 * 3600 : null;
  return synced[p]
    ? [
        { kind: "followers", state, fetched: 18, expected: 18, started: NOW - 2 * 3600 - 60, finished: at },
        { kind: "following", state, fetched: 16, expected: 16, started: NOW - 2 * 3600 - 60, finished: at },
      ]
    : [];
}

function day(offset: number): string {
  return new Date((NOW - offset * DAY) * 1000).toISOString().slice(0, 10);
}

export function audienceFixture(p: string, verb: string, a: Args): unknown {
  const at = synced[p] ? NOW - 2 * 3600 : null;
  const synced_at = { followers: at, following: at };
  if (verb === "followers" || verb === "following") {
    return { count: 18, items: lists(p)[verb].map((x) => ({ ...x, url: url(p, x.handle) })), next_cursor: null };
  }
  if (verb === "audience_sync") {
    synced[p] += 1;
    return { account: "preview.me", complete: true, stopped: "done", pages: 2, fetched: 34, total_expected: 34, lists: listStates(p), changes: { gained: 4, lost: 3 }, first_snapshot: synced[p] === 1, continue_with: null, retry_after_seconds: null };
  }
  if (verb === "audience_mark") {
    const set = ((a.list === "keep" ? keep : hidden)[p] ??= new Set());
    const all = [...lists(p).followers, ...lists(p).following, person(p, 30), person(p, 31), person(p, 32)];
    for (const hnd of (a.handles as string[]) ?? []) {
      const id = all.find((x) => x.handle === hnd)?.id;
      if (id) a.remove ? set.delete(id) : set.add(id);
    }
    return { list: a.list, changed: 1, removed: !!a.remove, not_found: [], total: set.size };
  }
  if (verb === "audience_import") {
    synced[p] ||= 1;
    const side = (n: number, source: string) => ({ fetched: n, expected: n, source, at: NOW });
    return { account: "preview.me", files: [], imported: { followers: { accounts: 18, with_date: 18 }, following: { accounts: 16, with_date: 16 } }, sources: { followers: "site", following: "export" }, completeness: { followers: side(18, "site"), following: side(16, "export") }, warning: null, not_following_back: rows(p, "not_following_back", {}).length };
  }
  if (verb === "audience_export") {
    return { path: `/preview/social/exports/${p}-preview.me-${a.view}.csv`, rows: rows(p, String(a.view), a).length, view: a.view, truncated: false };
  }
  if (verb !== "audience") return undefined;
  const view = String(a.view ?? "");
  if (view === "summary") {
    const counts = Object.fromEntries(VIEWS.map((v) => [v, synced[p] ? (synced[p] < 2 && (v === "unfollowers" || v === "new_followers") ? 0 : rows(p, v, {}).length) : 0]));
    return { view, account: "preview.me", counts: { ...counts, inactive_unknown: synced[p] ? 9 : 0 }, history: { snapshots: synced[p], started: synced[p] ? NOW - 20 * DAY : null, too_short: synced[p] < 2 }, lists: listStates(p), synced_at, completeness: synced[p] ? { followers: { fetched: 18, expected: 18, source: "site", at }, following: { fetched: 16, expected: p === "instagram" && synced[p] < 2 ? 19 : 16, source: "site", at } } : undefined };
  }
  if (view === "trend") {
    const days = synced[p] < 2 ? [] : Array.from({ length: 12 }, (_, i) => ({ day: day(11 - i), followers: 1180 + i * 9 - (i % 4) * 6, following: i % 3 === 2 ? null : 300 + i, ratio: i % 3 === 2 ? null : 3.9, gained: (i * 5) % 9, lost: (i * 3) % 5 }));
    return { view, account: "preview.me", days, total: days.length, too_short: synced[p] < 2, synced_at, retention: { buckets: [["under_7d", 14], ["7_30d", 31], ["30_90d", 22], ["90_365d", 0], ["over_365d", 0]].map(([bucket, count]) => ({ bucket, count })), since_before_history: 1113, history_started: NOW - 20 * DAY, history_days: 20 } };
  }
  const all = synced[p] && !(synced[p] < 2 && (view === "unfollowers" || view === "new_followers")) ? rows(p, view, a) : [];
  const offset = Number(a.offset ?? 0);
  const items = all.slice(offset, offset + Number(a.limit ?? 50));
  return {
    view,
    account: "preview.me",
    total: all.length,
    returned: items.length,
    offset,
    truncated: offset + items.length < all.length,
    unavailable_fields: p === "instagram" ? ["followers", "following", "ratio", "posts", "bio"] : [],
    items,
    empty_reason: all.length ? undefined : !synced[p] ? "not_synced" : synced[p] < 2 && (view === "unfollowers" || view === "new_followers") ? "history_too_short" : "no_matches",
    unknown: view === "inactive" ? 9 : undefined,
    synced_at,
  };
}
