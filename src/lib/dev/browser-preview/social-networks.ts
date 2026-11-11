type Args = Record<string, unknown>;
type Net = "reddit" | "pinterest" | "github";

const NOW = Math.floor(Date.now() / 1000);
const NET: Record<string, Net> = { rd: "reddit", pin: "pinterest", gh: "github" };
const signedIn: Record<Net, boolean> = { reddit: true, pinterest: false, github: true };
const github = { has_token: false, username: "preview-dev" as string | null };

const PEOPLE = [
  { id: "21", handle: "marina-dev", name: "Marina" },
  { id: "22", handle: "kenji", name: "Kenji Sato" },
  { id: "23", handle: "lucas_r", name: "Lucas R." },
  { id: "24", handle: "studio-norte", name: "Studio Norte" },
];
const ME = { id: "20", handle: "preview-dev", name: "Preview" };
const SUBS = ["rust", "sveltejs", "selfhosted", "DataHoarder"];
const REPOS = ["tauri-apps/tauri", "sveltejs/svelte", "rust-lang/rust", "yt-dlp/yt-dlp", "BurntSushi/ripgrep", "astral-sh/uv"];
const TITLES = [
  "What is the smallest desktop app you shipped this year?",
  "I moved my media library to plain folders and never looked back",
  "Show: a tiny tool that turns a repo into one prompt file",
  "Why does every scheduler charge per account?",
  "Weekly thread: what are you building?",
];

function art(seed: number, w = 480, h = 640): string {
  const hue = (seed * 53) % 360;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${w}" height="${h}"><rect width="100%" height="100%" fill="hsl(${hue} 60% 55%)"/><circle cx="${w * 0.35}" cy="${h * 0.6}" r="${w * 0.2}" fill="rgba(255,255,255,.3)"/></svg>`;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}

function author(n: number) {
  const who = PEOPLE[n % PEOPLE.length];
  return { ...who, avatar: art(Number(who.id), 80, 80), verified: false };
}

function page<T>(items: T[], next: string | null) {
  return { count: items.length, items, next_cursor: next };
}

function redditPost(n: number) {
  const sub = SUBS[n % SUBS.length];
  const id = (1000 + n).toString(36);
  return {
    id,
    url: `https://www.reddit.com/r/${sub}/comments/${id}/preview/`,
    author: author(n),
    text: n % 2 ? "Longer text of the post, the way a text post reads on the site." : "",
    created_at: NOW - n * 3100,
    media: n % 3 === 0 ? [{ kind: "image", url: art(n, 640, 400), width: 640, height: 400 }] : [],
    stats: { likes: 240 + n * 61, replies: 12 + n * 3 },
    liked: n % 4 === 0,
    extra: { kind: "link", title: TITLES[n % TITLES.length], subreddit: sub, vote: n % 4 === 0 ? 1 : 0, ...(n % 2 ? {} : { link: "https://blog.example/post", domain: "blog.example" }) },
  };
}

function pin(n: number) {
  const id = String(880000000 + n);
  return {
    id,
    url: `https://www.pinterest.com/pin/${id}/`,
    author: author(n),
    text: ["Desk setup with warm light", "Poster grid study", "Small balcony garden", "Ceramic mug shapes"][n % 4],
    created_at: NOW - n * 86000,
    media: [{ kind: "image", url: art(n), thumb: art(n), width: 480, height: 640 }],
    stats: { saves: 300 + n * 45, replies: n % 5 },
    extra: { kind: "pin", title: ["Warm desk", "Poster grid", "Balcony garden", "Mug shapes"][n % 4], board: { id: "55", name: "Ideas", url: "https://www.pinterest.com/preview-dev/ideas/" } },
  };
}

function repo(n: number) {
  const full = REPOS[n % REPOS.length];
  const owner = full.split("/")[0];
  return {
    id: full,
    url: `https://github.com/${full}`,
    author: { id: String(300 + n), handle: owner, name: "", avatar: art(300 + n, 80, 80), verified: false },
    text: "A short description of what this repository does and who it is for.",
    created_at: NOW - n * 5400,
    media: [],
    stats: { likes: 81000 - n * 9100, reposts: 2400 - n * 300, replies: 120 + n },
    extra: { kind: "repo", title: full, language: ["Rust", "TypeScript", "Rust", "Python", "Rust", "Rust"][n % 6], stars_gained: 420 - n * 50, period: "today" },
  };
}

function event(n: number) {
  const full = REPOS[n % REPOS.length];
  const titles = [`Starred ${full}`, `Pushed to main in ${full}`, `Released v2.${n} in ${full}`, `Opened pull request #${40 + n} in ${full}`];
  const title = titles[n % titles.length];
  return {
    id: String(5100000 + n),
    url: `https://github.com/${full}`,
    author: author(n),
    text: n % 4 === 1 ? "fix: keep the window size on restart\nchore: bump dependencies" : title,
    created_at: NOW - n * 4000,
    media: [],
    stats: {},
    extra: { kind: "event", event: "PushEvent", repo: full, title },
  };
}

function issue(n: number) {
  const full = REPOS[n % REPOS.length];
  return {
    id: `${full}#${100 + n}`,
    url: `https://github.com/${full}/issues/${100 + n}`,
    author: author(n),
    text: "Steps to reproduce, what was expected and what happened instead.",
    created_at: NOW - n * 7000,
    media: [],
    stats: { replies: 3 + n, likes: n },
    extra: { kind: n % 2 ? "pull" : "issue", title: ["Window forgets its size", "Add a dark tray icon", "Docs: fix the install steps"][n % 3], repo: full, number: 100 + n, state: "open" },
  };
}

function profile(p: Net, handle: string) {
  if (p === "reddit" && handle.startsWith("r/")) {
    return { id: `t5_${handle.slice(2)}`, handle, name: handle.slice(2), avatar: art(handle.length, 160, 160), bio: "A community about this topic.", url: `https://www.reddit.com/${handle}/`, verified: false, private: false, followers: 312000, followed_by_me: handle === "r/rust", extra: { kind: "subreddit", active: 1400 } };
  }
  if (p === "pinterest" && handle.includes("/")) {
    return { id: "55", handle, name: handle.split("/")[1], avatar: art(55, 160, 160), bio: "A board of saved ideas.", url: `https://www.pinterest.com/${handle}/`, verified: false, private: false, followers: 120, posts: 84, extra: { kind: "board", owner: handle.split("/")[0] } };
  }
  const who = PEOPLE.find((x) => x.handle === handle) ?? { ...ME, handle: handle || ME.handle };
  const base = p === "reddit" ? "https://www.reddit.com/user/" : p === "pinterest" ? "https://www.pinterest.com/" : "https://github.com/";
  return {
    ...who,
    avatar: art(Number(who.id), 160, 160),
    bio: "Designer and developer. Notes on tools and small experiments.",
    url: `${base}${who.handle}`,
    verified: false,
    private: false,
    followers: 1840,
    following: p === "reddit" ? undefined : 212,
    posts: p === "reddit" ? undefined : 64,
    likes: p === "reddit" ? 15200 : undefined,
    followed_by_me: who.id === "21",
    extra: { kind: "user", ...(p === "github" && who.handle === ME.handle ? { auth: github.has_token ? "token" : "username" } : {}) },
  };
}

function feed(make: (n: number) => unknown, a: Args, offset = 0) {
  const start = Number(a.cursor ?? 0) || 0;
  return page(Array.from({ length: 8 }, (_, i) => make(offset + start + i)), start < 16 ? String(start + 8) : null);
}

function comments(tree: boolean) {
  const texts = ["This is the way.", "Do you have a link?", "Same experience here.", "Thanks for sharing!", "It broke for me on Windows."];
  return page(
    texts.map((text, i) => ({ id: `c${i}`, author: author(i), text, created_at: NOW - i * 800, likes: 9 - i, ...(tree ? { extra: { depth: [0, 1, 2, 0, 1][i] } } : {}) })),
    null,
  );
}

export function networkFixture(name: string, a: Args): unknown {
  const p = NET[name.split("_")[0]];
  if (!p) return undefined;
  if (!signedIn[p]) throw `NOT_LOGGED_IN: sign in to ${p} in OmniGet first`;
  const verb = name.slice(name.indexOf("_") + 1);
  const make = p === "reddit" ? redditPost : p === "pinterest" ? pin : event;
  const people = () => page(PEOPLE.map((x) => profile(p, x.handle)), null);
  switch (verb) {
    case "me":
      return profile(p, ME.handle);
    case "feed":
      return feed(make, a);
    case "trending":
      return { ...page(REPOS.map((_, i) => repo(i)), null), source: "scraped_html" };
    case "saved":
      return feed(make, a, 40);
    case "user":
      return profile(p, String(a.username ?? ME.handle));
    case "user_posts":
      return feed(make, a, 80);
    case "subscriptions":
      return page(SUBS.map((x) => profile("reddit", `r/${x}`)), null);
    case "subreddit":
      return { subreddit: profile("reddit", `r/${String(a.subreddit).replace(/^r\//, "")}`), posts: feed(redditPost, a) };
    case "boards":
      return page(["ideas", "posters", "garden"].map((b) => profile("pinterest", `${ME.handle}/${b}`)), null);
    case "board":
      return { board: profile("pinterest", String(a.board)), pins: feed(pin, a) };
    case "followers":
    case "following":
    case "stargazers":
      return people();
    case "repos":
    case "forks":
      return page(REPOS.map((_, i) => repo(i)), null);
    case "repo":
      return { repo: repo(0), languages: { Rust: 81234, TypeScript: 20311 }, latest_release: null, readme_excerpt: "# Preview\n\nWhat this project does.", starred: null };
    case "releases":
    case "notifications":
      return feed(event, a, 120);
    case "post":
    case "pin": {
      const n = Number(String(a.post).match(/(\d+)/)?.[1] ?? 0) % 50;
      return { post: p === "reddit" ? redditPost(n) : pin(n), comments: comments(p === "reddit") };
    }
    case "issues":
      return a.post || a.number ? { post: issue(Number(String(a.post ?? a.number).match(/(\d+)\D*$/)?.[1] ?? 100) - 100), comments: comments(false) } : feed(issue, a);
    case "search": {
      const kind = String(a.kind ?? "");
      if (kind === "users") return people();
      if (kind === "subreddits") return page(SUBS.map((x) => profile("reddit", `r/${x}`)), null);
      if (kind === "boards") return page(["ideas", "posters"].map((b) => profile("pinterest", `${PEOPLE[0].handle}/${b}`)), null);
      if (p === "github") return kind === "issues" ? feed(issue, a) : page(REPOS.map((_, i) => repo(i)), null);
      return feed(make, a, 200);
    }
    case "vote":
    case "comment":
    case "submit":
    case "subscribe":
    case "save":
    case "follow":
    case "star":
    case "watch":
    case "issue_create":
      if (p === "github" && !github.has_token) throw 'TOKEN_REQUIRED: {"message":"Acting on GitHub needs a GitHub token","retryable":false,"next_step":"Save a token in the GitHub screen"}';
      return { ok: true };
    default:
      return undefined;
  }
}

export function networkStatus() {
  return (Object.keys(signedIn) as Net[]).map((platform) => ({
    platform,
    browser: platform !== "github",
    logged_in: signedIn[platform],
    ...(platform === "github" ? { auth: github.has_token ? "token" : github.username ? "username" : "none" } : {}),
  }));
}

export function networkLogin(platform: string): boolean {
  if (platform !== "reddit" && platform !== "pinterest") return false;
  signedIn[platform] = true;
  return true;
}

export const networkCommands: Record<string, (args: Args) => unknown> = {
  social_github_state: () => ({ ...github }),
  social_github_connect: (args) => {
    if (typeof args.token === "string") github.has_token = args.token.trim() !== "";
    if (typeof args.username === "string") github.username = args.username.trim() || null;
    signedIn.github = github.has_token || github.username !== null;
    return { ...github };
  },
};
