type Args = Record<string, unknown>;
type Platform = "x" | "instagram" | "tiktok";

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const NOW = Math.floor(Date.now() / 1000);
const PLATFORM: Record<string, Platform> = { x: "x", ig: "instagram", tt: "tiktok" };
const signedIn: Record<Platform, boolean> = { x: true, instagram: true, tiktok: false };

const PEOPLE = [
  { id: "11", handle: "marina.dev", name: "Marina", verified: true },
  { id: "12", handle: "kenji", name: "Kenji Sato", verified: false },
  { id: "13", handle: "lucas_r", name: "Lucas R.", verified: false },
  { id: "14", handle: "studio.norte", name: "Studio Norte", verified: true },
];
const ME = { id: "10", handle: "preview.me", name: "Preview", verified: false };

const TEXTS = [
  "Shipped the new export flow today. Smaller files, same quality.",
  "Anyone else keeping a folder of reference videos per project? Mine is getting out of hand.",
  "Morning light at the studio ☀️",
  "Thread: what I learned moving our docs to plain markdown.",
  "New track is out. Headphones on.",
  "Tried the 30-day sketch challenge. Day 12 and still going.",
];

function art(seed: number, w = 640, h = 640): string {
  const hue = (seed * 47) % 360;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${hue} 70% 62%)"/><stop offset="1" stop-color="hsl(${(hue + 60) % 360} 70% 42%)"/></linearGradient></defs><rect width="100%" height="100%" fill="url(#g)"/><circle cx="${w * 0.7}" cy="${h * 0.35}" r="${w * 0.12}" fill="rgba(255,255,255,.35)"/></svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

function url(p: Platform, handle: string, id: string): string {
  if (p === "x") return `https://x.com/${handle}/status/${id}`;
  if (p === "instagram") return `https://www.instagram.com/p/C${id}/`;
  return `https://www.tiktok.com/@${handle}/video/${id}`;
}

function post(p: Platform, n: number, author = PEOPLE[n % PEOPLE.length]) {
  const id = String(1800000000 + n);
  const media = p === "x" && n % 3 !== 0 ? [] : [{ kind: p === "tiktok" ? "video" : "image", url: art(n), thumb: art(n), width: 640, height: 640 }];
  return {
    id,
    url: url(p, author.handle, id),
    author: { ...author, avatar: art(Number(author.id), 80, 80) },
    text: TEXTS[n % TEXTS.length],
    created_at: NOW - n * 2700,
    media,
    stats: { likes: 120 + n * 37, replies: 4 + n, reposts: p === "x" ? 9 + n : undefined, views: 2400 + n * 900 },
    liked: n % 5 === 0,
    reposted: false,
  };
}

function page<T>(items: T[], next: string | null) {
  return { count: items.length, items, next_cursor: next };
}

function feed(p: Platform, args: Args, offset = 0, author?: (typeof PEOPLE)[number]) {
  const start = Number(args.cursor ?? 0) || 0;
  const items = Array.from({ length: 8 }, (_, i) => post(p, offset + start + i, author));
  return page(items, start < 16 ? String(start + 8) : null);
}

function profile(p: Platform, handle: string) {
  const who = PEOPLE.find((x) => x.handle === handle) ?? (handle === ME.handle ? ME : { id: "19", handle, name: handle, verified: false });
  return {
    ...who,
    avatar: art(Number(who.id), 160, 160),
    bio: "Designer and developer. Notes on tools, music and small experiments.",
    url: url(p, handle, "").replace(/\/(status|p|video)\/?$/, ""),
    private: false,
    followers: 18400,
    following: 312,
    posts: 845,
    likes: p === "tiktok" ? 920000 : undefined,
    followed_by_me: who.id === "11",
  };
}

const comments = (n: number) =>
  Array.from({ length: 5 }, (_, i) => ({
    id: `c${n}${i}`,
    author: { ...PEOPLE[(n + i) % PEOPLE.length], avatar: art(Number(PEOPLE[(n + i) % PEOPLE.length].id), 60, 60) },
    text: ["Love this!", "Where is this?", "Saving for later", "So good 🔥", "Can you share the settings?"][i],
    created_at: NOW - i * 900,
    likes: 3 + i,
  }));

function call(args: Args) {
  const name = String(args.tool ?? "");
  const a = (args.args ?? {}) as Args;
  const p = PLATFORM[name.split("_")[0]];
  if (!p) throw "UNKNOWN_TOOL";
  if (!signedIn[p]) throw `NOT_LOGGED_IN: sign in to ${p} in OmniGet first`;
  const verb = name.slice(name.indexOf("_") + 1);
  switch (verb) {
    case "me":
      return profile(p, ME.handle);
    case "feed":
      return feed(p, a);
    case "mentions":
      return feed(p, a, 40);
    case "bookmarks":
    case "saved":
      return feed(p, a, 80);
    case "profile":
      return profile(p, String(a.handle ?? a.username ?? ""));
    case "user_posts": {
      const handle = String(a.handle ?? a.username ?? "");
      return feed(p, a, 120, PEOPLE.find((x) => x.handle === handle) ?? ME);
    }
    case "post": {
      const n = Number(String(a.post).match(/(\d+)/)?.[1] ?? 0) % 100;
      const base = post(p, n);
      return p === "x"
        ? { post: base, replies: page(Array.from({ length: 4 }, (_, i) => ({ ...post(p, 200 + i), reply_to: base.id })), null) }
        : { post: base, comments: page(comments(n), null) };
    }
    case "comments":
      return page(comments(7), null);
    case "search":
      if (p === "instagram") return { users: PEOPLE.map((x) => profile(p, x.handle)), hashtags: [{ name: String(a.query), posts: 120000 }, { name: `${a.query}art`, posts: 8800 }] };
      if (a.kind === "users") return page(PEOPLE.map((x) => profile(p, x.handle)), null);
      return feed(p, a, 300);
    case "create_post":
      return { ok: true, post: { ...post(p, 999, ME), text: String(a.text ?? ""), created_at: NOW, stats: {} } };
    default:
      return { ok: true };
  }
}

export const socialFixtures: Record<string, (args: Args) => unknown> = {
  social_status: () => ({
    platforms: (["x", "instagram", "tiktok"] as Platform[]).map((platform) => ({ platform, browser: true, logged_in: signedIn[platform] })),
  }),
  social_login: async (args) => {
    const p = String(args.platform) as Platform;
    await sleep(2500);
    signedIn[p] = true;
    return { tabs: [], mode: "window" };
  },
  social_call: async (args) => {
    await sleep(250);
    return call(args);
  },
  social_approval_list: () => [],
  social_approval_answer: () => null,
};

